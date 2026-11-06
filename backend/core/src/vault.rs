//! The Vault's safety rules: secret values (and the Anthropic API key) are saved encrypted in
//! config.json, each secret goes only to the Bots it is meant for, names that would break Bots are
//! refused, and saved values are replaced with •••• anywhere they show up in chats.
//!
//! Encryption: one random 256-bit key per data folder encrypts every value with AES-256-GCM. The
//! key itself is kept by the system:
//! - Windows: protected with DPAPI (machine scope) in `vault.key`. Machine scope keeps working when
//!   the server runs as a background task with no one logged in; a copy of the files taken to
//!   another computer can't be decrypted.
//! - macOS: in the login Keychain (`vault.key` only says which Keychain item).
//! - Linux: in `vault.key`, readable only by your user account.
//!
//! If the key can't be read, saved values stay encrypted ("locked") instead of being lost, Bots
//! don't get them, and Settings › Vault asks for them again.

use std::path::Path;
use std::sync::{OnceLock, RwLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::config::{Config, Secret};

/// Start of an encrypted value in config.json.
const SEALED: &str = "obenc1:";
/// Values shorter than this are not hidden in chats (too likely to match ordinary words).
const MIN_REDACT: usize = 6;

struct VaultKey {
    key: Option<[u8; 32]>,
    /// Shown in Settings › Vault, e.g. "Encrypted with Windows DPAPI".
    label: String,
}

static KEY: OnceLock<VaultKey> = OnceLock::new();
/// Every saved value, longest first, so it can be hidden in chats.
static KNOWN: RwLock<Vec<String>> = RwLock::new(Vec::new());

// ------------------------------------------------------------------ the key

/// Load (or create) this data folder's key. Call before reading config.json; later calls do nothing.
pub fn init(data_dir: &Path) {
    KEY.get_or_init(|| {
        let vk = match load_or_create_key(data_dir) {
            Ok((key, label)) => match self_test(&key) {
                Ok(()) => VaultKey { key: Some(key), label },
                Err(e) => VaultKey { key: None, label: format!("Not encrypted: the encryption self-test failed ({e})") },
            },
            Err(e) => VaultKey { key: None, label: format!("Not encrypted: {e}") },
        };
        match vk.key {
            Some(_) => tracing::info!("vault: {}", vk.label),
            None => tracing::warn!("vault: {}", vk.label),
        }
        vk
    });
}

/// How saved values are protected, in plain words.
pub fn protection() -> String {
    KEY.get().map(|k| k.label.clone()).unwrap_or_else(|| "Not encrypted".into())
}

fn key() -> Option<&'static [u8; 32]> {
    KEY.get().and_then(|k| k.key.as_ref())
}

fn self_test(key: &[u8; 32]) -> anyhow::Result<()> {
    let probe = "openbot vault self-test";
    let sealed = encrypt(key, probe)?;
    anyhow::ensure!(decrypt(key, &sealed).as_deref() == Some(probe), "round trip mismatch");
    Ok(())
}

/// `vault.key` holds one line: `dpapi:<base64>` (Windows), `keychain:<item name>` (macOS) or
/// `file:<base64>` (the key itself, file readable only by this user).
fn load_or_create_key(dir: &Path) -> anyhow::Result<([u8; 32], String)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("vault.key");
    if !path.exists() {
        let key: [u8; 32] = rand::random();
        let line = store_key(&key)?;
        // create_new: if another openbot process made a key at the same moment, use that one.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(line.as_bytes())?;
                restrict(&path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let line = std::fs::read_to_string(&path)?;
    read_key(line.trim()).map_err(|e| anyhow::anyhow!("couldn't unlock the Vault key in {} ({e})", path.display()))
}

/// Keep the key with the best storage this computer has. Returns the line for vault.key.
fn store_key(key: &[u8; 32]) -> anyhow::Result<String> {
    #[cfg(windows)]
    {
        match dpapi::protect(key) {
            Ok(blob) => return Ok(format!("dpapi:{}", B64.encode(blob))),
            Err(e) => tracing::warn!("vault: DPAPI unavailable ({e}); keeping the key in a private file"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let item = format!("vault-key-{}", crate::util::random_token(12));
        match keychain::store(&item, key) {
            Ok(()) => return Ok(format!("keychain:{item}")),
            Err(e) => tracing::warn!("vault: Keychain unavailable ({e}); keeping the key in a private file"),
        }
    }
    Ok(format!("file:{}", B64.encode(key)))
}

fn read_key(line: &str) -> anyhow::Result<([u8; 32], String)> {
    let to_key = |v: Vec<u8>| -> anyhow::Result<[u8; 32]> {
        v.try_into().map_err(|_| anyhow::anyhow!("the key has the wrong length"))
    };
    if let Some(b) = line.strip_prefix("file:") {
        return Ok((to_key(B64.decode(b)?)?, "Encrypted. The key is in a file only your user account can read".into()));
    }
    #[cfg(windows)]
    if let Some(b) = line.strip_prefix("dpapi:") {
        return Ok((to_key(dpapi::unprotect(&B64.decode(b)?)?)?, "Encrypted with Windows DPAPI".into()));
    }
    #[cfg(target_os = "macos")]
    if let Some(item) = line.strip_prefix("keychain:") {
        return Ok((to_key(keychain::load(item)?)?, "Encrypted. The key is in your macOS Keychain".into()));
    }
    anyhow::bail!("unknown key type")
}

fn restrict(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    const ENTROPY: &[u8] = b"openbot vault key v1";

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    fn run(data: &[u8], protect: bool) -> anyhow::Result<Vec<u8>> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: the input blobs point at live slices; DPAPI allocates `out` with LocalAlloc,
        // which we copy and then free.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN | CRYPTPROTECT_LOCAL_MACHINE,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 || out.pbData.is_null() {
            anyhow::bail!("{}", std::io::Error::last_os_error());
        }
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe { LocalFree(out.pbData as _) };
        Ok(v)
    }

    pub fn protect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        run(data, true)
    }
    pub fn unprotect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        run(data, false)
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    //! Uses Apple's `security` tool, so the Keychain item belongs to that tool and stays readable
    //! after openbot is rebuilt or updated (no "allow access" pop-ups).
    use std::process::Command;

    const SERVICE: &str = "openbot";

    pub fn store(item: &str, key: &[u8; 32]) -> anyhow::Result<()> {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let out = Command::new("/usr/bin/security")
            .args(["add-generic-password", "-s", SERVICE, "-a", item, "-l", "openbot Vault key", "-w", &hex])
            .output()?;
        anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(())
    }

    pub fn load(item: &str) -> anyhow::Result<Vec<u8>> {
        let out = Command::new("/usr/bin/security").args(["find-generic-password", "-s", SERVICE, "-a", item, "-w"]).output()?;
        anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
        let hex = String::from_utf8_lossy(&out.stdout).trim().to_string();
        anyhow::ensure!(hex.len() == 64, "the Keychain item is not a key");
        (0..32)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(Into::into))
            .collect()
    }
}

// ------------------------------------------------------------------ values

fn encrypt(key: &[u8; 32], plain: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce: [u8; 12] = rand::random();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    let mut all = nonce.to_vec();
    all.extend(ct);
    Ok(format!("{SEALED}{}", B64.encode(all)))
}

fn decrypt(key: &[u8; 32], sealed: &str) -> Option<String> {
    let all = B64.decode(sealed.strip_prefix(SEALED)?).ok()?;
    if all.len() < 12 {
        return None;
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plain = cipher.decrypt(Nonce::from_slice(&all[..12]), &all[12..]).ok()?;
    String::from_utf8(plain).ok()
}

/// True when a value is still encrypted because the key couldn't unlock it.
pub fn is_locked(value: &str) -> bool {
    value.starts_with(SEALED)
}

/// The value as written to config.json (encrypted when a key is available).
pub fn seal(value: &str) -> String {
    if value.is_empty() || is_locked(value) {
        return value.to_string();
    }
    match key() {
        Some(k) => encrypt(k, value).unwrap_or_else(|_| value.to_string()),
        None => value.to_string(),
    }
}

/// The value read from config.json. Plain text (older files) is returned as is; a value the key
/// can't open stays encrypted (see `is_locked`).
pub fn open(stored: &str) -> String {
    if !is_locked(stored) {
        return stored.to_string();
    }
    key().and_then(|k| decrypt(k, stored)).unwrap_or_else(|| stored.to_string())
}

/// `#[serde(with = "crate::vault::sealed")]` for fields that hold a secret value.
pub mod sealed {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &str, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::seal(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(super::open(&String::deserialize(d)?))
    }
}

// ------------------------------------------------------------------ names

/// Names openbot, Claude Code or the system rely on. A secret with one of these names would
/// break Bots or change how Claude Code runs. Compared ignoring case (Windows ignores case).
pub const RESERVED_NAMES: [&str; 44] = [
    "PATH", "HOME", "USER", "USERNAME", "USERPROFILE", "LOGNAME", "APPDATA", "LOCALAPPDATA", "PROGRAMDATA",
    "PROGRAMFILES", "TEMP", "TMP", "TMPDIR", "SHELL", "TERM", "PWD", "LANG", "IFS", "ENV", "BASH_ENV",
    "SYSTEMROOT", "SYSTEMDRIVE", "WINDIR", "COMSPEC", "PATHEXT", "PSMODULEPATH", "DISPLAY", "WSLENV",
    "NODE_OPTIONS", "NODE_PATH", "NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE", "SSL_CERT_DIR", "PYTHONPATH",
    "PYTHONHOME", "GIT_SSH_COMMAND", "GIT_ASKPASS", "SSH_AUTH_SOCK", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY",
    "ALL_PROXY", "FTP_PROXY", "OPENBOT",
];
/// Name beginnings that are reserved the same way.
pub const RESERVED_PREFIXES: [&str; 9] =
    ["LD_", "DYLD_", "ANTHROPIC_", "CLAUDE", "OPENBOT_", "MCP_", "NPM_CONFIG_", "GIT_CONFIG_", "XDG_"];

/// Why a secret can't have this name (None = fine).
pub fn name_problem(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let shape_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !shape_ok {
        return Some("Use letters, numbers and _ only, like GITHUB_TOKEN".into());
    }
    if name.len() > 128 {
        return Some("Too long. Maximum length is 128.".into());
    }
    let up = name.to_ascii_uppercase();
    if RESERVED_NAMES.contains(&up.as_str()) {
        return Some(format!("{name} is reserved: the computer, Claude Code or openbot needs it to run Bots. Pick another name."));
    }
    if let Some(p) = RESERVED_PREFIXES.iter().find(|p| up.starts_with(*p)) {
        return Some(format!("Names starting with {p} are reserved: Claude Code or the computer uses them. Pick another name."));
    }
    None
}

// ------------------------------------------------------------------ which Bots get which secret

/// May this Bot use this secret? `bots: None` means every Bot.
pub fn allowed(s: &Secret, bot_id: &str) -> bool {
    s.bots.as_ref().map_or(true, |ids| ids.iter().any(|i| i == bot_id))
}

/// The secrets a Bot gets on its runs: allowed for it, unlocked, and with a usable name.
pub fn for_bot<'a>(cfg: &'a Config, bot_id: &str) -> Vec<&'a Secret> {
    cfg.vault
        .iter()
        .filter(|s| allowed(s, bot_id) && !s.value.is_empty() && !is_locked(&s.value) && name_problem(s.name.trim()).is_none())
        .collect()
}

// ------------------------------------------------------------------ hiding values in chats

/// Remember the current values so `redact` can hide them. Called whenever settings load or save.
pub fn remember_values(cfg: &Config) {
    let mut vals: Vec<String> = Vec::new();
    let mut add = |v: &str| {
        if v.chars().count() >= MIN_REDACT && !is_locked(v) {
            vals.push(v.to_string());
            // The chat also keeps an HTML copy of each message, where & < > " are escaped.
            let esc = v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
            if esc != v {
                vals.push(esc);
            }
        }
    };
    for s in &cfg.vault {
        add(&s.value);
    }
    for l in &cfg.logins {
        add(&l.password);
    }
    add(&cfg.api_key);
    vals.sort_by(|a, b| b.len().cmp(&a.len()));
    vals.dedup();
    if let Ok(mut k) = KNOWN.write() {
        *k = vals;
    }
}

/// `text` with every saved secret value replaced by ••••. None when nothing changed.
pub fn redact_str(text: &str) -> Option<String> {
    let known = KNOWN.read().ok()?;
    if known.is_empty() || !known.iter().any(|v| text.contains(v.as_str())) {
        return None;
    }
    let mut out = text.to_string();
    for v in known.iter() {
        if out.contains(v.as_str()) {
            out = out.replace(v.as_str(), "••••");
        }
    }
    Some(out)
}

pub fn redact(text: &str) -> String {
    redact_str(text).unwrap_or_else(|| text.to_string())
}

/// Hide secret values in every string inside a JSON value. Returns true if anything changed.
pub fn redact_json(v: &mut serde_json::Value) -> bool {
    use serde_json::Value;
    match v {
        Value::String(s) => match redact_str(s) {
            Some(r) => {
                *s = r;
                true
            }
            None => false,
        },
        Value::Array(a) => a.iter_mut().fold(false, |ch, x| redact_json(x) | ch),
        Value::Object(o) => o.values_mut().fold(false, |ch, x| redact_json(x) | ch),
        _ => false,
    }
}

/// Hide secret values in a chat message (its text, HTML copy and details). True if anything changed.
pub fn redact_message(m: &mut crate::db::Message) -> bool {
    let mut changed = false;
    if let Some(t) = redact_str(&m.text) {
        m.text = t;
        changed = true;
    }
    if let Some(h) = redact_str(&m.html) {
        m.html = h;
        changed = true;
    }
    redact_json(&mut m.meta) | changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_round_trip_and_wrong_key() {
        let k: [u8; 32] = rand::random();
        let s = encrypt(&k, "ghp_abcdef123456").unwrap();
        assert!(is_locked(&s));
        assert!(!s.contains("abcdef"));
        assert_eq!(decrypt(&k, &s).as_deref(), Some("ghp_abcdef123456"));
        let other: [u8; 32] = rand::random();
        assert_eq!(decrypt(&other, &s), None);
        assert_eq!(decrypt(&k, "obenc1:%%%"), None);
    }

    #[test]
    fn reserved_names() {
        for n in ["PATH", "path", "Home", "ANTHROPIC_API_KEY", "anthropic_base_url", "CLAUDE_CODE_X", "CLAUDECODE", "NODE_OPTIONS", "LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "WSLENV", "OPENBOT", "OPENBOT_X", "MCP_TIMEOUT"] {
            assert!(name_problem(n).is_some(), "{n} should be refused");
        }
        for n in ["GITHUB_TOKEN", "OPENAI_API_KEY", "STRIPE_KEY", "_X", "PATHFINDER_KEY", "HOMEASSISTANT_TOKEN"] {
            assert!(name_problem(n).is_none(), "{n} should be allowed");
        }
        assert!(name_problem("1ABC").is_some());
        assert!(name_problem("A-B").is_some());
    }

    #[test]
    fn redaction() {
        let mut cfg = Config::default();
        cfg.vault.push(Secret { name: "A".into(), value: "s3cr3t&<value>".into(), description: String::new(), bots: None });
        cfg.vault.push(Secret { name: "B".into(), value: "short".into(), description: String::new(), bots: None });
        remember_values(&cfg);
        assert_eq!(redact("token=s3cr3t&<value> ok"), "token=•••• ok");
        assert_eq!(redact("<p>s3cr3t&amp;&lt;value&gt;</p>"), "<p>••••</p>");
        assert_eq!(redact("short words stay"), "short words stay");
        let mut v = serde_json::json!({"output": ["x s3cr3t&<value>"], "n": 1});
        assert!(redact_json(&mut v));
        assert_eq!(v["output"][0], "x ••••");
        remember_values(&Config::default());
        assert_eq!(redact("s3cr3t&<value>"), "s3cr3t&<value>");
    }

    #[test]
    fn scope() {
        let mut cfg = Config::default();
        let s = |name: &str, bots: Option<Vec<String>>| Secret { name: name.into(), value: "value-123456".into(), description: String::new(), bots };
        cfg.vault.push(s("ALL", None));
        cfg.vault.push(s("ONLY_A", Some(vec!["a".into()])));
        cfg.vault.push(s("NOBODY", Some(vec![])));
        cfg.vault.push(s("PATH", None));
        cfg.vault.push(Secret { value: "obenc1:AAAA".into(), ..s("LOCKED", None) });
        let names = |id: &str| for_bot(&cfg, id).iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names("a"), ["ALL", "ONLY_A"]);
        assert_eq!(names("b"), ["ALL"]);
    }
}
