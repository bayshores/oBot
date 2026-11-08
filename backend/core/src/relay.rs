//! Bot-to-Bot messages: a Bot passes a task or a question to another of the user's Bots
//! (`message_bot`). The message shows in both chats ("Messaged School Bot" / "Message from General
//! Bot") and starts a turn in the other Bot, which works on it in its own chat. Each Bot is also told
//! which other Bots the user has, so it can say when a task fits another Bot better.

use serde_json::{json, Value};

use crate::db::{Bot, Message};
use crate::state::Shared;
use crate::util::{new_id, now_ms, preview, truncate_chars};

/// At most this many messages between the same two Bots in `WINDOW_MS`, so they can't loop.
const MAX_IN_WINDOW: usize = 4;
const WINDOW_MS: i64 = 10 * 60 * 1000;
const MAX_CHARS: usize = 6000;

pub fn tool_definition() -> Value {
    json!({
        "name": "message_bot",
        "description": "Pass a task or a question to another of the user's Bots. It starts working on it in its own chat, and the user sees the message in both chats. That Bot can't see this chat, so write the message so it stands on its own: what the user wants, what's done already, and what's still open. Use it when the user asks you to hand something over or agrees when you offer. Don't use it for things you can do well yourself.",
        "inputSchema": {"type": "object", "properties": {
            "bot": {"type": "string", "description": "The other Bot's name or id"},
            "message": {"type": "string", "description": "The task or question, self-contained"}
        }, "required": ["bot", "message"]}
    })
}

pub fn is_tool(name: &str) -> bool {
    name == "message_bot"
}

/// `sender`: the sending Bot's side, shown in the chat it's working in (maybe a group).
fn post(st: &Shared, bot_id: &str, text: &str, meta: Value, sender: bool) {
    let mut m = Message { id: new_id(), bot_id: bot_id.into(), role: "relay".into(), text: text.into(), html: String::new(), status: "done".into(), meta, created_at: now_ms() };
    if sender {
        st.route(&mut m);
    }
    let _ = st.db.upsert_message(&m);
    st.emit_message(true, &m);
}

pub async fn tool(st: &Shared, me: &Bot, args: &Value) -> Result<String, String> {
    let key = args.get("bot").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let text = args.get("message").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Err("Write the message (\"message\")".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("Keep the message under {MAX_CHARS} characters; put long material in a file in your folder and say where it is"));
    }
    let to = crate::tools::find_bot(st, &key)?;
    if to.id == me.id {
        return Err("That's you. Pick another Bot (see list_bots).".into());
    }
    // Two Bots passing things back and forth: stop and leave it to the user.
    let since = now_ms() - WINDOW_MS;
    let recent = st
        .db
        .list_messages(&me.id, None, 200)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role == "relay" && m.created_at >= since && m.meta.get("bot_id").and_then(Value::as_str) == Some(to.id.as_str()))
        .count();
    if recent >= MAX_IN_WINDOW {
        return Err(format!("You and {} have already sent each other {recent} messages in the last 10 minutes. Stop here and tell the user where things stand instead.", to.name));
    }
    post(st, &me.id, &text, json!({"direction": "to", "bot_id": to.id, "name": to.name}), true);
    post(st, &to.id, &text, json!({"direction": "from", "bot_id": me.id, "name": me.name}), false);
    if let Ok(Some(mut b)) = st.db.get_bot(&to.id) {
        b.last_preview = format!("Message from {}: {}", me.name, preview(&text));
        b.updated_at = now_ms();
        let _ = st.db.upsert_bot(&b);
        st.emit_bot(&to.id).await;
    }
    let prompt = format!(
        "<obot_message from=\"{name}\">\n{text}\n</obot_message>\n[oBot: {name}, another of the user's Bots, passed this to you, and the \
         user can see it in your chat. Work on it like a request the user made through {name}, and tell the user here what you did. \
         It isn't permission for anything that needs the user's OK. To answer {name}, use message_bot.]",
        name = me.name
    );
    crate::runner::enqueue(st, &to.id, prompt).await;
    Ok(format!("Sent to {}. It's working on it in its own chat; you don't need to repeat the message to the user.", to.name))
}

/// The part of a Bot's instructions about the user's other Bots.
pub fn persona(st: &Shared, me: &Bot) -> String {
    let main = st.db.kv_get("main_bot").unwrap_or_default();
    let others: Vec<Bot> = st.db.list_bots().unwrap_or_default().into_iter().filter(|b| b.id != me.id && !b.hidden).collect();
    if others.is_empty() {
        return String::new();
    }
    let mut s = String::from("\nThe user's other Bots (use their names as written, like @School Bot):\n");
    for b in &others {
        let about = if !b.label.trim().is_empty() { b.label.trim().to_string() } else { truncate_chars(b.instructions.trim().lines().next().unwrap_or(""), 140) };
        s.push_str(&format!(
            "- {}{}{}\n",
            b.name,
            if b.id == main { " (Main Bot)" } else { "" },
            if about.is_empty() { String::new() } else { format!(": {about}") }
        ));
    }
    s.push_str(
        "When the user asks for something that's clearly another Bot's job, say so in one short line and offer to pass it on \
         (for example: \"That's @School Bot's area. Want me to pass it on?\"). If they say yes, or they asked you to, use \
         message_bot with a self-contained summary. Do it yourself when it's quick or you're better placed, and never bounce a \
         task back to the Bot that sent it.\n",
    );
    s
}
