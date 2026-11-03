//! The settings file: `<data dir>/config.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::util::{hint, random_token};

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Secret {
    pub name: String,
    pub value: String,
    /// "What it is for" (shown to the bot so it knows when to use it).
    #[serde(default)]
    pub description: String,
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
    /// Password the UI must send. Generated on first run.
    pub token: String,
    /// "subscription" (use the Claude Code login) or "api_key".
    pub auth_mode: String,
    pub api_key: String,
    /// "ask" | "auto" | "accept_edits" | "allow_all"
    pub permission_mode: String,
    pub allow_rules: Vec<String>,
    /// "" | "opus" | "sonnet" | "haiku"
    pub default_model: String,
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
        }
    }
}

impl Config {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Config> {
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
        Ok(())
    }

    /// The `Settings` object the UI sees (secrets are masked).
    pub fn settings_json(&self) -> Value {
        json!({
            "auth_mode": self.auth_mode,
            "api_key_set": !self.api_key.is_empty(),
            "api_key_hint": hint(&self.api_key),
            "permission_mode": self.permission_mode,
            "allow_rules": self.allow_rules,
            "default_model": self.default_model,
            "workspace_root": self.workspace_root,
            "vault": self.vault.iter().map(|s| json!({"name": s.name, "hint": hint(&s.value), "description": s.description})).collect::<Vec<_>>(),
            "auto_review": self.auto_review,
            "review_allow": self.review_allow,
            "review_ask": self.review_ask,
            "mcp_servers": self.mcp_servers.iter().map(mask_mcp).collect::<Vec<_>>(),
            "timezone": self.timezone,
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
