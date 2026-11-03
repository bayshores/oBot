//! openbot desktop app.
//!
//! Starts the openbot server inside the app (only reachable from this computer), then opens a
//! window showing its UI. In Settings > Computer the UI can switch to a remote computer
//! (e.g. a Windows PC running `openbot-server` 24/7).

// No extra console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

/// Show an OS notification (called by the UI when a bot finishes or needs you).
#[tauri::command]
fn notify(app: tauri::AppHandle, title: String, body: String) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Number on the Dock / taskbar icon = unread messages across all bots.
#[tauri::command]
fn set_badge(app: tauri::AppHandle, count: i64) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_badge_count(if count > 0 { Some(count) } else { None });
    }
}

/// Is the openbot window focused? (No notification when you're already looking.)
#[tauri::command]
fn is_focused(app: tauri::AppHandle) -> bool {
    app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false)
}

fn main() {
    // Claude Code starts this same program with `mcp-approve` to ask the user for permission.
    if std::env::args().nth(1).as_deref() == Some("mcp-approve") {
        if let Err(e) = openbot_core::mcp::run_approval_mcp() {
            eprintln!("openbot approval helper failed: {e}");
        }
        return;
    }

    openbot_core::util::fix_path_env();

    let server = tauri::async_runtime::block_on(openbot_core::start(openbot_core::ServeOptions {
        host: "127.0.0.1".into(),
        port: 0,
        data_dir: None,
    }))
    .expect("openbot couldn't start its local server");

    let url = format!("http://127.0.0.1:{}/?token={}", server.addr.port(), server.token);

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![notify, set_badge, is_focused])
        .setup(move |app| {
            let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("openbot")
                .inner_size(1040.0, 760.0)
                .min_inner_size(720.0, 520.0);
            #[cfg(target_os = "macos")]
            let builder = builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running openbot");
}
