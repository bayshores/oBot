//! Shared server state + helpers to broadcast events to every open UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify, RwLock};

use crate::config::Config;
use crate::db::{Bot, Db, Message};

/// A message the user sent that is waiting for the bot.
pub struct Job {
    pub prompt: String,
}

/// A permission request waiting for the user to click Allow / Deny.
pub struct PendingApproval {
    pub bot_id: String,
    pub message_id: String,
    pub tool_name: String,
    pub input: Value,
    pub reply: oneshot::Sender<bool>,
}

/// Handle to a running `claude` process.
pub struct RunHandle {
    pub stop: Arc<Notify>,
}

pub struct AppState {
    pub db: Db,
    pub config: RwLock<Config>,
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub events: broadcast::Sender<String>,
    pub queues: Mutex<HashMap<String, mpsc::UnboundedSender<Job>>>,
    pub runs: Mutex<HashMap<String, RunHandle>>,
    pub approvals: Mutex<HashMap<String, PendingApproval>>,
    /// Live activity line per bot ("Searching the web"...).
    pub activity: std::sync::Mutex<HashMap<String, Value>>,
    /// Port we listen on (used by the approval helper to call back).
    pub port: u16,
    pub listen: String,
    /// Path of the current program (the approval helper is this same program with `mcp-approve`).
    pub self_exe: PathBuf,
    /// Failed logins / pairing tries per client.
    pub limiter: crate::security::RateLimiter,
    /// Last time each paired device was seen (device id → unix ms). Saved when devices are listed.
    pub device_seen: std::sync::Mutex<HashMap<String, i64>>,
    /// Only one pairing attempt at a time (the code file is shared).
    pub pair_lock: Mutex<()>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn emit(&self, ty: &str, mut payload: Value) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("type".into(), json!(ty));
        }
        let _ = self.events.send(payload.to_string());
    }

    pub async fn is_running(&self, bot_id: &str) -> bool {
        self.runs.lock().await.contains_key(bot_id)
    }

    /// Bot with the live `running` flag filled in.
    pub async fn bot_view(&self, mut b: Bot) -> Bot {
        b.running = self.is_running(&b.id).await;
        b.main = self.db.kv_get("main_bot").as_deref() == Some(b.id.as_str());
        b.conversation_id = b.session_id.clone();
        if b.running {
            b.activity = self
                .activity
                .lock()
                .ok()
                .and_then(|a| a.get(&b.id).cloned())
                .unwrap_or(Value::Null);
        }
        b
    }

    pub async fn emit_bot(&self, bot_id: &str) {
        if let Ok(Some(b)) = self.db.get_bot(bot_id) {
            let b = self.bot_view(b).await;
            self.emit("bot.updated", json!({ "bot": b }));
        }
    }

    pub fn emit_message(&self, created: bool, m: &Message) {
        let ty = if created { "message.created" } else { "message.updated" };
        self.emit(ty, json!({ "message": m }));
    }

    pub async fn save_config(&self) -> anyhow::Result<()> {
        let cfg = self.config.read().await.clone();
        cfg.save(&self.config_path)
    }
}
