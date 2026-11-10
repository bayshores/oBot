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
/// A file path in a Bot's folder as it goes in a web address ("shown/a b.png" -> "shown/a%20b.png").
pub fn url_path(rel: &str) -> String {
    let mut out = String::new();
    for c in rel.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(c as char),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

pub fn preview(s: &str) -> String {
    let flat = without_pictures(s).split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.replace(['*', '`', '#', '>'], "");
    let flat = flat.trim_start_matches(|c: char| c == '-' || c == '+' || c.is_whitespace());
    truncate_chars(flat.trim(), 120)
}

/// "Here ![Blue jeans](/api/…) look" -> "Here 🖼️ Blue jeans look" (for previews and notifications).
fn without_pictures(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("![") {
        let after = &rest[i + 2..];
        let Some(close) = after.find("](") else { break };
        let Some(end) = after[close + 2..].find(')') else { break };
        out.push_str(&rest[..i]);
        let alt = after[..close].trim();
        out.push_str(if alt.is_empty() { "🖼️ Picture" } else { "🖼️ " });
        out.push_str(alt);
        rest = &after[close + 2 + end + 1..];
    }
    out.push_str(rest);
    out
}

/// "A", "A and B", "A, B, and C".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Mask a secret so only its last 4 characters show: "sk-ant-api03-abcdef" -> "••••cdef".
/// Values of 8 characters or fewer show nothing but dots (4 would be half of it).
pub fn hint(secret: &str) -> String {
    let n = secret.chars().count();
    if n == 0 {
        return String::new();
    }
    if n <= 8 {
        return "••••".into();
    }
    let tail: String = secret.chars().skip(n - 4).collect();
    format!("••••{tail}")
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
            .args(["-ilc", "printf '__OBOT_PATH__%s' \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(idx) = s.rfind("__OBOT_PATH__") {
                let p = s[idx + "__OBOT_PATH__".len()..].trim().to_string();
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

/// A unix-ms time in the user's timezone (falls back to UTC).
pub fn local_time(ms: i64) -> chrono::NaiveDateTime {
    to_local(ms).unwrap_or_else(|| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().naive_utc())
}

pub fn now_local() -> chrono::NaiveDateTime {
    to_local(now_ms()).unwrap_or_else(|| chrono::Local::now().naive_local())
}

/// The user's timezone as an IANA name like "America/Los_Angeles" (Settings, else this computer's).
pub fn timezone_name() -> Option<String> {
    match tz() {
        Some(t) => Some(t.name().to_string()),
        None => iana_time_zone::get_timezone().ok().filter(|n| n.parse::<chrono_tz::Tz>().is_ok()),
    }
}

/// e.g. "America/Los_Angeles", or "this computer's timezone".
pub fn timezone_label() -> String {
    timezone_name().unwrap_or_else(|| "this computer's timezone".into())
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
        assert_eq!(to_wsl_path(std::path::Path::new(r"C:\Users\me\oBot\x")), "/mnt/c/Users/me/oBot/x");
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
