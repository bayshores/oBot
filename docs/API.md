# openbot HTTP / WebSocket API (contract between `ui/` and `openbot-core`)

All JSON. All `/api/*` routes (except `POST /api/pair`) need a token, either as header
`Authorization: Bearer <token>` or query `?token=<token>` (used for `<img src>` and the WebSocket).
Two kinds of token work:
* the **owner token** (`config.token`): used by the desktop app on the same computer, the approval helper,
  and SSH tunnels;
* a **device token** (`obd_…`): given to a paired device by `POST /api/pair`. Only its SHA-256 hash is stored.

Security rules (see `backend/core/src/security.rs`):
* The server listens on 127.0.0.1 by default. Remote devices come in through Tailscale Serve (HTTPS) or an SSH tunnel.
* `Host` must be an IP, `localhost`, a `*.ts.net` name, this computer's name (or `name.local`), or a name in
  `OPENBOT_ALLOWED_HOSTS` (comma list). Otherwise 403 (blocks DNS rebinding).
* `/api/*` requests with an `Origin` header are refused (403) unless the origin is `http://127.0.0.1:*`,
  `http://localhost:*`, `tauri://localhost`, or the same origin as the `Host` the request came in on.
  CORS allows exactly those origins.
* 20 failed tokens / pairing codes per minute from one client → 429 for 5 minutes (`Retry-After` set). Behind
  Tailscale Serve the client is the first `X-Forwarded-For` address (trusted only from loopback).

The UI is served by an openbot server at `/` and is opened as `/?token=<token>`.
The UI stores that token in `sessionStorage` and removes it from the address bar.

## Two kinds of server the UI talks to

* **Home server** (`location.origin`): the server that served the UI. On the desktop app this is the
  embedded local server. It always answers `/api/connection` and `/api/prefs`.
* **Computer server** (`apiBase`): where bots actually run. It's the home server when
  connection.mode == "local", or `connection.url` with `connection.token` when mode == "remote".
  Every other route below goes to the computer server.

Boot sequence in the UI:
1. `GET {home}/api/connection` → `{mode:"local"|"remote", url, token}`; set `apiBase`/`apiToken`.
2. `GET {home}/api/prefs` → free JSON object (theme, accent, spellcheck, userName…). `PUT` saves the whole object.
3. `GET {api}/api/bootstrap` → `{bots:[Bot], settings:Settings, computer:Computer, usage:Usage, version:string}`
4. Open WebSocket `ws(s)://{api host}/api/events?token=…`; reconnect with backoff (1s→10s) and re-bootstrap on reconnect.

## Objects

```ts
Bot = {
  id: string, name: string,
  color: string,          // hex like "#e0458f"
  shape: "blob"|"square"|"star"|"drop"|"flower",
  instructions: string,   // persona / standing instructions
  model: string,          // "" = default, or "opus" | "sonnet" | "haiku"
  hidden: boolean,
  created_at: number, updated_at: number,   // unix ms; updated_at = last activity (sort desc)
  last_preview: string,   // short text of last message for the sidebar
  unread: number,
  running: boolean,       // agent is working right now (show typing dots + stop button)
  activity: null | { kind: string, label: string },  // live status line while running, e.g.
                          // {kind:"searching_web", label:"Searching the web"}. kinds: thinking, searching_web,
                          // reading_web, reading_file, writing_file, running_commands, browsing, on_its_computer,
                          // connecting, waiting_for_you, working, status (bot-set text)
  workdir: string,        // folder on the computer
  label: string,          // short tagline under the name
  pinned: boolean,        // pinned tile above the list
  notifications: boolean, // desktop notifications for this bot
  main: boolean,          // the Main Bot (can't be deleted; star badge)
  conversation_id: string // Claude Code session id ("Copy conversation ID")
}

Message = {
  id: string, bot_id: string,
  role: "user" | "bot" | "tool" | "approval" | "error" | "system"
      | "widget" | "attachment" | "visualization" | "event",
  text: string,           // raw text / markdown
  html: string,           // server-rendered safe HTML for role "bot" (markdown). "" for others.
  status: "streaming" | "done" | "running" | "ok" | "failed" | "pending" | "allowed" | "denied" | "expired",  // expired = approval left open when openbot restarted
  meta: object,           // see below
  created_at: number
}
// meta by role:
//  user:     { attachments: [{ name, path, mime, size, url }] }     url is relative: "/api/bots/{id}/files/{path}" (append ?token=)
//  tool:     { tool_use_id, name, summary, output }            summary e.g. "Ran `ls -la`", output truncated text
//  approval: { approval_id, tool_name, summary, input }        status pending|allowed|denied
//  error / system: {}
//  widget:   { question, options: string[], allow_custom: bool, answer: string|null }   quick-reply buttons;
//            answer with POST /api/messages/:id/answer {value}
//  attachment: { name, path, mime, size, url }   a file the bot sent; text = optional caption
//  visualization: { title, url }   url like "/pages/<id>" (no token needed): show in <iframe sandbox="allow-scripts">
//  event:    { kind: "routine-created"|"routine-run"|"routine-paused", routine_id }   small centered line
// While an approval card is open, bot.activity = {kind:"waiting_for_you", label:"Waiting for your OK"}.
// NOTE: "tool" rows are hidden by default; show bot.activity instead.

Settings = {
  auth_mode: "subscription" | "api_key",
  api_key_set: boolean, api_key_hint: string,      // e.g. "sk-ant-…a1b2"
  permission_mode: "ask" | "auto" | "accept_edits" | "allow_all",
  allow_rules: string[],                             // e.g. ["Bash(git status)", "WebFetch"]
  default_model: string,                             // "" | "opus" | "sonnet" | "haiku"
  workspace_root: string,
  vault: [{ name: string, hint: string, description: string }]  // secrets exposed to bots as env vars
  auto_review: boolean,                              // Settings › Permissions › Auto-Review (default true)
  review_ask: string[],   // "Always Ask" plain-language rules (max 20, 1000 chars each; wins over allow)
  review_allow: string[], // "Allow without Asking" rules
  mcp_servers: [{ name, type: "http"|"stdio", url?, headers?, command?, args?, env? }]  // header/env values masked
  timezone: string,      // IANA name like "America/Los_Angeles"; "" = the computer's timezone. Routine times use it. PUT accepts it.
  runner: "native"|"wsl", // where bots run Claude Code. "wsl" (Windows hosts only) runs it inside WSL with its sandbox on. PUT accepts it.
  wsl_distro: string,    // WSL distro for runner "wsl" (default "Ubuntu"). PUT accepts it.
  wsl_available: bool,   // true when the server runs on Windows (the "wsl" runner can be used)
}
// permission_mode values: "ask" (Ask every time; auto-review may wave routine actions through),
//   "allow_all" (Always allow), "never" (Never allow: anything needing approval is denied), plus legacy "auto", "accept_edits".
// approval meta also has: decision ("allow"|"always"|"deny" once answered), proposed_rule (string shown for "Always allow"; saved into review_allow), reason (why it asked).

Computer = {
  hostname, os, os_version, arch, uptime_secs, cpu_count, mem_total, mem_used,  // numbers in bytes/secs
  is_remote_capable: boolean,     // server is listening on a non-loopback address
  listen: string,                 // e.g. "0.0.0.0:7777"
  data_dir: string,
  claude: { installed: boolean, path: string, version: string,
            logged_in: boolean, email: string, auth_method: string, subscription: string }
}

Usage = {
  rate_limit: null | { utilization: number /*0..1*/, resets_at: number /*unix s*/, window: string /*"seven_day"|"five_hour"*/, status: string },
  week_cost_usd: number, total_cost_usd: number,
  per_bot: [{ bot_id, cost_usd, turns }]
}
```

## Routes

| Method | Path | Body | Returns |
|---|---|---|---|
| GET | /api/connection | – | `{mode,url,token}` (home only) |
| PUT | /api/connection | `{mode,url,token}` | same (home only) |
| POST | /api/connection/test | `{url,token}` | `{ok:bool, error?:string, computer?:Computer}` (home only) |
| POST | /api/connection/pair | `{url, code}` | `{ok, token?, computer?, error?}` (home only): sends the code to `{url}/api/pair` with this computer's name. Then PUT /api/connection `{mode:"remote", url, token}` |
| POST | /api/pair | `{code, device_name}` (**no token**) | `{token, device_id, computer}`; 401 `{error}` for a wrong/expired code ("Wrong code. N tries left.") |
| POST | /api/pair/code | – | `{code:"123456", expires_at}`: new pairing code (10 minutes, single use, 5 tries). Replaces the previous code |
| GET | /api/devices | – | `[{id, name, created_at, last_seen, current}]` (`current` = the device making this request) |
| DELETE | /api/devices/:id | – | `{ok}` revokes that device's token right away |
| GET/PUT | /api/prefs | object | object (home only) |
| GET | /api/bootstrap | – | see above |
| GET | /api/bots | – | `[Bot]` |
| POST | /api/bots | `{name,color,shape,instructions,model}` | `Bot` |
| PATCH | /api/bots/:id | any of `{name,color,shape,instructions,model,hidden}` | `Bot` |
| DELETE | /api/bots/:id | – | `{ok:true}` |
| POST | /api/bots/:id/reset | – | `{ok:true}` (clears chat, starts new Claude conversation) |
| POST | /api/bots/:id/read | – | `{ok:true}` (unread → 0) |
| POST | /api/bots/:id/unread | – | `{ok:true}` ("Mark as Unread": unread → 1) |
| GET | /api/bots/:id/messages?before=<ms>&limit=200 | – | `[Message]` oldest→newest |
| POST | /api/bots/:id/messages | `{text, attachments:[{name,path,mime,url}]}` | `Message` (the user message) |
| POST | /api/bots/:id/stop | – | `{ok:true}` |
| POST | /api/bots/:id/upload | multipart, field `file` (one or more) | `[{name,path,mime,url}]` |
| GET | /api/bots/:id/files/*path | – | file bytes |
| POST | /api/approvals/:approval_id | `{decision:"allow"|"always"|"deny"}` | `{ok:true}` |
| GET | /api/settings | – | `Settings` |
| PUT | /api/settings | partial: `{auth_mode, api_key, permission_mode, allow_rules, default_model, workspace_root}` (`api_key:""` clears it) | `Settings` |
| POST | /api/vault | `{name,value}` | `Settings` |
| DELETE | /api/vault/:name | – | `Settings` |
| GET | /api/computer | `?refresh=1` re-checks Claude Code now | `Computer` |
| GET | /api/hello | – (no token) | `{app:"openbot", version, name}`: lets setup recognize an openbot computer |
| GET | /api/tailscale | – | home only: `{installed, running, needs_login, state, self:{name,dns,os,online}, peers:[{name,dns,os,online}]}` |
| POST | /api/discover | – | home only, ~4 s: `{tailscale, found:[{name, dns, os, url, version, computer_name}]}` your Tailscale devices that run openbot |
| GET | /api/usage | – | `Usage` |
| GET | /api/search?q= | – | `{bots:[Bot], messages:[Message]}` (max 50) |
| POST | /api/messages/:id/answer | `{value}` | `Message` (the user's answer, sent as a normal user message) |
| GET | /api/bots/:id/routines | – | `[Routine]` |
| PATCH | /api/routines/:id | any of `{name, prompt, schedule, enabled}` | `Routine` |
| DELETE | /api/routines/:id | – | `{ok}` |
| POST | /api/routines/:id/run | – | `{ok}` ("Test routine": run once now) |
| GET | /api/bots/:id/memories | – | `[{id, bot_id (null = about the user), text, created_at}]` |
| DELETE | /api/memories/:id | – | `{ok}` |
| GET | /pages/:id | – | HTML made by a bot (sandboxed by CSP). The server appends a script that posts `{type:'openbot:height', height}` to the parent. |
| PUT | /api/settings | also `{auto_review, review_ask, review_allow}` | `Settings` |
| POST | /api/vault | `{name, value, description}` (empty value = keep old value, change description) | `Settings` |
| POST | /api/bots/:id/make-main | – | `{ok}` ("Replace with different Bot") |
| DELETE | /api/bots/:id | – | 400 for the Main Bot |
| PATCH | /api/bots/:id | also `{label, pinned, notifications}` | `Bot` |
| GET | /api/bots/:id/skills | – | `[{name, description, body}]` (Claude Code skills in the bot folder) |
| PUT | /api/bots/:id/skills/:name | `{description, body}` (name: lowercase-dashes) | `{ok}` |
| DELETE | /api/bots/:id/skills/:name | – | `{ok}` |
| GET | /api/bots/:id/library | – | `[{name, path, size, modified, mime, url}]` newest first (Files library) |
| POST | /api/memories/import | `{text}` one fact per line | `{imported}` |
| POST | /api/mcp-servers | `{name, type:"http", url, headers}` or `{name, type:"stdio", command, args[], env}` | `Settings` |
| DELETE | /api/mcp-servers/:name | – | `Settings` |
| POST | /api/claude/update | – | `{ok, output}` runs `claude update` |
| POST | /api/claude/logout | – | `{ok, output}` runs `claude auth logout` (Settings › Sign Out) |
| POST | /api/open-folder | `{path}` | `{ok}` opens Finder/Explorer on the computer (local only) |
| GET (ws) | /api/events?token= | – | stream of events |

Errors: non-2xx with `{error: string}`.

## WebSocket events (server → UI), JSON `{type, ...}`

* `hello {version}`
* `bot.created {bot}` · `bot.updated {bot}` · `bot.deleted {bot_id}`
* `message.created {message}` · `message.updated {message}` (streaming text, tool finished, approval decided) · `messages.cleared {bot_id}`
* `usage.updated {usage}` · `settings.updated {settings}`
* `devices.updated` (a device was paired or removed; re-fetch /api/devices)
* `routines.updated {bot_id}` · `memories.updated {bot_id?}` (re-fetch if that bot's settings are open)
* `resync` (reload everything)

```ts
Routine = { id, bot_id, name, prompt, schedule, schedule_text /* "Every weekday at 08:00" */, enabled,
            next_run_at /* ms|null */, last_run_at, paused_for_unread }
// schedule: {kind:"once",at:ms} | {kind:"interval",minutes} | {kind:"daily"|"weekdays",time:"HH:MM"}
//         | {kind:"weekly",day:"monday",time} | {kind:"monthly",day:1..31,time}
```

## Desktop bridge (only inside the openbot desktop app)

When `window.__TAURI_INTERNALS__` exists, the UI can call:
* `invoke('notify', {title, body})` → OS notification (send when a bot finishes or needs you and the window isn't focused)
* `invoke('set_badge', {count})` → Dock/taskbar number = total unread across bots
* `invoke('is_focused')` → boolean
* `invoke('open_url', {url})` → opens an http(s) link in the default browser
