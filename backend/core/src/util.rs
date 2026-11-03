use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn random_token(len: usize) -> String {
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// Cut a string to at most `max` characters (not bytes), adding "…" when cut.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// One-line preview used in the sidebar.
pub fn preview(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.replace(['*', '`', '#', '>'], "");
    let flat = flat.trim_start_matches(|c: char| c == '-' || c == '+' || c.is_whitespace());
    truncate_chars(flat.trim(), 120)
}

/// Mask a secret like "sk-ant-api03-abcdef" -> "sk-ant-…cdef".
pub fn hint(secret: &str) -> String {
    let n = secret.chars().count();
    if n == 0 {
        return String::new();
    }
    if n <= 8 {
        return "••••".into();
    }
    let head: String = secret.chars().take(6).collect();
    let tail: String = secret.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

/// On Windows, stop child processes from flashing a console window.
#[cfg(windows)]
pub fn hide_console(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
pub fn hide_console(_cmd: &mut tokio::process::Command) {}

/// Apps started from the macOS Finder get a tiny PATH (/usr/bin:/bin...).
/// Ask the user's login shell for the real PATH so `claude`, `git`, `node`... are found.
pub fn fix_path_env() {
    #[cfg(target_os = "macos")]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = std::process::Command::new(shell)
            .args(["-ilc", "printf '__OPENBOT_PATH__%s' \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(idx) = s.rfind("__OPENBOT_PATH__") {
                let p = s[idx + "__OPENBOT_PATH__".len()..].trim().to_string();
                if !p.is_empty() {
                    std::env::set_var("PATH", p);
                }
            }
        }
    }
}

/// Best guess at this machine's LAN IP (no packets are actually sent).
pub fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip().to_string())
}

// ---------------------------------------------------------------- timezone
// The user's chosen timezone (Settings › General › Timezone). None = this computer's own timezone.
static TZ: std::sync::RwLock<Option<chrono_tz::Tz>> = std::sync::RwLock::new(None);

/// Set the timezone by IANA name like "America/Los_Angeles" ("" = auto). Returns false if unknown.
pub fn set_timezone(name: &str) -> bool {
    let tz = if name.trim().is_empty() {
        None
    } else {
        match name.trim().parse::<chrono_tz::Tz>() {
            Ok(t) => Some(t),
            Err(_) => return false,
        }
    };
    *TZ.write().unwrap_or_else(|e| e.into_inner()) = tz;
    true
}

fn tz() -> Option<chrono_tz::Tz> {
    *TZ.read().unwrap_or_else(|e| e.into_inner())
}

/// Wall-clock time (in the user's timezone) for a unix ms timestamp.
pub fn to_local(ms: i64) -> Option<chrono::NaiveDateTime> {
    use chrono::TimeZone;
    match tz() {
        Some(t) => t.timestamp_millis_opt(ms).single().map(|d| d.naive_local()),
        None => chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.naive_local()),
    }
}

/// Unix ms for a wall-clock time in the user's timezone.
pub fn from_local(n: chrono::NaiveDateTime) -> Option<i64> {
    use chrono::TimeZone;
    match tz() {
        Some(t) => t.from_local_datetime(&n).earliest().map(|d| d.timestamp_millis()),
        None => chrono::Local.from_local_datetime(&n).earliest().map(|d| d.timestamp_millis()),
    }
}

pub fn now_local() -> chrono::NaiveDateTime {
    to_local(now_ms()).unwrap_or_else(|| chrono::Local::now().naive_local())
}

/// e.g. "America/Los_Angeles", or "this computer's timezone".
pub fn timezone_label() -> String {
    match tz() {
        Some(t) => t.name().to_string(),
        None => "this computer's timezone".into(),
    }
}

/// `C:\Users\me\x` -> `/mnt/c/Users/me/x` (how a Windows path looks from inside WSL).
pub fn to_wsl_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), &s[2..])
    } else {
        s
    }
}

/// `/mnt/c/Users/me/x` -> `C:\Users\me\x` on Windows (paths that bots running in WSL hand back).
/// Anything else is returned unchanged.
pub fn from_wsl_path(p: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        let b = p.as_bytes();
        if p.starts_with("/mnt/") && b.len() >= 6 && b[5].is_ascii_alphabetic() && (b.len() == 6 || b[6] == b'/') {
            let rest = if b.len() > 6 { &p[6..] } else { "" };
            return std::path::PathBuf::from(format!("{}:{}", (b[5] as char).to_ascii_uppercase(), if rest.is_empty() { "\\".to_string() } else { rest.replace('/', "\\") }));
        }
    }
    std::path::PathBuf::from(p)
}

#[cfg(test)]
mod wsl_path_tests {
    use super::*;
    #[test]
    fn to_wsl() {
        assert_eq!(to_wsl_path(std::path::Path::new(r"C:\Users\me\OpenBot\x")), "/mnt/c/Users/me/OpenBot/x");
        assert_eq!(to_wsl_path(std::path::Path::new(r"\\?\D:\a b")), "/mnt/d/a b");
        assert_eq!(to_wsl_path(std::path::Path::new("/home/me")), "/home/me");
    }
    #[test]
    fn from_wsl_non_windows_is_unchanged() {
        if !cfg!(windows) {
            assert_eq!(from_wsl_path("/mnt/c/x"), std::path::PathBuf::from("/mnt/c/x"));
        }
    }
}
