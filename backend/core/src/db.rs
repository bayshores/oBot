//! SQLite storage. One small file: `<data dir>/oBot.db`.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub color: String,
    pub shape: String,
    pub instructions: String,
    pub model: String,
    pub hidden: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_preview: String,
    pub unread: i64,
    #[serde(default)]
    pub running: bool,
    /// What the bot is doing right now, e.g. {"kind":"searching_web","label":"Searching the web"}. Not stored.
    #[serde(default)]
    pub activity: Value,
    pub workdir: String,
    #[serde(skip_serializing, default)]
    pub session_id: String,
    /// Short tagline under the name.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub pinned: bool,
    /// Send desktop notifications for this bot.
    #[serde(default = "yes")]
    pub notifications: bool,
    /// The Main Bot (can't be deleted). Filled in when sent to the UI.
    #[serde(default)]
    pub main: bool,
    /// Claude Code session id, shown as "conversation ID".
    #[serde(default)]
    pub conversation_id: String,
    /// "" for a Bot, "group" for a group chat (see group.rs).
    #[serde(default)]
    pub kind: String,
    /// A group chat's Bots, in order.
    #[serde(default)]
    pub members: Vec<String>,
}

impl Bot {
    pub fn is_group(&self) -> bool {
        self.kind == "group"
    }
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message {
    pub id: String,
    pub bot_id: String,
    pub role: String,
    pub text: String,
    pub html: String,
    pub status: String,
    pub meta: Value,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Memory {
    pub id: String,
    pub bot_id: Option<String>,
    pub text: String,
    pub created_at: i64,
    /// "profile" (standing instructions and core facts: always in the Bot's prompt) or "log"
    /// (everything else: found with `recall`).
    pub tier: String,
}

pub const PROFILE: &str = "profile";
pub const LOG: &str = "log";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Routine {
    pub id: String,
    pub bot_id: String,
    pub name: String,
    pub prompt: String,
    pub schedule: Value,
    pub enabled: bool,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub unread_runs: i64,
    pub created_at: i64,
}

fn row_to_routine(r: &Row) -> rusqlite::Result<Routine> {
    let sched: String = r.get("schedule")?;
    Ok(Routine {
        id: r.get("id")?,
        bot_id: r.get("bot_id")?,
        name: r.get("name")?,
        prompt: r.get("prompt")?,
        schedule: serde_json::from_str(&sched).unwrap_or(Value::Null),
        enabled: r.get::<_, i64>("enabled")? != 0,
        next_run_at: r.get("next_run_at")?,
        last_run_at: r.get("last_run_at")?,
        unread_runs: r.get("unread_runs")?,
        created_at: r.get("created_at")?,
    })
}

pub struct Db {
    conn: Mutex<Connection>,
}

fn row_to_bot(r: &Row) -> rusqlite::Result<Bot> {
    Ok(Bot {
        id: r.get("id")?,
        name: r.get("name")?,
        color: r.get("color")?,
        shape: r.get("shape")?,
        instructions: r.get("instructions")?,
        model: r.get("model")?,
        hidden: r.get::<_, i64>("hidden")? != 0,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        last_preview: r.get("last_preview")?,
        unread: r.get("unread")?,
        running: false,
        activity: Value::Null,
        workdir: r.get("workdir")?,
        session_id: r.get("session_id")?,
        label: r.get("label")?,
        pinned: r.get::<_, i64>("pinned")? != 0,
        notifications: r.get::<_, i64>("notifications")? != 0,
        main: false,
        conversation_id: String::new(),
        kind: r.get("kind")?,
        members: serde_json::from_str(&r.get::<_, String>("members")?).unwrap_or_default(),
    })
}

fn row_to_msg(r: &Row) -> rusqlite::Result<Message> {
    let meta: String = r.get("meta")?;
    Ok(Message {
        id: r.get("id")?,
        bot_id: r.get("bot_id")?,
        role: r.get("role")?,
        text: r.get("text")?,
        html: r.get("html")?,
        status: r.get("status")?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
        created_at: r.get("created_at")?,
    })
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA secure_delete = ON;  -- overwritten text (e.g. a hidden secret) doesn't linger in free space
            CREATE TABLE IF NOT EXISTS bots (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT NOT NULL,
                shape TEXT NOT NULL,
                instructions TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                hidden INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_preview TEXT NOT NULL DEFAULT '',
                unread INTEGER NOT NULL DEFAULT 0,
                workdir TEXT NOT NULL,
                session_id TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                bot_id TEXT NOT NULL,
                role TEXT NOT NULL,
                text TEXT NOT NULL DEFAULT '',
                html TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'done',
                meta TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                seq INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_messages_bot ON messages(bot_id, created_at, seq);
            CREATE TABLE IF NOT EXISTS usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT NOT NULL,
                cost_usd REAL NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                bot_id TEXT,              -- NULL = about the user, shared by every bot
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                bot_id TEXT NOT NULL,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                note TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL,     -- running | done | failed | stopped | timed_out | interrupted
                exit_code INTEGER,
                log_path TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                pid INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_bot ON jobs(bot_id, started_at);
            CREATE TABLE IF NOT EXISTS routines (
                id TEXT PRIMARY KEY,
                bot_id TEXT NOT NULL,
                name TEXT NOT NULL,
                prompt TEXT NOT NULL,
                schedule TEXT NOT NULL,   -- JSON, see routines.rs
                enabled INTEGER NOT NULL DEFAULT 1,
                next_run_at INTEGER,
                last_run_at INTEGER,
                unread_runs INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            "#,
        )?;
        // Columns added after the first release. Memories saved before tiers existed were all in
        // the prompt, so they start as "profile".
        for (table, col, def) in [
            ("bots", "label", "TEXT NOT NULL DEFAULT ''"),
            ("bots", "pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("bots", "notifications", "INTEGER NOT NULL DEFAULT 1"),
            ("bots", "kind", "TEXT NOT NULL DEFAULT ''"),
            ("bots", "members", "TEXT NOT NULL DEFAULT '[]'"),
            ("memories", "tier", "TEXT NOT NULL DEFAULT 'profile'"),
            ("jobs", "pid", "INTEGER"),
        ] {
            let exists: bool = conn
                .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
            }
        }
        rerender_messages(&conn)?;
        Ok(Db { conn: Mutex::new(conn) })
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Saved Bot messages keep the HTML they were shown with. When the markdown renderer changes
/// (markdown::VERSION), render them again once so old messages look like new ones.
fn rerender_messages(conn: &Connection) -> anyhow::Result<()> {
    let done: Option<String> = conn.query_row("SELECT v FROM kv WHERE k = 'markdown_version'", [], |r| r.get(0)).optional()?;
    if done.as_deref() == Some(crate::markdown::VERSION) {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut read = tx.prepare("SELECT id, text FROM messages WHERE html != ''")?;
        let rows = read.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        let mut write = tx.prepare("UPDATE messages SET html = ?2 WHERE id = ?1")?;
        for (id, text) in rows {
            write.execute(params![id, crate::markdown::render(&text)])?;
        }
    }
    tx.execute("INSERT OR REPLACE INTO kv (k, v) VALUES ('markdown_version', ?1)", [crate::markdown::VERSION])?;
    tx.commit()?;
    Ok(())
}

impl Db {
    // ---------- bots ----------

    /// The user's Bots (not group chats).
    pub fn list_bots(&self) -> anyhow::Result<Vec<Bot>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM bots WHERE kind = '' ORDER BY updated_at DESC")?;
        let v = st.query_map([], row_to_bot)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn list_groups(&self) -> anyhow::Result<Vec<Bot>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM bots WHERE kind = 'group' ORDER BY updated_at DESC")?;
        let v = st.query_map([], row_to_bot)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn get_bot(&self, id: &str) -> anyhow::Result<Option<Bot>> {
        let c = self.c();
        Ok(c.query_row("SELECT * FROM bots WHERE id = ?1", [id], row_to_bot).optional()?)
    }

    pub fn upsert_bot(&self, b: &Bot) -> anyhow::Result<()> {
        self.c().execute(
            r#"INSERT INTO bots (id,name,color,shape,instructions,model,hidden,created_at,updated_at,last_preview,unread,workdir,session_id,label,pinned,notifications,kind,members)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
               ON CONFLICT(id) DO UPDATE SET name=?2,color=?3,shape=?4,instructions=?5,model=?6,hidden=?7,
               updated_at=?9,last_preview=?10,unread=?11,workdir=?12,session_id=?13,label=?14,pinned=?15,notifications=?16,
               kind=?17,members=?18"#,
            params![
                b.id, b.name, b.color, b.shape, b.instructions, b.model, b.hidden as i64,
                b.created_at, b.updated_at, crate::vault::redact(&b.last_preview), b.unread, b.workdir, b.session_id,
                b.label, b.pinned as i64, b.notifications as i64, b.kind, serde_json::to_string(&b.members).unwrap_or_else(|_| "[]".into())
            ],
        )?;
        Ok(())
    }

    pub fn delete_bot(&self, id: &str) -> anyhow::Result<()> {
        let c = self.c();
        c.execute("DELETE FROM messages WHERE bot_id = ?1", [id])?;
        c.execute("DELETE FROM memories WHERE bot_id = ?1", [id])?;
        c.execute("DELETE FROM jobs WHERE bot_id = ?1", [id])?;
        c.execute("DELETE FROM bots WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Hide saved secret values in chats, memories and sidebar previews saved earlier (for example before the
    /// secret was added to the Vault). Returns how many rows changed.
    pub fn redact_history(&self) -> anyhow::Result<usize> {
        let c = self.c();
        let mut changed = 0;
        // Read row by row and keep only the changed ones, so a long history isn't all in memory.
        let msgs = {
            let mut st = c.prepare("SELECT * FROM messages")?;
            let mut out = Vec::new();
            for m in st.query_map([], row_to_msg)? {
                let mut m = m?;
                if crate::vault::redact_message(&mut m) {
                    out.push(m);
                }
            }
            out
        };
        for m in msgs {
            c.execute(
                "UPDATE messages SET text=?2, html=?3, meta=?4 WHERE id=?1",
                params![m.id, m.text, m.html, m.meta.to_string()],
            )?;
            changed += 1;
        }
        let mems = {
            let mut st = c.prepare("SELECT id, text FROM memories")?;
            let v = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            v
        };
        for (id, text) in mems {
            if let Some(t) = crate::vault::redact_str(&text) {
                c.execute("UPDATE memories SET text=?2 WHERE id=?1", params![id, t])?;
                changed += 1;
            }
        }
        let previews = {
            let mut st = c.prepare("SELECT id, last_preview FROM bots")?;
            let v = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            v
        };
        for (id, text) in previews {
            if let Some(t) = crate::vault::redact_str(&text) {
                c.execute("UPDATE bots SET last_preview=?2 WHERE id=?1", params![id, t])?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    // ---------- messages ----------

    /// Save a chat message. Saved secret values are replaced with •••• first (see vault.rs).
    pub fn upsert_message(&self, m: &Message) -> anyhow::Result<()> {
        let mut m = m.clone();
        crate::vault::redact_message(&mut m);
        let c = self.c();
        c.execute(
            r#"INSERT INTO messages (id,bot_id,role,text,html,status,meta,created_at,seq)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,(SELECT COALESCE(MAX(seq),0)+1 FROM messages))
               ON CONFLICT(id) DO UPDATE SET text=?4, html=?5, status=?6, meta=?7"#,
            params![m.id, m.bot_id, m.role, m.text, m.html, m.status, m.meta.to_string(), m.created_at],
        )?;
        Ok(())
    }

    pub fn get_message(&self, id: &str) -> anyhow::Result<Option<Message>> {
        let c = self.c();
        Ok(c.query_row("SELECT * FROM messages WHERE id = ?1", [id], row_to_msg).optional()?)
    }

    pub fn delete_message(&self, id: &str) -> anyhow::Result<()> {
        self.c().execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }

    /// User messages still marked "Queued" (see runner::recover_queued).
    pub fn queued_messages(&self) -> anyhow::Result<Vec<Message>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM messages WHERE role = 'user' AND meta LIKE '%\"queued\":true%'")?;
        let v = st.query_map([], row_to_msg)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// The bot's last user message and every message after it (oldest first), for "Edit and resend".
    pub fn last_exchange(&self, bot_id: &str) -> anyhow::Result<Vec<Message>> {
        let c = self.c();
        let Some((at, seq)) = c
            .query_row(
                "SELECT created_at, seq FROM messages WHERE bot_id = ?1 AND role = 'user' ORDER BY created_at DESC, seq DESC LIMIT 1",
                [bot_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        else {
            return Ok(vec![]);
        };
        let mut st = c.prepare(
            "SELECT * FROM messages WHERE bot_id = ?1 AND (created_at > ?2 OR (created_at = ?2 AND seq >= ?3)) ORDER BY created_at, seq",
        )?;
        let v = st.query_map(params![bot_id, at, seq], row_to_msg)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Messages oldest -> newest. `before` = only messages created before this time (for paging).
    pub fn list_messages(&self, bot_id: &str, before: Option<i64>, limit: i64) -> anyhow::Result<Vec<Message>> {
        let c = self.c();
        let before = before.unwrap_or(i64::MAX);
        let mut st = c.prepare(
            "SELECT * FROM messages WHERE bot_id = ?1 AND created_at < ?2 ORDER BY created_at DESC, seq DESC LIMIT ?3",
        )?;
        let mut v = st
            .query_map(params![bot_id, before, limit], row_to_msg)?
            .collect::<Result<Vec<_>, _>>()?;
        v.reverse();
        Ok(v)
    }

    pub fn clear_messages(&self, bot_id: &str) -> anyhow::Result<()> {
        self.c().execute("DELETE FROM messages WHERE bot_id = ?1", [bot_id])?;
        Ok(())
    }

    /// Mark any approval / tool rows left "pending"/"running" (e.g. after a crash) as finished.
    pub fn close_dangling(&self) -> anyhow::Result<()> {
        let c = self.c();
        // Approvals still open when oBot stopped can't be answered any more.
        c.execute("UPDATE messages SET status='expired' WHERE role='approval' AND status='pending'", [])?;
        c.execute("UPDATE messages SET status='failed' WHERE role='tool' AND status='running'", [])?;
        c.execute("UPDATE messages SET status='done' WHERE role='bot' AND status='streaming'", [])?;
        Ok(())
    }

    pub fn search_messages(&self, q: &str, limit: i64) -> anyhow::Result<Vec<Message>> {
        let c = self.c();
        let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        let mut st = c.prepare(
            "SELECT * FROM messages WHERE role IN ('user','bot') AND text LIKE ?1 ESCAPE '\\' ORDER BY created_at DESC LIMIT ?2",
        )?;
        let v = st.query_map(params![like, limit], row_to_msg)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    // ---------- usage ----------

    pub fn add_usage(&self, bot_id: &str, cost: f64, at: i64) -> anyhow::Result<()> {
        self.c().execute(
            "INSERT INTO usage (bot_id, cost_usd, created_at) VALUES (?1, ?2, ?3)",
            params![bot_id, cost, at],
        )?;
        Ok(())
    }

    /// (week cost, total cost, per bot [(bot_id, cost, turns)])
    pub fn usage_summary(&self, week_start: i64) -> anyhow::Result<(f64, f64, Vec<(String, f64, i64)>)> {
        let c = self.c();
        let week: f64 = c.query_row(
            "SELECT COALESCE(SUM(cost_usd),0) FROM usage WHERE created_at >= ?1",
            [week_start],
            |r| r.get(0),
        )?;
        let total: f64 = c.query_row("SELECT COALESCE(SUM(cost_usd),0) FROM usage", [], |r| r.get(0))?;
        let mut st = c.prepare(
            "SELECT bot_id, SUM(cost_usd), COUNT(*) FROM usage GROUP BY bot_id ORDER BY SUM(cost_usd) DESC",
        )?;
        let per = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((week, total, per))
    }

    // ---------- key/value ----------

    pub fn kv_get(&self, k: &str) -> Option<String> {
        self.c()
            .query_row("SELECT v FROM kv WHERE k = ?1", [k], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn kv_set(&self, k: &str, v: &str) {
        let _ = self.c().execute(
            "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
            params![k, v],
        );
    }

    // ---------- background jobs (see jobs.rs) ----------

    pub fn upsert_job(&self, j: &crate::jobs::JobRow) -> anyhow::Result<()> {
        self.c().execute(
            r#"INSERT INTO jobs (id,bot_id,name,command,note,status,exit_code,log_path,started_at,ended_at,pid)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
               ON CONFLICT(id) DO UPDATE SET status=?6, exit_code=?7, ended_at=?10"#,
            params![j.id, j.bot_id, j.name, crate::vault::redact(&j.command), j.note, j.status, j.exit_code, j.log_path, j.started_at, j.ended_at, j.pid],
        )?;
        Ok(())
    }

    fn jobs_where(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> anyhow::Result<Vec<crate::jobs::JobRow>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT id,bot_id,name,command,note,status,exit_code,log_path,started_at,ended_at,pid FROM jobs {sql}"
        ))?;
        let v = st
            .query_map(p, |r| {
                Ok(crate::jobs::JobRow {
                    id: r.get(0)?,
                    bot_id: r.get(1)?,
                    name: r.get(2)?,
                    command: r.get(3)?,
                    note: r.get(4)?,
                    status: r.get(5)?,
                    exit_code: r.get(6)?,
                    log_path: r.get(7)?,
                    started_at: r.get(8)?,
                    ended_at: r.get(9)?,
                    pid: r.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn get_job(&self, id: &str) -> anyhow::Result<Option<crate::jobs::JobRow>> {
        Ok(self.jobs_where("WHERE id = ?1", &[&id])?.into_iter().next())
    }

    /// A bot's newest jobs first.
    pub fn jobs_for(&self, bot_id: &str, limit: i64) -> anyhow::Result<Vec<crate::jobs::JobRow>> {
        self.jobs_where("WHERE bot_id = ?1 ORDER BY started_at DESC LIMIT ?2", &[&bot_id, &limit])
    }

    pub fn running_jobs(&self) -> anyhow::Result<Vec<crate::jobs::JobRow>> {
        self.jobs_where("WHERE status = 'running'", &[])
    }

    // ---------- memories ----------

    pub fn last_user_text(&self, bot_id: &str) -> Option<String> {
        self.c()
            .query_row(
                "SELECT text FROM messages WHERE bot_id = ?1 AND role = 'user' ORDER BY created_at DESC, seq DESC LIMIT 1",
                [bot_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn add_memory(&self, m: &Memory) -> anyhow::Result<()> {
        self.c().execute(
            "INSERT INTO memories (id, bot_id, text, created_at, tier) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![m.id, m.bot_id, crate::vault::redact(&m.text), m.created_at, if m.tier == LOG { LOG } else { PROFILE }],
        )?;
        Ok(())
    }

    /// Memories this bot can see: its own plus the shared ones about the user.
    pub fn memories_for(&self, bot_id: &str) -> anyhow::Result<Vec<Memory>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, bot_id, text, created_at, tier FROM memories WHERE bot_id IS NULL OR bot_id = ?1 ORDER BY created_at",
        )?;
        let v = st
            .query_map([bot_id], |r| {
                Ok(Memory { id: r.get(0)?, bot_id: r.get(1)?, text: r.get(2)?, created_at: r.get(3)?, tier: r.get(4)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Change a memory's text and/or tier. False if there's no memory with this id.
    pub fn update_memory(&self, id: &str, text: Option<&str>, tier: Option<&str>) -> anyhow::Result<bool> {
        let c = self.c();
        let mut n = 0;
        if let Some(t) = text {
            n += c.execute("UPDATE memories SET text = ?2 WHERE id = ?1", params![id, crate::vault::redact(t)])?;
        }
        if let Some(t) = tier {
            n += c.execute("UPDATE memories SET tier = ?2 WHERE id = ?1", params![id, if t == LOG { LOG } else { PROFILE }])?;
        }
        if text.is_none() && tier.is_none() {
            n = c.query_row("SELECT COUNT(*) FROM memories WHERE id = ?1", [id], |r| r.get::<_, i64>(0))? as usize;
        }
        Ok(n > 0)
    }

    pub fn delete_memory(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.c().execute("DELETE FROM memories WHERE id = ?1", [id])? > 0)
    }

    // ---------- routines ----------

    pub fn upsert_routine(&self, r: &Routine) -> anyhow::Result<()> {
        self.c().execute(
            r#"INSERT INTO routines (id,bot_id,name,prompt,schedule,enabled,next_run_at,last_run_at,unread_runs,created_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
               ON CONFLICT(id) DO UPDATE SET bot_id=?2,name=?3,prompt=?4,schedule=?5,enabled=?6,next_run_at=?7,last_run_at=?8,unread_runs=?9"#,
            params![
                r.id, r.bot_id, r.name, r.prompt, r.schedule.to_string(), r.enabled as i64,
                r.next_run_at, r.last_run_at, r.unread_runs, r.created_at
            ],
        )?;
        Ok(())
    }

    pub fn get_routine(&self, id: &str) -> anyhow::Result<Option<Routine>> {
        Ok(self.c().query_row("SELECT * FROM routines WHERE id = ?1", [id], row_to_routine).optional()?)
    }

    pub fn routines_for(&self, bot_id: Option<&str>) -> anyhow::Result<Vec<Routine>> {
        let c = self.c();
        let v = match bot_id {
            Some(b) => {
                let mut st = c.prepare("SELECT * FROM routines WHERE bot_id = ?1 ORDER BY created_at")?;
                let v = st.query_map([b], row_to_routine)?.collect::<Result<Vec<_>, _>>()?;
                v
            }
            None => {
                let mut st = c.prepare("SELECT * FROM routines ORDER BY created_at")?;
                let v = st.query_map([], row_to_routine)?.collect::<Result<Vec<_>, _>>()?;
                v
            }
        };
        Ok(v)
    }

    pub fn due_routines(&self, now: i64) -> anyhow::Result<Vec<Routine>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT * FROM routines WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
        )?;
        let v = st.query_map([now], row_to_routine)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn delete_routine(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.c().execute("DELETE FROM routines WHERE id = ?1", [id])? > 0)
    }

    pub fn delete_routines_of_bot(&self, bot_id: &str) -> anyhow::Result<()> {
        let c = self.c();
        c.execute("DELETE FROM routines WHERE bot_id = ?1", [bot_id])?;
        c.execute("DELETE FROM memories WHERE bot_id = ?1", [bot_id])?;
        Ok(())
    }

    /// The user opened the bot: routine results were seen.
    pub fn mark_routines_read(&self, bot_id: &str) {
        let _ = self.c().execute("UPDATE routines SET unread_runs = 0 WHERE bot_id = ?1", [bot_id]);
    }
}
