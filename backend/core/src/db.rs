//! SQLite storage. One small file: `<data dir>/openbot.db`.

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
}

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
        // Columns added after the first release.
        for (col, def) in [
            ("label", "TEXT NOT NULL DEFAULT ''"),
            ("pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("notifications", "INTEGER NOT NULL DEFAULT 1"),
        ] {
            let exists: bool = conn
                .prepare("SELECT 1 FROM pragma_table_info('bots') WHERE name = ?1")?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE bots ADD COLUMN {col} {def}"))?;
            }
        }
        Ok(Db { conn: Mutex::new(conn) })
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- bots ----------

    pub fn list_bots(&self) -> anyhow::Result<Vec<Bot>> {
        let c = self.c();
        let mut st = c.prepare("SELECT * FROM bots ORDER BY updated_at DESC")?;
        let v = st.query_map([], row_to_bot)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn get_bot(&self, id: &str) -> anyhow::Result<Option<Bot>> {
        let c = self.c();
        Ok(c.query_row("SELECT * FROM bots WHERE id = ?1", [id], row_to_bot).optional()?)
    }

    pub fn upsert_bot(&self, b: &Bot) -> anyhow::Result<()> {
        self.c().execute(
            r#"INSERT INTO bots (id,name,color,shape,instructions,model,hidden,created_at,updated_at,last_preview,unread,workdir,session_id,label,pinned,notifications)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
               ON CONFLICT(id) DO UPDATE SET name=?2,color=?3,shape=?4,instructions=?5,model=?6,hidden=?7,
               updated_at=?9,last_preview=?10,unread=?11,workdir=?12,session_id=?13,label=?14,pinned=?15,notifications=?16"#,
            params![
                b.id, b.name, b.color, b.shape, b.instructions, b.model, b.hidden as i64,
                b.created_at, b.updated_at, b.last_preview, b.unread, b.workdir, b.session_id,
                b.label, b.pinned as i64, b.notifications as i64
            ],
        )?;
        Ok(())
    }

    pub fn delete_bot(&self, id: &str) -> anyhow::Result<()> {
        let c = self.c();
        c.execute("DELETE FROM messages WHERE bot_id = ?1", [id])?;
        c.execute("DELETE FROM memories WHERE bot_id = ?1", [id])?;
        c.execute("DELETE FROM bots WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---------- messages ----------

    pub fn upsert_message(&self, m: &Message) -> anyhow::Result<()> {
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
        // Approvals still open when openbot stopped can't be answered any more.
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
            "INSERT INTO memories (id, bot_id, text, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![m.id, m.bot_id, m.text, m.created_at],
        )?;
        Ok(())
    }

    /// Memories this bot can see: its own plus the shared ones about the user.
    pub fn memories_for(&self, bot_id: &str) -> anyhow::Result<Vec<Memory>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, bot_id, text, created_at FROM memories WHERE bot_id IS NULL OR bot_id = ?1 ORDER BY created_at",
        )?;
        let v = st
            .query_map([bot_id], |r| {
                Ok(Memory { id: r.get(0)?, bot_id: r.get(1)?, text: r.get(2)?, created_at: r.get(3)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
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
