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
