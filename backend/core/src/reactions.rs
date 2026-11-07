//! Emoji reactions and replies on chat messages. The user can react to any message (a 👍 on a
//! Bot's reply, say) and reply to one, which quotes it. The Bot that wrote it hears about both with
//! the user's next message. Bots react to the user's messages with the `react` tool.

use serde_json::{json, Value};

use crate::db::{Bot, Message};
use crate::state::Shared;
use crate::util::{preview, truncate_chars};

/// One emoji (with skin tone or joiners), not text.
fn valid_emoji(e: &str) -> bool {
    !e.is_empty() && e.chars().count() <= 8 && !e.chars().any(|c| c.is_ascii() || c.is_whitespace())
}

/// The Bot that wrote a message (in a group, the one whose name is on it). None for the user's own.
fn author(m: &Message) -> Option<String> {
    if m.role == "user" {
        return None;
    }
    Some(m.meta.get("speaker").and_then(Value::as_str).unwrap_or(&m.bot_id).to_string())
}

/// Add or remove one reaction (`by` = "user" or a Bot id). Returns false if nothing changed.
fn set(st: &Shared, m: &mut Message, emoji: &str, by: &str, on: bool) -> bool {
    let mut list: Vec<Value> = m.meta.get("reactions").and_then(Value::as_array).cloned().unwrap_or_default();
    let at = list.iter().position(|r| r["emoji"] == emoji && r["by"] == by);
    match (on, at) {
        (true, None) => list.push(json!({ "emoji": emoji, "by": by })),
        (false, Some(i)) => {
            list.remove(i);
        }
        _ => return false,
    }
    if !m.meta.is_object() {
        m.meta = json!({});
    }
    m.meta["reactions"] = json!(list);
    let _ = st.db.upsert_message(m);
    st.emit_message(false, m);
    true
}

fn note_key(chat: &str, bot: &str) -> String {
    format!("reaction_notes:{chat}:{bot}")
}

/// What the user did with a Bot's messages since it last heard from them (then forgotten).
pub fn take_notes(st: &Shared, chat: &str, bot: &str) -> String {
    let key = note_key(chat, bot);
    let notes = st.db.kv_get(&key).unwrap_or_default();
    if !notes.is_empty() {
        st.db.kv_set(&key, "");
    }
    notes
}

/// The user tapped an emoji on a message (or took it back).
pub async fn user_react(st: &Shared, msg_id: &str, emoji: &str, on: bool) -> Result<Message, String> {
    let emoji = emoji.trim();
    if !valid_emoji(emoji) {
        return Err("Pick an emoji".into());
    }
    let mut m = st.db.get_message(msg_id).map_err(|e| e.to_string())?.ok_or("No such message")?;
    if set(st, &mut m, emoji, "user", on) && on {
        if let Some(bot) = author(&m) {
            let key = note_key(&m.bot_id, &bot);
            let mut notes = st.db.kv_get(&key).unwrap_or_default();
            // Keep it short if the user reacts a lot before writing again.
            if notes.lines().count() < 10 {
                notes.push_str(&format!("[openbot: The user reacted {emoji} to your message \"{}\".]\n", truncate_chars(&preview(&m.text), 160)));
                st.db.kv_set(&key, &notes);
            }
        }
    }
    Ok(m)
}

/// What the user is replying to, saved on their message: {id, who, bot, quote}.
pub fn reply_meta(st: &Shared, chat: &str, reply_to: &str) -> Option<Value> {
    let m = st.db.get_message(reply_to).ok().flatten().filter(|m| m.bot_id == chat)?;
    let bot = author(&m);
    let who = match &bot {
        None => "You".to_string(),
        Some(id) => st.db.get_bot(id).ok().flatten().map(|b| b.name).unwrap_or_else(|| "A Bot".into()),
    };
    let quote = if !m.text.trim().is_empty() {
        truncate_chars(&preview(&m.text), 200)
    } else if let Some(name) = m.meta.get("name").and_then(Value::as_str) {
        name.to_string()
    } else {
        "an earlier message".to_string()
    };
    Some(json!({ "id": m.id, "who": who, "bot": bot, "quote": quote }))
}

/// For the Bot: what the user is replying to ("your message", "Food Bot's message", "their own").
pub fn reply_note(reply: &Value, me: &str) -> String {
    let quote = reply.get("quote").and_then(Value::as_str).unwrap_or("");
    let whose = match reply.get("bot").and_then(Value::as_str) {
        Some(b) if b == me => "your".to_string(),
        Some(_) => format!("{}'s", reply.get("who").and_then(Value::as_str).unwrap_or("another Bot")),
        None => "their own".to_string(),
    };
    format!("[openbot: The user is replying to {whose} message \"{quote}\".]\n")
}

pub fn tool_definition() -> Value {
    json!({
        "name": "react",
        "description": "React to one of the user's messages with an emoji, like a tapback (👍 to show you've got it, ❤️ for a thank-you, 😂 for a joke). It shows on their message. Use it now and then, not on every message, and never instead of answering something they asked.",
        "inputSchema": {"type": "object", "properties": {
            "emoji": {"type": "string", "description": "One emoji"},
            "message_id": {"type": "string", "description": "Which message (default: the user's latest one)"},
            "remove": {"type": "boolean", "description": "Take your reaction back"}
        }, "required": ["emoji"]}
    })
}

pub fn is_tool(name: &str) -> bool {
    name == "react"
}

pub async fn tool(st: &Shared, me: &Bot, args: &Value) -> Result<String, String> {
    let emoji = args.get("emoji").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !valid_emoji(&emoji) {
        return Err("Give one emoji, like 👍".into());
    }
    let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
    // The chat it's working in: its own, or a group.
    let (chat, _) = st.chat_of(&me.id);
    let wanted = args.get("message_id").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let mut m = if wanted.is_empty() {
        st.db
            .list_messages(&chat, None, 100)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find(|m| m.role == "user")
            .ok_or("There's no message from the user to react to")?
    } else {
        st.db.get_message(&wanted).ok().flatten().filter(|m| m.bot_id == chat && m.role == "user").ok_or("No such message from the user in this chat")?
    };
    set(st, &mut m, &emoji, &me.id, !remove);
    Ok(if remove { format!("Took back your {emoji}.") } else { format!("Reacted {emoji} to the user's message.") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_emoji_count() {
        assert!(valid_emoji("👍"));
        assert!(valid_emoji("👍🏽"));
        assert!(valid_emoji("❤️"));
        assert!(!valid_emoji("ok"));
        assert!(!valid_emoji("👍 nice"));
        assert!(!valid_emoji(""));
    }

    #[test]
    fn reply_notes_say_whose() {
        let r = json!({"who": "Food Bot", "bot": "f1", "quote": "Try the salmon"});
        assert_eq!(reply_note(&r, "f1"), "[openbot: The user is replying to your message \"Try the salmon\".]\n");
        assert!(reply_note(&r, "s2").contains("Food Bot's message"));
        assert!(reply_note(&json!({"who": "You", "bot": null, "quote": "hi"}), "s2").contains("their own message"));
    }
}
