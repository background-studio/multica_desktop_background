use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{
    host, lock,
    plugin::{self, PIPE_NAME, PLUGIN_ID, PLUGIN_PROTOCOL},
    StudioState,
};

#[derive(Debug, Deserialize)]
struct PluginRequest {
    id: String,
    cmd: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResult {
    plugin_protocol: u32,
    plugin_id: &'static str,
    version: &'static str,
    phase: String,
    message: String,
    active_targets: u32,
    paused: bool,
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = serve(app).await {
            eprintln!("Background Studio 插件 IPC 失败：{error}");
        }
    });
}

#[cfg(windows)]
async fn serve(app: AppHandle) -> Result<(), String> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(PIPE_NAME)
        .map_err(|error| format!("创建插件管道失败：{error}"))?;

    loop {
        server
            .connect()
            .await
            .map_err(|error| format!("等待插件管道连接失败：{error}"))?;
        let connected = server;
        server = ServerOptions::new()
            .create(PIPE_NAME)
            .map_err(|error| format!("重建插件管道失败：{error}"))?;

        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = handle_client(app, connected).await {
                eprintln!("插件 IPC 会话结束：{error}");
            }
        });
    }
}

#[cfg(not(windows))]
async fn serve(_app: AppHandle) -> Result<(), String> {
    Err("插件 IPC 仅支持 Windows。".to_string())
}

#[cfg(windows)]
async fn handle_client(
    app: AppHandle,
    client: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
) -> Result<(), String> {
    let (reader, mut writer) = tokio::io::split(client);
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| error.to_string())?
    {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<PluginRequest>(&line) {
            Ok(request) => dispatch(&app, request).await,
            Err(error) => json!({
                "id": "",
                "ok": false,
                "error": format!("无效请求：{error}")
            }),
        };
        let mut payload = serde_json::to_string(&response).map_err(|error| error.to_string())?;
        payload.push('\n');
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        writer.flush().await.map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn dispatch(app: &AppHandle, request: PluginRequest) -> serde_json::Value {
    let id = request.id.clone();
    let result = match request.cmd.as_str() {
        "status" => status_payload(app),
        "open-ui" => crate::open_main_window(app)
            .map(|_| json!({ "opened": true }))
            .map_err(|error| error),
        "apply" => apply_via_ipc(app.clone()).await,
        "pause" => pause_via_ipc(app.clone()).await,
        "restore" => restore_via_ipc(app.clone()).await,
        "quit-keep-target" => {
            host::quit_without_touching_multica(app.clone());
            Ok(json!({ "quitting": true }))
        }
        other => Err(format!("未知命令：{other}")),
    };
    match result {
        Ok(value) => json!({ "id": id, "ok": true, "result": value }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    }
}

fn status_payload(app: &AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<StudioState>();
    let status = state.runtime_status()?;
    let payload = StatusResult {
        plugin_protocol: PLUGIN_PROTOCOL,
        plugin_id: PLUGIN_ID,
        version: env!("CARGO_PKG_VERSION"),
        phase: status.phase.clone(),
        message: status.message,
        active_targets: status.active_targets,
        paused: status.phase == "paused",
    };
    serde_json::to_value(payload).map_err(|error| error.to_string())
}

async fn apply_via_ipc(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<StudioState>();
    let payload = {
        let state = app.state::<StudioState>();
        state.active_payload()?
    };
    let controller = std::sync::Arc::clone(&state.controller);
    let first_payload = payload.clone();
    let first = tauri::async_runtime::spawn_blocking(move || {
        lock(&controller)?.apply(first_payload, false)
    })
    .await
    .map_err(|error| error.to_string())?;
    let _ = state.refresh_runtime_status();
    match first {
        Ok(_) => status_payload(&app),
        Err(error) if error.contains("需要重启一次") => {
            let controller = std::sync::Arc::clone(&state.controller);
            let retry = tauri::async_runtime::spawn_blocking(move || {
                lock(&controller)?.apply(payload, true)
            })
            .await
            .map_err(|error| error.to_string())?;
            let _ = state.refresh_runtime_status();
            retry?;
            status_payload(&app)
        }
        Err(error) => Err(error),
    }
}

async fn pause_via_ipc(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<StudioState>();
    state
        .live_apply_generation
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let controller = std::sync::Arc::clone(&state.controller);
    tauri::async_runtime::spawn_blocking(move || lock(&controller)?.pause())
        .await
        .map_err(|error| error.to_string())??;
    let _ = state.refresh_runtime_status();
    status_payload(&app)
}

async fn restore_via_ipc(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<StudioState>();
    state
        .live_apply_generation
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let controller = std::sync::Arc::clone(&state.controller);
    tauri::async_runtime::spawn_blocking(move || lock(&controller)?.restore())
        .await
        .map_err(|error| error.to_string())??;
    let _ = state.refresh_runtime_status();
    status_payload(&app)
}

#[allow(dead_code)]
pub fn plugin_mode_enabled() -> bool {
    plugin::is_plugin_mode()
}
