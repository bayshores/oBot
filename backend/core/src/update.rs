//! oBot updates from GitHub Releases (github.com/bayshores/oBot/releases).
//!
//! The computer running obot-server checks the newest release. On Windows it can also replace its
//! own program with that release's `obot-server.exe` and start again: the file must carry a valid
//! signature from oBot's release key (`obot-server.exe.sig`), and the server must run under
//! `obot-server --supervise` (the installer sets that up), which starts the new program at once.
//! The Mac app updates itself with the same key and release (see frontend/desktop).

use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where releases are published.
pub const REPO: &str = "bayshores/oBot";
/// Public half of the key every release file is signed with (minisign, base64, as Tauri's signer
/// writes it). The private half is the TAURI_SIGNING_PRIVATE_KEY secret of the GitHub repository.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEM3M0IxQTA5RDAzRjk5MTgKUldRWW1UL1FDUm83eDZDMEV3S3VMMlcwYXBQY01ZU3czL0lhMFh1T2hyblFyZmlGZnFHVDQ3SUoK";
/// Exit code that asks the supervisor (`obot-server --supervise`) to start the program again now.
pub const RESTART_CODE: i32 = 75;
/// Set by the supervisor for the program it starts.
pub const SUPERVISED_ENV: &str = "OBOT_SUPERVISED";

/// The newest release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// This computer's program in the release, and its signature.
    pub program: Option<(String, String)>,
}

/// The release file with the server program for this computer.
pub fn program_name() -> Option<&'static str> {
    if cfg!(windows) {
        Some("obot-server.exe")
    } else if cfg!(target_os = "linux") {
        // Not published; lets the test computer try updates with its own test releases.
        Some("obot-server-linux")
    } else {
        None
    }
}

fn api_base() -> String {
    // Development builds can check a stand-in release server.
    #[cfg(debug_assertions)]
    if let Ok(u) = std::env::var("OBOT_UPDATE_API") {
        return u.trim_end_matches('/').to_string();
    }
    "https://api.github.com".into()
}

fn public_key() -> String {
    #[cfg(debug_assertions)]
    if let Ok(k) = std::env::var("OBOT_UPDATE_PUBKEY") {
        return k;
    }
    PUBLIC_KEY.to_string()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(300)).user_agent("oBot").build()
}

/// Ask GitHub for the newest release.
pub fn latest() -> Result<Release, String> {
    let url = format!("{}/repos/{REPO}/releases/latest", api_base());
    let v: Value = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Couldn't check for updates: {e}"))?
        .into_json()
        .map_err(|e| format!("Couldn't read the release list: {e}"))?;
    Ok(parse_release(&v))
}

fn parse_release(v: &Value) -> Release {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let assets: Vec<(String, String)> = v
        .get("assets")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| Some((x.get("name")?.as_str()?.to_string(), x.get("browser_download_url")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let url_of = |name: &str| assets.iter().find(|(n, _)| n == name).map(|(_, u)| u.clone());
    let program = program_name().and_then(|n| Some((url_of(n)?, url_of(&format!("{n}.sig"))?)));
    Release { version: s("tag_name").trim_start_matches('v').to_string(), notes: s("body"), page: s("html_url"), program }
}

/// Is version `a` newer than `b`? ("0.10.0" > "0.9.3"; anything after "-" is ignored.)
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split('-').next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (mut a, mut b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// Check `data` against a signature made by Tauri's signer with the release key.
pub fn verify(data: &[u8], signature: &str, key: &str) -> Result<(), String> {
    let text = |b64: &str| -> Result<String, String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| "unreadable signature or key".to_string())?;
        String::from_utf8(raw).map_err(|_| "unreadable signature or key".to_string())
    };
    let key = minisign_verify::PublicKey::decode(&text(key)?).map_err(|e| format!("bad release key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&text(signature)?).map_err(|e| format!("bad signature: {e}"))?;
    key.verify(data, &sig, true).map_err(|_| "The download's signature doesn't match oBot's release key".to_string())
}

/// What the app shows in Settings › Updates for this computer.
pub fn status(current: &str) -> Value {
    match latest() {
        Ok(r) => {
            let available = newer(&r.version, current);
            let (can, why) = can_install(&r);
            json!({"current": current, "latest": r.version, "available": available, "notes": r.notes, "page": r.page,
                   "can_install": available && can, "why_not": if available { why } else { String::new() }})
        }
        Err(e) => json!({"current": current, "error": e}),
    }
}

fn can_install(r: &Release) -> (bool, String) {
    if public_key().is_empty() {
        return (false, "This build of oBot can't check update signatures.".into());
    }
    if r.program.is_none() {
        return (false, "This release has no oBot program for this computer.".into());
    }
    if std::env::var(SUPERVISED_ENV).as_deref() != Ok("1") {
        return (false, "Run the oBot installer on this computer once more to turn on updates from the app.".into());
    }
    (true, String::new())
}

/// Download the newest release's program, check its signature and put it in place of this one.
/// The caller then exits with [`RESTART_CODE`] so the supervisor starts it.
pub fn install(current: &str) -> Result<String, String> {
    let r = latest()?;
    if !newer(&r.version, current) {
        return Err(format!("oBot {current} is already the newest version."));
    }
    let (ok, why) = can_install(&r);
    if !ok {
        return Err(why);
    }
    let (url, sig_url) = r.program.clone().unwrap_or_default();
    let get = |u: &str| -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        let resp = agent().get(u).call().map_err(|e| format!("Couldn't download the update: {e}"))?;
        std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 512 << 20), &mut buf).map_err(|e| format!("Couldn't download the update: {e}"))?;
        Ok(buf)
    };
    let program = get(&url)?;
    let signature = String::from_utf8(get(&sig_url)?).map_err(|_| "unreadable signature".to_string())?;
    verify(&program, &signature, &public_key())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    replace_program(&exe, &program)?;
    Ok(r.version)
}

/// Put `program` at `exe`. A running program can't be overwritten on Windows but can be renamed,
/// so the old one is moved aside (and deleted at a later start, see [`clean_old`]).
fn replace_program(exe: &Path, program: &[u8]) -> Result<(), String> {
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let new = exe.with_file_name(format!("{name}.new"));
    let old = exe.with_file_name(format!("{name}.old-{}", crate::util::now_ms()));
    std::fs::write(&new, program).map_err(|e| format!("Couldn't save the update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        format!("Couldn't move the old program aside: {e}")
    })?;
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("Couldn't put the update in place: {e}"));
    }
    Ok(())
}

/// Delete programs left behind by earlier updates (ones still running stay until next time).
pub fn clean_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().map(|n| n.to_string_lossy().into_owned())) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with(&format!("{name}.old-")) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// `obot-server --supervise ...`: run the server (with the other arguments) and start it again
/// when it asks to (after an update) or stops unexpectedly. Always starts the program file at the
/// original path, so after an update the new program runs.
pub fn supervise(args: Vec<String>) -> ! {
    let exe: PathBuf = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("obot-server"));
    loop {
        let status = std::process::Command::new(&exe).args(&args).env(SUPERVISED_ENV, "1").status();
        match status.as_ref().ok().and_then(|s| s.code()) {
            Some(RESTART_CODE) => continue,
            Some(0) => std::process::exit(0),
            _ => {
                eprintln!("obot-server stopped ({status:?}); starting it again in 5 seconds");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.0", "0.99.99"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("0.2.0-beta", "0.2.0"));
    }

    #[test]
    fn release_lists_this_computers_program() {
        let v = json!({"tag_name": "v0.3.1", "body": "Fixes", "html_url": "https://x/r", "assets": [
            {"name": "obot-server.exe", "browser_download_url": "https://x/w.exe"},
            {"name": "obot-server.exe.sig", "browser_download_url": "https://x/w.exe.sig"},
            {"name": "obot-server-linux", "browser_download_url": "https://x/l"},
            {"name": "obot-server-linux.sig", "browser_download_url": "https://x/l.sig"}]});
        let r = parse_release(&v);
        assert_eq!(r.version, "0.3.1");
        if cfg!(windows) {
            assert_eq!(r.program, Some(("https://x/w.exe".into(), "https://x/w.exe.sig".into())));
        } else if cfg!(target_os = "linux") {
            assert_eq!(r.program, Some(("https://x/l".into(), "https://x/l.sig".into())));
        }
    }

    // A throwaway key made only for this test (`cargo tauri signer generate`), and its signature
    // of "oBot update test".
    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ2RjcxNTUyN0I4NUIxMkMKUldRc3NZVjdVaFgzMXIwNjYvQ1ZYQ2t2M29JSytHZlVGSFhEdzlUVEQ4SG1maHZhTHg1TGpsc3EK";
    const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRc3NZVjdVaFgzMW50QytxSWVteFYrSXl3cWVESy9UTXd5Vm8xNFhndXFZVDNjbkorbmtGUXhQNlNzeTlaMXF2N3crSi9kWkZGY05GcEJtLzNqTlFxaWhmQjhmUVJBMEFnPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0MTIxMDI3CWZpbGU6ZGF0YS5iaW4KK21FYjNJMUVmd0labWMybGlEMDVTTlJ3N1M5Y0Qvb1hGZkY0dmNlMzRIYnl1U1M3NkdSMHI1Yld2M3V5UzdyVFhHbmdTdWJZKzZ4R2o1Y0poNjl1Q2c9PQo=";

    #[test]
    fn only_signed_downloads_pass() {
        assert!(verify(b"oBot update test", TEST_SIG, TEST_KEY).is_ok());
        assert!(verify(b"oBot update test!", TEST_SIG, TEST_KEY).is_err());
        assert!(verify(b"oBot update test", "bm90IGEgc2lnbmF0dXJl", TEST_KEY).is_err());
    }

    #[test]
    fn replacing_keeps_the_old_program_aside() {
        let dir = std::env::temp_dir().join(format!("obot-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("obot-server");
        std::fs::write(&exe, "old").unwrap();
        replace_program(&exe, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        let olds = std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains(".old-")).count();
        assert_eq!(olds, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
