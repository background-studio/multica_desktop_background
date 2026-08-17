use std::{
    collections::{HashMap, HashSet},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;

use crate::payload::{ActivePayload, PENDING_MEDIA_URL_KEY};

const MEDIA_CHUNK_BYTES: usize = 192 * 1024;
const PENDING_MEDIA_PARTS_KEY: &str = "__BACKGROUND_STUDIO_PENDING_MEDIA_PARTS__";

const REMOVE_RENDERER_PAYLOAD: &str = r#"(() => {
  const state = window.__MULTICA_BACKGROUND_STUDIO__;
  if (state?.cleanup) return state.cleanup();
  document.getElementById("multica-background-layer")?.remove();
  document.getElementById("multica-background-style")?.remove();
  document.documentElement?.classList.remove(
    "multica-background-active", "multica-background-home", "multica-background-task",
    "multica-background-home-disabled", "multica-background-task-disabled", "multica-background-fit-tile"
  );
  delete window.__MULTICA_BACKGROUND_STUDIO__;
  return true;
})()"#;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpTarget {
    id: String,
    #[serde(rename = "type")]
    target_type: String,
    url: String,
    web_socket_debugger_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpVersion {
    web_socket_debugger_url: String,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn validate_websocket_url(
    value: &str,
    port: u16,
    kind: &str,
    id: Option<&str>,
) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "CDP WebSocket 地址无效。".to_string())?;
    let hostname = url.host_str().unwrap_or_default();
    let loopback = matches!(hostname, "127.0.0.1" | "localhost" | "::1");
    let expected = id
        .map(|id| format!("/devtools/{kind}/{id}"))
        .unwrap_or_else(|| format!("/devtools/{kind}/"));
    let valid_path = id.map_or_else(
        || url.path().starts_with(&expected),
        |_| url.path() == expected,
    );
    if url.scheme() != "ws"
        || !loopback
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_path
    {
        return Err("CDP WebSocket 地址未通过本机回环校验。".to_string());
    }
    Ok(url.to_string())
}

fn fetch_json<T: for<'de> Deserialize<'de>>(port: u16, resource: &str) -> Result<T, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?
        .get(format!("http://127.0.0.1:{port}{resource}"))
        .header("Cache-Control", "no-store")
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("CDP 返回 HTTP {}", status.as_u16()));
    }
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("CDP HTTP 响应超过大小上限。".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub fn read_browser_identity(port: u16) -> Result<String, String> {
    let version: CdpVersion = fetch_json(port, "/json/version")?;
    let websocket =
        validate_websocket_url(&version.web_socket_debugger_url, port, "browser", None)?;
    let url = Url::parse(&websocket).map_err(|error| error.to_string())?;
    let id = url
        .path()
        .strip_prefix("/devtools/browser/")
        .filter(|id| valid_id(id))
        .ok_or_else(|| "CDP 浏览器身份无效。".to_string())?;
    Ok(id.to_string())
}

fn is_main_multica_target_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    match url.scheme() {
        "https" => match url.host_str() {
            Some("multica.ai") => true,
            Some(host) => host.ends_with(".multica.ai"),
            None => false,
        },
        // Multica Desktop 主界面是 Electron 本地 renderer，不是 https 页面。
        "file" => {
            let path = url.path().to_ascii_lowercase().replace('\\', "/");
            let renderer = path.ends_with("/out/renderer/index.html");
            let under_multica = path.contains("/@multicadesktop/")
                || path.contains("/%40multicadesktop/")
                || path.contains("/programs/multica/");
            renderer && under_multica
        }
        _ => false,
    }
}

fn list_targets(port: u16, browser_id: &str) -> Result<Vec<CdpTarget>, String> {
    if read_browser_identity(port)? != browser_id {
        return Err("CDP 浏览器身份已变化，拒绝继续注入。".to_string());
    }
    let targets: Vec<CdpTarget> = fetch_json(port, "/json/list")?;
    Ok(targets
        .into_iter()
        .filter(|target| {
            target.target_type == "page"
                && is_main_multica_target_url(&target.url)
                && valid_id(&target.id)
                && validate_websocket_url(
                    &target.web_socket_debugger_url,
                    port,
                    "page",
                    Some(&target.id),
                )
                .is_ok()
        })
        .collect())
}

enum WorkerRequest {
    Send {
        method: String,
        params: Value,
        response: mpsc::Sender<Result<Value, String>>,
    },
    Close,
}

type CdpSocket = WebSocket<MaybeTlsStream<TcpStream>>;

fn socket_command(
    socket: &mut CdpSocket,
    next_id: &mut u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = *next_id;
    *next_id += 1;
    socket
        .send(Message::Text(
            serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
                .map_err(|error| error.to_string())?
                .into(),
        ))
        .map_err(|error| error.to_string())?;
    loop {
        let message = socket.read().map_err(|error| error.to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("CDP 命令失败");
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
            return Err(format!("{message} ({code})"));
        }
        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
    }
}

struct CdpSession {
    sender: mpsc::Sender<WorkerRequest>,
    closed: Arc<AtomicBool>,
}

impl CdpSession {
    fn open(target: &CdpTarget, port: u16) -> Result<Self, String> {
        let websocket_url = validate_websocket_url(
            &target.web_socket_debugger_url,
            port,
            "page",
            Some(&target.id),
        )?;
        let (sender, receiver) = mpsc::channel();
        let (setup_sender, setup_receiver) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = Arc::clone(&closed);
        thread::Builder::new()
            .name(format!("cdp-{}", target.id))
            .spawn(move || {
                let setup = (|| {
                    let (mut socket, _) =
                        connect(websocket_url.as_str()).map_err(|error| error.to_string())?;
                    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(30)))
                            .map_err(|error| error.to_string())?;
                        // 大图 base64 写入可达数 MB，10s 写超时会把会话打坏并堵死后续命令。
                        stream
                            .set_write_timeout(Some(Duration::from_secs(120)))
                            .map_err(|error| error.to_string())?;
                    }
                    let mut next_id = 1;
                    socket_command(&mut socket, &mut next_id, "Runtime.enable", json!({}))?;
                    socket_command(&mut socket, &mut next_id, "Page.enable", json!({}))?;
                    Ok::<_, String>((socket, next_id))
                })();
                match setup {
                    Ok((mut socket, mut next_id)) => {
                        let _ = setup_sender.send(Ok(()));
                        while let Ok(request) = receiver.recv() {
                            match request {
                                WorkerRequest::Send {
                                    method,
                                    params,
                                    response,
                                } => {
                                    let result =
                                        socket_command(&mut socket, &mut next_id, &method, params);
                                    let _ = response.send(result);
                                }
                                WorkerRequest::Close => {
                                    let _ = socket.close(None);
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = setup_sender.send(Err(error));
                    }
                }
                worker_closed.store(true, Ordering::Relaxed);
            })
            .map_err(|error| error.to_string())?;
        setup_receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "CDP 连接超时。".to_string())??;
        Ok(Self { sender, closed })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn send(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.is_closed() {
            return Err("CDP 会话已关闭。".to_string());
        }
        // 大图 base64 注入可达数 MB；固定 11s 会误杀仍在写 socket 的命令，
        // 并让后续命令堵在 worker 队列里，表现为「正在处理」卡死、退出也无响应。
        let approx_bytes = params.to_string().len() as u64;
        let timeout_secs = (20 + approx_bytes / 80_000).clamp(20, 180);
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(WorkerRequest::Send {
                method: method.to_string(),
                params,
                response,
            })
            .map_err(|_| "CDP 会话已关闭。".to_string())?;
        match receiver.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok(result) => result,
            Err(_) => {
                // 超时后的 socket 仍可能在处理旧命令，不能继续向同一 worker 排队。
                self.closed.store(true, Ordering::Release);
                Err(format!("CDP 命令超时：{method}（已等待 {timeout_secs}s）"))
            }
        }
    }

    fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let result = self.send(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
                "userGesture": false
            }),
        )?;
        if let Some(details) = result.get("exceptionDetails") {
            let detail = details
                .pointer("/exception/description")
                .and_then(Value::as_str)
                .or_else(|| details.get("text").and_then(Value::as_str))
                .unwrap_or("未知异常");
            return Err(format!(
                "Multica 渲染页执行背景脚本失败：{}",
                detail.chars().take(300).collect::<String>()
            ));
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }
}

fn clear_pending_media(session: &CdpSession, revoke_url: bool) {
    let parts_key = serde_json::to_string(PENDING_MEDIA_PARTS_KEY).expect("key is serializable");
    let url_key = serde_json::to_string(PENDING_MEDIA_URL_KEY).expect("key is serializable");
    let revoke = if revoke_url {
        format!(
            "const url=window[{url_key}];if(typeof url==='string'&&url.startsWith('blob:'))URL.revokeObjectURL(url);"
        )
    } else {
        String::new()
    };
    let _ = session.evaluate(&format!(
        "(()=>{{{revoke}delete window[{parts_key}];delete window[{url_key}];return true;}})()"
    ));
}

fn upload_media(session: &CdpSession, payload: &ActivePayload) -> Result<(), String> {
    let parts_key =
        serde_json::to_string(PENDING_MEDIA_PARTS_KEY).map_err(|error| error.to_string())?;
    let url_key =
        serde_json::to_string(PENDING_MEDIA_URL_KEY).map_err(|error| error.to_string())?;
    session.evaluate(&format!(
        "(()=>{{const url=window[{url_key}];if(typeof url==='string'&&url.startsWith('blob:'))URL.revokeObjectURL(url);window[{url_key}]=undefined;window[{parts_key}]=[];return true;}})()"
    ))?;
    for chunk in payload.media_bytes.chunks(MEDIA_CHUNK_BYTES) {
        let encoded =
            serde_json::to_string(&STANDARD.encode(chunk)).map_err(|error| error.to_string())?;
        session.evaluate(&format!(
            "(()=>{{const binary=atob({encoded});const bytes=new Uint8Array(binary.length);for(let i=0;i<binary.length;i+=1)bytes[i]=binary.charCodeAt(i);window[{parts_key}].push(bytes);return bytes.length;}})()"
        ))?;
    }
    let mime =
        serde_json::to_string(&payload.media_mime_type).map_err(|error| error.to_string())?;
    session.evaluate(&format!(
        "(()=>{{const parts=window[{parts_key}];if(!Array.isArray(parts))throw new Error('背景媒体分块状态丢失');window[{url_key}]=URL.createObjectURL(new Blob(parts,{{type:{mime}}}));delete window[{parts_key}];return window[{url_key}];}})()"
    ))?;
    Ok(())
}

impl Drop for CdpSession {
    fn drop(&mut self) {
        let _ = self.sender.send(WorkerRequest::Close);
        self.closed.store(true, Ordering::Relaxed);
    }
}

pub fn window_controls_overlay_visible(port: u16, browser_id: &str) -> Result<bool, String> {
    let target = list_targets(port, browser_id)?
        .into_iter()
        .next()
        .ok_or_else(|| "Multica 主页面尚未创建。".to_string())?;
    let session = CdpSession::open(&target, port)?;
    Ok(session
        .evaluate("Boolean(navigator.windowControlsOverlay?.visible)")?
        .as_bool()
        .unwrap_or(false))
}

struct ManagedSession {
    session: CdpSession,
    early_script_id: Option<String>,
    revision: Option<String>,
}

struct InjectorInner {
    port: u16,
    browser_id: String,
    sessions: HashMap<String, ManagedSession>,
    payload: Option<ActivePayload>,
    paused: bool,
}

fn early_payload_for(payload: &str, revision: &str) -> String {
    let safe_revision = serde_json::to_string(revision).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(() => {{
  const revision = {safe_revision};
  const run = () => {{
    if (!document.documentElement) return false;
    try {{ {payload}; return true; }} catch {{ return false; }}
  }};
  if (!run()) {{
    const observer = new MutationObserver(() => {{
      if (run()) observer.disconnect();
    }});
    observer.observe(document.documentElement || document, {{ childList: true, subtree: true }});
    setTimeout(() => observer.disconnect(), 30000);
  }}
  return revision;
}})()"#
    )
}

fn remove_from_session(managed: &mut ManagedSession) {
    if let Some(identifier) = managed.early_script_id.take() {
        let _ = managed.session.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": identifier }),
        );
    }
    let _ = managed.session.evaluate(REMOVE_RENDERER_PAYLOAD);
    managed.revision = None;
}

const EARLY_TRANSPARENCY_SCRIPT: &str = r#"(() => {
  try {
    const style = document.createElement("style");
    style.id = "multica-background-early-transparency";
    style.textContent = "html,body,#root,.bg-app-shell,[data-slot=sidebar-wrapper],[data-sidebar=sidebar],[data-slot=sidebar-inner],[data-sidebar=menu-button][data-active],[data-sidebar=menu-button]:hover,[data-sidebar=menu-button]:focus-visible,.bg-page-canvas,.bg-page-canvas .bg-background,.bg-card,header,[role=button][aria-roledescription=sortable]>a[href*='/issues/']>[class~=bg-surface]{background:transparent!important;background-color:transparent!important}";
    (document.documentElement || document).appendChild(style);
  } catch {}
})()"#;

fn apply_to_session(managed: &mut ManagedSession, payload: &ActivePayload) -> Result<(), String> {
    let revision = &payload.revision;
    if managed.revision.as_deref() == Some(revision) {
        return Ok(());
    }
    if let Some(identifier) = managed.early_script_id.take() {
        let _ = managed.session.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": identifier }),
        );
    }
    managed
        .session
        .send("Page.setBypassCSP", json!({ "enabled": true }))?;
    let early_source = payload
        .early_script
        .as_deref()
        .map(|script| early_payload_for(script, revision))
        .unwrap_or_else(|| EARLY_TRANSPARENCY_SCRIPT.to_string());
    let early = managed.session.send(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": early_source }),
    )?;
    managed.early_script_id = early
        .get("identifier")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Err(error) = upload_media(&managed.session, payload)
        .and_then(|_| managed.session.evaluate(&payload.script).map(|_| ()))
    {
        clear_pending_media(&managed.session, true);
        return Err(error);
    }
    clear_pending_media(&managed.session, false);
    managed.revision = Some(revision.to_string());
    Ok(())
}

fn sync_inner(inner: &Arc<Mutex<InjectorInner>>, target_count: &AtomicUsize, force: bool) {
    let Ok(mut inner) = inner.lock() else {
        return;
    };
    let Ok(targets) = list_targets(inner.port, &inner.browser_id) else {
        target_count.store(inner.sessions.len(), Ordering::Relaxed);
        return;
    };
    let target_ids = targets
        .iter()
        .map(|target| target.id.as_str())
        .collect::<HashSet<_>>();
    inner
        .sessions
        .retain(|id, managed| target_ids.contains(id.as_str()) && !managed.session.is_closed());
    for target in targets {
        if inner.sessions.contains_key(&target.id) {
            continue;
        }
        let Ok(session) = CdpSession::open(&target, inner.port) else {
            continue;
        };
        let probe = session
            .evaluate("Boolean(document.documentElement)")
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if probe {
            inner.sessions.insert(
                target.id,
                ManagedSession {
                    session,
                    early_script_id: None,
                    revision: None,
                },
            );
        }
    }
    if !inner.paused {
        let payload = inner.payload.clone();
        if let Some(payload) = payload {
            for managed in inner.sessions.values_mut() {
                if force {
                    managed.revision = None;
                }
                if let Err(error) = apply_to_session(managed, &payload) {
                    eprintln!("CDP 注入失败: {error}");
                }
            }
        }
    }
    target_count.store(inner.sessions.len(), Ordering::Relaxed);
}

pub struct InjectorEngine {
    inner: Arc<Mutex<InjectorInner>>,
    target_count: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl InjectorEngine {
    pub fn new(port: u16, browser_id: String) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InjectorInner {
                port,
                browser_id,
                sessions: HashMap::new(),
                payload: None,
                paused: false,
            })),
            target_count: Arc::new(AtomicUsize::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn start(&mut self, payload: ActivePayload) -> Result<(), String> {
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "CDP 注入状态锁已损坏。".to_string())?;
            inner.payload = Some(payload);
            inner.paused = false;
        }
        sync_inner(&self.inner, &self.target_count, false);
        if self.thread.is_none() {
            let inner = Arc::clone(&self.inner);
            let count = Arc::clone(&self.target_count);
            let stopping = Arc::clone(&self.stopping);
            self.thread = Some(
                thread::Builder::new()
                    .name("multica-cdp-sync".to_string())
                    .spawn(move || {
                        while !stopping.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(1200));
                            if !stopping.load(Ordering::Relaxed) {
                                sync_inner(&inner, &count, false);
                            }
                        }
                    })
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(())
    }

    pub fn update(&self, payload: ActivePayload) -> Result<(), String> {
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "CDP 注入状态锁已损坏。".to_string())?;
            inner.payload = Some(payload);
            inner.paused = false;
        }
        sync_inner(&self.inner, &self.target_count, true);
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "CDP 注入状态锁已损坏。".to_string())?;
        inner.paused = true;
        for managed in inner.sessions.values_mut() {
            remove_from_session(managed);
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "CDP 注入状态锁已损坏。".to_string())?;
        inner.paused = true;
        for managed in inner.sessions.values_mut() {
            remove_from_session(managed);
            let _ = managed
                .session
                .send("Page.setBypassCSP", json!({ "enabled": false }));
        }
        inner.sessions.clear();
        self.target_count.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub fn active_targets(&self) -> u32 {
        self.target_count.load(Ordering::Relaxed) as u32
    }

    pub fn is_connected(&self) -> bool {
        let Ok(inner) = self.inner.lock() else {
            return false;
        };
        match read_browser_identity(inner.port) {
            Ok(id) => id == inner.browser_id,
            Err(_) => false,
        }
    }
}

impl Drop for InjectorEngine {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_websocket_urls() {
        assert!(validate_websocket_url(
            "ws://127.0.0.1:9227/devtools/page/page-1",
            9227,
            "page",
            Some("page-1")
        )
        .is_ok());
        for value in [
            "ws://192.168.1.2:9227/devtools/page/page-1",
            "wss://127.0.0.1:9227/devtools/page/page-1",
            "ws://127.0.0.1:9228/devtools/page/page-1",
            "ws://user@127.0.0.1:9227/devtools/page/page-1",
        ] {
            assert!(validate_websocket_url(value, 9227, "page", Some("page-1")).is_err());
        }
    }

    #[test]
    fn injects_only_multica_app_pages() {
        assert!(is_main_multica_target_url(
            "https://multica.ai/p/example/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(is_main_multica_target_url(
            "https://www.multica.ai/workspace/page"
        ));
        assert!(is_main_multica_target_url("https://app.multica.ai/tasks"));
        assert!(is_main_multica_target_url("https://staging.multica.ai/"));
        assert!(is_main_multica_target_url(
            "file:///C:/Users/example/AppData/Local/Programs/@multicadesktop/resources/app.asar/out/renderer/index.html"
        ));
        assert!(is_main_multica_target_url(
            "file:///C:/Users/example/AppData/Local/Programs/Multica/resources/app.asar/out/renderer/index.html"
        ));
        assert!(!is_main_multica_target_url(
            "file:///C:/Users/example/AppData/Local/Programs/@multicadesktop/resources/app.asar/out/renderer/other.html"
        ));
        assert!(!is_main_multica_target_url("http://localhost:3000/"));
        assert!(!is_main_multica_target_url(
            "https://evil.example/multica.ai"
        ));
        assert!(!is_main_multica_target_url(
            "https://multica.ai.evil.example/"
        ));
        assert!(!is_main_multica_target_url("https://notmultica.ai/"));
    }

    #[test]
    fn early_payload_contains_revision_and_cleanup_is_reversible() {
        let payload = early_payload_for("window.test = true", "revision-1");
        assert!(payload.contains("revision-1"));
        assert!(payload.contains("MutationObserver"));
        assert!(REMOVE_RENDERER_PAYLOAD.contains("cleanup"));
    }

    #[test]
    #[ignore = "requires a live Multica CDP endpoint on port 9227"]
    fn reads_live_multica_browser_identity() {
        let identity = read_browser_identity(9227).expect("read live browser identity");
        assert!(valid_id(&identity));
    }

    #[test]
    #[ignore = "requires a live Multica CDP endpoint on port 9227"]
    fn reads_live_multica_with_reqwest() {
        let body = reqwest::blocking::get("http://127.0.0.1:9227/json/version")
            .expect("connect with reqwest")
            .text()
            .expect("read response");
        assert!(body.contains("webSocketDebuggerUrl"));
    }

    #[test]
    #[ignore = "requires a live Multica CDP endpoint on port 9227"]
    fn connects_to_live_multica_page_session() {
        let identity = read_browser_identity(9227).expect("read live browser identity");
        let target = list_targets(9227, &identity)
            .expect("list live targets")
            .into_iter()
            .next()
            .expect("at least one Multica page target");
        let session = CdpSession::open(&target, 9227).expect("open CDP page session");
        assert_eq!(
            session
                .evaluate("Boolean(document.documentElement)")
                .expect("evaluate page probe"),
            Value::Bool(true)
        );
        assert_eq!(
            session
                .evaluate("Boolean(window.__MULTICA_BACKGROUND_STUDIO__)")
                .expect("verify injected background state"),
            Value::Bool(true)
        );
    }
}
