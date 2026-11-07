//! openbot's own tools for bots (served to Claude Code through the `openbot` MCP server in mcp.rs).
//! They give bots extra abilities: quick-reply buttons, sending files, showing pages,
//! memory, routines, check-ins, a status line, setting up or retiring the user's other Bots, and
//! email + calendar of connected accounts (accounts.rs).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::db::{Bot, Memory, Message, Routine};
use crate::routines::{self, routine_json};
use crate::state::Shared;
use crate::util::{new_id, now_ms, truncate_chars};

/// Tool definitions advertised over MCP (besides `approve`, which lives in mcp.rs).
pub fn definitions() -> Value {
    #[allow(non_snake_case)]
    let SKINS_JSON = json!(crate::bots::SKINS);
    let sched = json!({
        "type": "object",
        "description": "When it runs, in this computer's local time. One of: {kind:'once', at:'2026-11-03T09:00'} · {kind:'interval', minutes:60} · {kind:'daily', time:'08:00'} · {kind:'weekdays', time:'08:00'} · {kind:'weekly', day:'monday', time:'09:30'} · {kind:'monthly', day:1, time:'09:00'}",
        "properties": {
            "kind": {"type": "string", "enum": ["once", "interval", "daily", "weekdays", "weekly", "monthly"]},
            "at": {"type": "string"}, "minutes": {"type": "integer"}, "time": {"type": "string"},
            "day": {"type": ["string", "integer"]}
        },
        "required": ["kind"]
    });
    let mut list = json!([
        {
            "name": "ask_user",
            "description": "Ask the user a quick question with tap-to-answer buttons in the chat (like 'Which one?' with 2-5 options). Returns right away; the user's choice arrives later as their next message, so end your turn after calling this.",
            "inputSchema": {"type": "object", "properties": {
                "question": {"type": "string"},
                "options": {"type": "array", "items": {"type": "string"}, "description": "2 to 5 short answers"},
                "allow_custom": {"type": "boolean", "description": "Also let them type their own answer (default true)"}
            }, "required": ["question", "options"]}
        },
        {
            "name": "send_file",
            "description": "Send a file to the user in the chat (images show inline; other files appear as a download). Use this for anything you made for them: documents, images, spreadsheets, screenshots.",
            "inputSchema": {"type": "object", "properties": {
                "path": {"type": "string", "description": "File path, absolute or relative to your working folder"},
                "caption": {"type": "string"}
            }, "required": ["path"]}
        },
        {
            "name": "show_page",
            "description": "Show an interactive HTML page inside the chat (charts, tables, small apps, visual summaries). Give either full HTML or a path to an .html file. openbot is dark-themed: use a dark background (#0b0b0c) with light text unless the user asks otherwise. Scripts run in a sandbox without network access to the user's data.",
            "inputSchema": {"type": "object", "properties": {
                "title": {"type": "string"},
                "html": {"type": "string"},
                "path": {"type": "string"}
            }, "required": ["title"]}
        },
        {
            "name": "remember",
            "description": "Save a lasting fact so you (and, for facts about the user, every bot) remember it in future chats. Keep each fact short and self-contained. Pick the tier: \"profile\" is only for standing instructions and core identity (name, location, role, family, strong preferences, how they want you to work). Rules about how to do things (\"always\", \"never\", \"don't ask me to\", which way to sign in to a site, a workaround you were told to use) are standing instructions: put them in the profile. It's in your prompt every time, so keep it small. Everything else (events, plans, one-off details, things you did, what you learned) goes in \"log\", which you search with recall. To hand a fact to another of the user's Bots, give its name in \"bot\": it becomes that Bot's own memory.",
            "inputSchema": {"type": "object", "properties": {
                "fact": {"type": "string"},
                "tier": {"type": "string", "enum": ["profile", "log"], "description": "profile: a rule or standing instruction (always / never / don't ask me to / how to get into a site) or core identity. log: anything else"},
                "about": {"type": "string", "enum": ["user", "me"], "description": "'user' = about the user (shared with all bots), 'me' = only for this bot"},
                "bot": {"type": "string", "description": "Optional: another Bot's name or id to save this for (only that Bot sees it)"}
            }, "required": ["fact", "tier"]}
        },
        {
            "name": "recall",
            "description": "Search your memory. Give \"query\" (a few words) to find matching facts in your profile and log; leave it out to list the profile and the newest log entries. Results have ids, so you can update_memory or forget them. Search before saying you don't know something about the user. Give \"bot\" to look in another Bot's memory.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "bot": {"type": "string", "description": "Optional: another Bot's name or id"}
            }}
        },
        {
            "name": "update_memory",
            "description": "Fix a remembered fact's text, or move it between \"profile\" and \"log\".",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"},
                "fact": {"type": "string", "description": "New text (leave out to keep it)"},
                "tier": {"type": "string", "enum": ["profile", "log"]}
            }, "required": ["id"]}
        },
        {
            "name": "forget",
            "description": "Delete a remembered fact by id.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}
        },
        {
            "name": "check_back_later",
            "description": "Schedule yourself to continue later and message the user then (e.g. 'I'll check the tracking tomorrow at 9'). The note is what you'll be reminded of.",
            "inputSchema": {"type": "object", "properties": {
                "note": {"type": "string"},
                "in_minutes": {"type": "integer"},
                "at": {"type": "string", "description": "Local time like '2026-11-03T09:00' (use instead of in_minutes)"}
            }, "required": ["note"]}
        },
        {
            "name": "create_routine",
            "description": "Create a recurring task you'll run on a schedule (e.g. a morning briefing every weekday at 8:00). Only create one when the user asks for it or agrees. Give \"bot\" to create it for another of the user's Bots (it then runs in that Bot's chat).",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "prompt": {"type": "string", "description": "What to do each time"},
                "schedule": sched.clone(),
                "bot": {"type": "string", "description": "Optional: another Bot's name or id"}
            }, "required": ["name", "prompt", "schedule"]}
        },
        {
            "name": "list_routines",
            "description": "List your routines and scheduled check-ins. Give \"bot\" for another Bot's, or all:true for every Bot's.",
            "inputSchema": {"type": "object", "properties": {
                "bot": {"type": "string", "description": "Optional: another Bot's name or id"},
                "all": {"type": "boolean"}
            }}
        },
        {
            "name": "update_routine",
            "description": "Change, pause or resume a routine (yours or another Bot's, by id).",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"}, "name": {"type": "string"}, "prompt": {"type": "string"},
                "schedule": sched, "enabled": {"type": "boolean"}
            }, "required": ["id"]}
        },
        {
            "name": "delete_routine",
            "description": "Delete a routine (yours or another Bot's, by id).",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}
        },
        {
            "name": "move_routine",
            "description": "Hand a routine over to another of the user's Bots (it keeps its schedule and then runs in that Bot's chat).",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"},
                "to_bot": {"type": "string", "description": "The Bot's name or id"}
            }, "required": ["id", "to_bot"]}
        },
        {
            "name": "list_bots",
            "description": "List all of the user's Bots: name, id, tagline, a preview of their instructions, and how many routines and private memories each has.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "get_bot",
            "description": "Everything about one Bot: its full instructions, look, private memories and routines.",
            "inputSchema": {"type": "object", "properties": {
                "bot": {"type": "string", "description": "The Bot's name or id"}
            }, "required": ["bot"]}
        },
        {
            "name": "create_bot",
            "description": "Create a new Bot for the user (it appears in their sidebar and says hello in its own chat). Only do this when the user asks for it or agrees to your plan. Give it clear standing instructions, and hand it the private facts and routines that belong to it so it is ready from its first message. Facts about the user that every Bot should know don't need copying: those are shared already.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string", "description": "Short name, e.g. 'School Bot' (max 40 characters)"},
                "instructions": {"type": "string", "description": "Its standing instructions: its job, what to focus on, how to behave"},
                "label": {"type": "string", "description": "Optional tagline shown under its name (max 60 characters)"},
                "look": {"type": "string", "enum": SKINS_JSON.clone(), "description": "Optional color or pattern; a free color is picked if left out"},
                "memories": {"type": "array", "items": {"type": "string"}, "description": "Optional facts only this Bot should know (max 50)"},
                "routines": {"type": "array", "description": "Optional routines for it (max 10)", "items": {"type": "object", "properties": {
                    "name": {"type": "string"}, "prompt": {"type": "string"}, "schedule": sched.clone()
                }, "required": ["name", "prompt", "schedule"]}},
                "briefing": {"type": "string", "description": "Optional note for its first message: why it was set up and what the user wants from it"}
            }, "required": ["name", "instructions"]}
        },
        {
            "name": "update_bot",
            "description": "Change one of the user's Bots (or yourself): its name, tagline, look or standing instructions. Use add_instructions to append to its instructions instead of replacing them. Only when the user asks for it or agrees.",
            "inputSchema": {"type": "object", "properties": {
                "bot": {"type": "string", "description": "The Bot's name or id"},
                "name": {"type": "string"},
                "label": {"type": "string"},
                "look": {"type": "string", "enum": SKINS_JSON.clone()},
                "instructions": {"type": "string", "description": "Replaces its instructions"},
                "add_instructions": {"type": "string", "description": "Appended to its instructions"}
            }, "required": ["bot"]}
        },
        {
            "name": "delete_bot",
            "description": "Retire one of the user's Bots: its chat, its private memories and its routines are deleted. The user always gets an Allow / Deny card first, and this waits for their answer. Move anything worth keeping (routines, facts) to another Bot before deleting. The Main Bot and you yourself can't be deleted.",
            "inputSchema": {"type": "object", "properties": {
                "bot": {"type": "string", "description": "The Bot's name or id"},
                "reason": {"type": "string", "description": "One short sentence the user sees on the card"}
            }, "required": ["bot"]}
        },
        {
            "name": "set_status",
            "description": "Show a short status under your name while you work on something long (e.g. 'Comparing 12 flights').",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}
        }
    ]);
    // Email and calendar of the accounts the user connected (accounts.rs).
    if let Some(a) = list.as_array_mut() {
        a.extend(crate::accounts::tool_definitions());
        a.extend(crate::drive::tool_definitions());
        a.extend(crate::jobs::tool_definitions());
    }
    // Bots get their own screen where this computer can run one (Windows + WSL, or Linux), and
    // can sign in on it with Vault logins or a secure form.
    if crate::screen::platform_ok().is_ok() {
        if let Some(a) = list.as_array_mut() {
            a.extend(crate::screen::tool_definitions());
            a.extend(crate::logins::tool_definitions());
            a.extend(crate::secure_form::tool_definitions());
        }
    }
    list
}

/// "1 routine", "3 routines".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

/// Find one of the user's Bots by id or name (names match without caring about case).
fn find_bot(state: &Shared, key: &str) -> Result<Bot, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Say which Bot (its name or id)".into());
    }
    let bots = state.db.list_bots().map_err(|e| e.to_string())?;
    if let Some(b) = bots.iter().find(|b| b.id == key) {
        return Ok(b.clone());
    }
    let named: Vec<&Bot> = bots.iter().filter(|b| b.name.eq_ignore_ascii_case(key)).collect();
    match named.len() {
        1 => Ok(named[0].clone()),
        0 => Err(format!("No Bot called \"{key}\". Use list_bots to see their names.")),
        _ => Err(format!("Several Bots are called \"{key}\"; use the id from list_bots instead.")),
    }
}

/// The Bot a memory/routine tool acts on: another Bot when "bot" is given, otherwise the caller.
fn target_bot(state: &Shared, me: &Bot, args: &Value) -> Result<Bot, String> {
    let key = s(args, "bot");
    if key.is_empty() { Ok(me.clone()) } else { find_bot(state, key) }
}

fn check_look(look: &str) -> Result<(), String> {
    if look.is_empty() || crate::bots::SKINS.contains(&look) {
        Ok(())
    } else {
        Err(format!("Unknown look \"{look}\". Use one of: {}", crate::bots::SKINS.join(", ")))
    }
}

/// A routine ready to save (its schedule checked and its first run worked out).
fn new_routine(bot_id: &str, name: &str, prompt: &str, schedule: &Value) -> Result<Routine, String> {
    let sched = normalize_schedule(schedule)?;
    let next = routines::next_run(&sched, now_ms()).ok_or("That schedule never runs")?;
    let r = Routine {
        id: new_id()[..10].to_string(),
        bot_id: bot_id.into(),
        name: truncate_chars(name.trim(), 80),
        prompt: prompt.trim().into(),
        schedule: sched,
        enabled: true,
        next_run_at: Some(next),
        last_run_at: None,
        unread_runs: 0,
        created_at: now_ms(),
    };
    if r.name.is_empty() || r.prompt.is_empty() {
        return Err("A routine needs a name and a prompt".into());
    }
    Ok(r)
}

/// Parse "2026-11-03T09:00" (local) into unix ms.
fn parse_local(at: &str) -> Option<i64> {
    use chrono::NaiveDateTime;
    let at = at.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(at) {
        return Some(d.timestamp_millis());
    }
    for f in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(at, f) {
            return crate::util::from_local(n);
        }
    }
    None
}

fn normalize_schedule(v: &Value) -> Result<Value, String> {
    let mut v = v.clone();
    if v.get("kind").and_then(Value::as_str) == Some("once") {
        if let Some(at) = v.get("at").and_then(Value::as_str) {
            let ms = parse_local(at).ok_or("couldn't read \"at\"; use a local time like 2026-11-03T09:00")?;
            v["at"] = json!(ms);
        }
    }
    routines::validate(&v)
}

fn post(state: &Shared, bot_id: &str, role: &str, text: &str, meta: Value) -> Message {
    let m = Message {
        id: new_id(),
        bot_id: bot_id.into(),
        role: role.into(),
        text: text.into(),
        html: String::new(),
        status: "done".into(),
        meta,
        created_at: now_ms(),
    };
    let _ = state.db.upsert_message(&m);
    state.emit_message(true, &m);
    m
}

fn url_path(rel: &str) -> String {
    let mut out = String::new();
    for c in rel.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(c as char),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

/// What a tool hands back to the model: text, and for screen tools a screenshot.
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
    pub image: Option<(String, String)>, // (base64, mime type)
}

/// How many log entries go into the Bot's prompt (newest first) and into a plain `recall`.
pub const RECENT_LOG: usize = 15;

/// `recall`: with a query, the best word matches across profile and log; without one, the
/// profile plus the newest log entries.
pub fn recall_text(mems: &[Memory], query: &str) -> String {
    let line = |m: &Memory| format!("[{}] ({}, {}) {}", m.id, m.tier, if m.bot_id.is_some() { "me" } else { "user" }, m.text);
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        let profile: Vec<_> = mems.iter().filter(|m| m.tier != crate::db::LOG).collect();
        let logs: Vec<_> = mems.iter().filter(|m| m.tier == crate::db::LOG).collect();
        let mut out = String::from("Profile:\n");
        if profile.is_empty() {
            out.push_str("(empty)\n");
        }
        for m in &profile {
            out.push_str(&line(m));
            out.push('\n');
        }
        out.push_str(&format!("\nNewest log entries ({} of {}; search with query for older ones):\n", logs.len().min(30), logs.len()));
        for m in logs.iter().rev().take(30) {
            out.push_str(&line(m));
            out.push('\n');
        }
        return out;
    }
    // Score = how many query words appear (a word also matches the start of a longer word);
    // newer entries win ties.
    let mut scored: Vec<(usize, &Memory)> = mems
        .iter()
        .filter_map(|m| {
            let text: Vec<String> = m.text.to_lowercase().split(|c: char| !c.is_alphanumeric()).map(str::to_string).collect();
            let n = words.iter().filter(|w| text.iter().any(|t| t.starts_with(w.as_str()))).count();
            (n > 0).then_some((n, m))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.created_at.cmp(&a.1.created_at)));
    if scored.is_empty() {
        return format!("Nothing in memory matches \"{query}\". Try other words, or recall with no query to list the newest entries.");
    }
    scored.iter().take(20).map(|(_, m)| line(m)).collect::<Vec<_>>().join("\n")
}


/// Run one openbot tool.
pub async fn call(state: &Shared, bot_id: &str, name: &str, args: &Value) -> ToolResult {
    if name.starts_with("computer_") {
        if state.db.get_bot(bot_id).ok().flatten().is_none() {
            return ToolResult { text: "this bot no longer exists".into(), is_error: true, image: None };
        }
        return match crate::screen::computer_tool(state, bot_id, name, args).await {
            Ok(o) => ToolResult { text: o.text, is_error: false, image: o.image },
            Err(e) => ToolResult { text: e, is_error: true, image: None },
        };
    }
    if crate::logins::is_tool(name) || crate::secure_form::is_tool(name) {
        let out = if crate::secure_form::is_tool(name) {
            crate::secure_form::tool(state, bot_id, args).await
        } else {
            crate::logins::tool(state, bot_id, name, args).await
        };
        return match out {
            Ok(t) => ToolResult { text: t, is_error: false, image: None },
            Err(e) => ToolResult { text: e, is_error: true, image: None },
        };
    }
    if crate::accounts::is_tool(name) || crate::drive::is_tool(name) || crate::jobs::is_tool(name) {
        let bot = match state.db.get_bot(bot_id) {
            Ok(Some(b)) => b,
            _ => return ToolResult { text: "this bot no longer exists".into(), is_error: true, image: None },
        };
        let out = if crate::jobs::is_tool(name) {
            crate::jobs::tool(state, &bot, name, args).await
        } else if crate::drive::is_tool(name) {
            crate::drive::tool(state, &bot, name, args).await
        } else {
            crate::accounts::tool(state, &bot, name, args).await
        };
        return match out {
            Ok(t) => ToolResult { text: t, is_error: false, image: None },
            Err(e) => ToolResult { text: e, is_error: true, image: None },
        };
    }
    match run(state, bot_id, name, args).await {
        Ok(t) => ToolResult { text: t, is_error: false, image: None },
        Err(e) => ToolResult { text: e, is_error: true, image: None },
    }
}

async fn run(state: &Shared, bot_id: &str, name: &str, args: &Value) -> Result<String, String> {
    let bot = state
        .db
        .get_bot(bot_id)
        .map_err(|e| e.to_string())?
        .ok_or("this bot no longer exists")?;
    let workdir = PathBuf::from(&bot.workdir);

    match name {
        "ask_user" => {
            let q = s(args, "question");
            let options: Vec<String> = args
                .get("options")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|o| o.as_str().map(|x| truncate_chars(x.trim(), 80))).collect())
                .unwrap_or_default();
            if q.is_empty() || options.is_empty() {
                return Err("ask_user needs a question and at least one option".into());
            }
            let allow_custom = args.get("allow_custom").and_then(Value::as_bool).unwrap_or(true);
            post(
                state,
                bot_id,
                "widget",
                q,
                json!({"question": q, "options": options.iter().take(6).collect::<Vec<_>>(), "allow_custom": allow_custom, "answer": null}),
            );
            Ok("The question is showing in the chat. End your turn now; the user's answer will arrive as their next message.".into())
        }

        "send_file" => {
            let p = s(args, "path");
            if p.is_empty() {
                return Err("send_file needs a path".into());
            }
            let mut src = crate::util::from_wsl_path(p);
            if src.is_relative() {
                src = workdir.join(src);
            }
            if !src.is_file() {
                return Err(format!("No file at {}", src.display()));
            }
            // Files outside the bot folder are copied into it so the chat can serve them.
            let root = std::fs::canonicalize(&workdir).map_err(|e| e.to_string())?;
            let real = std::fs::canonicalize(&src).map_err(|e| e.to_string())?;
            let rel = if real.starts_with(&root) {
                real.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")
            } else {
                let dir = root.join("sent");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let fname = real.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
                std::fs::copy(&real, dir.join(&fname)).map_err(|e| e.to_string())?;
                format!("sent/{fname}")
            };
            let fname = Path::new(&rel).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let mime = mime_guess::from_path(&rel).first_or_octet_stream().to_string();
            let size = std::fs::metadata(&real).map(|m| m.len()).unwrap_or(0);
            post(
                state,
                bot_id,
                "attachment",
                s(args, "caption"),
                json!({"name": fname, "path": rel, "mime": mime, "size": size,
                       "url": format!("/api/bots/{bot_id}/files/{}", url_path(&rel))}),
            );
            Ok(format!("Sent {fname} to the user."))
        }

        "show_page" => {
            let title = s(args, "title");
            let html = if let Some(h) = args.get("html").and_then(Value::as_str).filter(|h| !h.trim().is_empty()) {
                h.to_string()
            } else {
                let p = s(args, "path");
                if p.is_empty() {
                    return Err("show_page needs html or a path".into());
                }
                let mut src = crate::util::from_wsl_path(p);
                if src.is_relative() {
                    src = workdir.join(src);
                }
                std::fs::read_to_string(&src).map_err(|e| format!("Couldn't read {}: {e}", src.display()))?
            };
            if html.len() > 2_000_000 {
                return Err("That page is too big (2 MB max)".into());
            }
            let id = new_id();
            let dir = state.data_dir.join("pages");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{id}.html")), html).map_err(|e| e.to_string())?;
            post(state, bot_id, "visualization", title, json!({"title": title, "url": format!("/pages/{id}")}));
            Ok("The page is showing in the chat.".into())
        }

        "remember" => {
            let fact = s(args, "fact");
            if fact.is_empty() {
                return Err("remember needs a fact".into());
            }
            let target = target_bot(state, &bot, args)?;
            let for_other = target.id != bot_id;
            let about_user = !for_other && s(args, "about") != "me";
            let tier = if s(args, "tier") == crate::db::PROFILE { crate::db::PROFILE } else { crate::db::LOG };
            let m = Memory {
                id: new_id()[..8].to_string(),
                bot_id: if about_user { None } else { Some(target.id.clone()) },
                text: truncate_chars(fact, 500),
                created_at: now_ms(),
                tier: tier.into(),
            };
            state.db.add_memory(&m).map_err(|e| e.to_string())?;
            state.emit("memories.updated", json!({"bot_id": target.id}));
            if for_other {
                Ok(format!("Saved for {} (id {}, {tier}). They'll know it from their next message.", target.name, m.id))
            } else {
                Ok(format!("Remembered in your {tier} (id {}).", m.id))
            }
        }

        "recall" => {
            let target = target_bot(state, &bot, args)?;
            let mems = state.db.memories_for(&target.id).map_err(|e| e.to_string())?;
            if mems.is_empty() {
                return Ok(if target.id == bot_id { "You don't have any saved memories yet.".into() } else { format!("{} has no saved memories yet.", target.name) });
            }
            Ok(recall_text(&mems, s(args, "query")))
        }

        "update_memory" => {
            let id = s(args, "id");
            let fact = s(args, "fact");
            let tier = match s(args, "tier") {
                "" => None,
                t if t == crate::db::PROFILE || t == crate::db::LOG => Some(t),
                _ => return Err("tier must be profile or log".into()),
            };
            let fact = (!fact.is_empty()).then(|| truncate_chars(fact, 500));
            if state.db.update_memory(id, fact.as_deref(), tier).map_err(|e| e.to_string())? {
                state.emit("memories.updated", json!({"bot_id": bot_id}));
                Ok("Updated.".into())
            } else {
                Err("No memory with that id".into())
            }
        }

        "forget" => {
            if state.db.delete_memory(s(args, "id")).map_err(|e| e.to_string())? {
                state.emit("memories.updated", json!({"bot_id": bot_id}));
                Ok("Forgotten.".into())
            } else {
                Err("No memory with that id".into())
            }
        }

        "check_back_later" => {
            let note = s(args, "note");
            let at = if let Some(m) = args.get("in_minutes").and_then(Value::as_i64) {
                now_ms() + m.max(1) * 60_000
            } else {
                parse_local(s(args, "at")).ok_or("Give in_minutes or a local time in \"at\" like 2026-11-03T09:00")?
            };
            let r = Routine {
                id: new_id()[..10].to_string(),
                bot_id: bot_id.into(),
                name: truncate_chars(if note.is_empty() { "Check-in" } else { note }, 60),
                prompt: note.into(),
                schedule: json!({"kind": "once", "at": at}),
                enabled: true,
                next_run_at: Some(at),
                last_run_at: None,
                unread_runs: 0,
                created_at: now_ms(),
            };
            state.db.upsert_routine(&r).map_err(|e| e.to_string())?;
            state.emit("routines.updated", json!({"bot_id": bot_id}));
            Ok(format!("OK, you'll be woken up {}.", routines::describe(&r.schedule).to_lowercase()))
        }

        "create_routine" => {
            let target = target_bot(state, &bot, args)?;
            let r = new_routine(&target.id, s(args, "name"), s(args, "prompt"), args.get("schedule").unwrap_or(&Value::Null))?;
            state.db.upsert_routine(&r).map_err(|e| e.to_string())?;
            let what = format!("“{}” · {}", r.name, routines::describe(&r.schedule));
            if target.id == bot_id {
                crate::runner::add_event(state, bot_id, &format!("Created routine {what}"), json!({"kind": "routine-created", "routine_id": r.id}));
            } else {
                crate::runner::add_event(state, bot_id, &format!("Created routine {what} for {}", target.name), json!({"kind": "routine-created", "routine_id": r.id, "bot_id": target.id}));
                crate::runner::add_event(state, &target.id, &format!("{} gave you the routine {what}", bot.name), json!({"kind": "routine-created", "routine_id": r.id, "bot_id": bot_id}));
            }
            state.emit("routines.updated", json!({"bot_id": target.id}));
            Ok(format!("Routine saved (id {}) for {}. {}.", r.id, if target.id == bot_id { "you".to_string() } else { target.name.clone() }, routines::describe(&r.schedule)))
        }

        "list_routines" => {
            let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
            let rs = if all {
                state.db.routines_for(None).map_err(|e| e.to_string())?
            } else {
                let target = target_bot(state, &bot, args)?;
                state.db.routines_for(Some(&target.id)).map_err(|e| e.to_string())?
            };
            if rs.is_empty() {
                return Ok("No routines yet.".into());
            }
            let names: std::collections::HashMap<String, String> =
                state.db.list_bots().map_err(|e| e.to_string())?.into_iter().map(|b| (b.id, b.name)).collect();
            Ok(rs
                .iter()
                .map(|r| {
                    let mut v = routine_json(r);
                    v["bot_name"] = json!(names.get(&r.bot_id).cloned().unwrap_or_default());
                    v.to_string()
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }

        "update_routine" => {
            let mut r = state.db.get_routine(s(args, "id")).map_err(|e| e.to_string())?.ok_or("No routine with that id")?;
            if let Some(n) = args.get("name").and_then(Value::as_str) {
                r.name = truncate_chars(n.trim(), 80);
            }
            if let Some(p) = args.get("prompt").and_then(Value::as_str) {
                r.prompt = p.trim().into();
            }
            if let Some(sc) = args.get("schedule") {
                r.schedule = normalize_schedule(sc)?;
            }
            if let Some(e) = args.get("enabled").and_then(Value::as_bool) {
                r.enabled = e;
                r.unread_runs = 0;
            }
            r.next_run_at = if r.enabled { routines::next_run(&r.schedule, now_ms()) } else { None };
            state.db.upsert_routine(&r).map_err(|e| e.to_string())?;
            state.emit("routines.updated", json!({"bot_id": r.bot_id}));
            Ok("Routine updated.".into())
        }

        "delete_routine" => {
            let r = state.db.get_routine(s(args, "id")).map_err(|e| e.to_string())?.ok_or("No routine with that id")?;
            state.db.delete_routine(&r.id).map_err(|e| e.to_string())?;
            if r.bot_id != bot_id {
                crate::runner::add_event(state, &r.bot_id, &format!("{} deleted the routine “{}”", bot.name, r.name), json!({"kind": "routine-deleted", "bot_id": bot_id}));
            }
            state.emit("routines.updated", json!({"bot_id": r.bot_id}));
            Ok("Routine deleted.".into())
        }

        "move_routine" => {
            let mut r = state.db.get_routine(s(args, "id")).map_err(|e| e.to_string())?.ok_or("No routine with that id")?;
            let to = find_bot(state, s(args, "to_bot"))?;
            if r.bot_id == to.id {
                return Ok(format!("That routine already belongs to {}.", to.name));
            }
            let from = state.db.get_bot(&r.bot_id).ok().flatten().map(|b| b.name).unwrap_or_else(|| "another Bot".into());
            let from_id = std::mem::replace(&mut r.bot_id, to.id.clone());
            r.unread_runs = 0;
            state.db.upsert_routine(&r).map_err(|e| e.to_string())?;
            let what = format!("“{}” · {}", r.name, routines::describe(&r.schedule));
            crate::runner::add_event(state, &from_id, &format!("Moved the routine {what} to {}", to.name), json!({"kind": "routine-moved", "routine_id": r.id, "bot_id": to.id}));
            crate::runner::add_event(state, &to.id, &format!("Took over the routine {what} from {from}"), json!({"kind": "routine-moved", "routine_id": r.id, "bot_id": from_id}));
            state.emit("routines.updated", json!({"bot_id": from_id}));
            state.emit("routines.updated", json!({"bot_id": to.id}));
            Ok(format!("Moved. {} now runs it ({}).", to.name, routines::describe(&r.schedule)))
        }

        "list_bots" => {
            let main = state.db.kv_get("main_bot").unwrap_or_default();
            let bots = state.db.list_bots().map_err(|e| e.to_string())?;
            let mut out = vec![];
            for b in bots {
                let routines = state.db.routines_for(Some(&b.id)).map(|v| v.len()).unwrap_or(0);
                let private = state.db.memories_for(&b.id).map(|v| v.iter().filter(|m| m.bot_id.is_some()).count()).unwrap_or(0);
                let mut tags = vec![];
                if b.id == main { tags.push("Main Bot"); }
                if b.id == bot_id { tags.push("you"); }
                if b.hidden { tags.push("hidden"); }
                let instr = b.instructions.trim().replace('\n', " ");
                out.push(format!(
                    "- {name} (id {id}){tags}{label} · {routines} · {private}\n  Instructions: {instr}",
                    name = b.name,
                    id = b.id,
                    tags = if tags.is_empty() { String::new() } else { format!(" [{}]", tags.join(", ")) },
                    label = if b.label.is_empty() { String::new() } else { format!(" — {}", b.label) },
                    instr = if instr.is_empty() { "(none)".to_string() } else { truncate_chars(&instr, 240) },
                    routines = count(routines, "routine", "routines"),
                    private = count(private, "private memory", "private memories"),
                ));
            }
            Ok(format!("{} Bots:\n{}", out.len(), out.join("\n")))
        }

        "get_bot" => {
            let b = find_bot(state, s(args, "bot"))?;
            let main = state.db.kv_get("main_bot").unwrap_or_default();
            let mems: Vec<Value> = state
                .db
                .memories_for(&b.id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|m| m.bot_id.is_some())
                .map(|m| json!({"id": m.id, "text": m.text}))
                .collect();
            let rs: Vec<Value> = state.db.routines_for(Some(&b.id)).map_err(|e| e.to_string())?.iter().map(routine_json).collect();
            Ok(json!({
                "id": b.id, "name": b.name, "label": b.label, "look": b.color,
                "main_bot": b.id == main, "is_you": b.id == bot_id,
                "instructions": b.instructions,
                "private_memories": mems,
                "routines": rs,
                "note": "Facts about the user are shared by every Bot and aren't listed here; use recall to see them."
            })
            .to_string())
        }

        "create_bot" => {
            let name = s(args, "name");
            if name.is_empty() || name.chars().count() > 40 {
                return Err("Give the Bot a name of 1 to 40 characters".into());
            }
            let instructions = s(args, "instructions");
            if instructions.is_empty() {
                return Err("Give the Bot standing instructions (its job and how to behave)".into());
            }
            if instructions.chars().count() > crate::bots::MAX_INSTRUCTIONS {
                return Err(format!("Instructions are too long (max {} characters)", crate::bots::MAX_INSTRUCTIONS));
            }
            let look = s(args, "look");
            check_look(look)?;
            let bots = state.db.list_bots().map_err(|e| e.to_string())?;
            if let Some(same) = bots.iter().find(|b| b.name.eq_ignore_ascii_case(name)) {
                return Err(format!("There's already a Bot called {} (id {}). Use update_bot to change it, or pick another name.", same.name, same.id));
            }
            if bots.len() >= 50 {
                return Err("The user already has 50 Bots; retire some first.".into());
            }
            // Check everything before creating anything, so a mistake doesn't leave half a Bot.
            let facts: Vec<String> = args
                .get("memories")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect())
                .unwrap_or_default();
            if facts.len() > 50 {
                return Err("At most 50 memories at once".into());
            }
            let routine_specs = args.get("routines").and_then(Value::as_array).cloned().unwrap_or_default();
            if routine_specs.len() > 10 {
                return Err("At most 10 routines at once".into());
            }
            let mut planned = vec![];
            for (i, r) in routine_specs.iter().enumerate() {
                let p = new_routine("", s(r, "name"), s(r, "prompt"), r.get("schedule").unwrap_or(&Value::Null))
                    .map_err(|e| format!("Routine {} (“{}”): {e}", i + 1, s(r, "name")))?;
                planned.push(p);
            }

            let nb = crate::bots::NewBot {
                name: name.into(),
                color: if look.is_empty() { crate::bots::free_color(&state.db) } else { look.into() },
                shape: "ball".into(),
                instructions: instructions.into(),
                model: String::new(),
                label: s(args, "label").into(),
            };
            let new = crate::bots::create(state, &nb).await.map_err(|e| e.to_string())?;
            for f in &facts {
                state
                    .db
                    .add_memory(&Memory { id: new_id()[..8].to_string(), bot_id: Some(new.id.clone()), text: truncate_chars(f, 500), created_at: now_ms(), tier: crate::db::PROFILE.into() })
                    .map_err(|e| e.to_string())?;
            }
            for mut r in planned {
                r.bot_id = new.id.clone();
                state.db.upsert_routine(&r).map_err(|e| e.to_string())?;
            }
            crate::runner::add_event(state, &new.id, &format!("Set up by {}", bot.name), json!({"kind": "bot-created-by", "bot_id": bot_id}));
            crate::runner::add_event(state, bot_id, &format!("Set up the Bot “{}”", new.name), json!({"kind": "bot-created", "bot_id": new.id}));
            state.emit("memories.updated", json!({"bot_id": new.id}));
            state.emit("routines.updated", json!({"bot_id": new.id}));
            crate::bots::say_hello(state, &new, s(args, "briefing")).await;
            Ok(format!(
                "Created {} (id {}) with {} and {}. It's in the user's sidebar and is saying hello in its own chat.",
                new.name,
                new.id,
                count(facts.len(), "private memory", "private memories"),
                count(routine_specs.len(), "routine", "routines")
            ))
        }

        "update_bot" => {
            let mut b = find_bot(state, s(args, "bot"))?;
            let mut changed = vec![];
            if let Some(n) = args.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
                if n.chars().count() > 40 {
                    return Err("Names are at most 40 characters".into());
                }
                if let Ok(other) = find_bot(state, n) {
                    if other.id != b.id {
                        return Err(format!("There's already a Bot called {}", other.name));
                    }
                }
                b.name = n.into();
                changed.push("name");
            }
            if let Some(l) = args.get("label").and_then(Value::as_str) {
                b.label = truncate_chars(l.trim(), 60);
                changed.push("tagline");
            }
            if let Some(l) = args.get("look").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty()) {
                check_look(l)?;
                b.color = l.into();
                b.shape = "ball".into();
                changed.push("look");
            }
            if let Some(i) = args.get("instructions").and_then(Value::as_str) {
                b.instructions = i.trim().into();
                changed.push("instructions");
            }
            if let Some(a) = args.get("add_instructions").and_then(Value::as_str).map(str::trim).filter(|a| !a.is_empty()) {
                b.instructions = if b.instructions.trim().is_empty() { a.into() } else { format!("{}\n\n{a}", b.instructions.trim_end()) };
                if !changed.contains(&"instructions") {
                    changed.push("instructions");
                }
            }
            if changed.is_empty() {
                return Err("Nothing to change: give name, label, look, instructions or add_instructions".into());
            }
            if b.instructions.chars().count() > crate::bots::MAX_INSTRUCTIONS {
                return Err(format!("Instructions would be too long (max {} characters)", crate::bots::MAX_INSTRUCTIONS));
            }
            b.updated_at = now_ms();
            state.db.upsert_bot(&b).map_err(|e| e.to_string())?;
            state.emit_bot(&b.id).await;
            let list = changed.join(", ");
            if b.id == bot_id {
                crate::runner::add_event(state, bot_id, &format!("Updated its own {list}"), json!({"kind": "bot-updated", "bot_id": b.id}));
            } else {
                crate::runner::add_event(state, bot_id, &format!("Updated {}: {list}", b.name), json!({"kind": "bot-updated", "bot_id": b.id}));
                crate::runner::add_event(state, &b.id, &format!("{} updated your {list}", bot.name), json!({"kind": "bot-updated-by", "bot_id": bot_id}));
            }
            Ok(format!("Updated {} ({list}). It applies from {} next message.", b.name, if b.id == bot_id { "your" } else { "its" }))
        }

        "delete_bot" => {
            let b = find_bot(state, s(args, "bot"))?;
            if b.id == bot_id {
                return Err("You can't delete yourself. The user can do it from the app.".into());
            }
            if state.db.kv_get("main_bot").as_deref() == Some(b.id.as_str()) {
                return Err(format!("{} is the user's Main Bot and can't be deleted.", b.name));
            }
            let rcount = state.db.routines_for(Some(&b.id)).map(|v| v.len()).unwrap_or(0);
            let reason = truncate_chars(s(args, "reason").trim_end_matches(['.', '!', ' ']), 200);
            let summary = format!(
                "Delete the Bot “{}”{}. {} deleted too.",
                b.name,
                if reason.is_empty() { String::new() } else { format!(" ({reason})") },
                if rcount > 0 { format!("Its chat, its own memories and its {} are", count(rcount, "routine", "routines")) } else { "Its chat and its own memories are".to_string() }
            );
            // Always asks, whatever the permission settings: this can't be undone.
            let ok = crate::api::ask_approval(state, bot_id, "mcp__openbot__delete_bot", &summary, &json!({"bot": b.name, "bot_id": b.id, "reason": reason}), "", "")
                .await
                .map_err(|e| e.to_string())?;
            if !ok {
                return Err(format!("The user chose to keep {}. Don't ask again unless they bring it up.", b.name));
            }
            crate::bots::delete(state, &b.id).await?;
            crate::runner::add_event(state, bot_id, &format!("Deleted the Bot “{}”", b.name), json!({"kind": "bot-deleted"}));
            Ok(format!("Deleted {}.", b.name))
        }

        "request_control" => {
            crate::screen::platform_ok()?;
            let reason = truncate_chars(s(args, "reason"), 200);
            let reason = if reason.is_empty() { "I need you to do something on my screen.".to_string() } else { reason };
            // Make sure the screen is up so "Open screen" works right away.
            state.screens.ensure(state, bot_id).await?;
            post(
                state,
                bot_id,
                "widget",
                &reason,
                json!({"question": reason, "options": ["I'm done"], "allow_custom": true, "answer": null, "screen": true}),
            );
            Ok("A card asking the user to take over your screen is in the chat. End your turn now; when they reply (usually \"I'm done\"), take a screenshot and continue.".into())
        }

        "set_status" => {
            crate::runner::set_activity(state, bot_id, Some(("status", &truncate_chars(s(args, "text"), 60)))).await;
            Ok("Status shown.".into())
        }

        other => Err(format!("Unknown openbot tool {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: &str, text: &str, tier: &str, at: i64) -> Memory {
        Memory { id: id.into(), bot_id: None, text: text.into(), created_at: at, tier: tier.into() }
    }

    #[test]
    fn recall_searches_and_lists() {
        let m = vec![
            mem("a", "Lives in San Diego", "profile", 1),
            mem("b", "Dentist appointment on Friday", "log", 2),
            mem("c", "Allergic to peanuts", "profile", 3),
            mem("d", "Booked a dentist cleaning for December", "log", 4),
        ];
        let found = recall_text(&m, "dentist");
        assert!(found.starts_with("[d]") && found.contains("[b]") && !found.contains("[a]"), "{found}");
        assert!(recall_text(&m, "zebra").starts_with("Nothing in memory matches"));
        let all = recall_text(&m, "");
        assert!(all.starts_with("Profile:") && all.contains("Newest log entries (2 of 2"), "{all}");
    }
}
