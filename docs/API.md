# oBot HTTP / WebSocket API (contract between `ui/` and `obot-core`)

All JSON. All `/api/*` routes (except `POST /api/pair`) need a token in the header
`Authorization: Bearer <token>`. A token is never accepted in the address (it could end up in logs):
* WebSockets (browsers can't set that header) offer it as a subprotocol: `["obot", "obot.auth.<token>"]`
  for `/api/events` (the server answers `oBot`), `["binary", "obot.auth.<token>"]` for a Bot screen.
* Bot files in `<img src>` and download links use `?fk=<file_key>` instead: a key from `/api/bootstrap`
  that opens only `GET /api/bots/:id/files/*`. It's made from the credential's hash, so it stops working
  when that device is removed.
Two kinds of token work:
* the **owner token** (`config.token`): used by the desktop app on the same computer, the approval helper,
  and SSH tunnels;
* a **device token** (`obd_…`): given to a paired device by `POST /api/pair`. Only its SHA-256 hash is stored.

Security rules (see `backend/core/src/security.rs`):
* The server listens on 127.0.0.1 by default. Remote devices come in through Tailscale Serve (HTTPS) or an SSH tunnel.
* `Host` must be an IP, `localhost`, a `*.ts.net` name, this computer's name (or `name.local`), or a name in
  `OBOT_ALLOWED_HOSTS` (comma list). Otherwise 403 (blocks DNS rebinding).
* `/api/*` requests with an `Origin` header are refused (403) unless the origin is `http://127.0.0.1:*`,
  `http://localhost:*`, `tauri://localhost`, or the same origin as the `Host` the request came in on.
  CORS allows exactly those origins.
* 20 failed tokens / pairing codes per minute from one client → 429 for 5 minutes (`Retry-After` set). Behind
  Tailscale Serve the client is the first `X-Forwarded-For` address (trusted only from loopback).

The UI is served by an oBot server at `/`. The desktop app opens it as `/?token=<token>`; the UI stores
that token in `sessionStorage` and removes it from the address bar. A browser on another device (a phone
over Tailscale) has no token: it shows a pairing screen, trades a 6-digit code for a device token with
`POST /api/pair`, and keeps that in `localStorage`. The pairing link `https://<computer>.ts.net/#pair=<code>`
(the QR code in Settings › Computer › Pair a new device) fills the code in; the part after `#` never
leaves the phone. `#bot=<id>` opens that chat (a tapped phone notification).

**Phones** (home-screen web app): `/manifest.webmanifest`, `/icons/phone-{180,192,512}.png` and the service
worker `/sw.js` are served without a token. The service worker shows Web Push notifications and opens the
chat when one is tapped; it caches nothing.

## Two kinds of server the UI talks to

* **Home server** (`location.origin`): the server that served the UI. On the desktop app this is the
  embedded local server. It always answers `/api/connection` and `/api/prefs`.
* **Computer server** (`apiBase`): where bots actually run. It's the home server when
  connection.mode == "local", or `connection.url` with `connection.token` when mode == "remote".
  Every other route below goes to the computer server.

Boot sequence in the UI:
1. `GET {home}/api/connection` → `{mode:"local"|"remote", url, token}`; set `apiBase`/`apiToken`.
2. `GET {home}/api/prefs` → free JSON object (theme, accent, spellcheck, userName…). `PUT` saves the whole object.
3. `GET {api}/api/bootstrap` → `{bots:[Bot], settings:Settings, computer:Computer, usage:Usage, version:string, file_key:string}`
4. Open WebSocket `ws(s)://{api host}/api/events` (token as a subprotocol, see above); reconnect with backoff
   (1s→10s) and re-bootstrap on reconnect. When the page comes back to the front after 15 s or more (a phone
   app that was in the background), reconnect right away.
5. Tell the server whether someone is using this window: send `{"type":"presence","active":bool}` on the
   socket (active = in front, focused, and touched in the last 2 minutes). Phone notifications wait while
   any window is active.

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
  running: boolean,       // agent is working right now (show typing dots + stop button); false once its reply
                          // is done, while Claude Code finishes up (a message sent then isn't marked queued)
  activity: null | { kind: string, label: string },  // live status line while running, e.g.
                          // {kind:"searching_web", label:"Searching the web"}. kinds: thinking, searching_web,
                          // reading_web, reading_file, writing_file, running_commands, browsing, on_its_computer,
                          // connecting, waiting_for_you, working, status (bot-set text)
  workdir: string,        // folder on the computer
  label: string,          // short tagline under the name
  pinned: boolean,        // pinned tile above the list
  notifications: boolean, // desktop notifications for this bot
  main: boolean,          // the Main Bot (can't be deleted; star badge)
  conversation_id: string,// Claude Code session id ("Copy conversation ID")
  kind: "" | "group",     // "group" = a group chat (below); it's in the same list as the Bots
  members: string[]       // group chats: the member Bot ids, in order (1–6)
}
// A group chat uses the Bot routes for its chat (messages, read, stop, retry, reset, PATCH name/members/pinned/
// notifications, DELETE). When the user writes, each member Bot (or only the ones named with @Name) takes a turn: it
// reads what was said since its last turn there and answers, or replies PASS to stay quiet. A Bot can name another
// member with @Name to give it a turn (at most 6 such turns per user message). Each Bot keeps a separate Claude Code
// conversation per group and works in its own folder; what it posts (replies, tool rows, cards) goes to the group
// with meta.speaker = its id. While it runs: running = true, activity = {kind, label: "Food Bot and School Bot are
// working" (or "<Bot>: <what it's doing>"), members: [ids working]}.

Message = {
  id: string, bot_id: string,
  role: "user" | "bot" | "tool" | "approval" | "error" | "system"
      | "widget" | "form" | "attachment" | "visualization" | "calendar" | "relay" | "note" | "event",
  text: string,           // raw text / markdown
  html: string,           // server-rendered safe HTML for role "bot" (markdown). "" for others.
  status: "done" | "running" | "ok" | "failed" | "stopped" | "declined" | "pending" | "allowed" | "denied" | "expired" | "skipped" | "replaced",
          // expired = approval left open when the turn ended or oBot restarted; skipped = approval closed because the
          // user pressed Stop or sent a new message (a new message while a card waits stops the turn and is read right away); stopped = a tool cut off by Stop or by a new
          // message; declined = a tool that didn't run because the user said no (or moved on) at its card; replaced = a secure form closed because the Bot asked again with a newer one
  meta: object,           // see below
  created_at: number
}
// meta by role:
//  user:     { attachments: [{ name, path, mime, size, url }],    url is relative: "/api/bots/{id}/files/{path}" (append ?fk=<file_key>)
//              queued?: bool,     sent while the bot was busy; waiting ("Send now" = POST /api/bots/:id/steer)
//              not_sent?: bool }  the user pressed Stop (or oBot restarted) before the bot read it
//  tool:     { tool_use_id, name, summary, output }            summary e.g. "Ran `ls -la`", output truncated text
//  approval: { approval_id, tool_name, summary, input }        status pending|allowed|denied
//  error:    { code, actions[] }  code: "usage_limit"|"rate_limit"|"overloaded"|"too_long"|"auth"|"setup"|"crashed"|"other"
//            actions (buttons, shown on the newest error only): "retry" (POST /api/bots/:id/retry),
//            "new_conversation" (POST /api/bots/:id/new-conversation), "edit" (only for the user's own turns; POST /api/bots/:id/rollback,
//            then put the returned text/attachments back in the message box). rate_limit / overloaded /
//            crashed are retried automatically 3 times (after 5, 20 and 60 s; bot.activity kind "retrying")
//            before the card appears.
//  system:   {}
//  widget:   { question, options: string[], allow_custom: bool, answer: string|null }   quick-reply buttons;
//            answer with POST /api/messages/:id/answer {value}
//  form:     { title, reason, host, fields: [{id, label, type: "text"|"email"|"tel"|"password"|"code", target}],
//              save_available: bool, filled: string[], missing: string[], saved: "added"|"updated"|string|null }
//            a secure form (request_form). status pending|filled|dismissed|replaced (a newer form from the Bot closes older open ones). Fill it with
//            POST /api/messages/:id/form; the values go into the page on the Bot's screen and are never saved here.
//  attachment: { name, path, mime, size, url }   a file the bot sent; text = optional caption
//  visualization: { title, url }   url like "/pages/<id>" (no token needed): show in <iframe sandbox="allow-scripts">
//  note:     {}   a short line the Bot wrote right before a tool call ("Checking the syllabus."); hidden like tool rows,
//            and shown as bot.activity.label while it works. Longer text (lists, paragraphs, questions) stays a "bot" message.
//  speaker:  (any role a Bot posts, in a group chat) the id of the Bot that posted it
//  reactions: (any message) [{ emoji, by: "user" | bot id }]   tapback reactions (the user's, or a Bot's via `react`)
//  reply_to: (user) { id, who ("You" or the Bot's name), bot (null = the user's own), quote }   the message being replied to
//  relay:    { direction: "to"|"from", bot_id, name }   a message one Bot passed to another (message_bot); it is in
//            both chats ("Messaged <name>" / "Message from <name>"); text = the message
//  calendar: an event the Bot added or changed without asking (a calendar card): { action: "create"|"update",
//            account, provider, calendar (name, "" = main), id, title, start, end (unix ms), all_day, first_day,
//            last_day ("2026-11-05"), when, location, organizer, guests: [{email, name, response: "yes"|"no"|"maybe"|"waiting",
//            optional, self, organizer}], repeat (text like "every week on Mon"), video (call link), link (opens the
//            event), description, day_events: [{title, start, end, all_day}] (the user's other events that day) }
//  event:    { kind, routine_id?, job_id?, bot_id? }   small centered line. kind: "routine-created" | "routine-run"
//            | "routine-paused" | "memory-tidy" | "compacted" (older messages summarized) | "conversation-reset"
//            | "retry" | "steered" (stopped by "Send now") | "job-started" | "job-done" | "job-failed" | "job-stopped" | bot-…
// While an approval card is open, bot.activity = {kind:"waiting_for_you", label:"Waiting for your OK"}.
// NOTE: "tool" rows are hidden by default; show bot.activity instead.

Settings = {
  auth_mode: "subscription" | "api_key",
  api_key_set: boolean, api_key_hint: string,      // e.g. "••••a1b2" (only the last 4 characters)
  api_key_locked: boolean,                           // saved but can't be decrypted on this computer
  permission_mode: "ask" | "auto" | "accept_edits" | "allow_all",
  allow_rules: string[],                             // e.g. ["Bash(git status)", "WebFetch"]
  default_model: string,                             // "" = model_default, or a model name / alias ("claude-opus-5-5", "opus"...)
  model_default: string,                             // the built-in default: "claude-sonnet-5-5"
  effort: "low" | "medium" | "high" | "xhigh" | "max", // Claude Code --effort for every run (default "xhigh")
  workspace_root: string,
  vault: [{ name: string, hint: string,              // hint: "••••abcd" (last 4 only; "••••" for values of 8 characters or fewer)
            description: string,
            bots: string[] | null,                   // Bot ids that get it; null = every Bot
            locked: boolean,                         // saved but can't be decrypted here: no Bot gets it until re-entered
            name_problem: string | null }]           // set if the name is now reserved: no Bot gets it
                                                     // secrets are passed to the allowed Bots as env vars
  logins: [{ id, sites: string[], username, hints, bots: string[] | null,
             has_password: boolean, locked: boolean, updated_at }],   // never the password
  contacts: [{ id, full_name, address1, address2, city, region, postal_code, country,
               phones: string[], emails: string[], label, hints, bots, updated_at }],   // first phone/email = primary
  vault_limits: { logins: number, contacts: number },
  vault_protection: string,                          // how values are saved, e.g. "Encrypted with Windows DPAPI"
  vault_reserved: { names: string[], prefixes: string[] }  // refused secret names (compared ignoring case)
  accounts: [{ id, provider: "google"|"microsoft", email, name, services: string,
               bots: string[] | null,                // Bot ids that may use it; null = every Bot
               added_at, needs_reconnect: boolean,
               drive: boolean,                       // Google account allowed Google Drive
               missing: string[] }]                  // features it hasn't allowed yet ("Google Drive", "other calendars
                                                     // and free time"): show "Update permissions" (signs in again)
                                                     // (Settings › Connections; tokens are never sent)
  accounts_ready: { google: boolean, microsoft: boolean }  // can this build connect accounts?
  auto_review: boolean,                              // Settings › Permissions › Auto-Review (default true)
  auto_update: boolean,                              // Settings › Updates › Update automatically (default true)
  review_ask: string[],   // "Ask first" plain-language rules (max 20, 1000 chars each; wins over allow)
  review_allow: string[], // "Auto-approve" rules
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
| PUT | /api/connection | `{mode,url,token}` | same (home only). 403 for a paired device: only the computer's own app changes this |
| POST | /api/connection/test | `{url,token}` | `{ok:bool, error?:string, computer?:Computer}` (home only) |
| POST | /api/connection/pair | `{url, code}` | `{ok, token?, computer?, error?}` (home only, not from a paired device): sends the code to `{url}/api/pair` with this computer's name. Then PUT /api/connection `{mode:"remote", url, token}` |
| POST | /api/pair | `{code, device_name}` (**no token**) | `{token, device_id, computer}`; 401 `{error}` for a wrong/expired code ("Wrong code. N tries left.") |
| POST | /api/pair/code | – | `{code:"123456", expires_at, link, qr}`: new pairing code (10 minutes, single use, 5 tries). Replaces the previous code. When Tailscale runs here, `link` is `https://<this computer>.ts.net/#pair=<code>` and `qr` an SVG QR code of it (else both `""`) |
| GET | /api/push/key | – | `{key}`: this computer's VAPID public key (base64url) for `pushManager.subscribe` |
| POST | /api/push/subscribe | the browser's `PushSubscription` JSON `{endpoint, keys:{p256dh, auth}}` | `{ok}`. Saved with the device that sent it (at most 20); removing the device removes it |
| POST | /api/push/unsubscribe | `{endpoint}` | `{ok}` |
| POST | /api/push/test | `{endpoint}` | `{ok}` after a test notification went out to that one device; 502 `{error}` with the push service's answer |
| GET | /api/devices | – | `[{id, name, created_at, last_seen, current}]` (`current` = the device making this request) |
| DELETE | /api/devices/:id | – | `{ok}` revokes that device's token right away |
| GET/PUT | /api/prefs | object | object (home only) |
| GET | /api/bootstrap | – | see above |
| GET | /api/bots | – | `[Bot]` |
| POST | /api/bots | `{name,color,shape,instructions,model}` | `Bot` |
| PATCH | /api/bots/:id | any of `{name,color,shape,instructions,model,hidden}`; group chats: `{name, members, pinned, notifications}` | `Bot` |
| POST | /api/groups | `{name ("" = the Bots' names), members:[bot ids, 1–6]}` | `Bot` (kind "group"; `bot.created` event) |
| DELETE | /api/bots/:id | – | `{ok:true}` |
| POST | /api/bots/:id/reset | – | `{ok:true}` (clears chat, starts new Claude conversation) |
| POST | /api/bots/:id/read | – | `{ok:true}` (unread → 0) |
| POST | /api/bots/:id/unread | – | `{ok:true}` ("Mark as Unread": unread → 1) |
| GET | /api/bots/:id/messages?before=<ms>&limit=200 | – | `[Message]` oldest→newest |
| POST | /api/bots/:id/messages | `{text, attachments:[{name,path,mime,url}], reply_to?: message id}` | `Message` (the user message; a reply gets `meta.reply_to`, and the Bot is told what it quotes) |
| POST | /api/messages/:id/react | `{emoji, on}` (`on:false` takes the user's reaction back) | `Message` (`meta.reactions`). The Bot that wrote it hears about it with the user's next message |
| POST | /api/bots/:id/stop | – | `{ok:true}` (also drops the user's queued messages: they get `meta.not_sent`; oBot's own queued turns still run) |
| POST | /api/bots/:id/steer | `{message_id}` | `{ok}` "Send now": stops the current turn and runs the queued messages right away. `ok:false` if that message is no longer queued |
| POST | /api/bots/:id/retry | – | `{ok:true}` "Try again": runs the last turn's prompt again (400 if there is none or the bot is busy) |
| POST | /api/bots/:id/new-conversation | – | `{ok:true}` Claude Code starts a new conversation; chat, memory, files, routines stay |
| POST | /api/bots/:id/rollback | – | `{text, attachments}` "Edit and resend": deletes the last user message and everything after it (`messages.deleted`); the next message tells the bot the old one was withdrawn. 400 while the bot is working |
| POST | /api/bots/:id/upload | multipart, field `file` (one or more) | `[{name,path,mime,url}]` |
| GET | /api/bots/:id/files/*path | – | file bytes |
| POST | /api/approvals/:approval_id | `{decision:"allow"|"always"|"deny"}` | `{ok:true}` |
| GET | /api/settings | – | `Settings` |
| PUT | /api/settings | partial: `{auth_mode, api_key, permission_mode, allow_rules, default_model, effort, workspace_root}` (`api_key:""` clears it) | `Settings` |
| POST | /api/vault | `{name, value, description, bots}`, see below | `Settings` |
| POST | /api/accounts/connect | `{provider:"google"\|"microsoft", redirect:"http://127.0.0.1:<port>/oauth/callback", reconnect?: account id}` | `{url}`: the sign-in page to open in the browser |
| POST | /api/accounts/finish | `{state, code}` or `{url}` (the address the browser landed on) | `{ok, email}`; also emits `account.connected` and `settings.updated` |
| PATCH | /api/accounts/:id | `{bots}` (null = every Bot, else a non-empty list of Bot ids) | `Settings` |
| DELETE | /api/accounts/:id | – (Google access is also revoked) | `Settings` |
| POST | /api/github/connect | – | `{user_code, verification_uri, expires_in}`: the code to enter on github.com. The result arrives as `github.connected {login}` or `github.failed {error}` (and `settings.updated`) |
| DELETE | /api/github/connect | – | `{ok}`: stops waiting for the code |
| PATCH | /api/github | `{bots}` (null = every Bot, else Bot ids) | `Settings` |
| DELETE | /api/github | – (oBot forgets the token; the user can also remove oBot on github.com) | `Settings` |
| GET | /oauth/callback | `?state&code` (no token; a browser page load) | HTML page. Finishes the sign-in here, or passes it to the remote computer from `/api/connection` |
| DELETE | /api/vault/:name | – | `Settings` |
| POST | /api/vault/logins | `{sites, username, password, hints, bots}` | `Settings`. `sites`: 1 to 10 addresses, tidied to hosts like `ucsd.edu` (`www.` dropped); a login also fills on subdomains. 400 if the Vault is full (2000) |
| POST | /api/vault/logins/:id | same; empty `password` keeps the saved one | `Settings` |
| DELETE | /api/vault/logins/:id | – | `Settings` |
| POST | /api/vault/contacts | `{full_name, address1, address2, city, region, postal_code, country, phones, emails, label, hints, bots}` | `Settings` (at most 50) |
| POST | /api/vault/contacts/:id | same | `Settings` |
| DELETE | /api/vault/contacts/:id | – | `Settings` |
| POST | /api/vault/import | `{rows:[{url, username, password, name}], apply}`: rows the app read from a password manager's CSV export | `{ready, duplicates, errors, skipped_full, items:[{row, title, username, status:"ready"\|"duplicate"\|"error"\|"full", error?}]}`; with `apply:true` the ready ones are added and `settings` is included. Same website + username = already in the Vault |
| POST | /api/messages/:id/form | `{values:{<field id>: value}, save}` or `{dismiss:true}` | `Message`. Types the values into the page on the Bot's screen, only if it's still on `meta.host`; 400 with a sentence if not (the form stays open). `save` also saves the password as a Vault login |
| GET | /api/computer | `?refresh=1` re-checks Claude Code now | `Computer` |
| GET | /api/hello | – (no token) | `{app:"oBot", version, name}`: lets setup recognize an oBot computer |
| GET | /api/tailscale | – | home only: `{installed, running, needs_login, state, self:{name,dns,os,online}, peers:[{name,dns,os,online}]}` |
| POST | /api/discover | – | home only, ~4 s: `{tailscale, found:[{name, dns, os, url, version, computer_name}]}` your Tailscale devices that run oBot |
| GET | /api/usage | – | `Usage` |
| GET | /api/search?q= | – | `{bots:[Bot], messages:[Message]}` (max 50) |
| POST | /api/messages/:id/answer | `{value}` | `Message` (the user's answer, sent as a normal user message) |
| GET | /api/bots/:id/routines | – | `[Routine]` |
| PATCH | /api/routines/:id | any of `{name, prompt, schedule, enabled}` | `Routine` |
| DELETE | /api/routines/:id | – | `{ok}` |
| POST | /api/routines/:id/run | – | `{ok}` ("Test routine": run once now) |
| GET | /api/bots/:id/memories | – | `[{id, bot_id (null = about the user), text, created_at}]` |
| DELETE | /api/memories/:id | – | `{ok}` |
| GET | /pages/:id | – | HTML made by a bot (sandboxed by CSP). The server appends a script that posts `{type:'obot:height', height}` to the parent. |
| PUT | /api/settings | also `{auto_review, auto_update, review_ask, review_allow}` | `Settings` |
| POST | /api/vault | `{name, value, description, bots}`: adds or edits a secret. Empty `value` = keep the saved one. `bots`: null = every Bot, else a non-empty list of Bot ids. 400 for reserved names (see `vault_reserved`), values over 4 KiB, or an empty `bots` list. Saved values are replaced with `••••` in chats and memories from then on, including older ones. | `Settings` |
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
| GET | /api/update | – | Newest oBot release for this computer: `{current, latest, available, notes, page, can_install, why_not}` or `{current, error}` (see docs/releasing.md) |
| POST | /api/update | – | `{ok, version}`: downloads that release's server program, checks its signature, puts it in place, stops working Bots and starts again (Windows, under `obot-server --supervise`) |
| POST | /api/claude/logout | – | `{ok, output}` runs `claude auth logout` (Settings › Sign Out) |
| POST | /api/open-folder | `{path}` | `{ok}` opens Finder/Explorer on the computer (local only) |
| GET | /api/screens/status | – | `{supported, ready, missing:[program], browser, where, setup_running, running:[bot_id]}` (Bot screens; see below) |
| POST | /api/screens/setup | – | `{started}`; installs the screen programs and a browser inside WSL (Windows hosts). Progress: `screens.setup` events. On Linux hosts it fails with the command to run yourself |
| GET | /api/bots/:id/screen | – | `{supported, reason, running, width, height}` |
| POST | /api/bots/:id/screen/start | – | `{running, password, width, height}` starts the bot's screen if needed. `password` is the one-off VNC password for the viewer |
| POST | /api/bots/:id/screen/stop | – | `{running:false}` closes the bot's browser and screen (its browser profile, so logins, stays) |
| GET (ws) | /api/bots/:id/screen/ws | – | WebSocket (subprotocol `binary`) piped to the screen's VNC server; the UI connects noVNC here |
| GET | /vendor/*path | – | third-party UI files (`frontend/ui/vendor/`, e.g. noVNC) |
| GET (ws) | /api/events | – | stream of events (subprotocol `oBot`); the UI sends `presence` messages back |

Errors: non-2xx with `{error: string}`.

### Phone notifications (push.rs)

A phone that added oBot to its home screen turns on Settings › General › Notifications: the UI asks for
permission, subscribes with the key from `/api/push/key` and sends the subscription to `/api/push/subscribe`.
oBot then sends a Web Push message (encrypted for that phone, so the push service can't read it) for the
same things the desktop app notifies about: a finished reply, a file or page, an approval, a question or a
secure form. Only when no oBot window is `active` (see `presence` above), only for messages from the last
30 minutes, at most one "finished" notification per chat every 5 seconds, and never for hidden Bots or Bots
with notifications turned off. The message is `{title, body, bot, tag, badge}` (`badge` = unread count for the
app icon); approvals and questions go out with `Urgency: high`, and `Topic` is the chat id. A 404/410 from the
push service removes the subscription. The VAPID private key is in `config.json` under `push`, encrypted.

## WebSocket events (server → UI), JSON `{type, ...}`

* `hello {version}`
* `bot.created {bot}` · `bot.updated {bot}` · `bot.deleted {bot_id}`
* `message.created {message}` (a Bot reply arrives once, when it's finished; while it's being written, `bot.activity.label` is "Writing") · `message.updated {message}` (tool finished, approval decided, queued mark changed) · `messages.cleared {bot_id}` · `messages.deleted {bot_id, ids[]}`
* `usage.updated {usage}` · `settings.updated {settings}` · `account.connected {id, email, provider}` (a sign-in finished) · `github.connected {login}` · `github.failed {error}`
* `devices.updated` (a device was paired or removed; re-fetch /api/devices)
* `routines.updated {bot_id}` · `memories.updated {bot_id?}` (re-fetch if that bot's settings are open)
* `screen.updated {bot_id, running}` (a bot's screen started or stopped)
* `screens.setup {state:"running"|"done"|"error", log?, error?}`
* `resync` (reload everything)

```ts
Routine = { id, bot_id, name, prompt, schedule, schedule_text /* "Every weekday at 08:00" */, enabled,
            next_run_at /* ms|null */, last_run_at, paused_for_unread }
// schedule: {kind:"once",at:ms} | {kind:"interval",minutes} | {kind:"daily"|"weekdays",time:"HH:MM"}
//         | {kind:"weekly",day:"monday",time} | {kind:"monthly",day:1..31,time}
```

## Bot tools (the `oBot` MCP server)

Every bot gets these tools from oBot (`backend/core/src/tools.rs`); they never need approval except `delete_bot` and the email/calendar actions marked below.

| Tool | What it does |
|---|---|
| `ask_user {question, options[], allow_custom?}` | tap-to-answer card (a `widget` message) |
| `send_file {path, caption?}` · `show_page {title, html \| path}` | file in the chat · sandboxed HTML page |
| `remember {fact, tier: "profile"\|"log", about?: "user"\|"me", bot?}` | save a fact. `profile` (standing instructions and rules like "never ask me to sign in to X", core identity) is in the bot's prompt every turn; `log` (everything else) is searched with `recall`, and only its newest 15 entries are in the prompt. `about:"user"` facts are shared by all bots; `bot` saves a private fact for another bot |
| `recall {query?, bot?}` · `update_memory {id, fact?, tier?}` · `forget {id}` | search facts by words (no query: profile + newest log) · fix one or move it between tiers · delete one |
| `check_back_later {note, in_minutes \| at}` | one-time wake-up |
| `run_in_background {command, name, note?, max_minutes?}` · `background_jobs {id?}` · `stop_background_job {id}` | a long command run by the oBot server (not Claude Code), output in `<data>/bots/<id>/jobs/<job>.log`. Goes through the same permission check as a shell command (mode, saved rules, auto-review, then an Allow / Deny card); with `runner = "wsl"` jobs run outside Claude Code's sandbox, so they always get a card. Saved prefix rules like `Bash(git log:*)` never cover commands with `; & | $( \` < >` or newlines. Anything the command leaves running is stopped when it ends. When it ends (or oBot restarts while it runs) the bot gets a wake-up turn with the exit code and last 40 lines. Max 5 running per bot, 24 h each |
| `create_routine {name, prompt, schedule, bot?}` · `list_routines {bot?, all?}` · `update_routine {id, …}` · `delete_routine {id}` · `move_routine {id, to_bot}` | routines, for this bot or another |
| `list_bots` · `get_bot {bot}` | see the user's bots (instructions, private memories, routines) |
| `create_bot {name, instructions, label?, look?, memories?[], routines?[], briefing?}` | new bot, ready from its first message; it says hello in its own chat. Names must be unique; max 50 bots |
| `update_bot {bot, name?, label?, look?, instructions?, add_instructions?}` | change a bot (or itself) |
| `delete_bot {bot, reason?}` | retire a bot. Always shows an Allow / Deny card and waits, whatever the permission mode. Not the Main Bot, not itself |
| `message_bot {bot, message}` | pass a task or question to another Bot: the message shows in both chats (`relay`) and starts a turn in that Bot, which works on it in its own chat. At most 4 messages between the same two Bots in 10 minutes. Each Bot's instructions list the user's other Bots, so it can say when a task fits another one better (`backend/core/src/relay.rs`) |
| `react` | `{emoji, message_id?, remove?}` | React to one of the user's messages (default: their latest) with an emoji; shows as a pill on it |
| `set_status {text}` | status line under the bot's name |
| `list_accounts` | the connected accounts this bot may use (Settings › Connections) |
| `email_search {account?, query?, max?}` · `email_read {account?, id}` · `email_attachment {account?, id, attachment_id}` | find and read email; save an attachment into the bot's folder (`attachments/`) |
| `email_send {account?, to[], cc[]?, bcc[]?, subject?, body, reply_to?, attachments[]?, draft?}` | send or reply. Always shows the whole email on an Allow / Deny card and waits (no card for `draft:true`, which only saves a draft) |
| `email_update {account?, ids[], read?, archive?, star?, trash?, add_labels[]?, remove_labels[]?}` | organize email; `trash` asks first. Labels are Gmail only |
| `calendar_list {account?}` | the account's calendars (name, id, main, read only) (`backend/core/src/calendar.rs`) |
| `calendar_events {account?, calendar?, from?, to?, query?, max?}` · `calendar_get {account?, calendar?, id}` | events (default the next 7 days; `calendar` is a name or id, `"all"` = every shown calendar) · one event with guests and replies, reminders, repeat rule, video link and link |
| `calendar_free_time {account?, people[]?, from?, to?, minutes?, day_start?, day_end?, weekends?, max?}` | free times when the user and `people` are all free, inside working hours (default 09:00–18:00, weekdays, 30 min) |
| `calendar_create {account?, calendar?, title, start, end, all_day?, time_zone?, location?, description?, attendees[]?, optional_attendees[]?, repeat[]? (RRULE lines), reminders[]? ({minutes, method: popup\|email}), video_call?, color?, show_as?, visibility?, notify? (all\|external\|none)}` · `calendar_update {…, id, add_attendees[]?, remove_attendees[]?}` | add or change an event. When guests get an email, the user sees the event on a card (Send invite / Send update · Don't send) and the tool waits; otherwise it's saved and a `calendar` message shows it. Outlook: one reminder, no colors, always emails guests, Teams links for work/school accounts |
| `calendar_delete {account?, calendar?, id, notify?}` · `calendar_respond {account?, calendar?, id, response: yes\|maybe\|no, comment?}` | delete an event · reply to an invitation. Both always show a card first |
| `drive_search {account?, query?, folder?, shared_with_me?, max?}` · `drive_list {account?, folder?}` | find Google Drive files (name or text inside) · list a folder. `folder` is an id or a My Drive path like `School/CSE 21`. Google accounts only, and only ones that allowed Drive (`backend/core/src/drive.rs`) |
| `drive_download {account?, id, to?, format?}` | save a file into the bot's folder (default `drive/<name>`, max 100 MB). Docs, Sheets, Slides and Drawings are converted (default PDF; `docx`, `xlsx`, `pptx`, `csv`, `txt`, `md`, `png`) |
| `drive_upload {account?, path, folder?, name?, replace?}` | upload a file from the bot's folder (max 100 MB). Stops if the folder has a file with that name, unless `replace` (Drive keeps the old version) |
| `drive_create_folder {account?, path, parent?}` · `drive_move {account?, id, folder?, name?}` · `drive_copy {account?, id, folder?, name?}` | make folders (all levels of a path; returns an existing one) · move and/or rename · copy a file (not folders) |
| `drive_share {account?, id, with, role?, message?}` | share a file or folder with `"anyone"` (anyone with the link, not searchable) or an email address (Google emails them, with `message`). `role`: `reader` (default), `commenter`, `writer`. Always shows an Allow / Deny card and waits; returns the link |
| `drive_trash {account?, id}` | move a file or folder to Drive's trash. Always shows an Allow / Deny card and waits |
| `github_repos {query?, max?}` · `github_read {repo, path?, ref?}` | the user's repos (own, collaborator, organizations) · a file's text or a folder's list. Only Bots picked under Settings › Connections › GitHub (`backend/core/src/github.rs`) |
| `github_issues {repo, state?, kind?, query?, max?}` · `github_issue {repo, number}` | list or search issues and pull requests · one with its comments (pull requests also reviews, line comments and changed files) |
| `github_comment {repo, number, body}` · `github_create_issue {repo, title, body?, labels?}` | comment or open an issue as the user. Always shows an Allow / Deny card and waits |
| `github_clone {repo, to?, ref?}` · `github_push {path, branch?}` | a working copy in the bot's folder (default `github/<name>`; updates an existing one with a fast-forward pull), with commits credited to the user's GitHub noreply address · push it: the card lists the commits; never force-pushes, refuses when GitHub has commits the copy lacks |
| `github_api {method, path, body?}` | any other REST call; methods other than GET show an Allow / Deny card first |

### How a turn runs

* Every run uses `--model` (the bot's model, else Settings, else `claude-sonnet-5-5`) and `--effort` (Settings, default `xhigh`).
* The current time is sent at the start of each message (`[oBot: It's Friday 2026-11-06 20:01 now.]`), not in the
  system prompt, so the system prompt stays the same between messages and the cached conversation is reused.
* For the same reason a conversation keeps the instructions it started with. When they change (a new memory, a rename,
  new routines, another Bot's tagline), the Bot gets the changed lines as a note at the start of its next message
  (`[oBot: Your setup changed since this conversation started …]`). A new conversation, or Claude Code summarizing
  the old one, starts from the current instructions.
* The message goes to Claude Code as one JSON line on stdin (`--input-format stream-json`). With plain text, Claude Code
  sometimes wrote a different number into its own tool descriptions from one run to the next, and any change there
  makes the model re-read the whole conversation at full price.
* Runs set `CLAUDE_CODE_AUTO_COMPACT_WINDOW=200000` (unless obot-server was started with its own value), so Claude
  Code summarizes older messages once a conversation reaches about 200k tokens instead of near the model's 1M limit.
  Every message re-reads the whole conversation, so a long one makes every message cost more.
  Their settings also set `autoCompactEnabled: true`, so turning summarizing off for your own Claude Code on the PC
  (`/config`, saved in `~/.claude.json`) doesn't turn it off for Bots.
* Bots don't load any Claude Code settings files: runs use `--strict-mcp-config` and `--setting-sources=` (none). So the
  only MCP servers are `oBot` plus Settings › Computer › Plugins, and the computer's personal setup (`~/.claude`: plugins,
  skills, hooks, permission rules, CLAUDE.md, output style) is skipped. The bot folder's own `.claude/settings*.json` and
  `.claude/agents` are skipped too, because a bot can write those itself to give itself permissions. Only oBot's
  `--settings` file below applies. The auto-review check uses the same `--setting-sources=`.
* Skills in the bot folder (`.claude/skills/<name>/SKILL.md`, see `/api/bots/:id/skills`) are copied before each run into
  `<data>/bots/<id>/skills-plugin` and loaded with `--plugin-dir` (`backend/core/src/skills.rs`). Each SKILL.md keeps only
  `name`, `description` and `when_to_use`; fields like `allowed-tools` and `hooks` are dropped. Links aren't copied, and a
  skill folder is limited to 200 files and 5 MB.
* Each run gets a settings file (`<data>/bots/<id>/run-settings.json`) with two Claude Code hooks that start the oBot program as
  `<obot> hook stop|precompact --config <bot's mcp.json>` (`backend/core/src/hooks.rs`):
  * **Stop**: if the bot's last reply promises a follow-up ("I'll check back…") but nothing was scheduled, it is sent back once to call `check_back_later` or correct itself.
  * **PreCompact**: before Claude Code summarizes older messages, the user's latest requests are saved as a `log` memory. The chat shows a `compacted` event.
* Turns oBot starts itself (routines, check-ins, background-job wake-ups, memory tidy-up) send the model a tagged block
  (`<obot_routine>`, `<obot_wakeup>`, `<obot_task>`) with numbered instructions; the chat only shows a short event line.
* Weekly at night (and right after a memory import, for the Main Bot) bots with 12+ facts get a memory tidy-up turn
  (`backend/core/src/memory_tidy.rs`): merge duplicates, fix stale facts, keep the profile small. One bot per hour; a copy of
  the memories is saved first to `<data>/bots/<id>/memory-backup.json`. Overdue after 9 days, it runs at any hour.

`account` is the account's email address (it can be left out when the bot has only one). Times are the user's
local time (`2026-11-05T14:00`, the timezone in Settings). Email/calendar approval cards carry
`meta.input.kind` = `"email"`, `"email-trash"` or `"event"` with the fields the UI previews; they never offer "Always allow".
Event cards carry the same fields as a `calendar` message plus `action` (`create`, `update`, `delete`, `respond`),
`removed` (guests taken off), and for replies `response` and `comment`.

`bot` arguments take a bot's name (any case) or id. `look` is one of the avatar skins (`violet` … `camo`).
Actions on other bots leave an `event` message in both chats with `meta {kind, bot_id}`
(`bot-created`, `bot-created-by`, `bot-updated`, `bot-updated-by`, `bot-deleted`, `routine-created`,
`routine-moved`, `routine-deleted`); the UI shows an "Open <name>" link for `meta.bot_id`.

## Bot screens

Each bot can have its own Linux screen: a virtual display (Xvfb, 1280×800) with the openbox
window manager, a Chrome/Chromium window with the bot's own profile
(`~/.obot/screens/<bot>/profile` inside Linux), and an x11vnc server on 127.0.0.1 only.
On Windows hosts it runs inside WSL (`wsl_distro`); on Linux hosts directly. Screens start on
first use and stop after 30 minutes with no bot run and no viewer. A starting screen waits for the browser
window (it reopens the last tabs), and the first computer tool result then tells the bot its screen was off
and its logins are kept.

The bot gets these MCP tools (each returns a screenshot): `computer_screenshot`,
`computer_click {x,y,button?,count?}`, `computer_move {x,y}`, `computer_drag {from_x,from_y,to_x,to_y}`,
`computer_scroll {x,y,direction,amount?}`, `computer_type {text}`, `computer_key {keys}` (xdotool
names like `ctrl+l`), `computer_open {url, new_tab?}` (opens in the tab in front unless `new_tab`),
`computer_tabs {action: "list"|"switch"|"close"|"close_others", id?}` and `computer_wait {seconds?}`.
A Bot's browser keeps at most 6 tabs (blank, then least recently used ones close first), and when a
screen starts only the 3 newest of the reopened tabs are kept. While it uses them,
`bot.activity.kind` is `on_its_computer`. `request_control {reason}` posts a widget message with
`meta.screen:true` and the option "I'm done": the user opens the screen, chooses Use screen,
does the step (a login, a code, a payment), and answers.

Signing in (logins.rs, secure_form.rs, browser.js): `list_logins {site?}` shows the Bot its Vault
logins without passwords; `fill_login {id?}` types the username and password into the page in front,
only when that page is `https://` on one of the login's websites or a subdomain. Without an id (or with
an unknown one) it uses the only login for that website, and lists the choices when there are several. `request_form
{title, reason?, fields:[{label, type, target?}]}` posts a `form` message tied to the website in
front; what the user types there is typed into the page (never shown to the Bot), optionally saved as a
login, and the Bot then gets a note saying which boxes were filled. `list_contact_info` returns the
Vault's contact info. oBot finds the page in front through the browser's local debugging port
(`--remote-debugging-port=0`, 127.0.0.1 only, port read from `DevToolsActivePort`): the visible tab,
or with several windows the one whose title matches the active window. Filling needs Node.js 22+
inside Linux (installed with the screen setup).

The app's viewer (noVNC) is view-only until the user chooses Use screen.

## Desktop bridge (only inside the oBot desktop app)

When `window.__TAURI_INTERNALS__` exists, the UI can call:
* `invoke('notify', {title, body})` → OS notification (send when a bot finishes or needs you and the window isn't focused)
* `invoke('set_badge', {count})` → Dock/taskbar number = total unread across bots
* `invoke('is_focused')` → boolean
* `invoke('open_url', {url})` → opens an http(s) link in the default browser
