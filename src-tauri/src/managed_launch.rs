use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const MSG_WAITING: &str = "已启用，等待 Multica 启动";
pub const MSG_EXISTING: &str = "Multica 已在运行，点立即接管可重启";
pub const MSG_TAKING_OVER: &str = "正在接管 Multica";
pub const MSG_AUTO_APPLIED: &str = "背景已自动应用";
pub const MSG_SUSPENDED: &str = "托管已暂停";
pub const MSG_DEBUG_TIMEOUT: &str = "调试会话未在 45 秒内就绪，请关闭 Multica 后重试";
pub const MSG_BUSY: &str = "正在接管/应用中";

const CONFIRM_TICKS: u8 = 2;
pub const DEBUG_PORT_WAIT: Duration = Duration::from_secs(45);

pub struct BusyGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> BusyGuard<'a> {
    pub fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        if flag.swap(true, Ordering::AcqRel) {
            None
        } else {
            Some(Self { flag })
        }
    }
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessKey {
    pub pid: u32,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRecord {
    pub key: ProcessKey,
    pub has_debug_args: bool,
    pub debug_port: Option<u16>,
}

impl ProcessRecord {
    #[cfg(test)]
    pub fn new(pid: u32, created_at: u64) -> Self {
        Self {
            key: ProcessKey { pid, created_at },
            has_debug_args: false,
            debug_port: None,
        }
    }

    #[cfg(test)]
    pub fn with_debug(mut self, port: Option<u16>) -> Self {
        self.has_debug_args = true;
        self.debug_port = port;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub processes: Vec<ProcessRecord>,
    pub has_ready_debug_session: bool,
    pub engine_alive: bool,
    pub now: Instant,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            processes: Vec::new(),
            has_ready_debug_session: false,
            engine_alive: false,
            now: Instant::now(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatcherAction {
    Wait,
    ReportExistingUnmanaged,
    WaitForDebugPort,
    ReportDebugTimeout,
    Attach,
    Takeover,
    KeepActive,
    ReleaseAndWait,
    Suspend,
}

#[derive(Debug, Default)]
pub struct WatcherState {
    paused: bool,
    baseline: Option<HashSet<ProcessKey>>,
    in_flight: bool,
    session_active: bool,
    pending_ticks: u8,
    debug_wait_started: Option<Instant>,
    debug_stalled: bool,
}

impl WatcherState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn suspend(&mut self) {
        self.paused = true;
        self.in_flight = false;
        self.pending_ticks = 0;
        self.clear_debug_wait();
    }

    pub fn rearm(&mut self) {
        self.paused = false;
    }

    pub fn mark_managed_active(&mut self, processes: &[ProcessKey]) {
        self.paused = false;
        self.in_flight = false;
        self.session_active = true;
        self.pending_ticks = 0;
        self.clear_debug_wait();
        self.baseline = Some(processes.iter().copied().collect());
    }

    pub fn takeover_failed(&mut self, processes: &[ProcessKey]) {
        self.in_flight = false;
        self.session_active = false;
        self.pending_ticks = 0;
        self.clear_debug_wait();
        self.baseline = Some(processes.iter().copied().collect());
    }

    pub fn sync_to_current(&mut self, observation: &Observation) -> WatcherAction {
        self.in_flight = false;
        self.session_active = false;
        self.pending_ticks = 0;
        self.clear_debug_wait();
        // Armed watcher: empty baseline, not None (None = preexisting).
        self.baseline = Some(HashSet::new());
        self.decide(observation)
    }

    fn clear_debug_wait(&mut self) {
        self.debug_wait_started = None;
        self.debug_stalled = false;
    }

    fn wait_for_debug_port(&mut self, now: Instant, keys: HashSet<ProcessKey>) -> WatcherAction {
        let started = *self.debug_wait_started.get_or_insert(now);
        if now.saturating_duration_since(started) >= DEBUG_PORT_WAIT {
            self.in_flight = false;
            self.session_active = false;
            self.debug_stalled = true;
            self.pending_ticks = 0;
            self.baseline = Some(keys);
            return WatcherAction::ReportDebugTimeout;
        }
        WatcherAction::WaitForDebugPort
    }

    pub fn decide(&mut self, observation: &Observation) -> WatcherAction {
        if self.paused {
            return WatcherAction::Suspend;
        }

        let keys = keys_of(&observation.processes);
        let has_debug_args = observation
            .processes
            .iter()
            .any(|process| process.has_debug_args);

        if keys.is_empty() {
            let cleanup = self.session_active || self.in_flight || self.debug_stalled;
            self.in_flight = false;
            self.session_active = false;
            self.pending_ticks = 0;
            self.clear_debug_wait();
            self.baseline = Some(HashSet::new());
            return if cleanup {
                WatcherAction::ReleaseAndWait
            } else {
                WatcherAction::Wait
            };
        }

        if observation.engine_alive && self.session_active {
            self.in_flight = false;
            self.pending_ticks = 0;
            self.clear_debug_wait();
            match &mut self.baseline {
                Some(baseline) => baseline.extend(keys),
                None => self.baseline = Some(keys),
            }
            return WatcherAction::KeepActive;
        }

        if observation.has_ready_debug_session || observation.engine_alive {
            self.in_flight = false;
            self.session_active = true;
            self.pending_ticks = 0;
            self.clear_debug_wait();
            self.baseline = Some(keys);
            return WatcherAction::Attach;
        }

        if self.debug_stalled {
            return WatcherAction::ReportDebugTimeout;
        }

        if self.in_flight || has_debug_args {
            return self.wait_for_debug_port(observation.now, keys);
        }

        if self.session_active {
            self.session_active = false;
            self.pending_ticks = 0;
            self.baseline = Some(keys);
            return WatcherAction::ReleaseAndWait;
        }

        if self.baseline.is_none() {
            self.baseline = Some(keys);
            return WatcherAction::ReportExistingUnmanaged;
        }

        let baseline_alive = keys.iter().any(|key| {
            self.baseline
                .as_ref()
                .is_some_and(|baseline| baseline.contains(key))
        });
        if baseline_alive {
            self.pending_ticks = 0;
            return WatcherAction::ReportExistingUnmanaged;
        }

        self.pending_ticks = self.pending_ticks.saturating_add(1);
        if self.pending_ticks >= CONFIRM_TICKS {
            self.in_flight = true;
            self.pending_ticks = 0;
            return WatcherAction::Takeover;
        }
        WatcherAction::Wait
    }
}

pub fn keys_of(processes: &[ProcessRecord]) -> HashSet<ProcessKey> {
    processes.iter().map(|process| process.key).collect()
}

pub fn process_key_list(processes: &[ProcessRecord]) -> Vec<ProcessKey> {
    processes.iter().map(|process| process.key).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmedLaunch {
    Attach,
    Takeover,
    CancelExited,
    CancelStale,
}

pub fn confirm_launch_after_encode(
    planned: WatcherAction,
    original: &HashSet<ProcessKey>,
    current: &Observation,
) -> ConfirmedLaunch {
    let current_keys = keys_of(&current.processes);
    if current_keys.is_empty() {
        return ConfirmedLaunch::CancelExited;
    }
    if current.engine_alive || current.has_ready_debug_session {
        return ConfirmedLaunch::Attach;
    }
    if planned != WatcherAction::Takeover {
        return ConfirmedLaunch::CancelStale;
    }
    if original.iter().any(|key| current_keys.contains(key)) {
        ConfirmedLaunch::Takeover
    } else {
        ConfirmedLaunch::CancelStale
    }
}

pub fn normalize_executable_path(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

pub fn parse_remote_debugging_port(command_line: &str) -> Option<u16> {
    let marker = "--remote-debugging-port=";
    let rest = command_line.split(marker).nth(1)?;
    let token = rest
        .trim_start_matches('"')
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    token.parse().ok()
}

pub fn snapshot_executable_processes(executable: &str) -> Result<Vec<ProcessRecord>, String> {
    #[cfg(windows)]
    {
        windows_snapshot::snapshot_executable_processes(executable)
    }
    #[cfg(not(windows))]
    {
        let _ = executable;
        Ok(Vec::new())
    }
}

#[cfg(windows)]
mod windows_snapshot {
    use super::{
        normalize_executable_path, parse_remote_debugging_port, ProcessKey, ProcessRecord,
    };
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process_handle: HANDLE,
            information_class: u32,
            process_information: *mut core::ffi::c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    fn exe_file_name(path: &str) -> Option<String> {
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
    }

    fn utf16_to_string(buffer: &[u16]) -> String {
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    fn query_image_path(handle: HANDLE) -> Option<String> {
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
        if ok == 0 || size == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }

    fn query_creation_time(handle: HANDLE) -> Option<u64> {
        let mut creation = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = creation;
        let mut kernel = creation;
        let mut user = creation;
        let ok =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        if ok == 0 {
            return None;
        }
        Some(((creation.dwHighDateTime as u64) << 32) | u64::from(creation.dwLowDateTime))
    }

    fn query_command_line(handle: HANDLE) -> Option<String> {
        let mut needed = 0u32;
        unsafe {
            NtQueryInformationProcess(
                handle,
                PROCESS_COMMAND_LINE_INFORMATION,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        let size = if needed == 0 { 8192 } else { needed.max(16) };
        let mut buffer = vec![0u8; size as usize];
        let mut returned = 0u32;
        let status = unsafe {
            NtQueryInformationProcess(
                handle,
                PROCESS_COMMAND_LINE_INFORMATION,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut returned,
            )
        };
        if status != 0 || buffer.len() < std::mem::size_of::<UnicodeString>() {
            return None;
        }
        let header = unsafe { &*(buffer.as_ptr() as *const UnicodeString) };
        if header.buffer.is_null() || header.length == 0 {
            return None;
        }
        let units = (header.length as usize) / 2;
        let slice = unsafe { std::slice::from_raw_parts(header.buffer, units) };
        Some(String::from_utf16_lossy(slice))
    }

    pub fn snapshot_executable_processes(executable: &str) -> Result<Vec<ProcessRecord>, String> {
        let target = normalize_executable_path(executable);
        let target_name =
            exe_file_name(&target).ok_or_else(|| "目标可执行路径无效。".to_string())?;
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Err("无法创建进程快照。".to_string());
        }
        let _guard = HandleGuard(snapshot);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            cntUsage: 0,
            th32ProcessID: 0,
            th32DefaultHeapID: 0,
            th32ModuleID: 0,
            cntThreads: 0,
            th32ParentProcessID: 0,
            pcPriClassBase: 0,
            dwFlags: 0,
            szExeFile: [0; 260],
        };
        let mut records = Vec::new();
        let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) };
        while ok != 0 {
            let name = utf16_to_string(&entry.szExeFile).to_ascii_lowercase();
            if name == target_name {
                let handle = unsafe {
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID)
                };
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    let process = HandleGuard(handle);
                    if let (Some(image), Some(created_at)) =
                        (query_image_path(process.0), query_creation_time(process.0))
                    {
                        if normalize_executable_path(&image) == target {
                            let command_line = query_command_line(process.0);
                            let debug_port = command_line
                                .as_deref()
                                .and_then(parse_remote_debugging_port);
                            records.push(ProcessRecord {
                                key: ProcessKey {
                                    pid: entry.th32ProcessID,
                                    created_at,
                                },
                                has_debug_args: debug_port.is_some()
                                    || command_line.as_deref().is_some_and(|line| {
                                        line.contains("--remote-debugging-port=")
                                    }),
                                debug_port,
                            });
                        }
                    }
                }
            }
            ok = unsafe { Process32NextW(snapshot, &mut entry) };
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, created_at: u64) -> ProcessRecord {
        ProcessRecord::new(pid, created_at)
    }

    fn unmanaged(processes: Vec<ProcessRecord>) -> Observation {
        Observation {
            processes,
            ..Observation::default()
        }
    }

    fn at(now: Instant, processes: Vec<ProcessRecord>) -> Observation {
        Observation {
            processes,
            now,
            ..Observation::default()
        }
    }

    #[test]
    fn waits_when_no_process() {
        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
    }

    #[test]
    fn does_not_kill_preexisting_processes() {
        let mut watcher = WatcherState::new();
        let first = unmanaged(vec![record(11, 100), record(12, 101)]);
        assert_eq!(
            watcher.decide(&first),
            WatcherAction::ReportExistingUnmanaged
        );
        let with_child = unmanaged(vec![record(11, 100), record(12, 101), record(13, 102)]);
        assert_eq!(
            watcher.decide(&with_child),
            WatcherAction::ReportExistingUnmanaged
        );
    }

    #[test]
    fn takeovers_after_old_process_exits_and_new_process_starts() {
        let mut watcher = WatcherState::new();
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::ReportExistingUnmanaged
        );
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(21, 200)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(21, 200), record(22, 201)])),
            WatcherAction::Takeover
        );
    }

    #[test]
    fn does_not_kill_debug_launch_in_progress() {
        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        let launching = Observation {
            processes: vec![record(31, 300).with_debug(Some(9227))],
            ..Observation::default()
        };
        assert_eq!(watcher.decide(&launching), WatcherAction::WaitForDebugPort);
        assert_eq!(watcher.decide(&launching), WatcherAction::WaitForDebugPort);
    }

    #[test]
    fn attaches_ready_debug_session() {
        let mut watcher = WatcherState::new();
        let ready = Observation {
            processes: vec![record(41, 400).with_debug(Some(9227))],
            has_ready_debug_session: true,
            ..Observation::default()
        };
        assert_eq!(watcher.decide(&ready), WatcherAction::Attach);
        let alive = Observation {
            processes: vec![record(41, 400), record(42, 401)],
            has_ready_debug_session: true,
            engine_alive: true,
            ..Observation::default()
        };
        assert_eq!(watcher.decide(&alive), WatcherAction::KeepActive);
    }

    #[test]
    fn electron_children_do_not_retrigger_takeover() {
        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(51, 500)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(51, 500), record(52, 501)])),
            WatcherAction::Takeover
        );
        watcher.mark_managed_active(&[ProcessKey {
            pid: 61,
            created_at: 600,
        }]);
        let children = Observation {
            processes: vec![
                record(61, 600).with_debug(Some(9227)),
                record(62, 601).with_debug(Some(9227)),
                record(63, 602),
            ],
            has_ready_debug_session: true,
            engine_alive: true,
            ..Observation::default()
        };
        assert_eq!(watcher.decide(&children), WatcherAction::KeepActive);
        assert_eq!(watcher.decide(&children), WatcherAction::KeepActive);
    }

    #[test]
    fn rearms_after_target_exit() {
        let mut watcher = WatcherState::new();
        watcher.mark_managed_active(&[ProcessKey {
            pid: 71,
            created_at: 700,
        }]);
        let gone = Observation::default();
        assert_eq!(watcher.decide(&gone), WatcherAction::ReleaseAndWait);
        assert_eq!(watcher.decide(&gone), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(81, 800)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(81, 800)])),
            WatcherAction::Takeover
        );
    }

    #[test]
    fn paused_watcher_does_not_takeover() {
        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        watcher.suspend();
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(91, 900), record(92, 901)])),
            WatcherAction::Suspend
        );
        watcher.rearm();
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(91, 900)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(91, 900)])),
            WatcherAction::Takeover
        );
    }

    #[test]
    fn reused_pid_with_new_creation_time_is_a_new_process() {
        let mut watcher = WatcherState::new();
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(5, 1)])),
            WatcherAction::ReportExistingUnmanaged
        );
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(5, 99)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(5, 99)])),
            WatcherAction::Takeover
        );
    }

    #[test]
    fn debug_port_wait_times_out_without_killing_and_rearms_after_exit() {
        let mut watcher = WatcherState::new();
        let start = Instant::now();
        assert_eq!(watcher.decide(&at(start, Vec::new())), WatcherAction::Wait);

        let launching = vec![record(31, 300).with_debug(Some(9227))];
        assert_eq!(
            watcher.decide(&at(start, launching.clone())),
            WatcherAction::WaitForDebugPort
        );
        assert_eq!(
            watcher.decide(&at(start + Duration::from_secs(44), launching.clone())),
            WatcherAction::WaitForDebugPort
        );
        assert_eq!(
            watcher.decide(&at(start + DEBUG_PORT_WAIT, launching.clone())),
            WatcherAction::ReportDebugTimeout
        );

        let with_child = vec![
            record(31, 300).with_debug(Some(9227)),
            record(32, 301).with_debug(Some(9227)),
        ];
        assert_eq!(
            watcher.decide(&at(start + Duration::from_secs(60), with_child)),
            WatcherAction::ReportDebugTimeout
        );
        assert_eq!(
            watcher.decide(&at(start + Duration::from_secs(61), Vec::new())),
            WatcherAction::ReleaseAndWait
        );
        assert_eq!(
            watcher.decide(&at(start + Duration::from_secs(62), Vec::new())),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&at(start + Duration::from_secs(63), vec![record(41, 400)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&at(start + Duration::from_secs(64), vec![record(41, 400)])),
            WatcherAction::Takeover
        );
    }

    fn try_acquire_busy(flag: &AtomicBool, fail: bool) -> Result<(), &'static str> {
        let _guard = BusyGuard::acquire(flag).ok_or("busy")?;
        assert!(flag.load(Ordering::Acquire));
        if fail {
            return Err("failed");
        }
        Ok(())
    }

    #[test]
    fn busy_guard_clears_after_error_return() {
        let flag = AtomicBool::new(false);
        assert_eq!(try_acquire_busy(&flag, true), Err("failed"));
        assert!(!flag.load(Ordering::Acquire));
        assert_eq!(try_acquire_busy(&flag, false), Ok(()));
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn busy_guard_acquire_is_exclusive() {
        let flag = AtomicBool::new(false);
        let first = BusyGuard::acquire(&flag).expect("first acquire");
        assert!(BusyGuard::acquire(&flag).is_none());
        drop(first);
        assert!(BusyGuard::acquire(&flag).is_some());
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn concurrent_acquire_only_one_guard_wins() {
        use std::sync::{atomic::AtomicUsize, Arc, Barrier};
        let flag = Arc::new(AtomicBool::new(false));
        let start = Arc::new(Barrier::new(8));
        let tried = Arc::new(Barrier::new(8));
        let wins = Arc::new(AtomicUsize::new(0));
        let workers = (0..8)
            .map(|_| {
                let flag = Arc::clone(&flag);
                let start = Arc::clone(&start);
                let tried = Arc::clone(&tried);
                let wins = Arc::clone(&wins);
                std::thread::spawn(move || {
                    start.wait();
                    let guard = BusyGuard::acquire(flag.as_ref());
                    if guard.is_some() {
                        wins.fetch_add(1, Ordering::SeqCst);
                    }
                    tried.wait();
                    drop(guard);
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("join busy worker");
        }
        assert_eq!(wins.load(Ordering::SeqCst), 1);
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn closing_target_during_encode_does_not_relaunch() {
        let original = keys_of(&[record(11, 100)]);
        assert_eq!(
            confirm_launch_after_encode(
                WatcherAction::Takeover,
                &original,
                &Observation::default()
            ),
            ConfirmedLaunch::CancelExited
        );
        assert_eq!(
            confirm_launch_after_encode(WatcherAction::Attach, &original, &Observation::default()),
            ConfirmedLaunch::CancelExited
        );

        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::Takeover
        );
        assert_eq!(
            watcher.sync_to_current(&Observation::default()),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(21, 200)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(21, 200)])),
            WatcherAction::Takeover
        );
    }

    #[test]
    fn encode_reprobe_cancels_stale_generation_and_only_attaches_live_session() {
        let original = keys_of(&[record(11, 100), record(12, 101)]);
        assert_eq!(
            confirm_launch_after_encode(
                WatcherAction::Takeover,
                &original,
                &unmanaged(vec![record(11, 100), record(13, 102)])
            ),
            ConfirmedLaunch::Takeover
        );
        assert_eq!(
            confirm_launch_after_encode(
                WatcherAction::Takeover,
                &original,
                &unmanaged(vec![record(31, 300)])
            ),
            ConfirmedLaunch::CancelStale
        );
        assert_eq!(
            confirm_launch_after_encode(
                WatcherAction::Attach,
                &original,
                &unmanaged(vec![record(11, 100)])
            ),
            ConfirmedLaunch::CancelStale
        );
        assert_eq!(
            confirm_launch_after_encode(
                WatcherAction::Attach,
                &original,
                &Observation {
                    processes: vec![record(11, 100).with_debug(Some(9227))],
                    has_ready_debug_session: true,
                    ..Observation::default()
                }
            ),
            ConfirmedLaunch::Attach
        );

        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::Takeover
        );
        assert_eq!(
            watcher.sync_to_current(&unmanaged(vec![record(31, 300)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(31, 300)])),
            WatcherAction::Takeover
        );

        let mut watcher = WatcherState::new();
        assert_eq!(watcher.decide(&Observation::default()), WatcherAction::Wait);
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::Wait
        );
        assert_eq!(
            watcher.decide(&unmanaged(vec![record(11, 100)])),
            WatcherAction::Takeover
        );
        assert_eq!(
            watcher.sync_to_current(&Observation {
                processes: vec![record(41, 400).with_debug(Some(9227))],
                has_ready_debug_session: true,
                ..Observation::default()
            }),
            WatcherAction::Attach
        );
    }

    #[test]
    fn parses_remote_debugging_port() {
        assert_eq!(
            parse_remote_debugging_port(
                r#"Multica.exe --remote-debugging-port=9227 --inspect-brk=127.0.0.1:9238"#
            ),
            Some(9227)
        );
        assert_eq!(
            parse_remote_debugging_port(r#""Multica.exe" "--remote-debugging-port=9228""#),
            Some(9228)
        );
        assert_eq!(parse_remote_debugging_port("Multica.exe"), None);
    }

    #[test]
    fn snapshot_of_missing_exe_is_empty() {
        let records = snapshot_executable_processes(r"C:\definitely-missing-multica\Multica.exe")
            .expect("snapshot should succeed");
        assert!(records.is_empty());
    }

    #[test]
    fn snapshot_can_see_current_process() {
        let exe = std::env::current_exe().expect("current test executable");
        let records = snapshot_executable_processes(&exe.to_string_lossy()).expect("snapshot self");
        let pid = std::process::id();
        assert!(
            records.iter().any(|record| record.key.pid == pid),
            "expected current pid {pid} in {:?}",
            records
        );
        assert!(records.iter().all(|record| record.key.created_at > 0));
    }
}
