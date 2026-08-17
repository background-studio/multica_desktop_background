use std::{process::Command, sync::atomic::Ordering};

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, Wry,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

use crate::StudioState;

const AUTOSTART_NAME: &str = "Multica Background Studio";

pub struct TrayUi {
    status: MenuItem<Wry>,
    pause: MenuItem<Wry>,
}

pub fn sync_autostart(enabled: bool, start_hidden: bool) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|error| error.to_string())?;
    if enabled {
        let mut command = format!("\"{}\"", current_exe.display());
        if start_hidden {
            command.push_str(" --hidden");
        }
        run.set_value(AUTOSTART_NAME, &command)
            .map_err(|error| error.to_string())
    } else {
        match run.delete_value(AUTOSTART_NAME) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn show_error(app: &AppHandle, error: impl AsRef<str>) {
    app.dialog()
        .message(error.as_ref())
        .title("Multica Background Studio")
        .buttons(MessageDialogButtons::Ok)
        .show(|_| {});
}

pub fn quit_without_touching_multica(app: AppHandle) {
    let state = app.state::<StudioState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    app.exit(0);
}

pub fn setup_tray(app: &AppHandle) -> Result<TrayUi, String> {
    let status = MenuItem::with_id(app, "status", "状态：尚未连接 Multica", false, None::<&str>)
        .map_err(|error| error.to_string())?;
    let open = MenuItem::with_id(app, "open", "打开背景管理器", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let apply = MenuItem::with_id(app, "apply", "应用或重新应用", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let pause = MenuItem::with_id(app, "pause", "暂停背景", false, None::<&str>)
        .map_err(|error| error.to_string())?;
    let restore = MenuItem::with_id(app, "restore", "恢复官方外观", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        "退出 Studio（保留 Multica）",
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let separator_one = PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?;
    let separator_two = PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &separator_one,
            &open,
            &apply,
            &pause,
            &restore,
            &separator_two,
            &quit,
        ],
    )
    .map_err(|error| error.to_string())?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "应用图标资源不存在。".to_string())?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Multica Background Studio")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "apply" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = crate::apply_background(app.clone(), app.state(), None).await;
                    if let Err(error) = result {
                        show_error(&app, error);
                    }
                });
            }
            "pause" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = crate::pause_background(app.clone(), app.state()).await;
                    if let Err(error) = result {
                        show_error(&app, error);
                    }
                });
            }
            "restore" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = crate::restore_background(app.clone(), app.state()).await;
                    if let Err(error) = result {
                        show_error(&app, error);
                    }
                });
            }
            "quit" => {
                quit_without_touching_multica(app.clone());
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|error| error.to_string())?;
    Ok(TrayUi { status, pause })
}

pub fn update_tray(app: &AppHandle, ui: &TrayUi) {
    let state = app.state::<StudioState>();
    let Ok(status) = state.runtime_status() else {
        return;
    };
    let _ = ui.status.set_text(format!("状态：{}", status.message));
    let _ = ui.pause.set_enabled(status.phase == "active");
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(format!(
            "Multica Background Studio · {}",
            status.message
        )));
    }
}

pub fn open_data_directory(path: &std::path::Path) -> Result<(), String> {
    Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
