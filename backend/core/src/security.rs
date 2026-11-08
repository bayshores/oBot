//! Remote-access security: device pairing, per-device tokens, Origin/Host checks and
//! a failed-login rate limiter.
//!
//! How a new device gets access:
//!   1. On the host PC, `obot-server pair` (or the startup banner, or Settings) creates a
//!      6-digit pairing code. It works once, for 10 minutes, and dies after 5 wrong tries.
//!   2. The other device sends the code to `POST /api/pair`. It gets back its own long random
//!      token. The server only keeps a SHA-256 hash of that token.
//!   3. Each device can be revoked on its own (`DELETE /api/devices/{id}`).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use axum::http::{header, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::util::{now_ms, random_token};

pub const PAIR_CODE_TTL_MS: i64 = 10 * 60 * 1000;
pub const PAIR_CODE_MAX_TRIES: u32 = 5;
const FAIL_WINDOW_MS: i64 = 60 * 1000;
const FAIL_LIMIT: u32 = 20;
const BLOCK_MS: i64 = 5 * 60 * 1000;

/// A device that was paired with this computer.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// SHA-256 (hex) of the device's token. The token itself is never stored.
    pub token_hash: String,
    pub created_at: i64,
    pub last_seen: i64,
}

pub fn sha256_hex(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A brand-new device token (shown to the device once).
pub fn new_device_token() -> String {
    format!("obd_{}", random_token(40))
}

// ---------------------------------------------------------------- pairing codes

/// Kept in `<data dir>/pairing.json` so the `obot-server pair` command and the running
/// server (two different programs) share it.
#[derive(Serialize, Deserialize, Default)]
struct PairFile {
    code_hash: String,
    expires_at: i64,
    tries: u32,
}

fn pair_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("pairing.json")
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
}

/// Make a new 6-digit pairing code (replaces any older one). Returns (code, expires_at ms).
pub fn new_pairing_code(data_dir: &Path) -> anyhow::Result<(String, i64)> {
    use rand::Rng;
    let code = format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000u32));
    let expires_at = now_ms() + PAIR_CODE_TTL_MS;
    let f = PairFile { code_hash: sha256_hex(&code), expires_at, tries: 0 };
    std::fs::create_dir_all(data_dir)?;
    write_private(&pair_path(data_dir), &serde_json::to_vec(&f)?)?;
    Ok((code, expires_at))
}

pub enum PairCheck {
    Ok,
    Wrong { tries_left: u32 },
    /// No code, expired, or used up.
    NoCode,
}

/// Check a pairing code. A right code is used up; 5 wrong ones kill it.
pub fn check_pairing_code(data_dir: &Path, code: &str) -> PairCheck {
    let path = pair_path(data_dir);
    let Ok(bytes) = std::fs::read(&path) else { return PairCheck::NoCode };
    let Ok(mut f) = serde_json::from_slice::<PairFile>(&bytes) else {
        let _ = std::fs::remove_file(&path);
        return PairCheck::NoCode;
    };
    if f.code_hash.is_empty() || now_ms() > f.expires_at || f.tries >= PAIR_CODE_MAX_TRIES {
        let _ = std::fs::remove_file(&path);
        return PairCheck::NoCode;
    }
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if constant_eq(&sha256_hex(&code), &f.code_hash) {
        let _ = std::fs::remove_file(&path);
        return PairCheck::Ok;
    }
    f.tries += 1;
    if f.tries >= PAIR_CODE_MAX_TRIES {
        let _ = std::fs::remove_file(&path);
        return PairCheck::Wrong { tries_left: 0 };
    }
    let _ = write_private(&path, &serde_json::to_vec(&f).unwrap_or_default());
    PairCheck::Wrong { tries_left: PAIR_CODE_MAX_TRIES - f.tries }
}

// ---------------------------------------------------------------- rate limiting

#[derive(Default)]
pub struct FailRec {
    window_start: i64,
    count: u32,
    blocked_until: i64,
}

/// Counts failed logins / pairing tries per client. 20 failures in a minute → blocked 5 minutes.
#[derive(Default)]
pub struct RateLimiter {
    map: std::sync::Mutex<HashMap<String, FailRec>>,
}

impl RateLimiter {
    /// Seconds left on a block, if this client is blocked.
    pub fn blocked(&self, key: &str) -> Option<i64> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        map.get(key).filter(|r| r.blocked_until > now).map(|r| (r.blocked_until - now + 999) / 1000)
    }

    pub fn fail(&self, key: &str) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        if map.len() > 10_000 {
            map.retain(|_, r| r.blocked_until > now || now - r.window_start < FAIL_WINDOW_MS);
        }
        let r = map.entry(key.to_string()).or_default();
        if now - r.window_start > FAIL_WINDOW_MS {
            r.window_start = now;
            r.count = 0;
        }
        r.count += 1;
        if r.count >= FAIL_LIMIT {
            r.blocked_until = now + BLOCK_MS;
            tracing::warn!("too many failed logins from {key}; blocking it for 5 minutes");
        }
    }
}

/// Who is calling, for rate limiting. Behind Tailscale Serve every request comes from
/// 127.0.0.1, so then we use the first X-Forwarded-For address (only trusted from loopback).
pub fn client_key(peer: Option<SocketAddr>, headers: &HeaderMap) -> String {
    let ip = peer.map(|p| p.ip());
    if ip.map_or(true, |i| i.is_loopback()) {
        if let Some(f) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return f.to_string();
        }
    }
    ip.map(|i| i.to_string()).unwrap_or_else(|| "unknown".into())
}

// ---------------------------------------------------------------- Origin / Host checks

fn split_host_port(hostport: &str) -> (&str, Option<&str>) {
    if let Some(rest) = hostport.strip_prefix('[') {
        // [::1]:7777
        if let Some(end) = rest.find(']') {
            let host = &rest[..end];
            let port = rest[end + 1..].strip_prefix(':');
            return (host, port);
        }
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (hostport, None),
    }
}

fn is_loopback_name(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().map_or(false, |i| i.is_loopback())
}

/// Is the Host header one we expect? Blocks DNS-rebinding tricks, where a web page's own
/// domain is pointed at this computer.
pub fn host_allowed(headers: &HeaderMap) -> bool {
    let Some(h) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else { return true };
    let (host, _) = split_host_port(h.trim());
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || is_loopback_name(&host) || host.parse::<IpAddr>().is_ok() {
        return true;
    }
    if host.ends_with(".ts.net") {
        return true;
    }
    if let Some(name) = sysinfo::System::host_name() {
        let name = name.to_ascii_lowercase();
        if host == name || host == format!("{name}.local") {
            return true;
        }
    }
    if let Ok(extra) = std::env::var("OBOT_ALLOWED_HOSTS") {
        if extra.split(',').map(|s| s.trim().to_ascii_lowercase()).any(|s| !s.is_empty() && s == host) {
            return true;
        }
    }
    false
}

/// May a page from `origin` call this server? Allowed: this server's own address (same Host),
/// oBot pages on this computer (http://127.0.0.1:*, http://localhost:*) and the desktop app.
pub fn origin_allowed(origin: &HeaderValue, headers: &HeaderMap) -> bool {
    let Ok(origin) = origin.to_str() else { return false };
    let origin = origin.trim().to_ascii_lowercase();
    if matches!(origin.as_str(), "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost") {
        return true;
    }
    let Some((scheme, rest)) = origin.split_once("://") else { return false };
    if scheme != "http" && scheme != "https" {
        return false;
    }
    let (ohost, _) = split_host_port(rest);
    if scheme == "http" && is_loopback_name(ohost) {
        return true;
    }
    // Same origin as the address the request came in on (e.g. https://my-pc.tailnet.ts.net).
    if let Some(h) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        if host_allowed(headers) && rest == h.trim().to_ascii_lowercase() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        h
    }

    #[test]
    fn origins() {
        let local = hm("127.0.0.1:7777");
        let ok = |o: &str, h: &HeaderMap| origin_allowed(&HeaderValue::from_str(o).unwrap(), h);
        assert!(ok("http://127.0.0.1:51234", &local));
        assert!(ok("http://localhost:3000", &local));
        assert!(ok("tauri://localhost", &local));
        assert!(!ok("https://evil.example", &local));
        assert!(!ok("http://127.0.0.1.evil.example", &local));
        assert!(!ok("null", &local));
        let ts = hm("my-pc.tail1234.ts.net");
        assert!(ok("https://my-pc.tail1234.ts.net", &ts));
        assert!(!ok("https://other.tail1234.ts.net", &ts));
        // DNS rebinding: evil.example pointed at us → Host is evil.example → refused.
        let rebind = hm("evil.example:7777");
        assert!(!host_allowed(&rebind));
        assert!(!ok("http://evil.example:7777", &rebind));
        assert!(host_allowed(&hm("[::1]:7777")));
        assert!(host_allowed(&hm("100.71.1.2:7777")));
    }

    #[test]
    fn pairing_code_flow() {
        let dir = std::env::temp_dir().join(format!("oBot-pair-test-{}", random_token(8)));
        let (code, _) = new_pairing_code(&dir).unwrap();
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert!(matches!(check_pairing_code(&dir, wrong), PairCheck::Wrong { tries_left: 4 }));
        assert!(matches!(check_pairing_code(&dir, &code), PairCheck::Ok));
        // single use
        assert!(matches!(check_pairing_code(&dir, &code), PairCheck::NoCode));
        // dies after 5 wrong tries
        let (code, _) = new_pairing_code(&dir).unwrap();
        let wrong = if code == "000000" { "000001" } else { "000000" };
        for _ in 0..5 {
            let _ = check_pairing_code(&dir, wrong);
        }
        assert!(matches!(check_pairing_code(&dir, &code), PairCheck::NoCode));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn limiter_blocks() {
        let l = RateLimiter::default();
        for _ in 0..19 {
            l.fail("1.2.3.4");
        }
        assert!(l.blocked("1.2.3.4").is_none());
        l.fail("1.2.3.4");
        assert!(l.blocked("1.2.3.4").is_some());
        assert!(l.blocked("5.6.7.8").is_none());
    }
}
