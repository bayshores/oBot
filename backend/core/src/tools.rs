//! openbot's own tools for bots (served to Claude Code through the `openbot` MCP server in mcp.rs).
//! They give bots extra abilities: quick-reply buttons, sending files, showing pages,
//! memory, routines, check-ins and a status line.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::db::{Memory, Message, Routine};
use crate::routines::{self, routine_json};
use crate::state::Shared;
use crate::util::{new_id, now_ms, truncate_chars};

/// Tool definitions advertised over MCP (besides `approve`, which lives in mcp.rs).
pub fn definitions() -> Value {
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
            "description": "Save a lasting fact so you (and, for facts about the user, every bot) remember it in future chats. Use for preferences, names, routines, important details. Keep each fact short.",
            "inputSchema": {"type": "object", "properties": {
                "fact": {"type": "string"},
                "about": {"type": "string", "enum": ["user", "me"], "description": "'user' = about the user (shared with all bots), 'me' = only for this bot"}
            }, "required": ["fact"]}
        },
        {
            "name": "recall",
            "description": "List everything you remember (with ids, so you can forget outdated facts).",
            "inputSchema": {"type": "object", "properties": {}}
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
            "description": "Create a recurring task you'll run on a schedule (e.g. a morning briefing every weekday at 8:00). Only create one when the user asks for it or agrees.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "prompt": {"type": "string", "description": "What to do each time"},
                "schedule": sched
            }, "required": ["name", "prompt", "schedule"]}
        },
        {
            "name": "list_routines",
            "description": "List your routines and scheduled check-ins.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "update_routine",
            "description": "Change, pause or resume one of your routines.",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"}, "name": {"type": "string"}, "prompt": {"type": "string"},
                "schedule": sched, "enabled": {"type": "boolean"}
            }, "required": ["id"]}
        },
        {
            "name": "delete_routine",
            "description": "Delete one of your routines.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}
        },
        {
            "name": "set_status",
            "description": "Show a short status under your name while you work on something long (e.g. 'Comparing 12 flights').",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}
        }
    ]);
    // Bots get their own screen where this computer can run one (Windows + WSL, or Linux).
    if crate::screen::platform_ok().is_ok() {
        if let Some(a) = list.as_array_mut() {
            a.extend(crate::screen::tool_definitions());
        }
    }
    list
}

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("").trim()
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
            let about_user = s(args, "about") != "me";
            let m = Memory {
                id: new_id()[..8].to_string(),
                bot_id: if about_user { None } else { Some(bot_id.to_string()) },
                text: truncate_chars(fact, 500),
                created_at: now_ms(),
            };
            state.db.add_memory(&m).map_err(|e| e.to_string())?;
            state.emit("memories.updated", json!({"bot_id": bot_id}));
            Ok(format!("Remembered (id {}).", m.id))
        }

        "recall" => {
            let mems = state.db.memories_for(bot_id).map_err(|e| e.to_string())?;
            if mems.is_empty() {
                return Ok("You don't have any saved memories yet.".into());
            }
            Ok(mems
                .iter()
                .map(|m| format!("[{}] ({}) {}", m.id, if m.bot_id.is_some() { "me" } else { "user" }, m.text))
                .collect::<Vec<_>>()
                .join("\n"))
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
            let sched = normalize_schedule(args.get("schedule").unwrap_or(&Value::Null))?;
            let next = routines::next_run(&sched, now_ms()).ok_or("That schedule never runs")?;
            let r = Routine {
                id: new_id()[..10].to_string(),
                bot_id: bot_id.into(),
                name: truncate_chars(s(args, "name"), 80),
                prompt: s(args, "prompt").into(),
                schedule: sched,
                enabled: true,
                next_run_at: Some(next),
                last_run_at: None,
                unread_runs: 0,
                created_at: now_ms(),
            };
            if r.name.is_empty() || r.prompt.is_empty() {
                return Err("create_routine needs a name and a prompt".into());
            }
            state.db.upsert_routine(&r).map_err(|e| e.to_string())?;
            crate::runner::add_event(
                state,
                bot_id,
                &format!("Created routine “{}” · {}", r.name, routines::describe(&r.schedule)),
                json!({"kind": "routine-created", "routine_id": r.id}),
            );
            state.emit("routines.updated", json!({"bot_id": bot_id}));
            Ok(format!("Routine saved (id {}). {}.", r.id, routines::describe(&r.schedule)))
        }

        "list_routines" => {
            let rs = state.db.routines_for(Some(bot_id)).map_err(|e| e.to_string())?;
            if rs.is_empty() {
                return Ok("No routines yet.".into());
            }
            Ok(rs.iter().map(|r| routine_json(r).to_string()).collect::<Vec<_>>().join("\n"))
        }

        "update_routine" => {
            let mut r = state
                .db
                .get_routine(s(args, "id"))
                .map_err(|e| e.to_string())?
                .filter(|r| r.bot_id == bot_id)
                .ok_or("No routine with that id")?;
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
            state.emit("routines.updated", json!({"bot_id": bot_id}));
            Ok("Routine updated.".into())
        }

        "delete_routine" => {
            let id = s(args, "id");
            let ok = state
                .db
                .get_routine(id)
                .map_err(|e| e.to_string())?
                .filter(|r| r.bot_id == bot_id)
                .is_some();
            if !ok {
                return Err("No routine with that id".into());
            }
            state.db.delete_routine(id).map_err(|e| e.to_string())?;
            state.emit("routines.updated", json!({"bot_id": bot_id}));
            Ok("Routine deleted.".into())
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
