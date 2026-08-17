use std::{
    fs,
    net::TcpListener,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

use crate::{
    electron_wco,
    injector::{read_browser_identity, window_controls_overlay_visible, InjectorEngine},
    managed_launch::{
        snapshot_executable_processes, Observation, ProcessRecord, WatcherAction, MSG_AUTO_APPLIED,
        MSG_DEBUG_TIMEOUT, MSG_EXISTING, MSG_SUSPENDED, MSG_TAKING_OVER, MSG_WAITING,
    },
    models::RuntimeStatus,
    payload::ActivePayload,
    plugin,
    settings::write_json_transaction,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PREFERRED_DEBUG_PORT: u16 = 9227;
const RUNTIME_SCHEMA_VERSION: u8 = 2;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MulticaInstall {
    package_root: String,
    executable: String,
    version: String,
    package_full_name: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeState {
    schema_version: u8,
    port: u16,
    browser_id: String,
    wco_enabled: bool,
    package_full_name: String,
    executable: String,
    created_at: String,
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn encoded_powershell(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    STANDARD.encode(bytes)
}

fn run_powershell(script: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded_powershell(script),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| error.to_string())?;
    if child
        .wait_timeout(timeout)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err("PowerShell 操作超时。".to_string());
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            if stdout.is_empty() {
                "PowerShell 操作失败。".to_string()
            } else {
                stdout
            }
        } else {
            stderr
        })
    }
}

fn normalized_path(path: &str) -> String {
    Path::new(path)
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn candidate_executables() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        candidates.push(
            local
                .join("Programs")
                .join("@multicadesktop")
                .join("Multica.exe"),
        );
        candidates.push(local.join("Programs").join("Multica").join("Multica.exe"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(
            PathBuf::from(program_files)
                .join("Multica")
                .join("Multica.exe"),
        );
    }
    candidates
}

fn read_version(executable: &Path) -> String {
    let version_file = executable
        .parent()
        .map(|parent| parent.join("version"))
        .filter(|path| path.is_file());
    if let Some(path) = version_file {
        if let Ok(content) = fs::read_to_string(path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() && trimmed.len() <= 64 {
                return trimmed.to_string();
            }
        }
    }
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$info = [Diagnostics.FileVersionInfo]::GetVersionInfo({})
$version = "$($info.ProductVersion)".Trim()
if (-not $version) {{ $version = "$($info.FileVersion)".Trim() }}
if (-not $version) {{ throw '无法读取 Multica 版本。' }}
$version
"#,
        powershell_quote(&executable.to_string_lossy())
    );
    run_powershell(&script, Duration::from_secs(15)).unwrap_or_else(|_| "unknown".to_string())
}

fn discover_multica() -> Result<MulticaInstall, String> {
    for executable in candidate_executables() {
        if !executable.is_file() {
            continue;
        }
        let package_root = executable
            .parent()
            .ok_or_else(|| "Multica 安装路径无效。".to_string())?
            .to_string_lossy()
            .to_string();
        let version = read_version(&executable);
        let package_full_name = format!("Multica.Desktop@{version}");
        let install = MulticaInstall {
            package_root,
            executable: executable.to_string_lossy().to_string(),
            version,
            package_full_name,
        };
        let root = format!(
            "{}\\",
            normalized_path(&install.package_root).trim_end_matches('\\')
        );
        if !normalized_path(&install.executable).starts_with(&root) {
            continue;
        }
        if !normalized_path(&install.executable).ends_with("\\multica.exe") {
            continue;
        }
        return Ok(install);
    }
    Err("未找到官方 Multica 桌面应用（预期路径：%LOCALAPPDATA%\\Programs\\@multicadesktop\\Multica.exe）。"
        .to_string())
}

fn process_records_for(install: &MulticaInstall) -> Result<Vec<ProcessRecord>, String> {
    snapshot_executable_processes(&install.executable)
}

fn process_ids_for(install: &MulticaInstall) -> Result<Vec<u32>, String> {
    Ok(process_records_for(install)?
        .into_iter()
        .map(|process| process.key.pid)
        .collect())
}

fn debug_ports_for(install: &MulticaInstall) -> Result<Vec<u16>, String> {
    let mut ports = process_records_for(install)?
        .into_iter()
        .filter_map(|process| process.debug_port)
        .collect::<Vec<_>>();
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

fn is_disconnected_error(error: &str) -> bool {
    error.contains("CDP 会话已关闭")
        || error.contains("CDP 浏览器身份已变化")
        || error.contains("CDP 命令超时")
        || error.contains("CDP 连接超时")
        || error.contains("CDP 返回 HTTP")
}

fn session_has_wco(port: u16, browser_id: &str) -> bool {
    window_controls_overlay_visible(port, browser_id).unwrap_or(false)
}

fn stop_verified_multica(install: &MulticaInstall) -> Result<(), String> {
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$target = {}
$processes = @(Get-CimInstance Win32_Process -Filter "Name='Multica.exe'" | Where-Object {{
  $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($target, [StringComparison]::OrdinalIgnoreCase)
}})
foreach ($item in $processes) {{ Stop-Process -Id ([int]$item.ProcessId) -Force -ErrorAction SilentlyContinue }}
"#,
        powershell_quote(&normalized_path(&install.executable))
    );
    run_powershell(&script, Duration::from_secs(30))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !process_ids_for(install)?.is_empty() {
        if Instant::now() >= deadline {
            return Err("Multica 未能在 15 秒内完全退出。".to_string());
        }
        thread::sleep(Duration::from_millis(300));
    }
    Ok(())
}

fn launch_multica(install: &MulticaInstall, arguments: &[String]) -> Result<(), String> {
    let executable = PathBuf::from(&install.executable);
    if !executable.is_file() {
        return Err("Multica 可执行文件不存在。".to_string());
    }
    Command::new(&executable)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动 Multica 失败：{error}"))?;
    Ok(())
}

fn select_port(preferred: u16) -> Result<u16, String> {
    for port in preferred..=preferred.saturating_add(100) {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err("无法为 Multica 分配本机调试端口。".to_string())
}

pub struct ManagedProbe {
    pub processes: Vec<ProcessRecord>,
    pub has_ready_debug_session: bool,
    pub engine_alive: bool,
}

impl ManagedProbe {
    pub fn observation(&self) -> Observation {
        Observation {
            processes: self.processes.clone(),
            has_ready_debug_session: self.has_ready_debug_session,
            engine_alive: self.engine_alive,
            now: std::time::Instant::now(),
        }
    }
}

pub struct MulticaController {
    state_path: PathBuf,
    engine: Option<InjectorEngine>,
    state: Option<RuntimeState>,
    status: RuntimeStatus,
    install_cache: Option<MulticaInstall>,
    watcher_paused: bool,
}

impl MulticaController {
    pub fn load(data_directory: &Path) -> Self {
        let state_path = data_directory.join("runtime.json");
        let state = fs::read_to_string(&state_path)
            .ok()
            .and_then(|content| serde_json::from_str::<RuntimeState>(&content).ok())
            .filter(|state| {
                state.schema_version == RUNTIME_SCHEMA_VERSION
                    && state.wco_enabled
                    && !state.browser_id.is_empty()
            });
        Self {
            state_path,
            engine: None,
            state,
            status: RuntimeStatus::default(),
            install_cache: None,
            watcher_paused: false,
        }
    }

    pub fn status(&self) -> RuntimeStatus {
        let mut status = self.status.clone();
        if let Some(engine) = &self.engine {
            status.active_targets = engine.active_targets();
        }
        status
    }

    pub fn watcher_paused(&self) -> bool {
        self.watcher_paused
    }

    pub fn set_watcher_paused(&mut self, paused: bool) {
        self.watcher_paused = paused;
    }

    pub fn set_managed_status(&mut self, phase: &str, message: &str) {
        self.status.phase = phase.to_string();
        self.status.message = message.to_string();
        if phase != "error" {
            self.status.last_error = None;
        }
    }

    pub fn set_managed_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.status.phase = "error".to_string();
        self.status.message = error.clone();
        self.status.last_error = Some(error);
    }

    fn cached_install(&mut self) -> Result<MulticaInstall, String> {
        if let Some(install) = &self.install_cache {
            if Path::new(&install.executable).is_file() {
                return Ok(install.clone());
            }
            self.install_cache = None;
        }
        let install = discover_multica()?;
        self.install_cache = Some(install.clone());
        Ok(install)
    }

    fn saved_session_is_live(&self, install: &MulticaInstall) -> bool {
        let Some(state) = &self.state else {
            return false;
        };
        state.package_full_name == install.package_full_name
            && state.wco_enabled
            && normalized_path(&state.executable) == normalized_path(&install.executable)
            && read_browser_identity(state.port).ok().as_deref() == Some(state.browser_id.as_str())
            && session_has_wco(state.port, &state.browser_id)
    }

    fn live_debug_session_ready(processes: &[ProcessRecord]) -> bool {
        for process in processes {
            let Some(port) = process.debug_port else {
                continue;
            };
            let Ok(browser_id) = read_browser_identity(port) else {
                continue;
            };
            if session_has_wco(port, &browser_id) {
                return true;
            }
        }
        false
    }

    pub fn probe_managed(&mut self) -> Result<ManagedProbe, String> {
        let install = self.cached_install()?;
        let processes = process_records_for(&install)?;
        let engine_alive = self
            .engine
            .as_ref()
            .is_some_and(InjectorEngine::is_connected);
        let has_ready_debug_session = engine_alive
            || self.saved_session_is_live(&install)
            || Self::live_debug_session_ready(&processes);
        self.status.multica_version = Some(install.version);
        Ok(ManagedProbe {
            processes,
            has_ready_debug_session,
            engine_alive,
        })
    }

    pub fn apply_watcher_action_status(&mut self, action: WatcherAction) {
        match action {
            WatcherAction::Wait => {
                if !matches!(self.status.phase.as_str(), "active" | "paused") {
                    self.set_managed_status("idle", MSG_WAITING);
                }
            }
            WatcherAction::ReleaseAndWait => {
                self.set_managed_status("idle", MSG_WAITING);
            }
            WatcherAction::ReportExistingUnmanaged => {
                self.set_managed_status("idle", MSG_EXISTING);
            }
            WatcherAction::WaitForDebugPort | WatcherAction::Takeover | WatcherAction::Attach => {
                self.set_managed_status("starting", MSG_TAKING_OVER);
            }
            WatcherAction::ReportDebugTimeout => {
                self.set_managed_error(MSG_DEBUG_TIMEOUT);
            }
            WatcherAction::KeepActive | WatcherAction::Suspend => {}
        }
    }

    pub fn auto_takeover(&mut self, payload: ActivePayload) -> Result<RuntimeStatus, String> {
        self.set_managed_status("starting", MSG_TAKING_OVER);
        self.apply(payload, true)?;
        self.set_managed_status("active", MSG_AUTO_APPLIED);
        Ok(self.status())
    }

    pub fn attach_without_restart(
        &mut self,
        payload: ActivePayload,
    ) -> Result<RuntimeStatus, String> {
        self.set_managed_status("starting", MSG_TAKING_OVER);
        self.apply(payload, false)?;
        if self.status.phase == "active" {
            self.set_managed_status("active", MSG_AUTO_APPLIED);
        }
        Ok(self.status())
    }

    pub fn release_stale_session(&mut self) -> Result<RuntimeStatus, String> {
        self.engine = None;
        self.write_state(None)?;
        self.status.active_targets = 0;
        self.set_managed_status("idle", MSG_WAITING);
        Ok(self.status())
    }

    fn drop_disconnected_engine(&mut self) {
        if self
            .engine
            .as_ref()
            .is_some_and(|engine| !engine.is_connected())
        {
            self.engine = None;
        }
    }

    fn write_state(&mut self, state: Option<RuntimeState>) -> Result<(), String> {
        self.state = state;
        match &self.state {
            Some(state) => write_json_transaction(&self.state_path, state),
            None => match fs::remove_file(&self.state_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            },
        }
    }

    fn try_attach_saved(&mut self, install: &MulticaInstall) -> bool {
        let Some(state) = &self.state else {
            return false;
        };
        if state.package_full_name != install.package_full_name
            || !state.wco_enabled
            || normalized_path(&state.executable) != normalized_path(&install.executable)
            || read_browser_identity(state.port).ok().as_deref() != Some(&state.browser_id)
            || !window_controls_overlay_visible(state.port, &state.browser_id).unwrap_or(false)
        {
            return false;
        }
        self.engine = Some(InjectorEngine::new(state.port, state.browser_id.clone()));
        true
    }

    pub fn reconnect_saved(&mut self, payload: ActivePayload) -> Result<bool, String> {
        if self.state.is_none() {
            return Ok(false);
        }
        let install = discover_multica()?;
        if !self.try_attach_saved(&install) {
            self.write_state(None)?;
            self.status = RuntimeStatus::default();
            return Ok(false);
        }
        let result = self
            .engine
            .as_mut()
            .expect("engine set after saved session validation")
            .start(payload);
        match result {
            Ok(()) => {
                self.status.phase = "active".to_string();
                self.status.message = "已自动恢复背景会话".to_string();
                self.status.multica_version = Some(install.version);
                self.status.last_error = None;
                Ok(true)
            }
            Err(error) => {
                self.engine = None;
                self.status.phase = "error".to_string();
                self.status.message = error.clone();
                self.status.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn apply(
        &mut self,
        payload: ActivePayload,
        restart_existing: bool,
    ) -> Result<RuntimeStatus, String> {
        self.watcher_paused = false;
        self.status.phase = "starting".to_string();
        self.status.message = "正在连接 Multica".to_string();
        self.status.last_error = None;
        let result: Result<RuntimeStatus, String> = (|| {
            let install = self.cached_install()?;
            self.drop_disconnected_engine();
            if let Some(engine) = &self.engine {
                match engine.update(payload.clone()) {
                    Ok(()) => {
                        self.status.phase = "active".to_string();
                        self.status.message = "背景已实时应用".to_string();
                        self.status.multica_version = Some(install.version);
                        return Ok(self.status());
                    }
                    Err(error) if is_disconnected_error(&error) => {
                        self.engine = None;
                    }
                    Err(error) => return Err(error),
                }
            }
            if self.try_attach_saved(&install) {
                self.engine
                    .as_mut()
                    .expect("engine set after attach")
                    .start(payload)?;
                self.status.phase = "active".to_string();
                self.status.message = "已重新连接背景会话".to_string();
                self.status.multica_version = Some(install.version);
                return Ok(self.status());
            }
            for port in debug_ports_for(&install)? {
                let Ok(browser_id) = read_browser_identity(port) else {
                    continue;
                };
                if !window_controls_overlay_visible(port, &browser_id).unwrap_or(false) {
                    continue;
                }
                self.write_state(Some(RuntimeState {
                    schema_version: RUNTIME_SCHEMA_VERSION,
                    port,
                    browser_id: browser_id.clone(),
                    wco_enabled: true,
                    package_full_name: install.package_full_name.clone(),
                    executable: install.executable.clone(),
                    created_at: Utc::now().to_rfc3339(),
                }))?;
                let mut engine = InjectorEngine::new(port, browser_id);
                engine.start(payload.clone())?;
                self.engine = Some(engine);
                self.status.phase = "active".to_string();
                self.status.message = "已重新连接背景会话".to_string();
                self.status.multica_version = Some(install.version);
                return Ok(self.status());
            }
            let running = process_ids_for(&install)?;
            if !running.is_empty() && !restart_existing {
                return Err("Multica 需要重启一次以启用背景。".to_string());
            }
            if !running.is_empty() {
                stop_verified_multica(&install)?;
            }
            let port = select_port(PREFERRED_DEBUG_PORT)?;
            let browser_id = match electron_wco::launch_with_transparent_wco(
                Path::new(&install.executable),
                port,
            ) {
                Ok(browser_id) => browser_id,
                Err(error) => {
                    let recovery =
                        stop_verified_multica(&install).and_then(|_| launch_multica(&install, &[]));
                    return Err(match recovery {
                        Ok(()) => format!(
                            "透明原生按钮标题栏启动失败，已恢复官方 Multica：{error}"
                        ),
                        Err(recovery_error) => format!(
                            "透明原生按钮标题栏启动失败：{error}；恢复官方 Multica 也失败：{recovery_error}"
                        ),
                    });
                }
            };
            self.write_state(Some(RuntimeState {
                schema_version: RUNTIME_SCHEMA_VERSION,
                port,
                browser_id: browser_id.clone(),
                wco_enabled: true,
                package_full_name: install.package_full_name,
                executable: install.executable,
                created_at: Utc::now().to_rfc3339(),
            }))?;
            let mut engine = InjectorEngine::new(port, browser_id);
            engine.start(payload)?;
            self.engine = Some(engine);
            self.status.phase = "active".to_string();
            self.status.message = "背景已应用".to_string();
            self.status.multica_version = Some(install.version);
            Ok(self.status())
        })();
        if let Err(error) = &result {
            self.status.phase = if error.contains("需要重启一次") {
                "idle".to_string()
            } else {
                "error".to_string()
            };
            self.status.message = error.clone();
            self.status.last_error = Some(error.clone());
        }
        result
    }

    pub fn pause(&mut self) -> Result<RuntimeStatus, String> {
        self.watcher_paused = true;
        if let Some(engine) = &self.engine {
            engine.pause()?;
        }
        self.status.phase = "paused".to_string();
        self.status.message = if plugin::is_plugin_mode() {
            MSG_SUSPENDED.to_string()
        } else {
            "背景已暂停".to_string()
        };
        self.status.last_error = None;
        Ok(self.status())
    }

    pub fn restore(&mut self) -> Result<RuntimeStatus, String> {
        self.watcher_paused = true;
        self.status.phase = "restoring".to_string();
        self.status.message = "正在恢复官方外观".to_string();
        self.status.last_error = None;
        let result: Result<RuntimeStatus, String> = (|| {
            if let Some(mut engine) = self.engine.take() {
                engine.stop()?;
            }
            let install = self.cached_install()?;
            if !process_ids_for(&install)?.is_empty() {
                stop_verified_multica(&install)?;
                launch_multica(&install, &[])?;
            }
            self.write_state(None)?;
            self.status.phase = "idle".to_string();
            self.status.message = "已恢复官方外观".to_string();
            self.status.multica_version = Some(install.version);
            self.status.active_targets = 0;
            Ok(self.status())
        })();
        if let Err(error) = &result {
            self.status.phase = "error".to_string();
            self.status.message = error.clone();
            self.status.last_error = Some(error.clone());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_powershell_literals() {
        assert_eq!(powershell_quote("a'b"), "'a''b'");
    }

    #[test]
    fn selects_an_available_loopback_port() {
        let port = select_port(39_000).expect("available test port");
        assert!((39_000..=39_100).contains(&port));
    }

    #[test]
    fn treats_dead_cdp_errors_as_recoverable() {
        assert!(is_disconnected_error("CDP 会话已关闭。"));
        assert!(is_disconnected_error(
            "CDP 浏览器身份已变化，拒绝继续注入。"
        ));
        assert!(!is_disconnected_error("Multica 渲染页执行背景脚本失败：x"));
    }

    #[test]
    fn wait_and_existing_clear_stale_probe_error() {
        let dir =
            std::env::temp_dir().join(format!("multica-managed-status-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut controller = MulticaController::load(&dir);
        controller.set_managed_error("探测失败");
        controller.apply_watcher_action_status(WatcherAction::Wait);
        let status = controller.status();
        assert_eq!(status.phase, "idle");
        assert_eq!(status.message, MSG_WAITING);
        assert!(status.last_error.is_none());

        controller.set_managed_error("探测失败");
        controller.apply_watcher_action_status(WatcherAction::ReportExistingUnmanaged);
        let status = controller.status();
        assert_eq!(status.phase, "idle");
        assert_eq!(status.message, MSG_EXISTING);
        assert!(status.last_error.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires the official Multica desktop installation"]
    fn discovers_installed_multica_and_reads_processes() {
        let install = discover_multica().expect("discover official Multica");
        assert!(normalized_path(&install.executable).ends_with("\\multica.exe"));
        process_ids_for(&install).expect("query verified Multica processes");
        for port in debug_ports_for(&install).expect("query verified Multica debug ports") {
            read_browser_identity(port).expect("verify Multica browser identity");
        }
    }
}
