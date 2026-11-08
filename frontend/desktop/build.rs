// Build script for the desktop app. Lists the app's own commands (main.rs) so Tauri makes an
// "allow-<command>" permission for each; capabilities/default.json grants them to the page.
// Without this, Tauri refuses these commands from the page (it is served by the built-in local
// server, which Tauri treats as a remote address).
fn main() {
    let commands = tauri_build::AppManifest::new().commands(&["notify", "set_badge", "is_focused", "open_url", "save_file"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands)).expect("tauri build step failed");
}
