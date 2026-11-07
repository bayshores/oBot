//! Shared server state + helpers to broadcast events to every open UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify, RwLock};

use crate::config::Config;
use crate::db::{Bot, Db, Message};

/// Something waiting for the bot: a user message, or a turn openbot starts (routine, wake-up...).
pub struct Job {
    /// What the model is told.
    pub prompt: String,
    /// The user's chat message, when this is one (shown as "Queued" while it waits).
    pub user_msg: Option<String>,
    /// True when it arrived while the bot was working on something else.
    pub while_busy: bool,
    /// A turn in this group chat (see group.rs) instead of the Bot's own chat. Its prompt is made
    /// when the turn starts, from what was said in the group since the Bot's last turn there.
    pub group: Option<String>,
    /// Group turn started by another Bot's reply rather than the user: answer only if needed.
    pub winding_down: bool,
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
    /// Set by "Send now": the run stops so the waiting message can go right away (the queue is kept).
    pub steer: Arc<std::sync::atomic::AtomicBool>,
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
    /// Each bot's virtual screen (see screen.rs).
    pub screens: crate::screen::Screens,
    /// Bots taking a turn in a group chat right now: Bot id → group id. While it's there, what the
    /// Bot posts (replies, tool rows, cards) goes to the group.
    pub group_turns: std::sync::Mutex<HashMap<String, String>>,
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

    /// Where a Bot's messages go right now: the group it's answering in, or its own chat. Also
    /// returns who is speaking when it's a group.
    pub fn chat_of(&self, bot_id: &str) -> (String, Option<String>) {
        match self.group_turns.lock().ok().and_then(|g| g.get(bot_id).cloned()) {
            Some(group) => (group, Some(bot_id.to_string())),
            None => (bot_id.to_string(), None),
        }
    }

    /// Send a message a Bot is posting to the chat it's working in (see `chat_of`).
    pub fn route(&self, m: &mut Message) {
        let (chat, speaker) = self.chat_of(&m.bot_id);
        if let Some(s) = speaker {
            m.bot_id = chat;
            if !m.meta.is_object() {
                m.meta = json!({});
            }
            m.meta["speaker"] = json!(s);
        }
    }

    /// Bot with the live `running` flag filled in.
    pub async fn bot_view(&self, mut b: Bot) -> Bot {
        if b.is_group() {
            return self.group_view(b);
        }
        b.running = self.is_running(&b.id).await;
        b.main = self.db.kv_get("main_bot").as_deref() == Some(b.id.as_str());
        b.conversation_id = b.session_id.clone();
        b.last_preview = crate::vault::redact(&b.last_preview);
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

    /// A group chat is "running" while any of its Bots is taking a turn there.
    fn group_view(&self, mut b: Bot) -> Bot {
        let working: Vec<String> = {
            let g = self.group_turns.lock().unwrap_or_else(|e| e.into_inner());
            b.members.iter().filter(|m| g.get(*m) == Some(&b.id)).cloned().collect()
        };
        b.running = !working.is_empty();
        b.last_preview = crate::vault::redact(&b.last_preview);
        if b.running {
            let names: Vec<String> = working.iter().filter_map(|id| self.db.get_bot(id).ok().flatten()).map(|x| x.name).collect();
            let waiting = {
                let a = self.activity.lock().unwrap_or_else(|e| e.into_inner());
                working.iter().all(|id| a.get(id).and_then(|v| v.get("kind")).and_then(Value::as_str) == Some("waiting_for_you"))
            };
            let label = if waiting {
                "Waiting for you".to_string()
            } else if let [one] = names.as_slice() {
                let doing = self
                    .activity
                    .lock()
                    .ok()
                    .and_then(|a| a.get(&working[0]).and_then(|v| v.get("label")).and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_else(|| "Working".into());
                format!("{one}: {doing}")
            } else {
                format!("{} are working", crate::util::join_names(&names))
            };
            b.activity = json!({ "kind": "working", "label": label, "members": working });
        }
        b
    }

    pub async fn emit_bot(&self, bot_id: &str) {
        if let Ok(Some(b)) = self.db.get_bot(bot_id) {
            let b = self.bot_view(b).await;
            self.emit("bot.updated", json!({ "bot": b }));
        }
        // A Bot working in a group also changes the group's status line.
        let group = self.group_turns.lock().ok().and_then(|g| g.get(bot_id).cloned());
        if let Some(g) = group {
            if let Ok(Some(b)) = self.db.get_bot(&g) {
                let b = self.group_view(b);
                self.emit("bot.updated", json!({ "bot": b }));
            }
        }
    }

    /// Tell open apps about a new or changed chat message. Saved secret values are hidden first.
    pub fn emit_message(&self, created: bool, m: &Message) {
        let ty = if created { "message.created" } else { "message.updated" };
        let mut m = m.clone();
        crate::vault::redact_message(&mut m);
        self.emit(ty, json!({ "message": m }));
    }

    pub async fn save_config(&self) -> anyhow::Result<()> {
        let cfg = self.config.read().await.clone();
        cfg.save(&self.config_path)
    }
}
