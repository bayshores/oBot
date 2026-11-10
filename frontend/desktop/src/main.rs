//! oBot desktop app.
//!
//! Starts the oBot server inside the app (only reachable from this computer), then opens a
//! window showing its UI. In Settings > Computer the UI can switch to a remote computer
//! (e.g. a Windows PC running `obot-server` 24/7).

// No extra console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::webview::NewWindowResponse;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

/// Show an OS notification (called by the UI when a bot finishes or needs you).
#[tauri::command]
fn notify(app: tauri::AppHandle, title: String, body: String) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// A page sent with the "Send to oBot" bookmark (obot://share#<page>) that the window hasn't
/// taken yet. The page asks for it when it starts and when it's told one arrived.
static SHARED: Mutex<Option<String>> = Mutex::new(None);

#[tauri::command]
fn take_share() -> Option<String> {
    SHARED.lock().ok()?.take()
}

/// The page part of an obot://share#<page> link (see shareBookmarklet in app.js).
fn share_of(url: &url::Url) -> Option<String> {
    let page = url.fragment()?;
    let ok = url.scheme() == "obot"
        && url.host_str() == Some("share")
        && page.len() > 1
        && page.starts_with(['z', 'p'])
        && page.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| page.to_string())
}

/// Number on the Dock / taskbar icon = unread messages across all bots.
#[tauri::command]
fn set_badge(app: tauri::AppHandle, count: i64) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_badge_count(if count > 0 { Some(count) } else { None });
    }
}

/// Is the oBot window focused? (No notification when you're already looking.)
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
    os_open(&url, false).map_err(|e| e.to_string())
}

/// Hand a link or file to the system: open it with its default app, or (`reveal`) show the file in
/// Finder / Explorer.
fn os_open(target: &str, reveal: bool) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(target);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new(if reveal { "explorer" } else { "rundll32" });
        if reveal {
            c.arg(format!("/select,{target}"));
        } else {
            c.args(["url.dll,FileProtocolHandler", target]);
        }
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        let dir = if reveal { Path::new(target).parent().map(|d| d.to_string_lossy().into_owned()) } else { None };
        c.arg(dir.unwrap_or_else(|| target.to_string()));
        c
    };
    cmd.spawn().map(|_| ())
}

/// Files a Bot sent are opened right after saving only if they're documents, pictures or media.
/// Anything else (scripts, apps, installers) is shown in Finder instead, so one click never runs a
/// program.
fn safe_to_open(path: &Path) -> bool {
    const KINDS: &[&str] = &[
        "pdf", "txt", "md", "csv", "tsv", "json", "rtf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pages",
        "numbers", "key", "odt", "ods", "odp", "ipynb", "png", "jpg", "jpeg", "gif", "webp", "heic", "bmp",
        "tiff", "mp3", "m4a", "wav", "mp4", "mov", "m4v",
    ];
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    KINDS.contains(&ext.as_str())
}

/// A free path in `dir` for `name`: "notes.txt", then "notes (1).txt", "notes (2).txt"...
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned());
    let base = base.filter(|n| !n.trim().is_empty() && !n.starts_with('.')).unwrap_or_else(|| "file".into());
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (base.clone(), String::new()),
    };
    let mut path = dir.join(&base);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    path
}

/// Save a file a Bot sent into Downloads, then open it (`open`) or show it in Finder. The app
/// downloads it itself: the window can't show a file in a new window or reliably save one from the
/// remote computer. Returns the saved file's name and whether it was opened.
#[tauri::command]
async fn save_file(url: String, name: String, open: bool) -> Result<(String, bool), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) || !url.contains("/files/") {
        return Err("Only files from oBot can be saved".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let dir = dirs::download_dir().ok_or("Couldn't find the Downloads folder")?;
        let resp = ureq::get(&url).timeout(std::time::Duration::from_secs(300)).call().map_err(|e| e.to_string())?;
        let path = free_path(&dir, &name);
        let part = path.with_file_name(format!(".{}.part", path.file_name().unwrap_or_default().to_string_lossy()));
        let saved = std::fs::File::create(&part)
            .and_then(|mut f| std::io::copy(&mut resp.into_reader(), &mut f))
            .and_then(|_| std::fs::rename(&part, &path));
        if let Err(e) = saved {
            let _ = std::fs::remove_file(&part);
            return Err(e.to_string());
        }
        let file = path.to_string_lossy().into_owned();
        let opened = open && safe_to_open(&path) && os_open(&file, false).is_ok();
        if !opened {
            let _ = os_open(&file, true);
        }
        Ok((path.file_name().unwrap_or_default().to_string_lossy().into_owned(), opened))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// oBot's updater: the newest GitHub release's `latest.json`, checked with oBot's release key.
fn updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    if obot_core::update::PUBLIC_KEY.is_empty() {
        return Err("This build of oBot can't check for updates.".into());
    }
    let feed = format!("https://github.com/{}/releases/latest/download/latest.json", obot_core::update::REPO);
    app.updater_builder()
        .pubkey(obot_core::update::PUBLIC_KEY)
        .endpoints(vec![feed.parse().map_err(|e: url::ParseError| e.to_string())?])
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())
}

/// Settings › Updates: is there a newer version of this app?
#[tauri::command]
async fn app_update_check(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let current = app.package_info().version.to_string();
    // A failed check still reports this app's version, so Settings › Updates can show it.
    let found = match updater(&app)?.check().await {
        Ok(f) => f,
        Err(e) => return Ok(serde_json::json!({"current": current, "error": format!("Couldn't check for updates: {e}")})),
    };
    Ok(match found {
        Some(u) => serde_json::json!({"current": current, "latest": u.version, "available": true, "notes": u.body.unwrap_or_default()}),
        None => serde_json::json!({"current": current, "available": false}),
    })
}

/// Download the newer version (its signature is checked), put it in place of this app and restart.
#[tauri::command]
async fn app_update_install(app: tauri::AppHandle) -> Result<(), String> {
    let update = updater(&app)?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?
        .ok_or("This app is already the newest version.")?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| format!("Couldn't install the update: {e}"))?;
    app.restart();
}

fn main() {
    // Claude Code starts this same program with `mcp-approve` to ask the user for permission.
    if std::env::args().nth(1).as_deref() == Some("mcp-approve") {
        if let Err(e) = obot_core::mcp::run_approval_mcp() {
            eprintln!("oBot approval helper failed: {e}");
        }
        return;
    }
    // ...and with `hook <event>` at certain moments of a run (see obot_core::hooks).
    if std::env::args().nth(1).as_deref() == Some("hook") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(obot_core::hooks::run(&args));
    }

    obot_core::util::fix_path_env();

    let server = tauri::async_runtime::block_on(obot_core::start(obot_core::ServeOptions {
        host: "127.0.0.1".into(),
        port: 0,
        data_dir: None,
    }))
    .expect("oBot couldn't start its local server");

    let url = format!("http://127.0.0.1:{}/?token={}", server.addr.port(), server.token);

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().pubkey(obot_core::update::PUBLIC_KEY).build())
        .invoke_handler(tauri::generate_handler![notify, set_badge, is_focused, open_url, save_file, app_update_check, app_update_install, take_share])
        // Menu bar: oBot › Settings… (⌘,) and Connect to Remote Computer…
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
                "settings" => "window.obotOpenSettings && window.obotOpenSettings()",
                "remote" => "window.obotOpenSettings && window.obotOpenSettings('computer')",
                _ => return,
            };
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
                let _ = w.eval(call);
            }
        })
        .setup(move |app| {
            let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("oBot")
                .inner_size(1040.0, 760.0)
                .min_inner_size(720.0, 520.0)
                // oBot is always dark (no light mode), so the window frame and loading background match.
                .theme(Some(tauri::Theme::Dark))
                .background_color(tauri::window::Color(11, 11, 12, 255))
                // A link that asks for a new window opens in the normal web browser, never in a bare
                // oBot window.
                .on_new_window(|url, _| {
                    if matches!(url.scheme(), "http" | "https") {
                        let _ = os_open(url.as_str(), false);
                    }
                    NewWindowResponse::Deny
                });
            #[cfg(target_os = "macos")]
            let builder = builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
            builder.build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running oBot")
        .run(|_app, _event| {
            // "Send to oBot" in the user's browser opened obot://share#<page> (Info.plist registers
            // the obot:// scheme): bring the window forward and let the page take it.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &_event {
                if let Some(page) = urls.iter().find_map(share_of) {
                    if let Ok(mut s) = SHARED.lock() {
                        *s = Some(page);
                    }
                    if let Some(w) = _app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                        let _ = w.eval("window.obotShare && window.obotShare()");
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_links() {
        let page = |s: &str| share_of(&url::Url::parse(s).unwrap());
        assert_eq!(page("obot://share#zH4sIAAA-_x").as_deref(), Some("zH4sIAAA-_x"));
        assert_eq!(page("obot://share#q123"), None);
        assert_eq!(page("obot://share#z<script>"), None);
        assert_eq!(page("obot://other#z123"), None);
        assert_eq!(page("https://share#z123"), None);
    }

    #[test]
    fn config_uses_the_release_key() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["plugins"]["updater"]["pubkey"].as_str(), Some(obot_core::update::PUBLIC_KEY));
    }

    #[test]
    fn saved_names_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("oBot-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_path(&dir, "notes.txt"), dir.join("notes.txt"));
        std::fs::write(dir.join("notes.txt"), "a").unwrap();
        assert_eq!(free_path(&dir, "notes.txt"), dir.join("notes (1).txt"));
        // Only the file name is used, never a folder from the name.
        assert_eq!(free_path(&dir, "../../evil.sh"), dir.join("evil.sh"));
        assert_eq!(free_path(&dir, ""), dir.join("file"));
        assert_eq!(free_path(&dir, ".zshrc"), dir.join("file"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_documents_open_by_themselves() {
        assert!(safe_to_open(Path::new("/x/notes.TXT")));
        assert!(safe_to_open(Path::new("/x/report.pdf")));
        assert!(!safe_to_open(Path::new("/x/run.command")));
        assert!(!safe_to_open(Path::new("/x/setup.sh")));
        assert!(!safe_to_open(Path::new("/x/App.app")));
        assert!(!safe_to_open(Path::new("/x/noext")));
    }
}
