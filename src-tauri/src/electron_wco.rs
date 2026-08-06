use std::{
    collections::VecDeque,
    net::{TcpListener, TcpStream},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;

use crate::injector::{read_browser_identity, window_controls_overlay_visible};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PREFERRED_INSPECTOR_PORT: u16 = 9238;
const INSPECTOR_WAIT: Duration = Duration::from_secs(15);
const RENDERER_WAIT: Duration = Duration::from_secs(45);

const WCO_PATCH: &str = r##"(() => {
  const originalElectron = electron;
  const Original = originalElectron.BrowserWindow;
  if (typeof Original !== "function") {
    throw new Error("Electron BrowserWindow is unavailable");
  }
  const Patched = new Proxy(Original, {
    construct(target, args) {
      const options = { ...(args[0] || {}) };
      options.titleBarStyle = "hidden";
      options.titleBarOverlay = {
        color: "rgba(0, 0, 0, 0)",
        symbolColor: "#ffffff",
        height: 48
      };
      args[0] = options;
      return Reflect.construct(target, args, target);
    }
  });
  electron = new Proxy(originalElectron, {
    get(target, property, receiver) {
      if (property === "BrowserWindow") return Patched;
      return Reflect.get(target, property, receiver);
    }
  });
  globalThis.__MULTICA_BACKGROUND_WCO_PATCHED__ = true;
  return {
    patched: electron.BrowserWindow === Patched,
    originalName: Original.name
  };
})()"##;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectorTarget {
    #[serde(rename = "type")]
    target_type: String,
    title: String,
    web_socket_debugger_url: String,
}

type InspectorSocket = WebSocket<MaybeTlsStream<TcpStream>>;

struct InspectorSession {
    socket: InspectorSocket,
    next_id: u64,
    events: VecDeque<Value>,
}

impl InspectorSession {
    fn open(target: &InspectorTarget, port: u16) -> Result<Self, String> {
        let websocket = validate_inspector_websocket(&target.web_socket_debugger_url, port)?;
        let (mut socket, _) = connect(websocket.as_str()).map_err(|error| error.to_string())?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(20)))
                .map_err(|error| error.to_string())?;
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .map_err(|error| error.to_string())?;
        }
        Ok(Self {
            socket,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    fn command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.socket
            .send(Message::Text(
                serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
                    .map_err(|error| error.to_string())?
                    .into(),
            ))
            .map_err(|error| error.to_string())?;
        loop {
            let value = self.read_value()?;
            if value.get("method").is_some() && value.get("id").is_none() {
                self.events.push_back(value);
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(cdp_error(error));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn wait_event(&mut self, method: &str) -> Result<Value, String> {
        if let Some(index) = self
            .events
            .iter()
            .position(|event| event.get("method").and_then(Value::as_str) == Some(method))
        {
            return Ok(self.events.remove(index).unwrap_or(Value::Null));
        }
        loop {
            let value = self.read_value()?;
            if value.get("method").and_then(Value::as_str) == Some(method) {
                return Ok(value);
            }
            if value.get("method").is_some() && value.get("id").is_none() {
                self.events.push_back(value);
            }
        }
    }

    fn evaluate_on_frame(
        &mut self,
        call_frame_id: &str,
        expression: &str,
    ) -> Result<Value, String> {
        let result = self.command(
            "Debugger.evaluateOnCallFrame",
            json!({
                "callFrameId": call_frame_id,
                "expression": expression,
                "returnByValue": true
            }),
        )?;
        ensure_no_exception(&result)?;
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true
            }),
        )?;
        ensure_no_exception(&result)?;
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    fn read_value(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.read().map_err(|error| error.to_string())? {
                Message::Text(text) => {
                    return serde_json::from_str(&text).map_err(|error| error.to_string())
                }
                Message::Close(_) => return Err("Electron 主进程 Inspector 已关闭。".to_string()),
                _ => {}
            }
        }
    }
}

impl Drop for InspectorSession {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
    }
}

fn cdp_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Inspector 命令失败");
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
    format!("{message} ({code})")
}

fn ensure_no_exception(result: &Value) -> Result<(), String> {
    let Some(details) = result.get("exceptionDetails") else {
        return Ok(());
    };
    let description = details
        .pointer("/exception/description")
        .and_then(Value::as_str)
        .or_else(|| details.get("text").and_then(Value::as_str))
        .unwrap_or("未知异常");
    Err(format!(
        "Electron 主进程补丁执行失败：{}",
        description.chars().take(500).collect::<String>()
    ))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn validate_inspector_websocket(value: &str, port: u16) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "Electron Inspector 地址无效。".to_string())?;
    let hostname = url.host_str().unwrap_or_default();
    let id = url.path().trim_start_matches('/');
    if url.scheme() != "ws"
        || !matches!(hostname, "127.0.0.1" | "localhost" | "::1")
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_id(id)
    {
        return Err("Electron Inspector 地址未通过本机回环校验。".to_string());
    }
    Ok(url.to_string())
}

fn fetch_json<T: for<'de> Deserialize<'de>>(port: u16, resource: &str) -> Result<T, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?
        .get(format!("http://127.0.0.1:{port}{resource}"))
        .header("Cache-Control", "no-store")
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Inspector 返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Inspector 响应超过大小上限。".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn wait_for_inspector(port: u16) -> Result<InspectorTarget, String> {
    let deadline = Instant::now() + INSPECTOR_WAIT;
    loop {
        let attempt_error = match fetch_json::<Vec<InspectorTarget>>(port, "/json/list") {
            Ok(targets) => {
                if let Some(target) = targets.into_iter().find(|target| {
                    target.target_type == "node"
                        && target.title.starts_with("electron/")
                        && validate_inspector_websocket(&target.web_socket_debugger_url, port)
                            .is_ok()
                }) {
                    return Ok(target);
                }
                "未找到 Electron Node Inspector target".to_string()
            }
            Err(error) => error,
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "Multica 主进程 Inspector 未能启动（可能被 Electron fuse 禁用）：{attempt_error}"
            ));
        }
        thread::sleep(Duration::from_millis(120));
    }
}

fn select_inspector_port(renderer_port: u16) -> Result<u16, String> {
    for port in PREFERRED_INSPECTOR_PORT..=PREFERRED_INSPECTOR_PORT.saturating_add(100) {
        if port != renderer_port && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err("无法为 Multica 分配主进程 Inspector 端口。".to_string())
}

fn patch_line(source: &str) -> Result<usize, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let electron_line = lines
        .iter()
        .position(|line| line.contains("let electron = require(\"electron\")"))
        .ok_or_else(|| "Multica 主进程入口缺少预期的 Electron import。".to_string())?;
    lines
        .iter()
        .enumerate()
        .skip(electron_line + 1)
        .find_map(|(index, line)| line.starts_with("//#region src/").then_some(index))
        .ok_or_else(|| "无法定位 Multica 主进程 import 结束位置。".to_string())
}

fn first_call_frame(event: &Value) -> Result<(&str, &str, u64), String> {
    let frame = event
        .pointer("/params/callFrames/0")
        .ok_or_else(|| "Electron Inspector 暂停事件缺少调用帧。".to_string())?;
    let id = frame
        .get("callFrameId")
        .and_then(Value::as_str)
        .ok_or_else(|| "Electron Inspector 调用帧 ID 无效。".to_string())?;
    let script_id = frame
        .pointer("/location/scriptId")
        .and_then(Value::as_str)
        .ok_or_else(|| "Electron Inspector script ID 无效。".to_string())?;
    let line = frame
        .pointer("/location/lineNumber")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok((id, script_id, line))
}

fn schedule_inspector_close(session: &mut InspectorSession) -> Result<(), String> {
    session.evaluate(
        r#"(() => {
  const closeInspector = () => {
    try { process.mainModule.require("inspector").close(); } catch {}
  };
  setTimeout(closeInspector, 150);
  return true;
})()"#,
    )?;
    Ok(())
}

fn wait_for_inspector_close(port: u16) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(100),
        )
        .is_err()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("Electron 主进程 Inspector 未能按时关闭。".to_string())
}

pub fn launch_with_transparent_wco(
    executable: &Path,
    renderer_port: u16,
) -> Result<String, String> {
    if !executable.is_file() {
        return Err("Multica 可执行文件不存在。".to_string());
    }
    let inspector_port = select_inspector_port(renderer_port)?;
    Command::new(executable)
        .args([
            "--remote-debugging-address=127.0.0.1".to_string(),
            format!("--remote-debugging-port={renderer_port}"),
            "--remote-allow-origins=*".to_string(),
            format!("--inspect-brk=127.0.0.1:{inspector_port}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("启动 Multica 失败：{error}"))?;

    let target = wait_for_inspector(inspector_port)?;
    let mut session = InspectorSession::open(&target, inspector_port)?;
    let result = (|| {
        session.command("Runtime.enable", json!({}))?;
        session.command("Debugger.enable", json!({}))?;
        session.command("Runtime.runIfWaitingForDebugger", json!({}))?;

        // --inspect-brk first pauses at the first line of the actual main bundle.
        let first_pause = session.wait_event("Debugger.paused")?;
        let (_, script_id, _) = first_call_frame(&first_pause)?;
        let source = session
            .command("Debugger.getScriptSource", json!({ "scriptId": script_id }))?
            .get("scriptSource")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "无法读取 Multica 主进程入口脚本。".to_string())?;
        if !source.contains("titleBarStyle: \"hiddenInset\"")
            || !source.contains("new electron.BrowserWindow")
        {
            return Err("Multica 主进程窗口结构已变化，拒绝应用透明标题栏补丁。".to_string());
        }
        let line = patch_line(&source)?;
        session.command(
            "Debugger.setBreakpoint",
            json!({
                "location": {
                    "scriptId": script_id,
                    "lineNumber": line,
                    "columnNumber": 0
                }
            }),
        )?;
        session.command("Debugger.resume", json!({}))?;

        let import_pause = session.wait_event("Debugger.paused")?;
        let (call_frame_id, _, paused_line) = first_call_frame(&import_pause)?;
        if paused_line < line as u64 {
            return Err("Electron 主进程未在预期的 import 结束位置暂停。".to_string());
        }
        let patch = session.evaluate_on_frame(call_frame_id, WCO_PATCH)?;
        if patch.get("patched").and_then(Value::as_bool) != Some(true) {
            return Err("Electron BrowserWindow 代理未成功安装。".to_string());
        }
        session.command("Debugger.resume", json!({}))?;

        let deadline = Instant::now() + RENDERER_WAIT;
        let browser_id = loop {
            if let Ok(browser_id) = read_browser_identity(renderer_port) {
                if window_controls_overlay_visible(renderer_port, &browser_id).unwrap_or(false) {
                    break browser_id;
                }
            }
            if Instant::now() >= deadline {
                return Err("Multica 已启动，但透明原生按钮标题栏未生效。".to_string());
            }
            thread::sleep(Duration::from_millis(250));
        };

        schedule_inspector_close(&mut session)?;
        Ok(browser_id)
    })();

    if result.is_err() {
        // Never leave Multica suspended at a debugger pause.
        let _ = session.command("Runtime.runIfWaitingForDebugger", json!({}));
        let _ = session.command("Debugger.resume", json!({}));
    }
    drop(session);
    if result.is_ok() {
        wait_for_inspector_close(inspector_port)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_node_inspector_urls() {
        assert!(validate_inspector_websocket(
            "ws://127.0.0.1:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
            9238
        )
        .is_ok());
        for url in [
            "ws://192.168.1.2:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
            "ws://127.0.0.1:9239/04fc6552-0589-462d-a5b3-7696544e53e6",
            "wss://127.0.0.1:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
            "ws://user@127.0.0.1:9238/04fc6552-0589-462d-a5b3-7696544e53e6",
        ] {
            assert!(validate_inspector_websocket(url, 9238).is_err());
        }
    }

    #[test]
    fn finds_import_boundary_in_rolldown_bundle() {
        let source = r#"//#region \0rolldown/runtime.js
var helper = true;
//#endregion
let electron = require("electron");
let path = require("path");
//#region src/main/example.ts
const value = 1;"#;
        assert_eq!(patch_line(source).unwrap(), 5);
    }

    #[test]
    fn patch_enables_transparent_native_controls() {
        assert!(WCO_PATCH.contains("titleBarStyle = \"hidden\""));
        assert!(WCO_PATCH.contains("rgba(0, 0, 0, 0)"));
        assert!(WCO_PATCH.contains("symbolColor: \"#ffffff\""));
        assert!(WCO_PATCH.contains("height: 48"));
    }
}
