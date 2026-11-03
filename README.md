<p align="center"><img src="frontend/assets/icon.png" width="160" alt="openbot icon"></p>

# openbot

Always-on AI bots that chat like friends and actually do things on a computer, powered by **your Claude
subscription** (through Claude Code) or an **Anthropic API key**.

Written in Rust: a small native desktop app (Tauri) plus a fast server, instead of a 344 MB Electron app.

```
 ┌──────────────── your Mac / laptop ───────────────┐            ┌──────── Windows PC (on 24/7) ────────┐
 │  openbot app (window)                            │ Tailscale  │  openbot-server (127.0.0.1 only)       │
 │    └─ built-in local server ◄── "This computer"  │ ─────────► │    └─ runs `claude -p` for each bot    │
 │                                 "Remote computer"│ HTTPS+pair │       in C:\Users\you\OpenBot\<bot>     │
 └──────────────────────────────────────────────────┘            └────────────────────────────────────────┘
```

* **openbot app**: the chat window (sidebar of bots, chat, settings). Works on macOS and Windows.
* **openbot-server**: the "computer" your bots live on. Run it on a PC that stays on, and the bots keep
  working while your laptop is closed.

## What's in this folder

| Path | What it is |
|---|---|
| `frontend/ui/` | The interface (HTML/CSS/JS, no build step). Served by the Rust server. |
| `frontend/desktop/` | The desktop app (Tauri window + built-in local server). |
| `frontend/assets/` | The app icon (source SVG/PNG). |
| `backend/core/` | The engine (`openbot-core`): API server, Claude Code runner, approvals, storage, security. |
| `backend/server/` | The headless always-on server program (`openbot-server`). |
| `backend/install/windows/install-host.ps1` | One-time setup to make a Windows PC an always-on bot computer. |
| `docs/` | Documentation. Start at [`docs/README.md`](docs/README.md). |
| `CLAUDE.md` | Rules for keeping this repo clean (for people and AI assistants). |

## How it uses your Claude subscription

openbot doesn't log in to Claude itself. For each message it runs the official Claude Code program
(`claude -p ...`) on the bot's computer, and Claude Code uses whatever login that computer has.
That's how Claude Code is meant to be scripted, and it's for your own personal use.
**Settings › Claude Account** lets you switch to an Anthropic API key instead (billed per use by Anthropic).

Each bot is one long Claude Code conversation (`--resume`), with its own folder for files.

## Run it on this Mac

1. Install Rust (https://rustup.rs) and Claude Code (https://claude.com/claude-code). Run `claude` once and log in.
2. Install the Tauri build tool (one time, takes a few minutes):
   ```bash
   cargo install tauri-cli --version "^2" --locked
   ```
3. Build the app:
   ```bash
   cd frontend/desktop
   cargo tauri build
   ```
   The app is at `target/release/bundle/macos/openbot.app`. Drag it into Applications.

   To just try it without packaging: `cargo run -p openbot`.

## Make your Windows PC the 24/7 computer

On the **Windows PC**:

0. Install **Tailscale** (free, https://tailscale.com/download) on the PC **and** your Mac, and sign in
   to the same account on both. This is how your Mac reaches the PC privately, from anywhere.
1. Install **Rust**: https://rustup.rs (accept the "Visual Studio C++ Build Tools" step when it asks).
2. Install **Claude Code** in PowerShell: `irm https://claude.ai/install.ps1 | iex`.
   Then run `claude` once and log in with your Claude account.
   (Claude Code on Windows also needs **Git for Windows**: https://git-scm.com/download/win)
3. Copy this whole `openbot` folder to the PC and build the server:
   ```powershell
   cargo build --release -p openbot-server
   ```
4. Open **PowerShell as Administrator** in the folder and run:
   ```powershell
   Set-ExecutionPolicy -Scope Process Bypass
   .\backend\install\windows\install-host.ps1 -KeepAwake
   ```
   This starts openbot-server every time Windows boots (even before you sign in), restarts it if it
   crashes, and stops the PC from sleeping while plugged in. openbot only listens on the PC itself
   (`127.0.0.1:7777`), so nothing on your Wi-Fi or the internet can reach it. The script turns on
   **Tailscale Serve**, which gives openbot a private `https://<pc>.<tailnet>.ts.net` address that
   only your own Tailscale devices can open. At the end it prints that **address** and a
   **6-digit pairing code**.

   (Prefer to try it first? Run `target\release\openbot-server.exe` in a normal window, then
   `tailscale serve --bg 7777` in another one.)

On your **Mac**: open openbot › Settings › **Computer** › **Remote computer**, enter the address and the
pairing code, then click **Connect**. Your bots now live on the Windows PC.

### How it stays safe

* **Private by default.** The server listens on `127.0.0.1` only. Starting it on the network
  (`--host 0.0.0.0`) needs an extra `--allow-network` flag and prints a warning.
* **Encrypted.** Tailscale Serve gives real HTTPS inside your tailnet. Never use `tailscale funnel`
  (that would put openbot on the public internet) and never forward port 7777 on your router.
* **Each device pairs once.** A pairing code works once, for 10 minutes, and dies after 5 wrong
  tries. Each device then gets its own key; the PC only stores a fingerprint (hash) of it. You can see
  and remove devices in Settings › Computer › **Paired devices**, or with `openbot-server devices` / `unpair`.
* **Web pages can't sneak in.** Requests from other websites (by their Origin) and unknown host names
  are refused, so a malicious page in your browser can't talk to openbot.
* **Guessing is blocked.** 20 wrong tokens or codes in a minute from one address locks it out for 5 minutes.
* Without Tailscale you can use an SSH tunnel instead: `ssh -N -L 7777:127.0.0.1:7777 you@pc`, then
  connect the app to `http://127.0.0.1:7777` with **Use a token instead** and the token from
  `openbot-server token` on the PC.

### Useful commands on the PC

```powershell
openbot-server pair         # make a new 6-digit pairing code (for a new device)
openbot-server devices      # list paired devices
openbot-server unpair <id>  # remove a paired device
openbot-server token        # show the owner token (only for this PC or an SSH tunnel)
openbot-server new-token    # make a new owner token
Get-Content "$env:LOCALAPPDATA\openbot\openbot-server.log" -Tail 50   # see the log
.\backend\install\windows\install-host.ps1 -Uninstall                         # remove the auto-start
```

Data lives in `%APPDATA%\openbot` (bots, chats, settings). Bot folders are in `%USERPROFILE%\OpenBot`.

## Permissions (Settings › Permissions)

| Mode | What happens |
|---|---|
| Ask every time (default) | The bot shows an **Allow / Always allow / Deny** card in the chat before doing anything risky. |
| Let Claude decide | Claude Code's safety check approves routine actions and only asks about risky ones. |
| Auto-approve file edits | File changes go through; commands still ask. |
| Allow everything | Nothing asks. Only use this on a computer you're fine with the bot fully controlling. |

"Always allow" remembers that exact action. You can see and remove the list under Permissions.

## Building Windows/Mac installers automatically

Push this folder to a GitHub repository. `.github/workflows/build.yml` builds `openbot-server.exe`,
the Windows installer and the Mac `.dmg`. Download them from the repository's **Actions** tab.

## Developing the UI

Run the server with `OPENBOT_UI_DIR` pointing at `frontend/ui/` and it reads the UI files from disk on every
reload, so you don't need to rebuild:

```bash
OPENBOT_UI_DIR=$PWD/frontend/ui cargo run -p openbot-server -- --local-only
```

---

openbot is an independent project. It is not affiliated with or endorsed by Anthropic.
