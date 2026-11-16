//! This computer as a place where Bots that run on another computer can work
//! (Settings › Computer › "Let Bots work on this Mac").
//!
//! The oBot app here is connected to another computer that runs the Bots. With the switch on, a
//! helper in the app keeps asking that computer for work (`/api/my-computer/poll`, always an outgoing
//! call, so nothing listens here) and does it: run a command, read or write a file, copy a file.
//! The other side of this is user_computers.rs.
//!
//! What a Bot may do is decided here, on this computer, from settings only this computer can change:
//! - the switch (`this_computer.enabled`). Turning it off stops every command that is running;
//! - the folders: the Bots' own folder (`this_computer.home`) and the ones the user added. Files
//!   outside them are refused, also through `..` or a link;
//! - on macOS commands run in the system sandbox: they can write only in those folders and temp
//!   folders, can't read the user's other folders, keys or other apps' data, and can't start
//!   programs outside the sandbox.

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Notify;

use crate::state::Shared;
use crate::util::{hide_console, new_id, truncate_chars};

/// The largest file a Bot may copy to or from this computer.
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 5 * 1024 * 1024;
const MAX_COMMAND_CHARS: usize = 20_000;
const DEFAULT_SECONDS: u64 = 120;
const MAX_SECONDS: u64 = 600;
const DEFAULT_JOB_MINUTES: u64 = 120;
const MAX_JOB_MINUTES: u64 = 24 * 60;
/// How much command output goes back: the start and the end.
const HEAD_BYTES: usize = 10_000;
const TAIL_BYTES: usize = 20_000;
const READ_LINES: usize = 2000;
const READ_BYTES: usize = 100_000;
const TAIL_LINES: usize = 40;

/// Settings › Computer › "Let Bots work on this Mac" (saved in this computer's config.json).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Share {
    pub enabled: bool,
    /// The Bots' own folder here: each Bot works in `<home>/<its name>` unless it names another folder.
    pub home: String,
    /// Other folders the user lets Bots use.
    pub folders: Vec<String>,
}

impl Default for Share {
    fn default() -> Self {
        Self { enabled: false, home: default_home(), folders: vec![] }
    }
}

pub fn default_home() -> String {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("oBot").to_string_lossy().into_owned()
}

/// Commands can be kept inside the folders only on macOS (`sandbox-exec`).
pub fn has_sandbox() -> bool {
    cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file()
}

/// A Bot's name as a folder name: "Life Admin" stays, "../x" becomes "x".
pub fn safe_folder_name(bot: &str) -> String {
    let s: String = bot.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    if s.is_empty() { "Bot".into() } else { truncate_chars(&s, 60).trim_end_matches('…').to_string() }
}

/// The helper's live state.
#[derive(Default)]
pub struct Helper {
    /// Woken when the connection or the settings change.
    pub changed: Notify,
    /// What the other computer was last told (see `signature`), so it's told again after a change.
    said: Mutex<String>,
    /// "off" | "not_remote" | "needs_pairing" | "connecting" | "connected" | "error: …"
    status: Mutex<String>,
}

impl Helper {
    pub fn status(&self) -> String {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn set_status(&self, s: &str) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s.to_string();
    }
}

// ---------------------------------------------------------------- folders

/// The folders Bots may use, as real paths (links resolved). `home` is the first.
#[derive(Debug)]
struct Roots {
    home: PathBuf,
    all: Vec<PathBuf>,
}

fn roots(share: &Share) -> Result<Roots, String> {
    let home = PathBuf::from(share.home.trim());
    if !home.is_absolute() {
        return Err("The Bots' folder on this computer isn't set.".into());
    }
    std::fs::create_dir_all(&home).map_err(|e| format!("Couldn't make the Bots' folder {}: {e}", home.display()))?;
    // The folders were saved as real paths (`check_folder`). One that isn't anymore was swapped for
    // a link since, and would lead somewhere the user never picked: it's left out.
    let real = |p: &Path| p.canonicalize().ok().filter(|c| c == p && c.is_dir());
    let home = real(&home).ok_or_else(|| format!("The Bots' folder {} was changed into a link. Pick it again in Settings › Computer.", home.display()))?;
    let mut all = vec![home.clone()];
    for p in share.folders.iter().filter_map(|f| real(Path::new(f.trim()))) {
        if !all.contains(&p) {
            all.push(p);
        }
    }
    Ok(Roots { home, all })
}

fn inside(roots: &[PathBuf], p: &Path) -> bool {
    roots.iter().any(|r| p.starts_with(r))
}

/// Can this folder be opened to Bots? Returns its real path. A folder that holds oBot's own
/// settings (tokens, the Vault) can't: that rules out the disk, the home folder and ~/Library.
pub fn check_folder(path: &str, data_dir: &Path) -> Result<PathBuf, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err("Give the folder's full path.".into());
    }
    let real = p.canonicalize().map_err(|_| format!("There is no folder {}.", p.display()))?;
    if !real.is_dir() {
        return Err(format!("{} is a file, not a folder.", p.display()));
    }
    let data = data_dir.canonicalize().unwrap_or_else(|_| data_dir.to_path_buf());
    if data.starts_with(&real) || real.starts_with(&data) {
        return Err(format!("{} holds oBot's own settings, so it can't be opened to Bots. Pick a folder inside it instead.", p.display()));
    }
    Ok(real)
}

/// The folder a card's "Open folder and allow" opens for `path`: a file's own folder, and a folder
/// that doesn't exist yet is made when the one above it exists, so a Bot can start something new
/// there. `check_folder` judges it after.
fn folder_to_add(path: &str, data_dir: &Path) -> Result<String, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err("Give the folder's full path.".into());
    }
    if p.is_file() {
        return Ok(p.parent().unwrap_or(&p).to_string_lossy().into_owned());
    }
    if !p.exists() {
        let missing = || format!("There is no folder {}.", p.display());
        let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else { return Err(missing()) };
        let new = parent.canonicalize().map_err(|_| missing())?.join(name);
        let data = data_dir.canonicalize().unwrap_or_else(|_| data_dir.to_path_buf());
        if new.starts_with(&data) {
            return Err(format!("{} holds oBot's own settings, so it can't be opened to Bots.", p.display()));
        }
        std::fs::create_dir(&new).map_err(|e| format!("Couldn't make {}: {e}", p.display()))?;
    }
    Ok(p.to_string_lossy().into_owned())
}

/// The real path a Bot means, if it is inside the folders. A path that doesn't start at the disk's
/// root is inside the Bot's own folder. The file doesn't have to exist yet.
fn resolve(r: &Roots, bot: &str, path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Give a path.".into());
    }
    let user_home = dirs::home_dir().unwrap_or_default();
    let start = if path == "~" {
        user_home
    } else if let Some(rest) = path.strip_prefix("~/") {
        user_home.join(rest)
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        bot_folder(r, bot)?.join(path)
    };
    // The deepest part that exists gives the real location; what follows must be plain names.
    let mut cur = start.clone();
    let mut rest: Vec<std::ffi::OsString> = vec![];
    let real = loop {
        match cur.canonicalize() {
            Ok(c) => break c,
            Err(_) => {
                if cur.symlink_metadata().is_ok() {
                    return Err(format!("{} is a link that leads nowhere.", cur.display()));
                }
                let (Some(name), Some(parent)) = (cur.file_name(), cur.parent()) else {
                    return Err(format!("{path} isn't a usable path."));
                };
                rest.push(name.to_os_string());
                cur = parent.to_path_buf();
            }
        }
    };
    let mut out = real;
    for name in rest.iter().rev() {
        if !matches!(Path::new(name).components().next(), Some(Component::Normal(_))) {
            return Err(format!("{path} isn't a usable path."));
        }
        out.push(name);
    }
    if !inside(&r.all, &out) {
        return Err(format!(
            "{} is outside the folders the user opened to Bots on this computer ({}). Nothing was done. \
             Use one of those folders, or give the folder's full path (as cwd for a command): oBot then asks the user to open it to you.",
            out.display(),
            r.all.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(out)
}

/// `<home>/<Bot name>`, made on first use.
fn bot_folder(r: &Roots, bot: &str) -> Result<PathBuf, String> {
    let dir = r.home.join(safe_folder_name(bot));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    Ok(dir)
}

// ---------------------------------------------------------------- the macOS sandbox

/// The home folder is closed to commands as a whole, hidden files too: apps keep their logins in
/// all kinds of them. Opened again for reading: these, which toolchains and git need. (Folders on
/// PATH are opened too, see `tool_dirs`.)
const TOOL_DOTS: [&str; 7] = [".cargo", ".rustup", ".pyenv", ".rbenv", ".gitconfig", ".config/git", ".zshenv"];

/// Keys and logins that stay closed even inside an opened folder (the user may have opened one that
/// holds them).
const PRIVATE_DOTS: [&str; 22] = [
    ".ssh", ".aws", ".gnupg", ".kube", ".docker", ".azure", ".netrc", ".git-credentials", ".npmrc", ".pypirc",
    ".password-store", ".config/gh", ".config/gcloud", ".config/op", ".claude", ".claude.json", ".codex", ".gemini",
    ".cargo/credentials", ".cargo/credentials.toml", ".zsh_history", ".bash_history",
];

/// The folders on PATH inside the home folder, and where the programs linked from them are
/// installed (`~/.local/bin/node` -> `~/.hermes/node/bin/node` opens `~/.hermes/node`), so the
/// commands the user has keep working without opening the rest of the home folder.
fn tool_dirs(user_home: &Path) -> Vec<PathBuf> {
    let deeper = |p: &Path| p.starts_with(user_home) && p != user_home;
    let mut out: Vec<PathBuf> = vec![];
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path).filter_map(|d| d.canonicalize().ok()).filter(|d| deeper(d)) {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(target) = entry.path().canonicalize() else { continue };
            // `<prefix>/bin/<program>` -> `<prefix>`
            if let Some(prefix) = target.parent().filter(|b| b.ends_with("bin")).and_then(Path::parent) {
                if deeper(prefix) && !prefix.starts_with(&dir) && !out.iter().any(|o| prefix.starts_with(o)) {
                    out.push(prefix.to_path_buf());
                }
            }
        }
        out.push(dir);
    }
    out
}

fn sb_quote(p: &Path) -> String {
    format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""))
}

fn re_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if !c.is_alphanumeric() && c != '/' && c != '_' && c != '-' && c != ' ' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The rules for `sandbox-exec`. Later rules win over earlier ones.
fn sandbox_profile(user_home: &Path, roots: &[PathBuf], tools: &[PathBuf], tmp: &[PathBuf], data_dir: &Path) -> String {
    let mut p = String::from("(version 1)\n(allow default)\n");
    // Writing: only the folders and temp folders.
    p.push_str("(deny file-write* (subpath \"/\"))\n(allow file-write* (subpath \"/dev\"))\n");
    for t in tmp {
        p.push_str(&format!("(allow file-write* (subpath {}))\n", sb_quote(t)));
    }
    // Reading: nothing in the home folder (Documents, Library, hidden logins...) but the toolchains,
    // and no other disks.
    p.push_str(&format!("(deny file-read* (regex #\"^{}/\"))\n", re_escape(&user_home.to_string_lossy())));
    p.push_str("(deny file-read* (subpath \"/Volumes\"))\n");
    let opened = TOOL_DOTS.iter().map(|d| (user_home.join(d), "file-read*")).chain(tools.iter().map(|t| (t.clone(), "file-read*")));
    for (dir, rights) in opened.chain(roots.iter().map(|r| (r.clone(), "file-read* file-write*"))) {
        p.push_str(&format!("(allow {rights} (subpath {}))\n", sb_quote(&dir)));
        // Programs look at each folder above the one they use (finding their real path, git
        // looking for a repository).
        for a in dir.ancestors().skip(1).filter(|a| a.starts_with(user_home) && *a != user_home || a.starts_with("/Volumes")) {
            p.push_str(&format!("(allow file-read-metadata (literal {}))\n", sb_quote(a)));
        }
    }
    // After the folders, so none of them opens these again.
    for d in PRIVATE_DOTS {
        p.push_str(&format!("(deny file-read* file-write* (subpath {}))\n", sb_quote(&user_home.join(d))));
    }
    // No starting programs outside the sandbox: `open` and Apple events would hand the work to
    // an app that isn't in it. (launchd already refuses jobs from sandboxed programs.)
    p.push_str("(deny appleevent-send)\n");
    p.push_str("(deny mach-lookup (global-name \"com.apple.coreservices.launchservicesd\") (global-name \"com.apple.coreservices.appleevents\") (global-name-prefix \"com.apple.lsd.\"))\n");
    // oBot's own settings (tokens, the Vault) stay closed whatever the folders are.
    p.push_str(&format!("(deny file-read* file-write* (subpath {}))\n", sb_quote(data_dir)));
    p
}

/// Temp folders commands may write to.
fn temp_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/private/tmp")];
    if let Ok(t) = std::env::temp_dir().canonicalize() {
        if !v.contains(&t) {
            v.push(t);
        }
    }
    v
}

/// The command in the user's shell, in `cwd`, inside the sandbox where there is one.
fn shell(command: &str, cwd: &Path, r: &Roots, data_dir: &Path) -> Command {
    let mut c;
    #[cfg(target_os = "macos")]
    {
        let sh = std::env::var("SHELL").ok().filter(|s| s.ends_with("/zsh") || s.ends_with("/bash")).unwrap_or_else(|| "/bin/zsh".into());
        if has_sandbox() {
            let home = dirs::home_dir().unwrap_or_default();
            let home = home.canonicalize().unwrap_or(home);
            let data = data_dir.canonicalize().unwrap_or_else(|_| data_dir.to_path_buf());
            c = Command::new("/usr/bin/sandbox-exec");
            c.args(["-p", &sandbox_profile(&home, &r.all, &tool_dirs(&home), &temp_dirs(), &data), &sh, "-c", command]);
        } else {
            c = Command::new(sh);
            c.args(["-c", command]);
        }
        // Downloads that tools keep for next time go in the Bots' folder: they can't write to the usual places.
        let cache = r.home.join(".cache");
        for (k, sub) in [("XDG_CACHE_HOME", ""), ("npm_config_cache", "npm"), ("PIP_CACHE_DIR", "pip"), ("UV_CACHE_DIR", "uv"), ("YARN_CACHE_FOLDER", "yarn"), ("BUN_INSTALL_CACHE_DIR", "bun"), ("GOCACHE", "go-build"), ("GOMODCACHE", "go-mod")] {
            c.env(k, cache.join(sub));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (r, data_dir);
        if cfg!(windows) {
            match crate::jobs::git_bash() {
                Some(bash) => {
                    c = Command::new(bash);
                    c.args(["-lc", command]);
                }
                None => {
                    c = Command::new("powershell.exe");
                    c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
                }
            }
        } else {
            c = Command::new(if Path::new("/bin/bash").is_file() { "/bin/bash" } else { "sh" });
            c.args(["-c", command]);
        }
    }
    c.current_dir(cwd).stdin(Stdio::null()).kill_on_drop(true).env("OBOT", "1");
    #[cfg(unix)]
    c.process_group(0);
    hide_console(&mut c);
    c
}

// ---------------------------------------------------------------- running commands

/// Commands running now: request id or job id -> ("stop it", its process id).
static RUNNING: LazyLock<Mutex<HashMap<String, (Arc<Notify>, Option<u32>)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The switch went off: every running command is stopped. Each one's own task kills it and reports
/// how it ended.
fn stop_all() {
    for (stop, _) in RUNNING.lock().unwrap_or_else(|e| e.into_inner()).values() {
        stop.notify_one();
    }
}

/// The app is quitting: what is running ends now, with nothing left to wait for it.
pub fn kill_all() {
    for pid in RUNNING.lock().unwrap_or_else(|e| e.into_inner()).values().filter_map(|(_, pid)| *pid) {
        crate::jobs::kill_pid_tree(pid);
    }
}

/// When a process started (seconds since 1970), to tell it apart from a later one with the same id.
fn started_at(pid: u32) -> Option<u64> {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.start_time())
}

/// A background command.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Job {
    id: String,
    bot_id: String,
    name: String,
    command: String,
    log: String,
    /// running | done | failed | stopped | timed_out | interrupted
    status: String,
    exit_code: Option<i64>,
    pid: Option<u32>,
    /// When the process started (`started_at`).
    #[serde(default)]
    started: Option<u64>,
    /// The other computer knows how it ended.
    reported: bool,
}

static JOBS: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn jobs_file(data_dir: &Path) -> PathBuf {
    data_dir.join("this-computer").join("jobs.json")
}

fn save_jobs(data_dir: &Path) {
    let list: Vec<Job> = JOBS.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|j| j.status == "running" || !j.reported).cloned().collect();
    let path = jobs_file(data_dir);
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
    let _ = std::fs::write(path, serde_json::to_vec(&list).unwrap_or_default());
}

/// At startup: a command that was running when the app quit is stopped and reported as interrupted.
/// Only if it is still that process: after a restart its id may belong to another program.
fn recover_jobs(data_dir: &Path) {
    let Ok(text) = std::fs::read(jobs_file(data_dir)) else { return };
    let list: Vec<Job> = serde_json::from_slice(&text).unwrap_or_default();
    let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    for mut j in list {
        if j.status == "running" {
            if let Some(pid) = j.pid.filter(|p| j.started.is_some() && started_at(*p) == j.started) {
                crate::jobs::kill_pid_tree(pid);
            }
            j.status = "interrupted".into();
        }
        jobs.insert(j.id.clone(), j);
    }
}

/// The start and the end of a command's output.
#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    dropped: usize,
}

impl Capture {
    fn add(&mut self, mut data: &[u8]) {
        let room = HEAD_BYTES.saturating_sub(self.head.len());
        if room > 0 {
            let n = room.min(data.len());
            self.head.extend_from_slice(&data[..n]);
            data = &data[n..];
        }
        for b in data {
            if self.tail.len() == TAIL_BYTES {
                self.tail.pop_front();
                self.dropped += 1;
            }
            self.tail.push_back(*b);
        }
    }
    fn text(&self) -> String {
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        let mut s = String::from_utf8_lossy(&self.head).into_owned();
        if self.dropped > 0 {
            s.push_str(&format!("\n[… {} bytes of output left out …]\n", self.dropped));
        }
        s.push_str(&String::from_utf8_lossy(&tail));
        s
    }
}

async fn capture(mut pipe: impl tokio::io::AsyncRead + Unpin, into: Arc<Mutex<Capture>>) {
    let mut buf = vec![0u8; 8192];
    while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 {
            break;
        }
        into.lock().unwrap_or_else(|e| e.into_inner()).add(&buf[..n]);
    }
}

fn this_name() -> String {
    let name = sysinfo::System::host_name().unwrap_or_default();
    let name = name.trim().trim_end_matches(".local");
    if name.is_empty() { "this computer".into() } else { name.to_string() }
}

/// A hint when the sandbox is the likely reason a command failed.
fn sandbox_hint(output: &str) -> &'static str {
    if has_sandbox() && (output.contains("Operation not permitted") || output.contains("operation not permitted")) {
        "\n[oBot: Commands on this computer can write only in the folders the user opened to Bots and in temp folders, \
         and can't read the user's other folders or keys. \"Operation not permitted\" usually means the command reached outside them. \
         To work in another folder, run the command again with that folder as cwd: oBot asks the user to open it to you.]"
    } else {
        ""
    }
}

async fn run_command(st: &Shared, r: &Roots, id: &str, req: &Value) -> Result<String, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let (bot, command) = (s("bot"), s("command"));
    if command.is_empty() {
        return Err("Give a command.".into());
    }
    if command.chars().count() > MAX_COMMAND_CHARS {
        return Err("That command is too long. Split it into smaller steps, or write a script file and run that.".into());
    }
    let cwd = if s("cwd").is_empty() { bot_folder(r, &bot)? } else { resolve(r, &bot, &s("cwd"))? };
    if !cwd.is_dir() {
        return Err(format!("There is no folder {}.", cwd.display()));
    }
    let mut cmd = shell(&command, &cwd, r, &st.data_dir);
    if req.get("background").and_then(Value::as_bool) == Some(true) {
        return start_job(st, cmd, req, &command, &cwd);
    }
    let secs = req.get("timeout_s").and_then(Value::as_u64).unwrap_or(DEFAULT_SECONDS).clamp(1, MAX_SECONDS);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the command: {e}"))?;
    let out = Arc::new(Mutex::new(Capture::default()));
    let readers = [
        child.stdout.take().map(|p| tokio::spawn(capture(p, out.clone()))),
        child.stderr.take().map(|p| tokio::spawn(capture(p, out.clone()))),
    ];
    let stop = Arc::new(Notify::new());
    let pid = child.id();
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), (stop.clone(), pid));
    let (status, how) = tokio::select! {
        s = child.wait() => (s.ok(), ""),
        _ = stop.notified() => { crate::jobs::kill_tree(pid, &mut child).await; (None, "stopped") }
        _ = tokio::time::sleep(Duration::from_secs(secs)) => { crate::jobs::kill_tree(pid, &mut child).await; (None, "timeout") }
    };
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    // Nothing the command left running in the background keeps going unseen.
    #[cfg(unix)]
    if let Some(pid) = pid {
        let _ = std::process::Command::new("kill").args(["-KILL", &format!("-{pid}")]).stderr(Stdio::null()).status();
    }
    for r in readers.into_iter().flatten() {
        let _ = tokio::time::timeout(Duration::from_secs(3), r).await;
    }
    let output = out.lock().unwrap_or_else(|e| e.into_inner()).text();
    let output = if output.trim().is_empty() { "(no output)".to_string() } else { output };
    let first = match (how, status.and_then(|s| s.code())) {
        ("stopped", _) => "Stopped before it finished.".to_string(),
        ("timeout", _) => format!("Stopped: it was still running after {secs} seconds. For long commands use background: true."),
        (_, Some(code)) => format!("Exit code: {code}"),
        _ => "The command ended without an exit code.".to_string(),
    };
    Ok(format!("On {}, in {}\n{first}\n{output}{}", this_name(), cwd.display(), sandbox_hint(&output)))
}

fn start_job(st: &Shared, mut cmd: Command, req: &Value, command: &str, cwd: &Path) -> Result<String, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let running = JOBS.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|j| j.status == "running").count();
    if running >= 5 {
        return Err("5 background commands are already running on this computer. Wait for one to end or stop one.".into());
    }
    let id = new_id()[..8].to_string();
    let dir = st.data_dir.join("this-computer").join("jobs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let log = dir.join(format!("{id}.log"));
    let file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    cmd.stdout(Stdio::from(file.try_clone().map_err(|e| e.to_string())?)).stderr(Stdio::from(file));
    // It outlives this request (it is stopped by `my_computer_job`, its time limit, or the app quitting).
    cmd.kill_on_drop(false);
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the command: {e}"))?;
    let name = if s("name").is_empty() { truncate_chars(command, 40) } else { truncate_chars(&s("name"), 80) };
    let job = Job {
        id: id.clone(),
        bot_id: s("bot_id"),
        name: name.clone(),
        command: truncate_chars(command, 500),
        log: log.to_string_lossy().into_owned(),
        status: "running".into(),
        exit_code: None,
        pid: child.id(),
        started: child.id().and_then(started_at),
        reported: false,
    };
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), job);
    save_jobs(&st.data_dir);
    let stop = Arc::new(Notify::new());
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), (stop.clone(), child.id()));
    let minutes = req.get("max_minutes").and_then(Value::as_u64).unwrap_or(DEFAULT_JOB_MINUTES).clamp(1, MAX_JOB_MINUTES);
    let (st, jid) = (st.clone(), id.clone());
    tokio::spawn(async move {
        let pid = child.id();
        let (status, code) = tokio::select! {
            s = child.wait() => match s {
                Ok(s) => (if s.success() { "done" } else { "failed" }, s.code().map(i64::from)),
                Err(_) => ("failed", None),
            },
            _ = stop.notified() => { crate::jobs::kill_tree(pid, &mut child).await; ("stopped", None) }
            _ = tokio::time::sleep(Duration::from_secs(minutes * 60)) => { crate::jobs::kill_tree(pid, &mut child).await; ("timed_out", None) }
        };
        #[cfg(unix)]
        if let Some(pid) = pid {
            let _ = std::process::Command::new("kill").args(["-KILL", &format!("-{pid}")]).stderr(Stdio::null()).status();
        }
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&jid);
        if let Some(j) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&jid) {
            j.status = status.into();
            j.exit_code = code;
        }
        save_jobs(&st.data_dir);
        report_jobs(&st).await;
    });
    Ok(format!(
        "Started in the background on {}, in {} (job {id}: \"{name}\"). You'll be woken up when it ends; \
         my_computer_job shows its output so far or stops it.",
        this_name(),
        cwd.display()
    ))
}

/// The last lines of a job's output.
fn tail(path: &str) -> String {
    let Ok(text) = std::fs::read(path) else { return "(no output)".into() };
    let start = text.len().saturating_sub(64 * 1024);
    let text = String::from_utf8_lossy(&text[start..]);
    let all: Vec<&str> = text.lines().collect();
    let out = all[all.len().saturating_sub(TAIL_LINES)..].join("\n");
    if out.trim().is_empty() { "(no output)".into() } else { truncate_chars(&out, 6000) }
}

fn job_tool(req: &Value) -> Result<String, String> {
    let id = req.get("job_id").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let bot_id = req.get("bot_id").and_then(Value::as_str).unwrap_or("");
    let job = JOBS.lock().unwrap_or_else(|e| e.into_inner()).get(&id).filter(|j| j.bot_id == bot_id).cloned();
    let Some(j) = job else {
        return Err(format!("No background command with the id \"{id}\" on {}.", this_name()));
    };
    if req.get("stop").and_then(Value::as_bool) == Some(true) {
        return match RUNNING.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
            Some((n, _)) => {
                n.notify_one();
                Ok(format!("Stopping \"{}\".", j.name))
            }
            None => Err(format!("\"{}\" isn't running ({}).", j.name, j.status)),
        };
    }
    let exit = j.exit_code.map(|c| format!(", exit code {c}")).unwrap_or_default();
    Ok(format!("\"{}\" on {}: {}{exit}\nCommand: {}\nLatest output:\n{}", j.name, this_name(), j.status, j.command, tail(&j.log)))
}

// ---------------------------------------------------------------- files

/// Opens a file `resolve` let through, checking the opened file itself: a command running in a
/// folder could swap in a link after `resolve` looked. A link as the last part is refused, and on
/// macOS (where commands are kept inside the folders) the opened file's real path must still be
/// inside them. Only plain files: a pipe would never end. A file to write isn't emptied until
/// after the check.
fn open_inside(r: &Roots, path: &Path, write: bool) -> Result<std::fs::File, String> {
    let mut o = std::fs::OpenOptions::new();
    o.read(!write).write(write).create(write);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut o, libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let f = o.open(path).map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
    if !f.metadata().is_ok_and(|m| m.is_file()) {
        return Err(format!("{} isn't a plain file.", path.display()));
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::{ffi::OsStrExt, io::AsRawFd};
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        if unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
            return Err(format!("Couldn't check {}: {}", path.display(), std::io::Error::last_os_error()));
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        let real = Path::new(std::ffi::OsStr::from_bytes(&buf[..end]));
        if !inside(&r.all, real) {
            return Err(format!("{} leads outside the folders the user opened to Bots ({}). Nothing was done.", path.display(), real.display()));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = r;
    if write {
        f.set_len(0).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    }
    Ok(f)
}

fn read_tool(r: &Roots, req: &Value) -> Result<String, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let path = resolve(r, &s("bot"), &s("path"))?;
    if path.is_dir() {
        let mut rows: Vec<String> = std::fs::read_dir(&path)
            .map_err(|e| format!("Couldn't open {}: {e}", path.display()))?
            .flatten()
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                match e.metadata() {
                    Ok(m) if m.is_dir() => format!("{name}/"),
                    Ok(m) => format!("{name}  ({} bytes)", m.len()),
                    Err(_) => name,
                }
            })
            .collect();
        rows.sort();
        let more = rows.len().saturating_sub(500);
        rows.truncate(500);
        let list = if rows.is_empty() { "(empty)".to_string() } else { rows.join("\n") };
        return Ok(format!("{} on {}:\n{list}{}", path.display(), this_name(), if more > 0 { format!("\n… and {more} more") } else { String::new() }));
    }
    if !path.exists() {
        return Err(format!("There is no file {} on {}.", path.display(), this_name()));
    }
    let file = open_inside(r, &path, false)?;
    let mut bytes = vec![];
    std::io::Read::read_to_end(&mut std::io::Read::take(file, MAX_FILE_BYTES + 1), &mut bytes).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("{} is too big to read.", path.display()));
    }
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(format!("{} isn't a text file ({} bytes). Copy it to your own folder with my_computer_copy to work with it.", path.display(), bytes.len()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let offset = req.get("offset").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
    let limit = req.get("limit").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(READ_LINES).clamp(1, READ_LINES);
    let total = text.lines().count();
    let mut out = String::new();
    let mut last = offset - 1;
    for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
        if out.len() > READ_BYTES {
            break;
        }
        out.push_str(&format!("{:>6}\t{}\n", i + 1, truncate_chars(line, 2000)));
        last = i + 1;
    }
    if total == 0 {
        return Ok(format!("{} is empty.", path.display()));
    }
    if last < total {
        out.push_str(&format!("[lines {offset} to {last} of {total}; read more with offset: {}]\n", last + 1));
    }
    Ok(out)
}

fn write_tool(r: &Roots, req: &Value) -> Result<String, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let content = req.get("content").and_then(Value::as_str).unwrap_or("");
    if content.len() > MAX_WRITE_BYTES {
        return Err("That is too much text for one file write. Copy the file with my_computer_copy instead.".into());
    }
    let path = resolve(r, &s("bot"), &s("path"))?;
    if path.is_dir() {
        return Err(format!("{} is a folder.", path.display()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    }
    let mut file = open_inside(r, &path, true)?;
    std::io::Write::write_all(&mut file, content.as_bytes()).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(format!("Wrote {} ({} bytes) on {}.", path.display(), content.len(), this_name()))
}

/// Copy a file from this computer to the Bot's computer: its bytes go up to `/api/my-computer/file/<id>`.
fn send_file(r: &Roots, conn: &Conn, id: &str, req: &Value) -> Result<String, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let path = resolve(r, &s("bot"), &s("path"))?;
    if !path.exists() {
        return Err(format!("There is no file {} on {}.", path.display(), this_name()));
    }
    if path.is_dir() {
        return Err(format!("{} is a folder. Copy files one at a time, or zip the folder with a command first.", path.display()));
    }
    let file = open_inside(r, &path, false)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is too big to copy ({} bytes; the limit is {} MB).", path.display(), meta.len(), MAX_FILE_BYTES / 1024 / 1024));
    }
    conn.agent(600)
        .post(&format!("{}/api/my-computer/file/{id}", conn.url))
        .set("Authorization", &format!("Bearer {}", conn.token))
        .set("Content-Type", "application/octet-stream")
        .set("Content-Length", &meta.len().to_string())
        .send(file)
        .map_err(|e| format!("The copy didn't get through: {}", http_error(e)))?;
    Ok(format!("{} ({} bytes) from {}", path.display(), meta.len(), this_name()))
}

/// Copy a file from the Bot's computer to this one: its bytes come from `/api/my-computer/file/<id>`.
fn receive_file(r: &Roots, conn: &Conn, id: &str, req: &Value) -> Result<String, String> {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let mut path = resolve(r, &s("bot"), &s("path"))?;
    if path.is_dir() {
        // "Put it in this folder": keep the file's own name.
        let name = safe_file_name(&s("name"));
        path = resolve(r, &s("bot"), &path.join(name).to_string_lossy())?;
    }
    let dir = path.parent().ok_or("That path has no folder.")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    let resp = conn
        .agent(600)
        .get(&format!("{}/api/my-computer/file/{id}", conn.url))
        .set("Authorization", &format!("Bearer {}", conn.token))
        .call()
        .map_err(|e| format!("The copy didn't get through: {}", http_error(e)))?;
    let part = dir.join(format!(".obot-copy-{id}"));
    let result = (|| {
        let mut out = open_inside(r, &part, true)?;
        let mut reader = std::io::Read::take(resp.into_reader(), MAX_FILE_BYTES + 1);
        let n = std::io::copy(&mut reader, &mut out).map_err(|e| e.to_string())?;
        if n > MAX_FILE_BYTES {
            return Err(format!("The file is too big to copy (the limit is {} MB).", MAX_FILE_BYTES / 1024 / 1024));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        Ok(n)
    })();
    match result {
        Ok(n) => Ok(format!("{} ({n} bytes) on {}", path.display(), this_name())),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(format!("Couldn't save {}: {e}", path.display()))
        }
    }
}

fn safe_file_name(name: &str) -> String {
    let n = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if n.is_empty() || n.starts_with('.') { "file".into() } else { n }
}

// ---------------------------------------------------------------- talking to the other computer

#[derive(Clone)]
struct Conn {
    url: String,
    token: String,
}

impl Conn {
    fn agent(&self, read_secs: u64) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(read_secs)).build()
    }
    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.agent(30)
            .post(&format!("{}{path}", self.url))
            .set("Authorization", &format!("Bearer {}", self.token))
            .send_json(body)
            .map_err(http_error)?
            .into_json::<Value>()
            .map_err(|e| e.to_string())
    }
    fn get(&self, path: &str, read_secs: u64) -> Result<Value, String> {
        self.agent(read_secs)
            .get(&format!("{}{path}", self.url))
            .set("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(http_error)?
            .into_json::<Value>()
            .map_err(|e| e.to_string())
    }
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let msg = r.into_json::<Value>().ok().and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
            if msg.is_empty() { format!("the other computer answered {code}") } else { format!("{msg} ({code})") }
        }
        other => other.to_string(),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn hello_body(share: &Share) -> Value {
    json!({
        "name": this_name(),
        "platform": crate::util::platform_name(),
        "os": sysinfo::System::long_os_version().unwrap_or_default(),
        "enabled": share.enabled,
        "home": share.home,
        "folders": share.folders,
        "sandbox": has_sandbox(),
        "version": crate::VERSION,
    })
}

/// What the other computer must hear again when any of it changes.
fn signature(conn: &Conn, share: &Share) -> String {
    format!("{}|{}|{}", conn.url, crate::security::sha256_hex(&conn.token), hello_body(share))
}

/// Tell the other computer whether Bots may work here, and in which folders.
async fn hello(st: &Shared, conn: &Conn, share: &Share) -> Result<(), String> {
    let (c, body, sig) = (conn.clone(), hello_body(share), signature(conn, share));
    blocking(move || c.post("/api/my-computer/hello", body)).await?;
    *st.this_computer.said.lock().unwrap_or_else(|e| e.into_inner()) = sig;
    Ok(())
}

/// The connection to the Bots' computer, when this app uses one and was paired with a code.
async fn connection(st: &Shared) -> Result<(Conn, Share), &'static str> {
    let cfg = st.config.read().await;
    let c = &cfg.connection;
    if c.mode != "remote" || c.url.trim().is_empty() {
        return Err("not_remote");
    }
    if !c.token.starts_with("obd_") {
        return Err("needs_pairing");
    }
    Ok((Conn { url: c.url.trim().trim_end_matches('/').to_string(), token: c.token.clone() }, cfg.this_computer.clone()))
}

/// After the settings changed: tell the other computer now instead of at the next round.
pub fn settings_changed(st: &Shared) {
    let st = st.clone();
    tokio::spawn(async move {
        if let Ok((conn, share)) = connection(&st).await {
            let _ = hello(&st, &conn, &share).await;
        }
        st.this_computer.changed.notify_one();
    });
}

/// Tell the other computer how background commands ended, so their Bots are woken.
async fn report_jobs(st: &Shared) {
    let Ok((conn, _)) = connection(st).await else { return };
    let waiting: Vec<Job> = JOBS.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|j| j.status != "running" && !j.reported).cloned().collect();
    for j in waiting {
        let body = json!({
            "bot_id": j.bot_id, "job_id": j.id, "name": j.name, "command": j.command,
            "status": j.status, "exit_code": j.exit_code, "tail": tail(&j.log),
        });
        let c = conn.clone();
        if blocking(move || c.post("/api/my-computer/job-done", body)).await.is_ok() {
            if let Some(x) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&j.id) {
                x.reported = true;
            }
        }
    }
    save_jobs(&st.data_dir);
}

/// Do one request from the other computer and send back the answer.
async fn handle(st: Shared, conn: Conn, req: Value) {
    let id = req.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let kind = req.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    // The settings as they are now: a request that arrives after the switch went off is refused.
    let share = st.config.read().await.this_computer.clone();
    let result = if !share.enabled {
        Err(format!("The user turned off \"Let Bots work on this computer\" on {}. Nothing was done.", this_name()))
    } else {
        match roots(&share) {
            Err(e) => Err(e),
            Ok(r) => match kind.as_str() {
                "run" => run_command(&st, &r, &id, &req).await,
                "job" => job_tool(&req),
                "read" | "write" | "send" | "receive" => {
                    let (c, i, q, k) = (conn.clone(), id.clone(), req.clone(), kind.clone());
                    blocking(move || match k.as_str() {
                        "read" => read_tool(&r, &q),
                        "write" => write_tool(&r, &q),
                        "send" => send_file(&r, &c, &i, &q),
                        _ => receive_file(&r, &c, &i, &q),
                    })
                    .await
                }
                other => Err(format!("This version of oBot on {} can't do \"{other}\". Update the oBot app there.", this_name())),
            },
        }
    };
    let body = match result {
        Ok(text) => json!({ "id": id, "ok": true, "text": text }),
        Err(text) => json!({ "id": id, "ok": false, "text": text }),
    };
    // The answer matters more than speed: try a few times over a shaky connection.
    for wait in [0, 2, 5, 10] {
        tokio::time::sleep(Duration::from_secs(wait)).await;
        let (c, b) = (conn.clone(), body.clone());
        if blocking(move || c.post("/api/my-computer/reply", b)).await.is_ok() {
            return;
        }
    }
}

/// Start the helper. It idles until this app is connected to another computer.
pub fn spawn(st: Shared) {
    tokio::spawn(async move {
        recover_jobs(&st.data_dir);
        let mut backoff = 2;
        loop {
            let (conn, share) = match connection(&st).await {
                Ok(x) => x,
                Err(why) => {
                    st.this_computer.set_status(why);
                    st.this_computer.said.lock().unwrap_or_else(|e| e.into_inner()).clear();
                    st.this_computer.changed.notified().await;
                    continue;
                }
            };
            let said = st.this_computer.said.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if said != signature(&conn, &share) {
                if let Err(e) = hello(&st, &conn, &share).await {
                    st.this_computer.set_status(&format!("error: {e}"));
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                        _ = st.this_computer.changed.notified() => {}
                    }
                    backoff = (backoff * 2).min(30);
                    continue;
                }
            }
            if !share.enabled {
                st.this_computer.set_status("off");
                st.this_computer.changed.notified().await;
                continue;
            }
            st.this_computer.set_status("connected");
            report_jobs(&st).await;
            let c = conn.clone();
            match blocking(move || c.get("/api/my-computer/poll?wait=25", 40)).await {
                Ok(v) => {
                    backoff = 2;
                    if v.get("hello").and_then(Value::as_bool) == Some(true) {
                        st.this_computer.said.lock().unwrap_or_else(|e| e.into_inner()).clear();
                    }
                    for id in v.get("cancel").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                        if let Some((n, _)) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
                            n.notify_one();
                        }
                    }
                    for req in v.get("requests").and_then(Value::as_array).cloned().unwrap_or_default() {
                        tokio::spawn(handle(st.clone(), conn.clone(), req));
                    }
                }
                Err(e) => {
                    st.this_computer.set_status(&format!("error: {e}"));
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                        _ = st.this_computer.changed.notified() => {}
                    }
                    backoff = (backoff * 2).min(30);
                }
            }
        }
    });
}

/// `GET /api/this-computer`: the settings and whether the helper is connected.
pub async fn view(st: &Shared) -> Value {
    let share = st.config.read().await.this_computer.clone();
    json!({
        "enabled": share.enabled,
        "home": share.home,
        "default_home": default_home(),
        "folders": share.folders,
        "sandbox": has_sandbox(),
        "name": this_name(),
        "status": st.this_computer.status(),
    })
}

/// `PUT /api/this-computer` `{enabled?, home?, folders?}`.
pub async fn update(st: &Shared, body: &Value) -> Result<Value, String> {
    let mut share = st.config.read().await.this_computer.clone();
    let was_on = share.enabled;
    if let Some(on) = body.get("enabled").and_then(Value::as_bool) {
        share.enabled = on;
    }
    if let Some(home) = body.get("home").and_then(Value::as_str) {
        let home = if home.trim().is_empty() { default_home() } else { home.trim().to_string() };
        std::fs::create_dir_all(&home).map_err(|e| format!("Couldn't make {home}: {e}"))?;
        share.home = check_folder(&home, &st.data_dir)?.to_string_lossy().into_owned();
    }
    let mut list: Option<Vec<String>> = body.get("folders").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
    if let Some(add) = body.get("add_folder").and_then(Value::as_str) {
        let folder = folder_to_add(add, &st.data_dir)?;
        list.get_or_insert_with(|| share.folders.clone()).push(folder);
    }
    if let Some(list) = list {
        let mut folders: Vec<String> = vec![];
        for f in &list {
            let real = check_folder(f, &st.data_dir)?.to_string_lossy().into_owned();
            if real != share.home && !folders.contains(&real) {
                folders.push(real);
            }
        }
        if folders.len() > 30 {
            return Err("That's more than 30 folders.".into());
        }
        share.folders = folders;
    }
    if share.enabled {
        // The Bots' folder exists from the moment the switch is on.
        std::fs::create_dir_all(&share.home).map_err(|e| format!("Couldn't make {}: {e}", share.home))?;
        check_folder(&share.home, &st.data_dir)?;
    }
    // Shown right away; the helper sets the real state a moment later.
    match (share.enabled, st.this_computer.status().as_str()) {
        (true, "off") => st.this_computer.set_status("connecting"),
        (false, "connected" | "connecting") => st.this_computer.set_status("off"),
        _ => {}
    }
    let off = was_on && !share.enabled;
    st.config.write().await.this_computer = share;
    st.save_config().await.map_err(|e| e.to_string())?;
    if off {
        stop_all();
    }
    settings_changed(st);
    Ok(view(st).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("obot-this-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn paths_stay_inside_the_folders() {
        let d = scratch("paths");
        let (home, extra, secret) = (d.join("oBot"), d.join("project"), d.join("secret"));
        for p in [&home, &extra, &secret] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(secret.join("key.txt"), "x").unwrap();
        let share = Share { enabled: true, home: home.to_string_lossy().into(), folders: vec![extra.to_string_lossy().into()] };
        let r = roots(&share).unwrap();
        // A path that doesn't start at the root is in the Bot's own folder, which is made on first use.
        assert_eq!(resolve(&r, "Life Admin", "notes/a.md").unwrap(), home.join("Life Admin/notes/a.md"));
        assert!(home.join("Life Admin").is_dir());
        // A folder the user added, and a new file in it.
        assert_eq!(resolve(&r, "Life Admin", &extra.join("new/x.txt").to_string_lossy()).unwrap(), extra.join("new/x.txt"));
        // Outside: straight, through "..", and through a link.
        assert!(resolve(&r, "Life Admin", &secret.join("key.txt").to_string_lossy()).is_err());
        assert!(resolve(&r, "Life Admin", "../../secret/key.txt").is_err());
        assert!(resolve(&r, "Life Admin", &extra.join("nope/../../secret/key.txt").to_string_lossy()).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, extra.join("link")).unwrap();
            assert!(resolve(&r, "Life Admin", &extra.join("link/key.txt").to_string_lossy()).is_err());
            assert!(resolve(&r, "Life Admin", &extra.join("link/new.txt").to_string_lossy()).is_err());
        }
        // A Bot's name can't lead out of the Bots' folder.
        assert_eq!(resolve(&r, "../../secret", "a").unwrap(), home.join("secret/a"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_card_opens_its_folder_here() {
        let d = scratch("add");
        let (data, work) = (d.join("data"), d.join("work"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(work.join("project")).unwrap();
        std::fs::write(work.join("project/notes.md"), "x").unwrap();
        let st = crate::state::test_state(&data);
        let add = |p: PathBuf| {
            let st = st.clone();
            async move { update(&st, &json!({ "add_folder": p.to_string_lossy() })).await }
        };
        let folders = |v: Value| v["folders"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect::<Vec<_>>();
        let project = work.join("project").to_string_lossy().into_owned();
        // A file opens its own folder; opening it again changes nothing.
        assert_eq!(folders(add(work.join("project/notes.md")).await.unwrap()), vec![project.clone()]);
        assert_eq!(folders(add(work.join("project")).await.unwrap()), vec![project.clone()]);
        // A folder that doesn't exist yet is made, when the one above it exists.
        let fresh = work.join("TESCstuff");
        assert_eq!(folders(add(fresh.clone()).await.unwrap()), vec![project.clone(), fresh.to_string_lossy().into_owned()]);
        assert!(fresh.is_dir());
        assert!(add(work.join("nope/deeper")).await.is_err());
        assert!(!work.join("nope").exists());
        // Nothing is made inside oBot's own settings.
        assert!(add(data.join("new")).await.is_err());
        assert!(!data.join("new").exists());
        assert_eq!(st.config.read().await.this_computer.folders.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bot_names_become_folder_names() {
        assert_eq!(safe_folder_name("Life Admin"), "Life Admin");
        assert_eq!(safe_folder_name("../x/y"), "x y");
        assert_eq!(safe_folder_name("  .hidden "), "hidden");
        assert_eq!(safe_folder_name("///"), "Bot");
    }

    #[test]
    fn folders_with_obots_settings_cant_be_opened() {
        let d = scratch("check");
        let data = d.join("home/Library/oBot");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(d.join("home/Code/x")).unwrap();
        assert!(check_folder(&d.join("home/Code/x").to_string_lossy(), &data).is_ok());
        assert!(check_folder(&d.join("home").to_string_lossy(), &data).is_err());
        assert!(check_folder(&data.to_string_lossy(), &data).is_err());
        assert!(check_folder(&d.join("home/missing").to_string_lossy(), &data).is_err());
        assert!(check_folder("relative/path", &data).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sandbox_rules_close_the_home_folder_and_open_the_folders() {
        let home = Path::new("/Users/sam");
        let roots = [PathBuf::from("/Users/sam/oBot"), PathBuf::from("/Users/sam/Code/site")];
        let p = sandbox_profile(home, &roots, &[PathBuf::from("/Users/sam/.hermes/node")], &[PathBuf::from("/private/tmp")], Path::new("/Users/sam/Library/Application Support/oBot"));
        let at = |s: &str| p.find(s).unwrap_or_else(|| panic!("missing: {s}"));
        // Everything is closed first, then the folders and toolchains are opened (later rules win).
        let closed = at("(deny file-read* (regex #\"^/Users/sam/\"))");
        assert!(at("(deny file-write* (subpath \"/\"))") < at("(allow file-read* file-write* (subpath \"/Users/sam/oBot\"))"));
        assert!(closed < at("(allow file-read* file-write* (subpath \"/Users/sam/Code/site\"))"));
        assert!(closed < at("(allow file-read* (subpath \"/Users/sam/.cargo\"))"));
        assert!(closed < at("(allow file-read* (subpath \"/Users/sam/.hermes/node\"))"));
        // Keys are closed after the folders, so no folder opens them again.
        assert!(at("(allow file-read* file-write* (subpath \"/Users/sam/Code/site\"))") < at("(deny file-read* file-write* (subpath \"/Users/sam/.ssh\"))"));
        assert!(p.contains("(deny file-read* file-write* (subpath \"/Users/sam/.cargo/credentials.toml\"))"));
        // No handing work to apps outside the sandbox.
        assert!(p.contains("(deny appleevent-send)") && p.contains("com.apple.coreservices.launchservicesd"));
        // The folder above an opened one can be looked at, not read.
        assert!(p.contains("(allow file-read-metadata (literal \"/Users/sam/Code\"))"));
        // oBot's own settings are closed last, so nothing reopens them.
        assert!(p.trim_end().ends_with("(deny file-read* file-write* (subpath \"/Users/sam/Library/Application Support/oBot\"))"));
    }

    /// The real sandbox on this Mac: what must fail fails, and the user's tools still work.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_sandbox_keeps_commands_in() {
        if !has_sandbox() {
            return;
        }
        let d = scratch("sandbox");
        let (root, data) = (d.join("oBot"), d.join("data"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let home = dirs::home_dir().unwrap().canonicalize().unwrap();
        let profile = sandbox_profile(&home, &[root.clone()], &tool_dirs(&home), &temp_dirs(), &data);
        let run = |cmd: &str| std::process::Command::new("/usr/bin/sandbox-exec").args(["-p", &profile, "/bin/zsh", "-c", cmd]).current_dir(&root).output().unwrap();
        let ok = |cmd: &str| run(cmd).status.success();
        // Handing work to something outside the sandbox: LaunchServices (`open`) and Apple events.
        assert!(!ok("open -g -j -a Calculator"), "open started an app");
        // (A blocked Apple event waits instead of failing: given 5 seconds, it must not get through.)
        assert!(!ok("perl -e 'alarm 5; exec @ARGV' osascript -e 'tell application \"Finder\" to get name'"), "osascript sent an Apple event");
        // Writing outside the folders; reading the home folder, hidden logins, or linking them in.
        assert!(!ok("touch ~/Library/Caches/obot-sandbox-test"));
        for p in [".ssh", ".aws", ".netrc", ".config", ".zsh_history", ".claude.json", "Documents", "Desktop"] {
            if home.join(p).exists() {
                assert!(!ok(&format!("ls -a ~/{p}")), "~/{p} could be read");
            }
        }
        if home.join(".zsh_history").exists() {
            assert!(!ok("ln ~/.zsh_history h"), "a hard link to a closed file was made");
        }
        // Work in the folder, and the user's tools (from wherever they are installed), still go.
        assert!(ok("echo hi > a.txt && cat a.txt && git --version"));
        for tool in ["node -v", "npm -v", "cargo --version", "python3 -c 1", "uv --version"] {
            if which::which(tool.split(' ').next().unwrap()).is_ok() {
                let out = run(tool);
                assert!(out.status.success(), "{tool} failed in the sandbox: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn file_tools_check_the_file_they_opened() {
        use std::os::unix::fs::symlink;
        let d = scratch("open");
        let (home, secret) = (d.join("oBot"), d.join("secret"));
        std::fs::create_dir_all(home.join("B/dir")).unwrap();
        std::fs::create_dir_all(&secret).unwrap();
        std::fs::write(secret.join("key.txt"), "KEY").unwrap();
        let r = roots(&Share { enabled: true, home: home.to_string_lossy().into(), folders: vec![] }).unwrap();
        let path = |p: &str| json!({ "bot": "B", "path": p });
        // Plain files work.
        write_tool(&r, &json!({ "bot": "B", "path": "a.txt", "content": "hello" })).unwrap();
        assert!(read_tool(&r, &path("a.txt")).unwrap().contains("hello"));
        // A link as the last part is refused.
        symlink(secret.join("key.txt"), home.join("B/last")).unwrap();
        assert!(open_inside(&r, &home.join("B/last"), false).is_err());
        // A folder swapped for a link after `resolve` looked: refused, and the file outside is untouched.
        #[cfg(target_os = "macos")]
        {
            let planned = resolve(&r, "B", "dir/key.txt").unwrap();
            std::fs::remove_dir(home.join("B/dir")).unwrap();
            symlink(&secret, home.join("B/dir")).unwrap();
            assert!(open_inside(&r, &planned, false).is_err());
            assert!(open_inside(&r, &planned, true).is_err());
            assert_eq!(std::fs::read_to_string(secret.join("key.txt")).unwrap(), "KEY");
        }
        // A pipe is refused at once instead of hanging.
        assert!(std::process::Command::new("mkfifo").arg(home.join("B/pipe")).status().unwrap().success());
        assert!(read_tool(&r, &path("pipe")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_swapped_for_a_link_is_left_out() {
        let d = scratch("swap");
        let (home, extra, secret) = (d.join("oBot"), d.join("project"), d.join("secret"));
        for p in [&home, &extra, &secret] {
            std::fs::create_dir_all(p).unwrap();
        }
        let share = Share { enabled: true, home: home.to_string_lossy().into(), folders: vec![extra.to_string_lossy().into()] };
        assert_eq!(roots(&share).unwrap().all.len(), 2);
        std::fs::remove_dir(&extra).unwrap();
        std::os::unix::fs::symlink(&secret, &extra).unwrap();
        let r = roots(&share).unwrap();
        assert_eq!(r.all, vec![home]);
        assert!(resolve(&r, "B", &extra.join("x").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_restart_only_stops_the_same_process() {
        let d = scratch("recover");
        let spawn = || {
            let mut c = std::process::Command::new("sleep");
            std::os::unix::process::CommandExt::process_group(c.arg("30"), 0);
            c.spawn().unwrap()
        };
        let (mut same, mut other) = (spawn(), spawn());
        let job = |id: &str, pid: u32, started: Option<u64>| Job {
            id: id.into(), bot_id: "b".into(), name: "n".into(), command: "sleep 30".into(), log: String::new(),
            status: "running".into(), exit_code: None, pid: Some(pid), started, reported: false,
        };
        // `other` stands for a program that got the id of an old job: it started at another time.
        let list = [job("recover-a", same.id(), started_at(same.id())), job("recover-b", other.id(), started_at(other.id()).map(|t| t - 100))];
        std::fs::create_dir_all(jobs_file(&d).parent().unwrap()).unwrap();
        std::fs::write(jobs_file(&d), serde_json::to_vec(&list).unwrap()).unwrap();
        recover_jobs(&d);
        std::thread::sleep(Duration::from_millis(300));
        assert!(same.try_wait().unwrap().is_some(), "the job's own process should be stopped");
        assert!(other.try_wait().unwrap().is_none(), "another program must be left alone");
        assert_eq!(JOBS.lock().unwrap().get("recover-b").unwrap().status, "interrupted");
        let _ = other.kill();
        let _ = std::fs::remove_dir_all(&d);
    }

    fn state(d: &Path) -> Shared {
        crate::state::test_state(d)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn turning_the_switch_off_stops_what_is_running() {
        let d = scratch("off");
        std::fs::create_dir_all(d.join("data")).unwrap();
        let st = state(&d.join("data"));
        update(&st, &json!({ "enabled": true, "home": d.join("oBot") })).await.unwrap();
        let share = st.config.read().await.this_computer.clone();
        let r = roots(&share).unwrap();
        let started = run_command(&st, &r, "off-job", &json!({ "bot": "B", "bot_id": "b1", "command": "sleep 300", "background": true })).await.unwrap();
        let job = started.split("(job ").nth(1).unwrap()[..8].to_string();
        let waiting = {
            let (st, r) = (st.clone(), roots(&share).unwrap());
            tokio::spawn(async move { run_command(&st, &r, "off-fg", &json!({ "bot": "B", "command": "sleep 300" })).await })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        update(&st, &json!({ "enabled": false })).await.unwrap();
        let out = tokio::time::timeout(Duration::from_secs(10), waiting).await.expect("the command kept running").unwrap().unwrap();
        assert!(out.contains("Stopped before it finished"), "{out}");
        for _ in 0..100 {
            if JOBS.lock().unwrap().get(&job).is_some_and(|j| j.status == "stopped") {
                let _ = std::fs::remove_dir_all(&d);
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the background command kept running");
    }

    #[test]
    fn long_output_keeps_its_start_and_end() {
        let mut c = Capture::default();
        c.add(b"start\n");
        c.add(&vec![b'x'; HEAD_BYTES + TAIL_BYTES + 500]);
        c.add(b"\nend");
        let t = c.text();
        assert!(t.starts_with("start\n") && t.ends_with("\nend") && t.contains("bytes of output left out"));
    }
}
