//! The settings file: `<data dir>/config.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::util::{hint, random_token};

/// The model Bots use unless Settings or the Bot says otherwise.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
/// Claude Code's effort level unless Settings says otherwise.
pub const DEFAULT_EFFORT: &str = "xhigh";
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Secret {
    pub name: String,
    /// Saved encrypted in config.json (see vault.rs); plain text in memory.
    #[serde(default, with = "crate::vault::sealed")]
    pub value: String,
    /// "What it is for" (shown to the bot so it knows when to use it).
    #[serde(default)]
    pub description: String,
    /// Which Bots get this secret (their ids). None = every Bot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bots: Option<Vec<String>>,
}

/// Which computer the UI should talk to. Only meaningful on the "home" server
/// (the one embedded in the desktop app).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Connection {
    pub mode: String, // "local" | "remote"
    pub url: String,
    pub token: String,
}

impl Default for Connection {
    fn default() -> Self {
        Self { mode: "local".into(), url: String::new(), token: String::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Owner token: used by the desktop app on this same computer and the approval helper.
    /// Other devices pair with a code and get their own token instead. Generated on first run.
    pub token: String,
    /// "subscription" (use the Claude Code login) or "api_key".
    pub auth_mode: String,
    /// Saved encrypted in config.json (see vault.rs).
    #[serde(with = "crate::vault::sealed")]
    pub api_key: String,
    /// "ask" | "auto" | "accept_edits" | "allow_all"
    pub permission_mode: String,
    pub allow_rules: Vec<String>,
    /// Model for every Bot that doesn't pick its own: a full model name or an alias ("opus",
    /// "sonnet", "haiku", "fable"). "" = DEFAULT_MODEL.
    pub default_model: String,
    /// Claude Code effort level for every run: "low" | "medium" | "high" | "xhigh" | "max".
    pub effort: String,
    /// Folder that holds one sub-folder per bot.
    pub workspace_root: String,
    pub vault: Vec<Secret>,
    pub connection: Connection,
    pub prefs: Value,
    /// Optional full path to the `claude` program (if it isn't on PATH).
    pub claude_path: String,
    /// Auto-review: a fast Claude model checks each action that needs approval against the rules below.
    pub auto_review: bool,
    /// "Allow without Asking" rules (plain language).
    pub review_allow: Vec<String>,
    /// "Always Ask" rules (plain language). These win over allow rules.
    pub review_ask: Vec<String>,
    /// Extra MCP servers for every bot: {name, type:"http"|"stdio", url, headers, command, args, env}.
    pub mcp_servers: Vec<Value>,
    /// IANA timezone for routines, e.g. "America/Los_Angeles". "" = this computer's timezone.
    pub timezone: String,
    /// Devices paired with a pairing code. Each has its own token (only its hash is stored).
    pub devices: Vec<crate::security::Device>,
    /// Where bots run Claude Code: "native" (directly on this computer) or "wsl" (Windows only:
    /// inside a WSL Linux distro, with Claude Code's sandbox turned on).
    pub runner: String,
    /// The WSL distro used when runner = "wsl".
    pub wsl_distro: String,
    /// Website logins in the Vault (Settings › Vault › Logins; see logins.rs).
    pub logins: Vec<crate::logins::Login>,
    /// Contact info in the Vault (Settings › Vault › Contact info).
    pub contacts: Vec<crate::logins::Contact>,
    /// Connected Google / Microsoft accounts (Settings › Connections).
    pub accounts: Vec<crate::accounts::Account>,
    /// Override the built-in Google / Microsoft sign-in apps (for development).
    pub oauth: crate::accounts::OAuthApps,
    /// Phone notifications: this computer's push key and the phones that subscribed (see push.rs).
    pub push: crate::push::PushConfig,
    /// The connected GitHub account (see github.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<crate::github::Connection>,
}

impl Default for Config {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("OpenBot");
        Self {
            token: random_token(32),
            auth_mode: "subscription".into(),
            api_key: String::new(),
            permission_mode: "ask".into(),
            allow_rules: vec![],
            default_model: String::new(),
            effort: DEFAULT_EFFORT.into(),
            workspace_root: root.to_string_lossy().into_owned(),
            vault: vec![],
            connection: Connection::default(),
            prefs: json!({}),
            claude_path: String::new(),
            auto_review: true,
            review_allow: vec![],
            review_ask: vec![],
            mcp_servers: vec![],
            timezone: String::new(),
            devices: vec![],
            runner: "native".into(),
            wsl_distro: "Ubuntu".into(),
            logins: vec![],
            contacts: vec![],
            accounts: vec![],
            oauth: Default::default(),
            push: Default::default(),
            github: None,
        }
    }
}

impl Config {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Config> {
        // The Vault key must be ready before secret values are read (they are saved encrypted).
        crate::vault::init(path.parent().unwrap_or(Path::new(".")));
        let mut cfg = if path.exists() {
            let text = std::fs::read_to_string(path)?;
            serde_json::from_str::<Config>(&text).unwrap_or_default()
        } else {
            Config::default()
        };
        if cfg.token.len() < 16 {
            cfg.token = random_token(32);
        }
        if cfg.workspace_root.trim().is_empty() {
            cfg.workspace_root = Config::default().workspace_root;
        }
        if !cfg.prefs.is_object() {
            cfg.prefs = json!({});
        }
        // Saving right away also encrypts values that older versions saved as plain text.
        cfg.save(path)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, path)?;
        crate::vault::remember_values(self);
        Ok(())
    }

    /// The `Settings` object the UI sees (secrets are masked).
    pub fn settings_json(&self) -> Value {
        json!({
            "auth_mode": self.auth_mode,
            "api_key_set": !self.api_key.is_empty(),
            "api_key_locked": crate::vault::is_locked(&self.api_key),
            "api_key_hint": if crate::vault::is_locked(&self.api_key) { String::new() } else { hint(&self.api_key) },
            "permission_mode": self.permission_mode,
            "allow_rules": self.allow_rules,
            "default_model": self.default_model,
            "model_default": DEFAULT_MODEL,
            "effort": self.effort,
            "workspace_root": self.workspace_root,
            "vault": self.vault.iter().map(|s| {
                let locked = crate::vault::is_locked(&s.value);
                json!({"name": s.name, "hint": if locked { String::new() } else { hint(&s.value) },
                       "description": s.description, "bots": s.bots, "locked": locked,
                       "name_problem": crate::vault::name_problem(&s.name)})
            }).collect::<Vec<_>>(),
            "logins": self.logins.iter().map(crate::logins::view).collect::<Vec<_>>(),
            "contacts": self.contacts,
            "vault_limits": {"logins": crate::logins::MAX_LOGINS, "contacts": crate::logins::MAX_CONTACTS},
            "vault_protection": crate::vault::protection(),
            "vault_reserved": {"names": &crate::vault::RESERVED_NAMES[..], "prefixes": &crate::vault::RESERVED_PREFIXES[..]},
            "auto_review": self.auto_review,
            "review_allow": self.review_allow,
            "review_ask": self.review_ask,
            "mcp_servers": self.mcp_servers.iter().map(mask_mcp).collect::<Vec<_>>(),
            "timezone": self.timezone,
            "runner": self.runner,
            "wsl_distro": self.wsl_distro,
            "wsl_available": cfg!(windows),
            "accounts": self.accounts.iter().map(crate::accounts::view).collect::<Vec<_>>(),
            "accounts_ready": self.oauth.ready(),
            "github": self.github.as_ref().map(crate::github::view),
        })
    }
}

/// Hide header/env values of MCP servers when showing them in the UI.
fn mask_mcp(v: &Value) -> Value {
    let mut v = v.clone();
    for key in ["headers", "env"] {
        if let Some(obj) = v.get_mut(key).and_then(Value::as_object_mut) {
            for (_, val) in obj.iter_mut() {
                if let Some(s) = val.as_str() {
                    *val = json!(hint(s));
                }
            }
        }
    }
    v
}

/// Where openbot keeps its files.
/// macOS: ~/Library/Application Support/openbot · Windows: %APPDATA%\openbot · Linux: ~/.local/share/openbot
pub fn default_data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("OPENBOT_DATA_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("openbot")
}
