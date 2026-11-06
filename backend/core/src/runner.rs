//! Runs Claude Code for a bot and turns its live output into chat messages.
//!
//! Every user message becomes one `claude -p` run in the bot's working folder.
//! `--resume <session>` keeps the whole conversation going across runs, so each bot
//! is one long-lived Claude Code conversation.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Notify};

use crate::computer::find_claude;
use crate::config::Config;
use crate::db::{Bot, Message};
use crate::markdown;
use crate::state::{Job, RunHandle, Shared};
use crate::util::{hide_console, new_id, now_ms, preview, truncate_chars};

// ---------------------------------------------------------------- queue

/// Queue a prompt for a bot. Messages sent while the bot is busy are merged into its next turn.
pub async fn enqueue(state: &Shared, bot_id: &str, prompt: String) {
    let mut queues = state.queues.lock().await;
    let needs_worker = queues.get(bot_id).map_or(true, |tx| tx.is_closed());
    if needs_worker {
        let (tx, rx) = mpsc::unbounded_channel();
        queues.insert(bot_id.to_string(), tx);
        tokio::spawn(worker(state.clone(), bot_id.to_string(), rx));
    }
    if let Some(tx) = queues.get(bot_id) {
        let _ = tx.send(Job { prompt });
    }
}

async fn worker(state: Shared, bot_id: String, mut rx: mpsc::UnboundedReceiver<Job>) {
    while let Some(first) = rx.recv().await {
        let mut prompt = first.prompt;
        while let Ok(more) = rx.try_recv() {
            prompt.push_str("\n\n");
            prompt.push_str(&more.prompt);
        }
        let stopped = Arc::new(AtomicBool::new(false));
        if let Err(e) = run_turn(&state, &bot_id, &prompt, stopped.clone()).await {
            add_simple(&state, &bot_id, "error", &e.to_string());
        }
        if stopped.load(Ordering::SeqCst) {
            // "Stop" also throws away anything queued behind the current turn.
            while rx.try_recv().is_ok() {}
        }
    }
}

/// Ask a running bot to stop.
pub async fn stop(state: &Shared, bot_id: &str) {
    if let Some(h) = state.runs.lock().await.get(bot_id) {
        h.stop.notify_one();
    }
    deny_pending(state, bot_id).await;
}

async fn deny_pending(state: &Shared, bot_id: &str) {
    let mut approvals = state.approvals.lock().await;
    let ids: Vec<String> = approvals
        .iter()
        .filter(|(_, p)| p.bot_id == bot_id)
        .map(|(k, _)| k.clone())
        .collect();
    for id in ids {
        if let Some(p) = approvals.remove(&id) {
            let _ = p.reply.send(false);
            if let Ok(Some(mut m)) = state.db.get_message(&p.message_id) {
                m.status = "denied".into();
                let _ = state.db.upsert_message(&m);
                state.emit_message(false, &m);
            }
        }
    }
}

/// A small centered line in the chat ("Created routine ...").
pub fn add_event(state: &Shared, bot_id: &str, text: &str, meta: Value) -> Message {
    let m = Message {
        id: new_id(),
        bot_id: bot_id.into(),
        role: "event".into(),
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

/// Update the live status line under a bot's name. `None` clears it.
pub async fn set_activity(state: &Shared, bot_id: &str, a: Option<(&str, &str)>) {
    let changed = {
        let mut map = state.activity.lock().unwrap_or_else(|e| e.into_inner());
        match a {
            Some((kind, label)) => {
                let v = json!({ "kind": kind, "label": crate::vault::redact(label) });
                if map.get(bot_id) == Some(&v) {
                    false
                } else {
                    map.insert(bot_id.to_string(), v);
                    true
                }
            }
            None => map.remove(bot_id).is_some(),
        }
    };
    if changed {
        state.emit_bot(bot_id).await;
    }
}

/// Map a Claude Code tool name to friendly activity text.
pub fn activity_for(name: &str) -> (&'static str, String) {
    match name {
        "WebSearch" => ("searching_web", "Searching the web".into()),
        "WebFetch" => ("reading_web", "Reading the web".into()),
        "Read" | "Glob" | "Grep" | "NotebookRead" | "LS" | "Skill" => ("reading_file", "Reading files".into()),
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => ("writing_file", "Drafting the file".into()),
        "Bash" | "PowerShell" | "BashOutput" | "KillShell" => ("running_commands", "Running commands".into()),
        "Task" | "Agent" => ("working", "Working with a helper".into()),
        "TodoWrite" => ("thinking", "Planning".into()),
        "mcp__openbot__ask_user" | "mcp__openbot__request_control" => ("waiting_for_you", "Waiting for you".into()),
        n if n.starts_with("mcp__openbot__computer_") => ("on_its_computer", "On its computer".into()),
        "mcp__openbot__show_page" => ("working", "Making a page".into()),
        "mcp__openbot__send_file" => ("working", "Sending a file".into()),
        "mcp__openbot__remember" | "mcp__openbot__recall" | "mcp__openbot__forget" => ("thinking", "Remembering".into()),
        "mcp__openbot__list_bots" | "mcp__openbot__get_bot" => ("reading_file", "Looking at your Bots".into()),
        "mcp__openbot__create_bot" => ("working", "Setting up a Bot".into()),
        "mcp__openbot__update_bot" | "mcp__openbot__move_routine" => ("working", "Updating your Bots".into()),
        "mcp__openbot__delete_bot" => ("waiting_for_you", "Waiting for your OK".into()),
        "mcp__openbot__list_accounts" => ("thinking", "Checking accounts".into()),
        "mcp__openbot__email_search" | "mcp__openbot__email_read" | "mcp__openbot__email_attachment" => ("reading_file", "Reading email".into()),
        "mcp__openbot__email_send" => ("working", "Writing an email".into()),
        "mcp__openbot__email_update" => ("working", "Organizing email".into()),
        "mcp__openbot__calendar_events" => ("reading_file", "Checking the calendar".into()),
        n if n.starts_with("mcp__openbot__calendar_") => ("working", "Updating the calendar".into()),
        n if n.starts_with("mcp__openbot__") => ("working", "Working".into()),
        n if n.starts_with("mcp__playwright") || n.contains("browser") => ("browsing", "Browsing the web".into()),
        n if n.starts_with("mcp__computer") => ("on_its_computer", "On its computer".into()),
        n if n.starts_with("mcp__") => {
            let server = n.split("__").nth(1).unwrap_or("a service").replace(['_', '-'], " ");
            ("connecting", format!("Connecting to {server}"))
        }
        _ => ("working", "Working".into()),
    }
}

pub fn add_simple(state: &Shared, bot_id: &str, role: &str, text: &str) -> Message {
    let m = Message {
        id: new_id(),
        bot_id: bot_id.into(),
        role: role.into(),
        text: text.into(),
        html: String::new(),
        status: "done".into(),
        meta: json!({}),
        created_at: now_ms(),
    };
    let _ = state.db.upsert_message(&m);
    state.emit_message(true, &m);
    m
}

// ---------------------------------------------------------------- one turn

async fn run_turn(state: &Shared, bot_id: &str, prompt: &str, stopped: Arc<AtomicBool>) -> anyhow::Result<()> {
    let stop = Arc::new(Notify::new());
    state
        .runs
        .lock()
        .await
        .insert(bot_id.to_string(), RunHandle { stop: stop.clone() });
    set_activity(state, bot_id, Some(("thinking", "Thinking"))).await;
    state.emit_bot(bot_id).await;

    let mut result = drive(state, bot_id, prompt, stop.clone(), stopped.clone(), true).await;
    if let Ok(Outcome::ResumeFailed) = result {
        // The saved conversation is gone (e.g. bot moved to another computer): start a fresh one.
        if let Ok(Some(mut b)) = state.db.get_bot(bot_id) {
            b.session_id.clear();
            let _ = state.db.upsert_bot(&b);
        }
        result = drive(state, bot_id, prompt, stop, stopped, false).await;
    }

    state.runs.lock().await.remove(bot_id);
    if let Ok(mut map) = state.activity.lock() {
        map.remove(bot_id);
    }
    deny_pending(state, bot_id).await;
    state.emit_bot(bot_id).await;
    result.map(|_| ())
}

#[derive(PartialEq)]
enum Outcome {
    Done,
    ResumeFailed,
}

fn persona(state: &Shared, bot: &Bot, cfg: &Config) -> String {
    let host = sysinfo::System::host_name().unwrap_or_else(|| "this computer".into());
    let os = sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into());
    let mut s = format!(
        "You are \"{name}\", a personal AI assistant that lives in the openbot app.\n\
         The user chats with you like in a messaging app, so reply like a helpful friend texting: \
         warm, clear and to the point, in short paragraphs. Use Markdown only when it helps (lists, code, tables).\n\
         You run on the user's computer \"{host}\" ({os}) and can use its tools (shell, files, web) to actually get \
         things done instead of only giving advice. Your own working folder is: {workdir}\n\
         Files the user attaches are saved in the \"uploads\" folder inside your working folder.\n",
        name = bot.name,
        workdir = bot.workdir,
    );
    s.push_str(
        "\nopenbot gives you extra tools (mcp__openbot__*): ask_user (tap-to-answer buttons), send_file \
         (deliver files/images into the chat), show_page (interactive HTML in the chat), remember/recall/forget \
         (long-term memory), check_back_later (wake yourself up later and message the user), create_routine / \
         list_routines / update_routine / delete_routine (recurring tasks) and set_status. Use them like a \
         capable assistant would: remember useful facts about the user without being asked, send files instead \
         of pasting long content, and when you promise to follow up later, schedule it with check_back_later.\n\
         Memory works like this: facts saved with about:'user' are shared by all of the user's Bots; facts saved \
         with about:'me' are yours alone.\n\
         \nYou can also look after the user's other Bots: list_bots and get_bot (see them), create_bot (a new Bot \
         with its instructions, private memories and routines, ready from its first message), update_bot (name, \
         tagline, look, instructions), delete_bot (retire one; the user always confirms with an Allow / Deny card), \
         move_routine (hand a routine to another Bot), and the \"bot\" option of remember, recall, create_routine and \
         list_routines (act for another Bot). Only create, change or delete Bots when the user asks for it or agrees \
         to your plan. When splitting work across Bots, move each Bot's private facts and routines to it, and \
         retire old Bots only after everything worth keeping has been moved.\n",
    );
    if crate::screen::platform_ok().is_ok() {
        s.push_str(
            "\nYou also have your own computer screen: a 1280 x 800 Linux desktop with a Chromium browser that \
             keeps your logins. The user can watch it live and take control. Use it for websites that need a real \
             browser (clicking through pages, filling forms, logged-in sites): computer_open (open a URL), \
             computer_screenshot, computer_click, computer_type, computer_key, computer_scroll, computer_move, \
             computer_drag and computer_wait. Every action returns a fresh screenshot; look at it before the next \
             step. Never type passwords, 2FA codes or payment details yourself: call request_control, say what you \
             need, and end your turn. Do the same for CAPTCHAs or when you're stuck.\n",
        );
    }
    s.push_str(&format!(
        "Current local time for the user: {} ({}). Routine and check-in times use this timezone.\n",
        crate::util::now_local().format("%A %Y-%m-%d %H:%M"),
        crate::util::timezone_label()
    ));
    if let Ok(mems) = state.db.memories_for(&bot.id) {
        if !mems.is_empty() {
            s.push_str("\nWhat you remember (from earlier chats):\n");
            for m in mems.iter().take(200) {
                let who = if m.bot_id.is_some() { "you" } else { "user" };
                s.push_str(&format!("- [{}|{who}] {}\n", m.id, m.text));
            }
        }
    }
    if let Ok(rs) = state.db.routines_for(Some(&bot.id)) {
        let active: Vec<_> = rs.iter().filter(|r| r.enabled).collect();
        if !active.is_empty() {
            s.push_str("\nYour active routines / check-ins:\n");
            for r in active {
                s.push_str(&format!("- [{}] {} · {}\n", r.id, r.name, crate::routines::describe(&r.schedule)));
            }
        }
    }
    s.push_str(&crate::accounts::persona(cfg, &bot.id));
    let secrets = crate::vault::for_bot(cfg, &bot.id);
    if !secrets.is_empty() {
        s.push_str("\nSecrets the user saved for you (available as environment variables in your shell; never print their values):\n");
        for sec in secrets {
            let what = if sec.description.trim().is_empty() { "no description" } else { sec.description.trim() };
            s.push_str(&format!("- ${} — {what}\n", sec.name.trim()));
        }
    }
    if !bot.instructions.trim().is_empty() {
        s.push_str("\nStanding instructions from the user:\n");
        s.push_str(bot.instructions.trim());
        s.push('\n');
    }
    s
}

fn build_command(
    state: &Shared,
    bot: &Bot,
    cfg: &Config,
    claude: &PathBuf,
    resume: bool,
) -> anyhow::Result<Command> {
    let bot_dir = state.data_dir.join("bots").join(&bot.id);
    std::fs::create_dir_all(&bot_dir)?;
    std::fs::create_dir_all(&bot.workdir)?;

    // runner = "wsl" (Windows only): Claude Code runs inside a WSL Linux distro with its sandbox on.
    // The bot sees Windows paths as /mnt/c/..., so every path we hand it is translated.
    let wsl = cfg!(windows) && cfg.runner == "wsl";
    let p = |path: &std::path::Path| if wsl { crate::util::to_wsl_path(path) } else { path.to_string_lossy().into_owned() };

    let persona_path = bot_dir.join("persona.md");
    let mut persona_text = persona(state, bot, cfg);
    if wsl {
        persona_text = persona_text.replace(&bot.workdir, &crate::util::to_wsl_path(std::path::Path::new(&bot.workdir)));
    }
    std::fs::write(&persona_path, persona_text)?;

    let mut args: Vec<String> = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--append-system-prompt-file".into(),
        p(&persona_path),
        // Rebuild the system prompt each run so new memories, routines and the time are current.
        "--system-prompt-snapshot".into(),
        "off".into(),
    ];
    if resume && !bot.session_id.is_empty() {
        args.push("--resume".into());
        args.push(bot.session_id.clone());
    }
    let model = if !bot.model.is_empty() { bot.model.clone() } else { cfg.default_model.clone() };
    if !model.is_empty() {
        args.push("--model".into());
        args.push(model);
    }

    let mode = match cfg.permission_mode.as_str() {
        "auto" => "auto",
        "accept_edits" => "acceptEdits",
        "allow_all" => "bypassPermissions",
        "never" => "dontAsk",
        _ => "default",
    };
    args.push("--permission-mode".into());
    args.push(mode.into());

    // openbot's own MCP server: approvals + extra tools (ask_user, send_file, routines...).
    let mcp_path = bot_dir.join("mcp.json");
    let mut servers = serde_json::Map::new();
    // The user's own MCP servers (Settings › Permissions/Computer › MCP servers).
    for srv in &cfg.mcp_servers {
        let Some(name) = srv.get("name").and_then(Value::as_str) else { continue };
        if name.is_empty() || name == "openbot" {
            continue;
        }
        let entry = if srv.get("type").and_then(Value::as_str) == Some("http") {
            json!({"type": "http", "url": srv.get("url").cloned().unwrap_or(json!("")),
                   "headers": srv.get("headers").cloned().unwrap_or(json!({}))})
        } else {
            json!({"type": "stdio", "command": srv.get("command").cloned().unwrap_or(json!("")),
                   "args": srv.get("args").cloned().unwrap_or(json!([])),
                   "env": srv.get("env").cloned().unwrap_or(json!({}))})
        };
        servers.insert(name.to_string(), entry);
    }
    // In WSL the helper is still this Windows program (started through WSL interop, so it reaches
    // the server on Windows' 127.0.0.1). WSLENV lists the variables WSL may pass to it.
    let mut helper_env = json!({
        "OPENBOT_URL": format!("http://127.0.0.1:{}", state.port),
        "OPENBOT_TOKEN": cfg.token,
        "OPENBOT_BOT_ID": bot.id,
    });
    if wsl {
        helper_env["WSLENV"] = json!("OPENBOT_URL:OPENBOT_TOKEN:OPENBOT_BOT_ID");
    }
    servers.insert("openbot".into(), json!({
        "type": "stdio",
        "command": p(&state.self_exe),
        "args": ["mcp-approve"],
        "env": helper_env,
    }));
    let mcp = json!({ "mcpServers": servers });
    std::fs::write(&mcp_path, serde_json::to_vec_pretty(&mcp)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&mcp_path, std::fs::Permissions::from_mode(0o600));
    }
    args.push("--mcp-config".into());
    args.push(p(&mcp_path));
    if wsl {
        // Claude Code's sandbox (bubblewrap) works in WSL2: commands can only write inside the bot's folder.
        let settings_path = bot_dir.join("sandbox-settings.json");
        let settings = json!({ "sandbox": { "enabled": true, "autoAllowBashIfSandboxed": true } });
        std::fs::write(&settings_path, serde_json::to_vec_pretty(&settings)?)?;
        args.push("--settings".into());
        args.push(p(&settings_path));
    }
    if mode != "bypassPermissions" {
        // Route "may I?" questions to the chat as Allow / Deny cards.
        args.push("--permission-prompt-tool".into());
        args.push("mcp__openbot__approve".into());
    }
    // The bot asks questions in the chat instead of through a terminal-only picker.
    args.push("--disallowedTools".into());
    args.push("AskUserQuestion".into());
    // openbot's own tools never need approval.
    args.push("--allowedTools".into());
    args.push("mcp__openbot".into());
    args.extend(cfg.allow_rules.iter().cloned());

    let mut cmd = if wsl {
        // wsl.exe -d <distro> --cd <workdir> --exec sh -c 'exec "$HOME/.local/bin/claude" "$@"' claude <args>
        let mut c = Command::new("wsl.exe");
        c.args(["-d", &cfg.wsl_distro, "--cd", &bot.workdir, "--exec", "sh", "-c", r#"exec "$HOME/.local/bin/claude" "$@""#, "claude"]);
        c.args(&args);
        c
    } else {
        let mut c = Command::new(claude);
        c.args(&args);
        c
    };
    cmd.current_dir(&bot.workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("OPENBOT", "1");

    // A locked key (still encrypted, see vault.rs) is useless, so it counts as not set.
    let api_key_ok = cfg.auth_mode == "api_key" && !cfg.api_key.is_empty() && !crate::vault::is_locked(&cfg.api_key);
    if api_key_ok {
        cmd.env("ANTHROPIC_API_KEY", &cfg.api_key);
    } else {
        // Use the Claude subscription login of this computer.
        cmd.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN");
    }
    let mut passed: Vec<String> = vec!["OPENBOT".into()];
    if api_key_ok {
        passed.push("ANTHROPIC_API_KEY".into());
    }
    // Only the secrets meant for this Bot (and never ones with a reserved name).
    for s in crate::vault::for_bot(cfg, &bot.id) {
        cmd.env(s.name.trim(), &s.value);
        passed.push(s.name.trim().to_string());
    }
    if wsl {
        // Windows -> WSL only passes the environment variables listed in WSLENV.
        let mut list = std::env::var("WSLENV").unwrap_or_default();
        for n in passed {
            if !list.is_empty() {
                list.push(':');
            }
            list.push_str(&n);
        }
        cmd.env("WSLENV", list);
    }
    hide_console(&mut cmd);
    Ok(cmd)
}

fn interrupt(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]);
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
        let _ = c.status();
    }
    #[cfg(not(windows))]
    {
        // SIGINT lets Claude Code end the turn cleanly.
        let _ = std::process::Command::new("kill").args(["-INT", &pid.to_string()]).status();
    }
}

async fn drive(
    state: &Shared,
    bot_id: &str,
    prompt: &str,
    stop: Arc<Notify>,
    stopped: Arc<AtomicBool>,
    resume: bool,
) -> anyhow::Result<Outcome> {
    let bot = state.db.get_bot(bot_id)?.ok_or_else(|| anyhow!("This bot was deleted."))?;
    let cfg = state.config.read().await.clone();
    let in_wsl = cfg!(windows) && cfg.runner == "wsl";
    let claude = if in_wsl { Some(PathBuf::from("wsl.exe")) } else { find_claude(&cfg.claude_path) }.ok_or_else(|| {
        anyhow!(
            "Claude Code isn't installed on this computer. Install it (https://claude.com/claude-code), \
             then open a terminal, run `claude` once and log in."
        )
    })?;

    let mut cmd = build_command(state, &bot, &cfg, &claude, resume)?;
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("Couldn't start Claude Code ({}): {e}", claude.display()))?;
    let pid = child.id();

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await.ok();
        drop(stdin);
    }

    let stderr_buf = Arc::new(tokio::sync::Mutex::new(String::new()));
    if let Some(mut err) = child.stderr.take() {
        let buf = stderr_buf.clone();
        tokio::spawn(async move {
            let mut chunk = vec![0u8; 4096];
            while let Ok(n) = err.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                let mut b = buf.lock().await;
                b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                if b.len() > 8000 {
                    let cut = b.len() - 4000;
                    let cut = (cut..b.len()).find(|i| b.is_char_boundary(*i)).unwrap_or(b.len());
                    *b = b[cut..].to_string();
                }
            }
        });
    }

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no output from Claude Code"))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut turn = Turn::new(state.clone(), bot_id.to_string());
    let mut deadline: Option<tokio::time::Instant> = None;

    loop {
        let grace = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(l)) => turn.handle(&l).await,
                _ => break,
            },
            _ = stop.notified(), if deadline.is_none() => {
                stopped.store(true, Ordering::SeqCst);
                interrupt(pid);
                deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
            },
            _ = grace => {
                let _ = child.start_kill();
                break;
            }
        }
    }

    let status = match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Ok(Ok(s)) => Some(s),
        _ => {
            let _ = child.start_kill();
            None
        }
    };
    let stderr = stderr_buf.lock().await.clone();

    if turn.resume_failed || (!turn.got_result && resume && stderr.contains("No conversation found")) {
        turn.cleanup(true);
        return Ok(Outcome::ResumeFailed);
    }
    turn.finish(stopped.load(Ordering::SeqCst), status.map(|s| s.success()).unwrap_or(false), &stderr)
        .await;
    Ok(Outcome::Done)
}

// ---------------------------------------------------------------- output parsing

struct Turn {
    state: Shared,
    bot_id: String,
    cur_api_msg: String,
    /// content-block index -> message id (text blocks of the current model message)
    blocks: HashMap<u64, String>,
    /// messages currently streaming in
    live: HashMap<String, (Message, bool /*saved*/, Instant /*last emit*/)>,
    streamed_api_msgs: HashSet<String>,
    /// tool_use_id -> tool message
    tools: HashMap<String, Message>,
    got_result: bool,
    resume_failed: bool,
}

pub fn tool_summary(name: &str, input: &Value, past: bool) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let short = |p: String| {
        let parts: Vec<&str> = p.split(['/', '\\']).filter(|x| !x.is_empty()).collect();
        if parts.len() <= 2 { p.clone() } else { parts[parts.len() - 2..].join("/") }
    };
    let verb = |past_v: &str, now_v: &str| if past { past_v.to_string() } else { now_v.to_string() };
    match name {
        "Bash" | "PowerShell" => {
            let c = truncate_chars(&s("command").replace('\n', " "), 90);
            format!("{} `{c}`", verb("Ran", "Run"))
        }
        "Read" => format!("{} {}", verb("Read", "Read"), short(s("file_path"))),
        "Write" => format!("{} {}", verb("Wrote", "Write"), short(s("file_path"))),
        "Edit" | "MultiEdit" => format!("{} {}", verb("Edited", "Edit"), short(s("file_path"))),
        "NotebookEdit" => format!("{} {}", verb("Edited", "Edit"), short(s("notebook_path"))),
        "Glob" => format!("{} files matching {}", verb("Looked for", "Look for"), s("pattern")),
        "Grep" => format!("{} “{}”", verb("Searched for", "Search for"), truncate_chars(&s("pattern"), 60)),
        "WebFetch" => format!("{} {}", verb("Opened", "Open"), truncate_chars(&s("url"), 80)),
        "WebSearch" => format!("{} the web for “{}”", verb("Searched", "Search"), truncate_chars(&s("query"), 60)),
        "TodoWrite" => verb("Updated its to-do list", "Update its to-do list"),
        "Task" | "Agent" => format!("{} a helper: {}", verb("Started", "Start"), truncate_chars(&s("description"), 60)),
        "Skill" => format!("{} the {} skill", verb("Used", "Use"), s("skill")),
        n if n.starts_with("mcp__") => {
            let mut p = n.splitn(3, "__").skip(1);
            let server = p.next().unwrap_or("");
            let tool = p.next().unwrap_or("").replace('_', " ");
            format!("{} {server} › {tool}", verb("Used", "Use"))
        }
        n => format!("{} {n}", verb("Used", "Use")),
    }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|it| match it.get("type").and_then(Value::as_str) {
                Some("text") => it.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                Some("image") => "[image]".to_string(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

impl Turn {
    fn new(state: Shared, bot_id: String) -> Self {
        Self {
            state,
            bot_id,
            cur_api_msg: String::new(),
            blocks: HashMap::new(),
            live: HashMap::new(),
            streamed_api_msgs: HashSet::new(),
            tools: HashMap::new(),
            got_result: false,
            resume_failed: false,
        }
    }

    fn new_msg(&self, role: &str, text: &str, status: &str, meta: Value) -> Message {
        Message {
            id: new_id(),
            bot_id: self.bot_id.clone(),
            role: role.into(),
            text: text.into(),
            html: if role == "bot" { markdown::render(text) } else { String::new() },
            status: status.into(),
            meta,
            created_at: now_ms(),
        }
    }

    async fn handle(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        // Skip the inner chatter of helper sub-agents; the chat shows the main conversation.
        if v.get("parent_tool_use_id").map_or(false, |p| !p.is_null()) {
            return;
        }
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "system" => {
                if v.get("subtype").and_then(Value::as_str) == Some("init") {
                    self.set_session(&v);
                }
            }
            "stream_event" => self.stream_event(&v["event"]).await,
            "assistant" => self.assistant(&v["message"]),
            "user" => {
                self.tool_results(&v["message"]);
                set_activity(&self.state, &self.bot_id, Some(("thinking", "Thinking"))).await;
            }
            "rate_limit_event" => self.rate_limit(&v["rate_limit_info"]),
            "result" => self.result(&v).await,
            _ => {}
        }
    }

    fn set_session(&self, v: &Value) {
        let Some(sid) = v.get("session_id").and_then(Value::as_str) else { return };
        if let Ok(Some(mut b)) = self.state.db.get_bot(&self.bot_id) {
            if b.session_id != sid {
                b.session_id = sid.to_string();
                let _ = self.state.db.upsert_bot(&b);
            }
        }
    }

    async fn stream_event(&mut self, ev: &Value) {
        let idx = ev.get("index").and_then(Value::as_u64).unwrap_or(0);
        match ev.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                self.cur_api_msg = ev.pointer("/message/id").and_then(Value::as_str).unwrap_or("").to_string();
                self.blocks.clear();
            }
            "content_block_start" => {
                let bt = ev.pointer("/content_block/type").and_then(Value::as_str).unwrap_or("");
                if bt == "thinking" {
                    set_activity(&self.state, &self.bot_id, Some(("thinking", "Thinking"))).await;
                } else if bt == "tool_use" {
                    let name = ev.pointer("/content_block/name").and_then(Value::as_str).unwrap_or("");
                    let (kind, label) = activity_for(name);
                    set_activity(&self.state, &self.bot_id, Some((kind, &label))).await;
                }
                if bt == "text" {
                    let m = self.new_msg("bot", "", "streaming", json!({}));
                    self.blocks.insert(idx, m.id.clone());
                    self.live.insert(m.id.clone(), (m, false, Instant::now()));
                    self.streamed_api_msgs.insert(self.cur_api_msg.clone());
                }
            }
            "content_block_delta" => {
                if ev.pointer("/delta/type").and_then(Value::as_str) != Some("text_delta") {
                    return;
                }
                let delta = ev.pointer("/delta/text").and_then(Value::as_str).unwrap_or("");
                let Some(id) = self.blocks.get(&idx).cloned() else { return };
                let Some((m, saved, last)) = self.live.get_mut(&id) else { return };
                m.text.push_str(delta);
                if !*saved {
                    if m.text.trim().is_empty() {
                        return;
                    }
                    m.html = markdown::render(&m.text);
                    let _ = self.state.db.upsert_message(m);
                    self.state.emit_message(true, m);
                    *saved = true;
                    *last = Instant::now();
                } else if last.elapsed() >= Duration::from_millis(50) {
                    m.html = markdown::render(&m.text);
                    self.state.emit_message(false, m);
                    *last = Instant::now();
                }
            }
            "content_block_stop" => {
                if let Some(id) = self.blocks.remove(&idx) {
                    self.finalize(&id).await;
                }
            }
            _ => {}
        }
    }

    async fn finalize(&mut self, id: &str) {
        let Some((mut m, saved, _)) = self.live.remove(id) else { return };
        if m.text.trim().is_empty() {
            if saved {
                let _ = self.state.db.delete_message(&m.id);
            }
            return;
        }
        m.status = "done".into();
        m.text = m.text.trim().to_string();
        m.html = markdown::render(&m.text);
        let _ = self.state.db.upsert_message(&m);
        self.state.emit_message(!saved, &m);
        self.bump_bot(&m.text, 1).await;
    }

    /// Update the sidebar preview / unread count.
    async fn bump_bot(&self, text: &str, unread: i64) {
        if let Ok(Some(mut b)) = self.state.db.get_bot(&self.bot_id) {
            b.last_preview = preview(text);
            b.updated_at = now_ms();
            b.unread += unread;
            let _ = self.state.db.upsert_bot(&b);
            self.state.emit_bot(&self.bot_id).await;
        }
    }

    fn assistant(&mut self, msg: &Value) {
        let api_id = msg.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let Some(blocks) = msg.get("content").and_then(Value::as_array) else { return };
        for b in blocks {
            match b.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => {
                    // Only used when live streaming didn't deliver this text.
                    if self.streamed_api_msgs.contains(&api_id) {
                        continue;
                    }
                    let text = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
                    if text.is_empty() {
                        continue;
                    }
                    let m = self.new_msg("bot", text, "done", json!({}));
                    let _ = self.state.db.upsert_message(&m);
                    self.state.emit_message(true, &m);
                    let st = self.state.clone();
                    let (bid, t) = (self.bot_id.clone(), m.text.clone());
                    tokio::spawn(async move {
                        if let Ok(Some(mut bot)) = st.db.get_bot(&bid) {
                            bot.last_preview = preview(&t);
                            bot.updated_at = now_ms();
                            bot.unread += 1;
                            let _ = st.db.upsert_bot(&bot);
                            st.emit_bot(&bid).await;
                        }
                    });
                }
                "tool_use" => {
                    let id = b.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    if id.is_empty() || self.tools.contains_key(&id) || name == "mcp__openbot__approve" {
                        continue;
                    }
                    let input = b.get("input").cloned().unwrap_or(json!({}));
                    let summary = tool_summary(&name, &input, true);
                    let m = self.new_msg(
                        "tool",
                        &summary,
                        "running",
                        json!({ "tool_use_id": id, "name": name, "summary": summary, "output": "" }),
                    );
                    let _ = self.state.db.upsert_message(&m);
                    self.state.emit_message(true, &m);
                    self.tools.insert(id, m);
                }
                _ => {}
            }
        }
    }

    fn tool_results(&mut self, msg: &Value) {
        let Some(blocks) = msg.get("content").and_then(Value::as_array) else { return };
        for b in blocks {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
            let Some(m) = self.tools.get_mut(id) else { continue };
            let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            m.status = if is_error { "failed" } else { "ok" }.into();
            let out = result_text(b.get("content").unwrap_or(&Value::Null));
            m.meta["output"] = json!(truncate_chars(&out, 4000));
            let _ = self.state.db.upsert_message(m);
            self.state.emit_message(false, m);
        }
    }

    fn rate_limit(&self, info: &Value) {
        if info.is_null() {
            return;
        }
        let rl = json!({
            "utilization": info.get("utilization").and_then(Value::as_f64).unwrap_or(0.0),
            "resets_at": info.get("resetsAt").and_then(Value::as_i64).unwrap_or(0),
            "window": info.get("rateLimitType").and_then(Value::as_str).unwrap_or(""),
            "status": info.get("status").and_then(Value::as_str).unwrap_or(""),
        });
        self.state.db.kv_set("rate_limit", &rl.to_string());
        let usage = crate::api::usage_json(&self.state);
        self.state.emit("usage.updated", json!({ "usage": usage }));
    }

    async fn result(&mut self, v: &Value) {
        self.got_result = true;
        self.set_session(v);

        if let (Some(sid), Some(total)) = (
            v.get("session_id").and_then(Value::as_str),
            v.get("total_cost_usd").and_then(Value::as_f64),
        ) {
            // Claude Code reports the whole conversation's total; store only the new part.
            let key = format!("cost:{sid}");
            let prev: f64 = self.state.db.kv_get(&key).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let delta = if total >= prev { total - prev } else { total };
            self.state.db.kv_set(&key, &total.to_string());
            let _ = self.state.db.add_usage(&self.bot_id, delta, now_ms());
            let usage = crate::api::usage_json(&self.state);
            self.state.emit("usage.updated", json!({ "usage": usage }));
        }

        let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
            || v.get("subtype").and_then(Value::as_str).map_or(false, |s| s != "success");
        if is_error {
            let mut text = v
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("errors")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"))
                })
                .unwrap_or_default();
            if text.contains("No conversation found") {
                self.resume_failed = true;
                return;
            }
            if text.trim().is_empty() {
                text = format!(
                    "The run ended early ({}).",
                    v.get("subtype").and_then(Value::as_str).unwrap_or("error")
                );
            }
            let lower = text.to_lowercase();
            if lower.contains("/login") || lower.contains("not logged in") || lower.contains("invalid api key") {
                text.push_str(
                    "\n\nTip: on the bot's computer open a terminal, run `claude` and log in, \
                     or add an API key in Settings › Claude Account.",
                );
            }
            add_simple(&self.state, &self.bot_id, "error", &text);
        }
    }

    /// Close anything still open without adding messages.
    fn cleanup(&mut self, delete: bool) {
        for (_, (m, saved, _)) in self.live.drain() {
            if saved && delete {
                let _ = self.state.db.delete_message(&m.id);
            }
        }
    }

    async fn finish(&mut self, stopped: bool, success: bool, stderr: &str) {
        let ids: Vec<String> = self.live.keys().cloned().collect();
        for id in ids {
            self.finalize(&id).await;
        }
        for (_, m) in self.tools.iter_mut() {
            if m.status == "running" {
                m.status = "failed".into();
                let _ = self.state.db.upsert_message(m);
                self.state.emit_message(false, m);
            }
        }
        if stopped {
            add_simple(&self.state, &self.bot_id, "system", "Stopped");
        } else if !self.got_result && !success {
            let tail = truncate_chars(stderr.trim(), 1500);
            let msg = if tail.is_empty() {
                "Claude Code stopped unexpectedly.".to_string()
            } else {
                format!("Claude Code stopped unexpectedly:\n\n{tail}")
            };
            add_simple(&self.state, &self.bot_id, "error", &msg);
        }
    }
}
