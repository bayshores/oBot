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

/// Open an http(s) link in the default web browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only web links can be opened".into());
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

fn main() {
    // Claude Code starts this same program with `mcp-approve` to ask the user for permission.
    if std::env::args().nth(1).as_deref() == Some("mcp-approve") {
        if let Err(e) = openbot_core::mcp::run_approval_mcp() {
            eprintln!("openbot approval helper failed: {e}");
        }
        return;
    }
    // ...and with `hook <event>` at certain moments of a run (see openbot_core::hooks).
    if std::env::args().nth(1).as_deref() == Some("hook") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(openbot_core::hooks::run(&args));
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
        .invoke_handler(tauri::generate_handler![notify, set_badge, is_focused, open_url])
        // Menu bar: openbot › Settings… (⌘,) and Connect to Remote Computer…
        .menu(|app| {
            let menu = tauri::menu::Menu::default(app)?;
            #[cfg(target_os = "macos")]
            if let Some(tauri::menu::MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
                use tauri::menu::{MenuItem, PredefinedMenuItem};
                let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
                let remote = MenuItem::with_id(app, "remote", "Connect to Remote Computer…", true, None::<&str>)?;
                app_menu.insert(&PredefinedMenuItem::separator(app)?, 1)?;
                app_menu.insert(&settings, 2)?;
                app_menu.insert(&remote, 3)?;
            }
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            let call = match event.id().as_ref() {
                "settings" => "window.openbotOpenSettings && window.openbotOpenSettings()",
                "remote" => "window.openbotOpenSettings && window.openbotOpenSettings('computer')",
                _ => return,
            };
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
                let _ = w.eval(call);
            }
        })
        .setup(move |app| {
            let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("openbot")
                .inner_size(1040.0, 760.0)
                .min_inner_size(720.0, 520.0)
                // openbot is always dark (no light mode), so the window frame and loading background match.
                .theme(Some(tauri::Theme::Dark))
                .background_color(tauri::window::Color(11, 11, 12, 255));
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
