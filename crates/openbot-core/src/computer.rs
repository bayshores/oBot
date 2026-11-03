//! Facts about the computer the bots run on, and finding the `claude` program.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::process::Command;

use crate::state::AppState;
use crate::util::hide_console;

/// Find the Claude Code program. Order: setting → PATH → common install folders.
pub fn find_claude(override_path: &str) -> Option<PathBuf> {
    if !override_path.trim().is_empty() {
        let p = PathBuf::from(override_path.trim());
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(p) = which::which("claude") {
        return Some(p);
    }
    let home = dirs::home_dir()?;
    let mut candidates: Vec<PathBuf> = vec![
        home.join(".local/bin/claude"),
        home.join(".claude/local/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ];
    if cfg!(windows) {
        candidates.insert(0, home.join(".local\\bin\\claude.exe"));
        if let Ok(appdata) = std::env::var("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("npm\\claude.cmd"));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

async fn run_quick(path: &PathBuf, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    hide_console(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(15), cmd.output()).await.ok()?.ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

static CLAUDE_CACHE: std::sync::Mutex<Option<(std::time::Instant, String, Value)>> = std::sync::Mutex::new(None);

/// Version + login status of Claude Code on this computer (cached for 30 seconds).
pub async fn claude_info(override_path: &str) -> Value {
    if let Ok(c) = CLAUDE_CACHE.lock() {
        if let Some((at, key, v)) = c.as_ref() {
            if key == override_path && at.elapsed() < Duration::from_secs(30) {
                return v.clone();
            }
        }
    }
    let v = claude_info_uncached(override_path).await;
    if let Ok(mut c) = CLAUDE_CACHE.lock() {
        *c = Some((std::time::Instant::now(), override_path.to_string(), v.clone()));
    }
    v
}

async fn claude_info_uncached(override_path: &str) -> Value {
    let Some(path) = find_claude(override_path) else {
        return json!({ "installed": false, "path": "", "version": "", "logged_in": false,
                       "email": "", "auth_method": "", "subscription": "" });
    };
    let (version, status) = tokio::join!(
        run_quick(&path, &["--version"]),
        run_quick(&path, &["auth", "status"])
    );
    let status: Value = status
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    json!({
        "installed": true,
        "path": path.to_string_lossy(),
        "version": version.unwrap_or_default().replace(" (Claude Code)", ""),
        "logged_in": status.get("loggedIn").and_then(Value::as_bool).unwrap_or(false),
        "email": status.get("email").and_then(Value::as_str).unwrap_or(""),
        "auth_method": status.get("authMethod").and_then(Value::as_str).unwrap_or(""),
        "subscription": status.get("subscriptionType").and_then(Value::as_str).unwrap_or(""),
    })
}

pub async fn computer_json(state: &AppState) -> Value {
    let claude_path = state.config.read().await.claude_path.clone();
    let claude = claude_info(&claude_path).await;
    let sys = tokio::task::spawn_blocking(|| {
        let mut s = sysinfo::System::new();
        s.refresh_memory();
        s.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
        (s.total_memory(), s.used_memory(), s.cpus().len())
    })
    .await
    .unwrap_or((0, 0, 0));
    let listen = state.listen.clone();
    let loopback = listen.starts_with("127.") || listen.starts_with("localhost") || listen.starts_with("[::1]");
    json!({
        "hostname": sysinfo::System::host_name().unwrap_or_default(),
        "os": sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.into()),
        "os_version": sysinfo::System::os_version().unwrap_or_default(),
        "arch": std::env::consts::ARCH,
        "uptime_secs": sysinfo::System::uptime(),
        "cpu_count": sys.2,
        "mem_total": sys.0,
        "mem_used": sys.1,
        "is_remote_capable": !loopback,
        "listen": listen,
        "data_dir": state.data_dir.to_string_lossy(),
        "claude": claude,
    })
}
