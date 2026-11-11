//! Claude Code hooks for Bots.
//!
//! Each run gets a settings file (see `runner::build_command`) telling Claude Code to start this
//! same program as `<obot> hook <event> --config <bot's mcp.json>` at these moments:
//!
//! - **Stop** (the Bot is about to end its turn): if its last reply promises to follow up later
//!   ("I'll check back tomorrow") but it never scheduled anything, it's sent back once to call
//!   check_back_later (or say it won't follow up).
//! - **PreCompact** (Claude Code is about to summarize older messages to save space): the user's
//!   recent requests are saved as a memory log entry, so the summary loses less.
//! - **PostToolUse** of ask_user, request_form and request_control (`after-ask`): the card is in the
//!   chat and the Bot must wait for the user, so its turn ends right there. Otherwise it often wrote
//!   the same thing again as a reply under the card.
//!
//! Hooks must never break a run: any problem means "do nothing" (exit 0, no output).

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

/// What ask_user, request_form and request_control answer when their card is in the chat.
pub const ASKED: &str = "End your turn now";

/// Entry point for `hook <event> --config <path> [--distro <name>]`. Returns the exit code.
pub fn run(args: &[String]) -> i32 {
    let event = args.first().map(String::as_str).unwrap_or("");
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let mut input = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    let Ok(input) = serde_json::from_str::<Value>(&input) else { return 0 };
    if event == "after-ask" {
        // Only when the card really went up (the tool says "End your turn now"), not after an error.
        if input.get("tool_response").is_some_and(|r| r.to_string().contains(ASKED)) {
            println!("{}", json!({ "continue": false, "stopReason": "" }));
        }
        return 0;
    }
    let transcript = input
        .get("transcript_path")
        .and_then(Value::as_str)
        .map(|p| local_path(p, opt("--distro").as_deref()))
        .map(|p| read_end(&p, 2 << 20))
        .unwrap_or_default();
    let mut turn = last_turn(&transcript);
    // Claude Code hands over the final reply directly; the transcript file may not have it yet.
    if let Some(last) = input.get("last_assistant_message").and_then(Value::as_str) {
        if !turn.text.contains(last.trim()) {
            turn.text.push_str(last);
        }
    }
    match event {
        "stop" => {
            // Claude Code sets stop_hook_active when the Bot is already continuing because of us.
            if input.get("stop_hook_active").and_then(Value::as_bool).unwrap_or(false) {
                return 0;
            }
            if let Some(reason) = stop_check(&turn) {
                println!("{}", json!({ "decision": "block", "reason": reason }));
            }
            0
        }
        "precompact" => {
            if let (Some(cfg), Some(fact)) = (opt("--config"), precompact_fact(&transcript)) {
                save_memory(&local_path(&cfg, opt("--distro").as_deref()), &fact);
            }
            0
        }
        _ => 0,
    }
}

/// The last `max` bytes of a file, starting at a whole line (transcripts can be huge).
fn read_end(path: &std::path::Path, max: u64) -> String {
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max);
    let mut buf = vec![];
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    match (start > 0, text.find('\n')) {
        (true, Some(i)) => text[i + 1..].to_string(),
        _ => text,
    }
}

/// A path Claude Code gave us. In WSL mode it's a Linux path but we're the Windows program.
fn local_path(p: &str, distro: Option<&str>) -> PathBuf {
    if cfg!(windows) && p.starts_with('/') {
        if p.starts_with("/mnt/") {
            return crate::util::from_wsl_path(p);
        }
        if let Some(d) = distro {
            return PathBuf::from(format!("\\\\wsl.localhost\\{d}{}", p.replace('/', "\\")));
        }
    }
    PathBuf::from(p)
}

/// What happened since the last real user message: the Bot's text and the tools it called.
#[derive(Default, Debug)]
struct Turn {
    text: String,
    tools: Vec<String>,
}

/// What the user (or oBot for them) wrote in a transcript entry. Not Claude Code's own entries:
/// its notes to the model (`isMeta`) and the summary it writes in place of older messages.
fn user_text(entry: &Value) -> Option<String> {
    let flag = |name: &str| entry.get(name).and_then(Value::as_bool) == Some(true);
    if entry.get("type").and_then(Value::as_str) != Some("user") || flag("isMeta") || flag("isCompactSummary") {
        return None;
    }
    match entry.pointer("/message/content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(items)) => {
            let texts: Vec<&str> = items
                .iter()
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|i| i.get("text").and_then(Value::as_str))
                .collect();
            (!texts.is_empty()).then(|| texts.join("\n"))
        }
        _ => None,
    }
}

fn last_turn(transcript: &str) -> Turn {
    let entries: Vec<Value> = transcript.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let start = entries.iter().rposition(|e| user_text(e).is_some()).map_or(0, |i| i + 1);
    let mut t = Turn::default();
    for e in &entries[start..] {
        if e.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        for item in e.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    t.text.push_str(item.get("text").and_then(Value::as_str).unwrap_or(""));
                    t.text.push('\n');
                }
                Some("tool_use") => t.tools.push(item.get("name").and_then(Value::as_str).unwrap_or("").to_string()),
                _ => {}
            }
        }
    }
    t
}

const PROMISES: [&str; 14] = [
    "i'll check back", "i will check back", "i'll check again", "i'll check in", "i'll follow up", "i will follow up",
    "i'll let you know when", "i'll get back to you", "i'll remind you", "i will remind you", "i'll keep an eye",
    "i'll keep checking", "i'll monitor", "i'll ping you",
];
const SCHEDULERS: [&str; 4] = ["check_back_later", "create_routine", "update_routine", "run_in_background"];

/// Why the Bot should keep going, if it promised a follow-up it didn't schedule.
fn stop_check(t: &Turn) -> Option<String> {
    let text = t.text.to_lowercase().replace('’', "'");
    let promised = PROMISES.iter().any(|p| text.contains(p));
    let scheduled = t.tools.iter().any(|n| SCHEDULERS.iter().any(|s| n.ends_with(s)));
    (promised && !scheduled).then(|| {
        "[oBot] Your reply promises to follow up later, but nothing is scheduled, so you'd never be woken up. \
         Call check_back_later now with a note of what to do (or create_routine for something recurring). \
         If you didn't mean to follow up, end with one short line correcting your promise instead."
            .to_string()
    })
}

/// The user's own words in a message, without the notes oBot put above them: one-line ones like
/// "[oBot: It's Friday 2026-11-06 20:01 now.]", and the changed-instructions and routine-posts notes,
/// which run over several lines and end with a line that is just "]". (Chats from before the rename
/// use "[openbot".)
fn own_words(mut t: &str) -> &str {
    loop {
        t = t.trim_start();
        let Some(note) = t.strip_prefix("[oBot").or_else(|| t.strip_prefix("[openbot")) else { return t };
        let long = note.starts_with(": Your setup changed") || note.starts_with(": Routines posted");
        let end = if long { "\n]\n" } else { "\n" };
        t = t.split_once(end).map(|x| x.1).unwrap_or("");
    }
}

/// A memory log entry with the user's latest requests, or None if there's nothing worth saving.
fn precompact_fact(transcript: &str) -> Option<String> {
    let requests: Vec<String> = transcript
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|e| user_text(&e))
        .map(|t| {
            // oBot's own wake-ups and routines are tagged; keep only a short label for them.
            // (Chats from before the rename use "<openbot".)
            let t = t.trim();
            if t.starts_with("<obot") || t.starts_with("<openbot") {
                crate::util::truncate_chars(t.lines().next().unwrap_or(""), 80)
            } else {
                crate::util::truncate_chars(&own_words(t).replace('\n', " "), 160)
            }
        })
        .filter(|t| !t.trim().is_empty())
        .collect();
    if requests.is_empty() {
        return None;
    }
    let recent = &requests[requests.len().saturating_sub(4)..];
    let date = crate::util::now_local().format("%Y-%m-%d %H:%M");
    Some(crate::util::truncate_chars(
        &format!("Conversation summarized on {date}. Latest requests before that: {}", recent.join(" | ")),
        480,
    ))
}

/// Save a memory through the oBot server (same route the Bot's own tools use).
fn save_memory(mcp_config: &PathBuf, fact: &str) {
    let Ok(text) = std::fs::read_to_string(mcp_config) else { return };
    let Ok(cfg) = serde_json::from_str::<Value>(&text) else { return };
    let env = &cfg["mcpServers"]["obot"]["env"];
    let (Some(url), Some(token), Some(bot)) = (
        env.get("OBOT_URL").and_then(Value::as_str),
        env.get("OBOT_TOKEN").and_then(Value::as_str),
        env.get("OBOT_BOT_ID").and_then(Value::as_str),
    ) else {
        return;
    };
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let _ = agent
        .post(&format!("{url}/api/internal/tool"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(json!({"bot_id": bot, "name": "remember", "arguments": {"fact": fact, "tier": "log", "about": "me"}}));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(v: Value) -> String {
        v.to_string()
    }

    fn transcript(reply: &str, tools: &[&str]) -> String {
        let mut content = vec![json!({"type": "text", "text": reply})];
        for t in tools {
            content.push(json!({"type": "tool_use", "name": t, "input": {}}));
        }
        [
            line(json!({"type": "user", "message": {"content": "old request"}})),
            line(json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "I'll check back tomorrow (old turn)"}]}})),
            line(json!({"type": "user", "message": {"content": "track my package"}})),
            line(json!({"type": "assistant", "message": {"content": content}})),
            line(json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "ok"}]}})),
        ]
        .join("\n")
    }

    #[test]
    fn unscheduled_promise_is_caught() {
        let t = last_turn(&transcript("It's in transit. I’ll check back tomorrow morning.", &[]));
        assert!(stop_check(&t).is_some());
    }

    #[test]
    fn scheduled_promise_passes() {
        let t = last_turn(&transcript("I'll check back tomorrow.", &["mcp__obot__check_back_later"]));
        assert!(stop_check(&t).is_none());
    }

    #[test]
    fn no_promise_passes() {
        let t = last_turn(&transcript("Delivered on Tuesday.", &[]));
        assert!(stop_check(&t).is_none());
    }

    #[test]
    fn transcript_end_starts_at_a_line() {
        let p = std::env::temp_dir().join(format!("obhook-{}.jsonl", std::process::id()));
        std::fs::write(&p, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(read_end(&p, 7), "cccc\n");
        assert_eq!(read_end(&p, 100), "aaaa\nbbbb\ncccc\n");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn precompact_keeps_latest_requests() {
        let f = precompact_fact(&transcript("x", &[])).unwrap();
        assert!(f.contains("old request | track my package"), "{f}");
    }

    #[test]
    fn claude_codes_own_summary_is_not_a_request() {
        let summary = json!({"type": "user", "isCompactSummary": true, "message": {"content": "This session is being continued"}});
        assert_eq!(user_text(&summary), None);
        assert_eq!(user_text(&json!({"type": "user", "message": {"content": "hi"}})), Some("hi".to_string()));
    }

    #[test]
    fn notes_above_a_message_are_not_the_request() {
        assert_eq!(own_words("plain"), "plain");
        assert_eq!(own_words("[oBot: It's Friday 2026-11-06 20:01 now.]\nbuy socks"), "buy socks");
        let noted = "[oBot: It's Friday 2026-11-06 20:01 now.]\n[oBot: Your setup changed since this conversation \
                     started. These lines are now part of your instructions:\n- [m1|user] Likes teal\nThese lines no \
                     longer apply:\n- - [m2|user] Old\n]\nbuy socks\nin blue";
        assert_eq!(own_words(noted), "buy socks\nin blue");
        assert_eq!(own_words("[openbot: It's Friday now.]\n"), "");
        let news = "[oBot: Routines posted since your last turn in this conversation. The user may be answering \
                    one:\n[Mon 2026-11-09 08:00] You: Two new emails.\nReply to Ana?\n]\nyes, reply";
        assert_eq!(own_words(news), "yes, reply");
    }
}
