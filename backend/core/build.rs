// Embeds every file in frontend/ui (including subfolders like vendor/novnc) into the program,
// so the server works without the folder.
use std::io::Write;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            println!("cargo:rerun-if-changed={}", p.display());
            collect(&p, root, out);
        } else if p.is_file() {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, p));
        }
    }
}

fn main() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/ui");
    let ui = std::fs::canonicalize(&ui).expect("ui folder missing");
    println!("cargo:rerun-if-changed={}", ui.display());
    let mut files = vec![];
    collect(&ui, &ui, &mut files);
    // The phone home-screen icons (generated sizes live with the desktop app's icons).
    let icons = ui.join("../desktop/icons");
    for n in [180, 192, 512] {
        files.push((format!("icons/phone-{n}.png"), icons.join(format!("phone-{n}.png"))));
    }
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("ui_assets.rs");
    let mut f = std::fs::File::create(out).unwrap();
    writeln!(f, "pub static UI_FILES: &[(&str, &[u8])] = &[").unwrap();
    for (rel, p) in files {
        println!("cargo:rerun-if-changed={}", p.display());
        writeln!(f, "    ({:?}, include_bytes!({:?})),", rel, p.display().to_string()).unwrap();
    }
    writeln!(f, "];").unwrap();
}
