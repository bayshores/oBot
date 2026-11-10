//! Group chats: the user and up to 6 of their Bots in one chat.
//!
//! A group is stored like a Bot (`kind = "group"`, with `members`), so it has its own chat, preview
//! and unread count. When the user writes, a fast model picks who answers (usually one Bot, see
//! [`pick`]); the Bots they @name answer instead when they name some. A Bot reads what was said
//! since its last turn there and answers, or replies PASS to stay quiet. A Bot can bring in another
//! member with @Name; those follow-up turns are capped so Bots can't keep talking to each other.
//!
//! Each Bot keeps a separate Claude Code conversation for each group, and works in its own folder
//! with its own tools. While it takes a group turn, everything it posts goes to the group
//! (`AppState::group_turns`).

use serde_json::{json, Value};

use crate::db::{Bot, Message};
use crate::state::Shared;
use crate::util::{join_names, new_id, now_ms, truncate_chars};

pub const MAX_MEMBERS: usize = 6;
/// Turns started by Bots (@Name) after one message from the user.
const MAX_BOT_TURNS: i64 = 6;
const MAX_NAME: usize = 72;

pub fn session_key(group: &str, bot: &str) -> String {
    format!("group_session:{group}:{bot}")
}
fn seen_key(group: &str, bot: &str) -> String {
    format!("group_seen:{group}:{bot}")
}
fn bot_turns_key(group: &str) -> String {
    format!("group_bot_turns:{group}")
}
/// The Bots picked to answer the user's latest message (a JSON list of ids).
fn picked_key(group: &str) -> String {
    format!("group_picked:{group}")
}

/// The Bot chose to stay quiet.
pub fn is_pass(text: &str) -> bool {
    text.trim().trim_matches(|c: char| !c.is_alphanumeric()).eq_ignore_ascii_case("pass")
}

fn get_group(st: &Shared, id: &str) -> Option<Bot> {
    st.db.get_bot(id).ok().flatten().filter(Bot::is_group)
}

fn name_of(st: &Shared, id: &str) -> String {
    st.db.get_bot(id).ok().flatten().map(|b| b.name).unwrap_or_else(|| "A Bot".into())
}

/// Check a member list: real, distinct Bots, at least one and at most 6.
fn check_members(st: &Shared, ids: &[String]) -> Result<Vec<String>, String> {
    let bots = st.db.list_bots().map_err(|e| e.to_string())?;
    let mut out: Vec<String> = vec![];
    for id in ids {
        if !bots.iter().any(|b| &b.id == id) {
            return Err("One of those Bots doesn't exist anymore".into());
        }
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    if out.is_empty() {
        return Err("Pick at least one Bot".into());
    }
    if out.len() > MAX_MEMBERS {
        return Err(format!("A group chat can have up to {MAX_MEMBERS} Bots"));
    }
    Ok(out)
}

fn names(st: &Shared, ids: &[String]) -> Vec<String> {
    ids.iter().map(|id| name_of(st, id)).collect()
}

fn event(st: &Shared, group: &str, text: &str) {
    crate::runner::add_event(st, group, text, json!({"kind": "group"}));
}

pub async fn create(st: &Shared, name: &str, members: &[String]) -> Result<Bot, String> {
    let members = check_members(st, members)?;
    let names = names(st, &members);
    let name = if name.trim().is_empty() { join_names(&names) } else { name.trim().to_string() };
    let id = new_id();
    // Files the user attaches in the group are saved here.
    let dir = st.data_dir.join("groups").join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let now = now_ms();
    let g = Bot {
        id,
        name: truncate_chars(&name, MAX_NAME),
        color: String::new(),
        shape: "group".into(),
        instructions: String::new(),
        model: String::new(),
        hidden: false,
        created_at: now,
        updated_at: now,
        last_preview: String::new(),
        unread: 0,
        running: false,
        activity: Value::Null,
        workdir: dir.to_string_lossy().into_owned(),
        session_id: String::new(),
        label: String::new(),
        pinned: false,
        notifications: true,
        main: false,
        conversation_id: String::new(),
        kind: "group".into(),
        members: members.clone(),
    };
    st.db.upsert_bot(&g).map_err(|e| e.to_string())?;
    event(st, &g.id, &format!("You started a group chat with {}", join_names(&names)));
    let view = st.bot_view(g.clone()).await;
    st.emit("bot.created", json!({ "bot": view }));
    Ok(g)
}

/// Rename the group or change who's in it.
pub async fn update(st: &Shared, id: &str, name: Option<String>, members: Option<Vec<String>>) -> Result<Bot, String> {
    let mut g = get_group(st, id).ok_or("No such group chat")?;
    if let Some(n) = name {
        let n = n.trim();
        g.name = if n.is_empty() { join_names(&names(st, &g.members)) } else { truncate_chars(n, MAX_NAME) };
    }
    if let Some(m) = members {
        let m = check_members(st, &m)?;
        let added: Vec<String> = m.iter().filter(|x| !g.members.contains(x)).cloned().collect();
        let removed: Vec<String> = g.members.iter().filter(|x| !m.contains(x)).cloned().collect();
        for r in &removed {
            stop_member(st, id, r).await;
        }
        if !added.is_empty() {
            event(st, id, &format!("You added {}", join_names(&names(st, &added))));
        }
        if !removed.is_empty() {
            event(st, id, &format!("You removed {}", join_names(&names(st, &removed))));
        }
        g.members = m;
    }
    st.db.upsert_bot(&g).map_err(|e| e.to_string())?;
    st.emit_bot(id).await;
    Ok(g)
}

pub async fn delete(st: &Shared, id: &str) -> Result<(), String> {
    let g = get_group(st, id).ok_or("No such group chat")?;
    stop(st, id).await;
    // Let stopping Bots finish first, so nothing is posted into the deleted chat.
    for _ in 0..100 {
        if working(st, id).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    st.db.delete_bot(id).map_err(|e| e.to_string())?;
    for m in &g.members {
        for k in [session_key(id, m), seen_key(id, m)] {
            st.db.kv_set(&k, "");
        }
    }
    let _ = std::fs::remove_dir_all(st.data_dir.join("groups").join(id));
    st.emit("bot.deleted", json!({ "bot_id": id }));
    Ok(())
}

/// A Bot was deleted: take it out of every group.
pub async fn forget_bot(st: &Shared, bot_id: &str) {
    for mut g in st.db.list_groups().unwrap_or_default() {
        if g.members.iter().any(|m| m == bot_id) {
            g.members.retain(|m| m != bot_id);
            let _ = st.db.upsert_bot(&g);
            st.emit_bot(&g.id).await;
        }
    }
}

/// The Bots answering in this group right now.
fn working(st: &Shared, group: &str) -> Vec<String> {
    let g = st.group_turns.lock().unwrap_or_else(|e| e.into_inner());
    g.iter().filter(|(_, v)| v.as_str() == group).map(|(k, _)| k.clone()).collect()
}

async fn stop_member(st: &Shared, group: &str, bot: &str) {
    if working(st, group).iter().any(|b| b == bot) {
        crate::runner::stop(st, bot).await;
    }
}

/// Stop in the group: every Bot answering there stops.
pub async fn stop(st: &Shared, group: &str) {
    let bots = working(st, group);
    for b in &bots {
        crate::runner::stop(st, b).await;
    }
    if !bots.is_empty() {
        crate::runner::add_simple(st, group, "system", "Stopped");
    }
}

/// Members named with @Name in `text`.
fn mentioned(st: &Shared, members: &[String], text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    members.iter().filter(|m| lower.contains(&format!("@{}", name_of(st, m).to_lowercase()))).cloned().collect()
}

/// The user wrote in the group. The Bots they @mentioned answer, or only `only` (the Bot whose
/// question they answered), or otherwise the Bots [`pick`] chooses (usually one).
pub async fn user_posted(st: &Shared, group: &Bot, text: &str, only: Option<&str>) {
    st.db.kv_set(&bot_turns_key(&group.id), "0");
    let chosen = match only.filter(|o| group.members.iter().any(|m| m == o)) {
        Some(o) => vec![o.to_string()],
        None => mentioned(st, &group.members, text),
    };
    // A Bot here waiting on a card: the user moved on, so the card is skipped and the Bot reads
    // the new message right after.
    let mut cut = false;
    for b in working(st, &group.id) {
        if st.approvals.lock().await.values().any(|p| p.bot_id == b) {
            crate::runner::cut_short(st, &b).await;
            cut = true;
        }
    }
    if cut {
        crate::runner::add_event(st, &group.id, "Stopped to read your new message", json!({"kind": "steered"}));
    }
    let (st, group, text) = (st.clone(), group.clone(), text.to_string());
    // Picking takes a few seconds, so the message is saved and shown first.
    tokio::spawn(async move {
        let answer = if !chosen.is_empty() {
            chosen
        } else if group.members.len() == 1 {
            group.members.clone()
        } else {
            crate::runner::set_activity(&st, &group.id, Some(("thinking", "Picking who answers"))).await;
            let p = pick(&st, &group, &text).await;
            crate::runner::set_activity(&st, &group.id, None).await;
            p
        };
        st.db.kv_set(&picked_key(&group.id), &json!(answer).to_string());
        for m in answer {
            crate::runner::enqueue_group(&st, &m, &group.id, false).await;
        }
    });
}

/// Which Bots should answer the user's message: a fast model reads the Bots' jobs and the recent
/// chat and picks the one that fits (more only when the user asks for several, or for separate
/// things that are different Bots' jobs). If that fails, the Bot that spoke last answers.
async fn pick(st: &Shared, group: &Bot, text: &str) -> Vec<String> {
    let bots: Vec<Bot> = group.members.iter().filter_map(|m| st.db.get_bot(m).ok().flatten()).collect();
    let recent = st.db.list_messages(&group.id, None, 12).unwrap_or_default();
    let last_speaker = recent.iter().rev().find_map(|m| m.meta.get("speaker").and_then(Value::as_str).map(str::to_string));
    let fallback = || vec![last_speaker.clone().filter(|s| group.members.contains(s)).unwrap_or_else(|| group.members[0].clone())];
    let about = |b: &Bot| {
        let job = [b.label.trim(), b.instructions.trim()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(". ");
        format!("- {}: {}", b.name, truncate_chars(&job, 400))
    };
    let chat: Vec<String> = recent
        .iter()
        .filter_map(|m| match m.role.as_str() {
            "user" => Some(format!("The user: {}", truncate_chars(m.text.trim(), 300))),
            "bot" => m.meta.get("speaker").and_then(Value::as_str).map(|s| format!("{}: {}", name_of(st, s), truncate_chars(m.text.trim(), 300))),
            _ => None,
        })
        .collect();
    let prompt = format!(
        "You decide which assistants (\"Bots\") answer the user's newest message in a group chat. Each Bot has its own job.\n\n\
         Bots:\n{}\n\nRecent messages, oldest first:\n{}\n\n\
         Rules:\n\
         - Pick ONE Bot: the one whose job fits the newest message best. If the message continues a conversation with \
         a Bot (a reply, \"yeah\", a follow-up question), pick that Bot.\n\
         - Pick more than one only when the user asks for several or all of them (\"everyone\", \"each of you\") or asks \
         separate things that are clearly different Bots' jobs.\n\
         - The Bot you pick can ask another Bot for help itself, so don't add Bots just in case.\n\
         Reply with ONLY a JSON object: {{\"answer\": [\"<Bot name>\"]}}",
        bots.iter().map(about).collect::<Vec<_>>().join("\n"),
        if chat.is_empty() { format!("The user: {}", truncate_chars(text, 300)) } else { chat.join("\n") },
    );
    let cfg = st.config.read().await.clone();
    let Some(reply) = crate::review::quick_answer(&cfg, &st.data_dir, &prompt, 30).await else {
        tracing::warn!("group {}: couldn't pick who answers; the last Bot to speak does", group.id);
        return fallback();
    };
    let names: Vec<(String, String)> = bots.iter().map(|b| (b.id.clone(), b.name.clone())).collect();
    let picked = parse_pick(&reply, &names);
    if picked.is_empty() { fallback() } else { picked }
}

/// The ids of the Bots named in the picker's `{"answer": [...]}` reply.
fn parse_pick(reply: &str, bots: &[(String, String)]) -> Vec<String> {
    let (Some(a), Some(b)) = (reply.find('{'), reply.rfind('}')) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&reply[a..=b.max(a)]) else { return vec![] };
    let mut out = vec![];
    for n in v.get("answer").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let n = n.trim().trim_start_matches('@').to_lowercase();
        if let Some((id, _)) = bots.iter().find(|(_, name)| name.to_lowercase() == n) {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
    }
    out
}

/// "Clear chat" in the group: each Bot starts a fresh conversation there.
pub async fn forget_history(st: &Shared, g: &Bot) {
    stop(st, &g.id).await;
    for m in &g.members {
        for k in [session_key(&g.id, m), seen_key(&g.id, m)] {
            st.db.kv_set(&k, "");
        }
    }
}

/// "Try again" on an error in the group: the Bots picked for the user's last message try again.
pub async fn retry(st: &Shared, group: &str) -> Result<(), String> {
    let g = get_group(st, group).ok_or("No such group chat")?;
    let picked: Vec<String> = st.db.kv_get(&picked_key(group)).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for m in g.members.iter().filter(|m| picked.is_empty() || picked.contains(m)) {
        crate::runner::enqueue_group(st, m, group, false).await;
    }
    Ok(())
}

/// The message for a Bot's turn: what was said in the group since its last turn there. None when
/// there's nothing new for it (then the turn is skipped).
pub fn turn_prompt(st: &Shared, group: &str, bot: &str, winding_down: bool) -> Option<String> {
    let g = get_group(st, group)?;
    if !g.members.iter().any(|m| m == bot) {
        return None;
    }
    let seen: i64 = st.db.kv_get(&seen_key(group, bot)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let msgs: Vec<Message> = st.db.list_messages(group, None, 200).ok()?.into_iter().filter(|m| m.created_at > seen).collect();
    let mut lines = vec![];
    let mut last = seen;
    for m in &msgs {
        let who = match m.role.as_str() {
            "user" => match m.meta.get("reply_to") {
                Some(r) => format!(
                    "The user (replying to {} \"{}\")",
                    match r.get("bot").and_then(Value::as_str) {
                        Some(b) if b == bot => "you".to_string(),
                        Some(_) => r.get("who").and_then(Value::as_str).unwrap_or("a Bot").to_string(),
                        None => "their own message".to_string(),
                    },
                    r.get("quote").and_then(Value::as_str).unwrap_or("")
                ),
                None => "The user".to_string(),
            },
            "bot" => match m.meta.get("speaker").and_then(Value::as_str) {
                Some(s) if s == bot => continue,
                Some(s) => name_of(st, s),
                None => continue,
            },
            _ => continue,
        };
        let mut text = truncate_chars(m.text.trim(), 4000);
        // Files the user attached, with full paths (they're in the group's folder, not the Bot's).
        if let Some(files) = m.meta.get("attachments").and_then(Value::as_array) {
            for f in files.iter().filter_map(|a| a.get("path").and_then(Value::as_str)) {
                let full = std::path::Path::new(&g.workdir).join(f);
                text.push_str(&format!("\n(attached file: {})", full.display()));
            }
        }
        lines.push(format!("{who}: {text}"));
        last = last.max(m.created_at);
    }
    if lines.is_empty() {
        return None;
    }
    st.db.kv_set(&format!("{}:prev", seen_key(group, bot)), &seen.to_string());
    st.db.kv_set(&seen_key(group, bot), &last.to_string());
    let mut p = crate::reactions::take_notes(st, group, bot);
    p.push_str(&format!("<obot_group name=\"{}\">\n{}\n</obot_group>", g.name.replace('"', "'"), lines.join("\n\n")));
    if !winding_down {
        let picked: Vec<String> = st.db.kv_get(&picked_key(group)).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let others: Vec<String> = picked.iter().filter(|m| m.as_str() != bot).map(|m| name_of(st, m)).collect();
        if !others.is_empty() {
            p.push_str(&format!("\n[oBot: {} also answer this message. Do only your part.]", join_names(&others)));
        }
    }
    if winding_down {
        p.push_str(
            "\n[oBot: Another Bot in the group named you. Answer if it asked you something; otherwise reply PASS. \
             Keep it short and don't start a new back-and-forth.]",
        );
    }
    Some(p)
}

/// The turn failed: next time the Bot reads those messages again.
pub fn rewind(st: &Shared, group: &str, bot: &str) {
    if let Some(prev) = st.db.kv_get(&format!("{}:prev", seen_key(group, bot))) {
        st.db.kv_set(&seen_key(group, bot), &prev);
    }
}

/// After a Bot's turn: members it @named get a turn too (up to the cap per user message).
pub async fn after_turn(st: &Shared, group: &str, bot: &str, started: i64) {
    let Some(g) = get_group(st, group) else { return };
    let said: String = st
        .db
        .list_messages(group, None, 100)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role == "bot" && m.created_at >= started && m.meta.get("speaker").and_then(Value::as_str) == Some(bot))
        .map(|m| m.text)
        .collect::<Vec<_>>()
        .join("\n");
    if said.is_empty() {
        return;
    }
    let others: Vec<String> = g.members.iter().filter(|m| m.as_str() != bot).cloned().collect();
    for m in mentioned(st, &others, &said) {
        let used: i64 = st.db.kv_get(&bot_turns_key(group)).and_then(|s| s.parse().ok()).unwrap_or(0);
        if used >= MAX_BOT_TURNS {
            return;
        }
        st.db.kv_set(&bot_turns_key(group), &(used + 1).to_string());
        crate::runner::enqueue_group(st, &m, group, true).await;
    }
}

/// The part of a Bot's instructions for a turn in this group.
pub fn persona(st: &Shared, group: &str, me: &Bot) -> String {
    let Some(g) = get_group(st, group) else { return String::new() };
    let mut s = format!(
        "\nRight now you're answering in a group chat called \"{}\" with the user and ",
        g.name
    );
    let others: Vec<Bot> = g.members.iter().filter(|m| **m != me.id).filter_map(|m| st.db.get_bot(m).ok().flatten()).collect();
    if others.is_empty() {
        s.push_str("no other Bots yet.\n");
    } else {
        s.push_str("these Bots:\n");
        for b in &others {
            let about = truncate_chars(b.label.trim(), 100);
            s.push_str(&format!("- {}{}\n", b.name, if about.is_empty() { String::new() } else { format!(": {about}") }));
        }
    }
    s.push_str(
        "Each message you get here holds what was said in the group since your last turn, inside <obot_group> tags. \
         Everyone in the group sees what you write, with your name on it.\n\
         - oBot picks who answers each of the user's messages, usually one Bot. When you get a turn, the message is for \
         you: answer it. Reply with only the word PASS (nothing is posted) if it's clearly another Bot's job.\n\
         - To ask a Bot in this group something, write @ and its name in your reply; it gets a turn to answer. \
         Don't use message_bot for Bots in this group.\n\
         - Tools, files and cards work as usual and show in the group.\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_is_recognized() {
        assert!(is_pass("PASS"));
        assert!(is_pass("  pass. "));
        assert!(is_pass("**PASS**"));
        assert!(!is_pass("I'll pass this to School Bot."));
        assert!(!is_pass("Passport photos are due."));
    }

    #[test]
    fn picker_reply_is_read() {
        let bots = vec![("a".to_string(), "Life Admin".to_string()), ("b".to_string(), "School Bot".to_string())];
        assert_eq!(parse_pick(r#"{"answer": ["Life Admin"]}"#, &bots), vec!["a"]);
        assert_eq!(parse_pick("Sure: {\"answer\": [\"@school bot\", \"Life Admin\", \"Nobody\"]}", &bots), vec!["b", "a"]);
        assert!(parse_pick("no json here", &bots).is_empty());
    }

    #[test]
    fn names_are_joined() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(join_names(&v(&["A"])), "A");
        assert_eq!(join_names(&v(&["A", "B"])), "A and B");
        assert_eq!(join_names(&v(&["A", "B", "C"])), "A, B, and C");
    }
}
