//! Bot screens: every bot can have its own virtual Linux desktop (1280 x 800) with a browser.
//!
//! - The bot sees and controls it with the computer_* tools (screenshots + xdotool).
//! - The user watches it live in the app and can take control; the app's noVNC viewer talks to
//!   `/api/bots/{id}/screen/ws`, which forwards to the screen's VNC server.
//!
//! Where it runs:
//! - Windows host: inside the WSL distro from Settings (`wsl.exe -d <distro> ...`).
//! - Linux host: directly.
//! - macOS host: not supported (no X server).
//!
//! Inside Linux each screen is: Xvfb :N (virtual display) + openbox (window manager) +
//! x11vnc on 127.0.0.1:(15900+N) with a random password + Chromium with a per-bot profile in
//! ~/.openbot/screens/<bot>/profile (so logins persist). On Windows, WSL forwards 127.0.0.1 ports
//! to Windows, so the server reaches the VNC port the same way on both systems.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::state::Shared;
use crate::util::{hide_console, now_ms};

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 800;
const FIRST_DISPLAY: u16 = 101;
/// Screens nobody watches or uses are stopped after this long (the browser profile is kept).
const IDLE_STOP_MS: i64 = 30 * 60 * 1000;

/// Linux programs a screen needs (checked by `status`, installed by `setup`).
const APT_PACKAGES: &str = "xvfb x11vnc xdotool imagemagick openbox x11-xserver-utils fonts-liberation fonts-dejavu-core fonts-noto-color-emoji";
const NEEDED: &[&str] = &["Xvfb", "x11vnc", "xdotool", "import", "openbox", "xsetroot"];

/// Finds the browser inside Linux: Playwright's Chromium, then Google Chrome, then a non-snap Chromium.
const FIND_BROWSER: &str = r#"B=$(ls -d "$HOME"/.cache/ms-playwright/chromium-*/chrome-linux*/chrome 2>/dev/null | sort -V | tail -1)
[ -z "$B" ] && B=$(command -v google-chrome || command -v google-chrome-stable || true)
[ -z "$B" ] && [ -x /usr/lib/chromium/chromium ] && B=/usr/lib/chromium/chromium
OBDIR="$HOME/.openbot/screens/$OB_BOT"; OBPROF="$OBDIR/profile"
# Close the bot's browser cleanly (so it doesn't ask to restore pages next time).
ob_browser_close() {
  pkill -TERM -f -- "--user-data-dir=$OBPROF" 2>/dev/null || return 0
  for i in $(seq 30); do pgrep -f -- "--user-data-dir=$OBPROF" >/dev/null || return 0; sleep 0.1; done
  pkill -KILL -f -- "--user-data-dir=$OBPROF" 2>/dev/null; sleep 0.3
}
# Stop a screen's process group and wait until it is really gone (its exit trap closes
# the browser, which must finish before a new browser starts on the same profile).
ob_killgroup() {
  kill -TERM -- -"$1" 2>/dev/null || return 0
  for i in $(seq 60); do kill -0 -- -"$1" 2>/dev/null || return 0; sleep 0.1; done
  kill -KILL -- -"$1" 2>/dev/null; sleep 0.2
}
# Turn on "Continue where you left off". Without it Chrome deletes session cookies (which most
# school/work sign-ins use) every time it starts, so the bot was logged out after each restart.
ob_keep_logins() {
  local pf="$OBPROF/Default/Preferences"; mkdir -p "$OBPROF/Default"
  [ -s "$pf" ] || { echo '{"session":{"restore_on_startup":1}}' >"$pf"; return 0; }
  command -v python3 >/dev/null || return 0
  python3 - "$pf" <<'PY' 2>/dev/null || true
import json, sys
p = sys.argv[1]
d = json.load(open(p))
if d.get("session", {}).get("restore_on_startup") != 1:
    d.setdefault("session", {})["restore_on_startup"] = 1
    json.dump(d, open(p, "w"), separators=(",", ":"))
PY
}
# Open a URL in the bot's browser (starts it if needed; otherwise a new tab).
ob_browser() {
  [ -n "$B" ] || { echo "No browser is installed for the screen" >&2; return 1; }
  if ! pgrep -f -- "--user-data-dir=$OBPROF" >/dev/null; then
    rm -f "$OBPROF"/Singleton* 2>/dev/null
    [ -f "$OBPROF/Default/Preferences" ] && sed -i 's/"exit_type":"Crashed"/"exit_type":"Normal"/; s/"exited_cleanly":false/"exited_cleanly":true/' "$OBPROF/Default/Preferences"
    ob_keep_logins
  fi
  setsid -f "$B" --user-data-dir="$OBPROF" --no-first-run --no-default-browser-check --disable-dev-shm-usage \
    --hide-crash-restore-bubble --password-store=basic --disable-features=Translate \
    --window-position=0,0 --window-size=1280,800 --start-maximized "$1" >>"$OBDIR/browser.log" 2>&1
}
"#;

/// Starts one screen and keeps running until it is stopped. Prints READY when usable.
const START_SCRIPT: &str = r#"set -u
N="$OB_DISPLAY"; P="$OB_PORT"; DIR="$HOME/.openbot/screens/$OB_BOT"
mkdir -p "$DIR/profile"; chmod 700 "$HOME/.openbot" "$HOME/.openbot/screens" "$DIR" 2>/dev/null
PGF="/tmp/openbot-screen-$N.pgid"
# A screen left over from an earlier run of openbot: stop it first.
__FIND_BROWSER__
[ -f "$PGF" ] && ob_killgroup "$(cat "$PGF")"
ob_browser_close
rm -f "/tmp/.X$N-lock" "/tmp/.X11-unix/X$N"
echo $$ > "$PGF"
trap 'trap - EXIT INT TERM HUP; ob_browser_close; kill -- -$$ 2>/dev/null; exit 0' EXIT INT TERM HUP
export DISPLAY=":$N"
unset WAYLAND_DISPLAY
Xvfb ":$N" -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
ok=0; for i in $(seq 100); do xdotool getdisplaygeometry >/dev/null 2>&1 && { ok=1; break; }; sleep 0.1; done
[ $ok = 1 ] || { echo "ERROR the virtual display didn't start"; exit 1; }
xsetroot -solid '#151517'
openbox >/dev/null 2>&1 &
x11vnc -storepasswd "$OB_PW" "$DIR/vncpass" >/dev/null 2>&1
x11vnc -display ":$N" -rfbport "$P" -localhost -rfbauth "$DIR/vncpass" -forever -shared -noxdamage -quiet >/dev/null 2>&1 &
ok=0; for i in $(seq 100); do (exec 3<>"/dev/tcp/127.0.0.1/$P") 2>/dev/null && { ok=1; break; }; sleep 0.1; done
[ $ok = 1 ] || { echo "ERROR the screen sharing server didn't start"; exit 1; }
ob_browser about:blank || true
echo READY
wait
"#;

struct Screen {
    display: u16,
    password: String,
    child: Child,
    last_used: i64,
    viewers: usize,
}

#[derive(Default)]
pub struct Screens {
    inner: Mutex<HashMap<String, Screen>>,
    /// Set once screens left behind by an earlier run of openbot have been stopped.
    swept: std::sync::atomic::AtomicBool,
}

/// Stops every screen and bot browser left running by an earlier run of openbot
/// (for example after the server was killed or the computer slept).
const SWEEP_SCRIPT: &str = r#"for f in /tmp/openbot-screen-*.pgid; do [ -f "$f" ] && ob_killgroup "$(cat "$f")"; rm -f "$f"; done
pkill -TERM -f -- "--user-data-dir=$HOME/.openbot/screens/" 2>/dev/null; sleep 0.5; true"#;

/// What the app needs to connect a viewer.
pub struct Info {
    pub display: u16,
    pub port: u16,
    pub password: String,
}

fn port_of(display: u16) -> u16 {
    15900 + display
}

/// Can this computer run bot screens at all? Err = a sentence for the user.
pub fn platform_ok() -> Result<(), String> {
    if cfg!(windows) || cfg!(target_os = "linux") {
        Ok(())
    } else {
        Err("Bot screens need a Windows (with WSL) or Linux computer. Connect openbot to one in Settings › Computer.".into())
    }
}

/// A command that runs `bash -c <script>` inside Linux, with `env` passed to it.
fn linux(cfg: &Config, script: &str, env: &[(&str, String)], root: bool) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("wsl.exe");
        c.args(["-d", &cfg.wsl_distro]);
        if root {
            c.args(["-u", "root"]);
        }
        c.args(["--exec", "setsid", "-w", "bash", "-c", script]);
        // Windows -> WSL only passes variables listed in WSLENV.
        let names: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
        if !names.is_empty() {
            let mut list = std::env::var("WSLENV").unwrap_or_default();
            for n in names {
                if !list.is_empty() {
                    list.push(':');
                }
                list.push_str(n);
            }
            c.env("WSLENV", list);
        }
        c
    } else {
        let mut c = Command::new("setsid");
        c.args(["-w", "bash", "-c", script]);
        c
    };
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    hide_console(&mut cmd);
    cmd
}

/// Run a short Linux command and return its output (stdout bytes, or an error sentence).
async fn run(cfg: &Config, script: &str, env: &[(&str, String)], secs: u64) -> Result<Vec<u8>, String> {
    let out = tokio::time::timeout(Duration::from_secs(secs), linux(cfg, script, env, false).output())
        .await
        .map_err(|_| "The screen didn't respond in time".to_string())?
        .map_err(|e| format!("Couldn't reach Linux ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "The screen command failed".into() } else { crate::util::truncate_chars(&err, 300) });
    }
    Ok(out.stdout)
}

/// Is everything installed? `{supported, reason, ready, missing[], browser}`
pub async fn status(cfg: &Config) -> Value {
    if let Err(reason) = platform_ok() {
        return json!({"supported": false, "reason": reason, "ready": false, "missing": []});
    }
    let script = format!(
        "{FIND_BROWSER}for t in {}; do command -v $t >/dev/null || echo \"missing $t\"; done; [ -n \"$B\" ] && echo \"browser $B\" || echo 'missing browser'",
        NEEDED.join(" ")
    );
    match run(cfg, &script, &[], 30).await {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out);
            let missing: Vec<String> = text.lines().filter_map(|l| l.strip_prefix("missing ").map(str::to_string)).collect();
            let browser = text.lines().find_map(|l| l.strip_prefix("browser ")).unwrap_or("").to_string();
            json!({"supported": true, "ready": missing.is_empty(), "missing": missing, "browser": browser,
                   "where": if cfg!(windows) { format!("WSL ({})", cfg.wsl_distro) } else { "Linux".into() }})
        }
        Err(e) => json!({"supported": true, "ready": false, "missing": ["linux"],
                         "reason": if cfg!(windows) { format!("Couldn't start WSL ({}): {e}. Install it with `wsl --install` in an admin PowerShell, or pick the right distro in Settings.", cfg.wsl_distro) } else { e }}),
    }
}

/// Install what screens need. Windows: runs apt as root inside WSL (no password needed).
/// Returns a short log, or an error sentence.
pub async fn setup(cfg: &Config) -> Result<String, String> {
    platform_ok()?;
    if !cfg!(windows) {
        return Err(format!(
            "Run this once in a terminal on this computer, then try again:\nsudo apt-get install -y {APT_PACKAGES} && npx -y playwright install --with-deps chromium"
        ));
    }
    let apt = format!("export DEBIAN_FRONTEND=noninteractive; apt-get update -qq && apt-get install -y -qq --no-install-recommends {APT_PACKAGES} >/dev/null && echo 'Installed screen programs'");
    let out = tokio::time::timeout(Duration::from_secs(1200), linux(cfg, &apt, &[], true).output())
        .await
        .map_err(|_| "Installing took too long".to_string())?
        .map_err(|e| format!("Couldn't start WSL: {e}"))?;
    if !out.status.success() {
        return Err(format!("Installing the screen programs failed: {}", crate::util::truncate_chars(String::from_utf8_lossy(&out.stderr).trim(), 400)));
    }
    let mut log = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // The browser: Playwright's Chromium (needs Node in Linux), else Google Chrome on x86-64.
    let browser = format!(
        r#"{FIND_BROWSER}[ -n "$B" ] && {{ echo "Browser already installed"; exit 0; }}
for f in "$HOME/.nvm/nvm.sh"; do [ -s "$f" ] && . "$f" >/dev/null 2>&1; done
if command -v npx >/dev/null; then npx -y playwright@1.56.1 install chromium >/tmp/openbot-browser.log 2>&1 && echo "Installed Chromium" && exit 0; fi
exit 3"#
    );
    match tokio::time::timeout(Duration::from_secs(1200), linux(cfg, &browser, &[], false).output()).await {
        Ok(Ok(o)) if o.status.success() => log.push_str(&format!("\n{}", String::from_utf8_lossy(&o.stdout).trim())),
        _ => {
            let chrome = "export DEBIAN_FRONTEND=noninteractive; [ \"$(dpkg --print-architecture)\" = amd64 ] || exit 4; cd /tmp && curl -fsSLo chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb && apt-get install -y -qq ./chrome.deb >/dev/null && rm -f chrome.deb && echo 'Installed Google Chrome'";
            let o = tokio::time::timeout(Duration::from_secs(1200), linux(cfg, chrome, &[], true).output())
                .await
                .map_err(|_| "Installing the browser took too long".to_string())?
                .map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err("Couldn't install a browser for the screens. Install Node.js in WSL (then try again) or Google Chrome.".into());
            }
            log.push_str(&format!("\n{}", String::from_utf8_lossy(&o.stdout).trim()));
        }
    }
    // Chromium's own libraries (Playwright knows the list).
    let deps = "for f in /home/*/.nvm/nvm.sh; do [ -s \"$f\" ] && . \"$f\" >/dev/null 2>&1; done; command -v npx >/dev/null && DEBIAN_FRONTEND=noninteractive npx -y playwright@1.56.1 install-deps chromium >/dev/null 2>&1; true";
    let _ = tokio::time::timeout(Duration::from_secs(1200), linux(cfg, deps, &[], true).output()).await;
    Ok(log)
}

fn random_password() -> String {
    // VNC passwords are at most 8 characters.
    crate::util::random_token(16).chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect()
}

impl Screens {
    /// Start the bot's screen if it isn't running. Returns how to reach it.
    pub async fn ensure(&self, state: &Shared, bot_id: &str) -> Result<Info, String> {
        platform_ok()?;
        let cfg = state.config.read().await.clone();
        let mut map = self.inner.lock().await;
        if !self.swept.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let _ = run(&cfg, &format!("{FIND_BROWSER}{SWEEP_SCRIPT}"), &[("OB_BOT", String::new())], 30).await;
        }
        if let Some(s) = map.get_mut(bot_id) {
            if matches!(s.child.try_wait(), Ok(None)) {
                s.last_used = now_ms();
                return Ok(Info { display: s.display, port: port_of(s.display), password: s.password.clone() });
            }
            map.remove(bot_id);
        }
        let used: Vec<u16> = map.values().map(|s| s.display).collect();
        let display = (FIRST_DISPLAY..FIRST_DISPLAY + 400).find(|d| !used.contains(d)).ok_or("Too many screens are open")?;
        let password = random_password();
        let script = START_SCRIPT.replace("__FIND_BROWSER__", FIND_BROWSER);
        let env = [
            ("OB_DISPLAY", display.to_string()),
            ("OB_PORT", port_of(display).to_string()),
            ("OB_PW", password.clone()),
            ("OB_BOT", bot_id.to_string()),
        ];
        let mut child = linux(&cfg, &script, &env, false).spawn().map_err(|e| format!("Couldn't start the screen: {e}"))?;
        let stdout = child.stdout.take().ok_or("no output from the screen")?;
        let mut lines = BufReader::new(stdout).lines();
        let ready = tokio::time::timeout(Duration::from_secs(40), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim() == "READY" {
                    return Ok(());
                }
                if let Some(e) = line.strip_prefix("ERROR ") {
                    return Err(e.to_string());
                }
            }
            Err("the screen stopped while starting".to_string())
        })
        .await;
        match ready {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = child.kill().await;
                let st = status(&cfg).await;
                if st["ready"] == json!(false) {
                    return Err("Bot screens aren't set up on this computer yet. Open Settings › Computer › Bot screens and choose Set up.".into());
                }
                return Err(format!("The screen couldn't start: {e}"));
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err("The screen took too long to start".into());
            }
        }
        // Keep draining output so the pipe never fills.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        map.insert(bot_id.to_string(), Screen { display, password: password.clone(), child, last_used: now_ms(), viewers: 0 });
        drop(map);
        state.emit("screen.updated", json!({"bot_id": bot_id, "running": true}));
        Ok(Info { display, port: port_of(display), password })
    }

    /// Bots whose screen is running now.
    pub async fn running_ids(&self) -> Vec<String> {
        let mut map = self.inner.lock().await;
        map.retain(|_, s| matches!(s.child.try_wait(), Ok(None)));
        map.keys().cloned().collect()
    }

    /// Stop the bot's screen (its browser profile stays).
    pub async fn stop(&self, state: &Shared, bot_id: &str) {
        let removed = self.inner.lock().await.remove(bot_id);
        if let Some(mut s) = removed {
            let cfg = state.config.read().await.clone();
            let script = format!("{FIND_BROWSER}PGF=/tmp/openbot-screen-$OB_DISPLAY.pgid; [ -f \"$PGF\" ] && ob_killgroup \"$(cat \"$PGF\")\"; ob_browser_close; rm -f \"$PGF\"; true");
            let _ = run(&cfg, &script, &[("OB_DISPLAY", s.display.to_string()), ("OB_BOT", bot_id.to_string())], 20).await;
            let _ = s.child.kill().await;
            state.emit("screen.updated", json!({"bot_id": bot_id, "running": false}));
        }
    }

    pub async fn is_running(&self, bot_id: &str) -> bool {
        let mut map = self.inner.lock().await;
        match map.get_mut(bot_id) {
            Some(s) => matches!(s.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    async fn touch(&self, bot_id: &str, viewers: isize) {
        if let Some(s) = self.inner.lock().await.get_mut(bot_id) {
            s.last_used = now_ms();
            s.viewers = (s.viewers as isize + viewers).max(0) as usize;
        }
    }

    pub async fn viewer_joined(&self, bot_id: &str) {
        self.touch(bot_id, 1).await;
    }
    pub async fn viewer_left(&self, bot_id: &str) {
        self.touch(bot_id, -1).await;
    }

    /// Stop screens nobody has used for a while (checked every minute).
    pub fn spawn_reaper(state: Shared) {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let idle: Vec<String> = {
                    let map = state.screens.inner.lock().await;
                    map.iter()
                        .filter(|(_, s)| s.viewers == 0 && now_ms() - s.last_used > IDLE_STOP_MS)
                        .map(|(id, _)| id.clone())
                        .collect()
                };
                for id in idle {
                    if state.runs.lock().await.contains_key(&id) {
                        continue;
                    }
                    state.screens.stop(&state, &id).await;
                }
            }
        });
    }
}

// ---------------------------------------------------------------- computer tools

/// A tool result for the bot: text plus an optional screenshot.
pub struct ToolOut {
    pub text: String,
    pub image: Option<(String, String)>, // (base64, mime)
}

async fn screenshot(cfg: &Config, display: u16) -> Result<(String, String), String> {
    let bytes = run(cfg, "export DISPLAY=\":$OB_DISPLAY\"; import -silent -window root -quality 75 jpg:-", &[("OB_DISPLAY", display.to_string())], 20).await?;
    if bytes.len() < 100 {
        return Err("The screenshot came back empty".into());
    }
    Ok((base64::engine::general_purpose::STANDARD.encode(bytes), "image/jpeg".into()))
}

fn num(args: &Value, k: &str) -> Result<i64, String> {
    args.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))).ok_or(format!("{k} is required"))
}

fn xy(args: &Value, kx: &str, ky: &str) -> Result<(i64, i64), String> {
    let (x, y) = (num(args, kx)?, num(args, ky)?);
    if !(0..WIDTH as i64).contains(&x) || !(0..HEIGHT as i64).contains(&y) {
        return Err(format!("({x}, {y}) is off the screen; it is {WIDTH} x {HEIGHT}"));
    }
    Ok((x, y))
}

/// xdotool key names: letters/digits/punctuation, F1-F24, and names like Return, Tab, Escape,
/// BackSpace, Delete, Home, End, Page_Up, Page_Down, Up, Down, Left, Right, joined with + for combos.
fn valid_keys(keys: &str) -> bool {
    !keys.trim().is_empty()
        && keys.len() <= 200
        && keys.split_whitespace().all(|combo| combo.split('+').all(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')))
}

/// Run one computer_* tool for a bot. The screen is started on first use.
pub async fn computer_tool(state: &Shared, bot_id: &str, name: &str, args: &Value) -> Result<ToolOut, String> {
    let info = state.screens.ensure(state, bot_id).await?;
    let cfg = state.config.read().await.clone();
    let d = ("OB_DISPLAY", info.display.to_string());
    let act = |script: &'static str, extra: Vec<(&'static str, String)>| {
        let mut env = vec![d.clone()];
        env.extend(extra);
        let cfg = cfg.clone();
        async move { run(&cfg, script, &env, 30).await.map(|_| ()) }
    };
    let done = match name {
        "computer_screenshot" => {
            let img = screenshot(&cfg, info.display).await?;
            return Ok(ToolOut { text: format!("Your screen ({WIDTH} x {HEIGHT})."), image: Some(img) });
        }
        "computer_click" => {
            let (x, y) = xy(args, "x", "y")?;
            let button = match args.get("button").and_then(Value::as_str).unwrap_or("left") {
                "right" => "3",
                "middle" => "2",
                _ => "1",
            };
            let count = args.get("count").and_then(Value::as_i64).unwrap_or(1).clamp(1, 3);
            act(
                "export DISPLAY=\":$OB_DISPLAY\"; xdotool mousemove --sync \"$OB_X\" \"$OB_Y\" click --repeat \"$OB_N\" --delay 80 \"$OB_B\"",
                vec![("OB_X", x.to_string()), ("OB_Y", y.to_string()), ("OB_N", count.to_string()), ("OB_B", button.to_string())],
            )
            .await?;
            format!("Clicked at ({x}, {y}).")
        }
        "computer_move" => {
            let (x, y) = xy(args, "x", "y")?;
            act("export DISPLAY=\":$OB_DISPLAY\"; xdotool mousemove --sync \"$OB_X\" \"$OB_Y\"", vec![("OB_X", x.to_string()), ("OB_Y", y.to_string())]).await?;
            format!("Moved the pointer to ({x}, {y}).")
        }
        "computer_drag" => {
            let (x1, y1) = xy(args, "from_x", "from_y")?;
            let (x2, y2) = xy(args, "to_x", "to_y")?;
            act(
                "export DISPLAY=\":$OB_DISPLAY\"; xdotool mousemove --sync \"$OB_X\" \"$OB_Y\" mousedown 1 sleep 0.1 mousemove --sync \"$OB_X2\" \"$OB_Y2\" sleep 0.1 mouseup 1",
                vec![("OB_X", x1.to_string()), ("OB_Y", y1.to_string()), ("OB_X2", x2.to_string()), ("OB_Y2", y2.to_string())],
            )
            .await?;
            format!("Dragged from ({x1}, {y1}) to ({x2}, {y2}).")
        }
        "computer_scroll" => {
            let (x, y) = xy(args, "x", "y")?;
            let dir = args.get("direction").and_then(Value::as_str).unwrap_or("down");
            let button = match dir {
                "up" => "4",
                "down" => "5",
                "left" => "6",
                "right" => "7",
                _ => return Err("direction must be up, down, left or right".into()),
            };
            let amount = args.get("amount").and_then(Value::as_i64).unwrap_or(5).clamp(1, 30);
            act(
                "export DISPLAY=\":$OB_DISPLAY\"; xdotool mousemove --sync \"$OB_X\" \"$OB_Y\" click --repeat \"$OB_N\" --delay 30 \"$OB_B\"",
                vec![("OB_X", x.to_string()), ("OB_Y", y.to_string()), ("OB_N", amount.to_string()), ("OB_B", button.to_string())],
            )
            .await?;
            format!("Scrolled {dir} {amount} steps at ({x}, {y}).")
        }
        "computer_type" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                return Err("text is required".into());
            }
            if text.chars().count() > 5000 {
                return Err("That's too much text to type at once (5000 characters max)".into());
            }
            act("export DISPLAY=\":$OB_DISPLAY\"; xdotool type --delay 12 -- \"$OB_TEXT\"", vec![("OB_TEXT", text.to_string())]).await?;
            format!("Typed {} characters.", text.chars().count())
        }
        "computer_key" => {
            let keys = args.get("keys").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if !valid_keys(&keys) {
                return Err("keys must be xdotool key names like Return, ctrl+l, ctrl+shift+t, Page_Down (several separated by spaces)".into());
            }
            act("export DISPLAY=\":$OB_DISPLAY\"; xdotool key --delay 60 -- $OB_KEYS", vec![("OB_KEYS", keys.clone())]).await?;
            format!("Pressed {keys}.")
        }
        "computer_open" => {
            let url = args.get("url").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if !(url.starts_with("https://") || url.starts_with("http://") || url == "about:blank") || url.len() > 4000 {
                return Err("url must start with https:// or http://".into());
            }
            let script = format!("export DISPLAY=\":$OB_DISPLAY\"; {FIND_BROWSER}ob_browser \"$OB_URL\" && sleep 2");
            run(&cfg, &script, &[d.clone(), ("OB_BOT", bot_id.to_string()), ("OB_URL", url.clone())], 30).await?;
            format!("Opened {url} in the browser on your screen.")
        }
        "computer_wait" => {
            let secs = args.get("seconds").and_then(Value::as_f64).unwrap_or(2.0).clamp(0.5, 30.0);
            tokio::time::sleep(Duration::from_millis((secs * 1000.0) as u64)).await;
            format!("Waited {secs} seconds.")
        }
        other => return Err(format!("Unknown screen tool {other}")),
    };
    state.screens.touch(bot_id, 0).await;
    // After every action, show the bot what its screen looks like now.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let image = screenshot(&cfg, info.display).await.ok();
    Ok(ToolOut { text: format!("{done} This is your screen now."), image })
}

/// MCP definitions for the screen tools.
pub fn tool_definitions() -> Vec<Value> {
    let xy = |desc: &str| json!({"type": "integer", "description": desc});
    vec![
        json!({"name": "computer_screenshot", "description": "See your own computer screen (a 1280 x 800 Linux desktop with a Chromium browser). The user can watch it live and take over. Coordinates in the other computer_* tools are pixels on this screenshot, (0,0) = top left.", "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "computer_open", "description": "Open a web page in the browser on your screen (new tab if the browser is already open). Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}}),
        json!({"name": "computer_click", "description": "Click on your screen at (x, y). Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"x": xy("pixels from the left"), "y": xy("pixels from the top"), "button": {"type": "string", "enum": ["left", "right", "middle"]}, "count": {"type": "integer", "description": "2 = double click"}}, "required": ["x", "y"]}}),
        json!({"name": "computer_type", "description": "Type text on your screen (into whatever has focus; click a field first). Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}}),
        json!({"name": "computer_key", "description": "Press keys on your screen, e.g. 'Return', 'Tab', 'ctrl+l', 'ctrl+t', 'Page_Down', 'BackSpace'. Several presses: separate with spaces. Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"keys": {"type": "string"}}, "required": ["keys"]}}),
        json!({"name": "computer_scroll", "description": "Scroll at (x, y). Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"x": xy("pixels from the left"), "y": xy("pixels from the top"), "direction": {"type": "string", "enum": ["up", "down", "left", "right"]}, "amount": {"type": "integer", "description": "steps, default 5"}}, "required": ["x", "y", "direction"]}}),
        json!({"name": "computer_move", "description": "Move the pointer to (x, y) without clicking (e.g. to hover a menu). Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"x": xy("pixels from the left"), "y": xy("pixels from the top")}, "required": ["x", "y"]}}),
        json!({"name": "computer_drag", "description": "Drag with the left button from one point to another. Returns a screenshot.", "inputSchema": {"type": "object", "properties": {"from_x": {"type": "integer"}, "from_y": {"type": "integer"}, "to_x": {"type": "integer"}, "to_y": {"type": "integer"}}, "required": ["from_x", "from_y", "to_x", "to_y"]}}),
        json!({"name": "computer_wait", "description": "Wait for a page to load or an animation to finish, then get a screenshot.", "inputSchema": {"type": "object", "properties": {"seconds": {"type": "number"}}}}),
        json!({"name": "request_control", "description": "Ask the user to take over your screen for something only they should do: logging in, a password or 2FA code, a CAPTCHA, a payment, or anything you're stuck on. A card with an 'Open screen' button appears in the chat. Returns right away: end your turn; the user's 'I'm done' arrives as their next message, then take a screenshot and continue.", "inputSchema": {"type": "object", "properties": {"reason": {"type": "string", "description": "What you need them to do, in one short sentence"}}, "required": ["reason"]}}),
    ]
}

/// Forward a WebSocket (from the app's viewer) to the screen's VNC server.
pub async fn proxy(state: Shared, bot_id: String, mut socket: axum::extract::ws::WebSocket, port: u16) {
    use axum::extract::ws::Message as WsMessage;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(tcp) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else {
        let _ = socket.send(WsMessage::Close(None)).await;
        return;
    };
    let _ = tcp.set_nodelay(true);
    state.screens.viewer_joined(&bot_id).await;
    let (mut rd, mut wr) = tcp.into_split();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            n = rd.read(&mut buf) => match n {
                Ok(0) | Err(_) => break,
                Ok(n) => if socket.send(WsMessage::Binary(buf[..n].to_vec().into())).await.is_err() { break; },
            },
            msg = socket.recv() => match msg {
                Some(Ok(WsMessage::Binary(b))) => if wr.write_all(&b).await.is_err() { break; },
                Some(Ok(WsMessage::Text(t))) => if wr.write_all(t.as_bytes()).await.is_err() { break; },
                Some(Ok(WsMessage::Ping(_))) | Some(Ok(WsMessage::Pong(_))) => {}
                _ => break,
            },
        }
    }
    state.screens.viewer_left(&bot_id).await;
}
