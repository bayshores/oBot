//! Creating, changing and deleting Bots. Shared by the API (the app's buttons) and by bots' own
//! tools (a Bot that sets up or retires other Bots), so both follow the same rules.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::Config;
use crate::db::{Bot, Db};
use crate::state::Shared;
use crate::util::{new_id, now_ms, truncate_chars};

/// Every look a Bot can have (the ids in frontend/ui/avatar.js). Solids first.
pub const SKINS: [&str; 24] = [
    "violet", "coral", "amber", "lime", "mint", "sky", "cobalt", "rose", "slate", "ink", "sand", "snow", // solids
    "zebra", "tiger", "cow", "dots", "checker", "sunset", "ocean", "aurora", "holo", "galaxy", "marble", "camo", // patterns
];

/// Longest instructions a Bot can have (about 4,000 words).
pub const MAX_INSTRUCTIONS: usize = 20_000;

/// What a new Bot starts with.
#[derive(Default)]
pub struct NewBot {
    pub name: String,
    pub color: String,
    pub shape: String,
    pub instructions: String,
    pub model: String,
    pub label: String,
}

fn folder_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let s = s.trim().trim_matches('-').to_string();
    if s.is_empty() { "Bot".into() } else { s }
}

/// A free solid color (the first one no other Bot uses), so new Bots are easy to tell apart.
pub fn free_color(db: &Db) -> String {
    let used: Vec<String> = db.list_bots().map(|v| v.into_iter().map(|b| b.color).collect()).unwrap_or_default();
    SKINS[..8].iter().find(|c| !used.iter().any(|u| u == *c)).unwrap_or(&SKINS[0]).to_string()
}

/// Build a Bot and its working folder (not saved yet).
pub fn build(db: &Db, cfg: &Config, b: &NewBot) -> anyhow::Result<Bot> {
    let root = PathBuf::from(&cfg.workspace_root);
    let used: Vec<String> = db.list_bots()?.into_iter().map(|b| b.workdir).collect();
    let base = folder_name(&b.name);
    let mut dir = root.join(&base);
    let mut n = 2;
    while used.contains(&dir.to_string_lossy().into_owned()) {
        dir = root.join(format!("{base} {n}"));
        n += 1;
    }
    std::fs::create_dir_all(&dir)?;
    let now = now_ms();
    Ok(Bot {
        id: new_id(),
        name: b.name.trim().to_string(),
        color: b.color.clone(),
        shape: b.shape.clone(),
        instructions: b.instructions.clone(),
        model: b.model.clone(),
        hidden: false,
        created_at: now,
        updated_at: now,
        last_preview: String::new(),
        unread: 0,
        running: false,
        activity: Value::Null,
        workdir: dir.to_string_lossy().into_owned(),
        session_id: String::new(),
        label: truncate_chars(b.label.trim(), 60),
        pinned: false,
        notifications: true,
        main: false,
        conversation_id: String::new(),
        kind: String::new(),
        members: vec![],
    })
}

/// Save a new Bot and show it in the app. Call `say_hello` after giving it anything it should
/// know on its first turn (memories, routines).
pub async fn create(st: &Shared, b: &NewBot) -> anyhow::Result<Bot> {
    let cfg = st.config.read().await.clone();
    let bot = build(&st.db, &cfg, b)?;
    st.db.upsert_bot(&bot)?;
    let view = st.bot_view(bot.clone()).await;
    st.emit("bot.created", json!({ "bot": view }));
    Ok(bot)
}

/// A new Bot says hello first. `briefing` is extra context for that first
/// message, e.g. why another Bot set it up.
pub async fn say_hello(st: &Shared, bot: &Bot, briefing: &str) {
    let mut intro = if bot.instructions.trim().is_empty() {
        "[oBot] You were just created and this is the start of your chat with the user. Say hi in one or two short, \
         friendly messages: introduce yourself by name, say what you can do on this computer, and ask what they'd \
         like help with or what you should focus on. Don't use any tools."
            .to_string()
    } else {
        "[oBot] You were just created and this is the start of your chat with the user. Say hi in one or two short, \
         friendly messages: introduce yourself by name and your role (from your standing instructions) and ask how \
         you can help. Don't use any tools."
            .to_string()
    };
    if !briefing.trim().is_empty() {
        intro.push_str(&format!(
            "\nAnother of the user's Bots set you up and left this note for you (use it, don't repeat it word for word): {}",
            truncate_chars(briefing.trim(), 2000)
        ));
    }
    crate::runner::enqueue(st, &bot.id, intro).await;
}

/// Delete a Bot with its chat, its own memories and its routines. The Main Bot can't be deleted.
pub async fn delete(st: &Shared, id: &str) -> Result<(), String> {
    if st.db.kv_get("main_bot").as_deref() == Some(id) {
        return Err("Your Main Bot can't be deleted. Make another Bot your Main Bot first.".into());
    }
    crate::runner::stop(st, id).await;
    crate::jobs::stop_for_bot(st, id);
    st.screens.stop(st, id).await;
    st.queues.lock().await.remove(id);
    st.db.delete_bot(id).map_err(|e| e.to_string())?;
    st.db.delete_routines_of_bot(id).map_err(|e| e.to_string())?;
    crate::group::forget_bot(st, id).await;
    // Take the Bot off any Vault secret or connected account that was limited to certain Bots.
    let vault_changed = {
        let mut cfg = st.config.write().await;
        let mut changed = false;
        for s in cfg.vault.iter_mut() {
            if let Some(ids) = s.bots.as_mut() {
                let before = ids.len();
                ids.retain(|b| b != id);
                changed |= ids.len() != before;
            }
        }
        for a in cfg.accounts.iter_mut() {
            if let Some(ids) = a.bots.as_mut() {
                let before = ids.len();
                ids.retain(|b| b != id);
                changed |= ids.len() != before;
            }
        }
        changed
    };
    if vault_changed {
        let _ = st.save_config().await;
        let settings = st.config.read().await.settings_json();
        st.emit("settings.updated", json!({ "settings": settings }));
    }
    st.emit("bot.deleted", json!({ "bot_id": id }));
    st.emit("routines.updated", json!({ "bot_id": id }));
    st.emit("memories.updated", json!({ "bot_id": id }));
    Ok(())
}
