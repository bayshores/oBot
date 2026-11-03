// Embeds every file in frontend/ui into the program, so the server works without the folder.
use std::io::Write;

fn main() {
    let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/ui");
    let ui = std::fs::canonicalize(&ui).expect("ui folder missing");
    println!("cargo:rerun-if-changed={}", ui.display());
    let mut entries: Vec<_> = std::fs::read_dir(&ui)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("ui_assets.rs");
    let mut f = std::fs::File::create(out).unwrap();
    writeln!(f, "pub static UI_FILES: &[(&str, &[u8])] = &[").unwrap();
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        println!("cargo:rerun-if-changed={}", e.path().display());
        writeln!(f, "    ({:?}, include_bytes!({:?})),", name, e.path().display().to_string()).unwrap();
    }
    writeln!(f, "];").unwrap();
}
