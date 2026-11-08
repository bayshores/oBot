# Using oBot from a phone

How other always-on, self-hosted agent apps put themselves on a phone, and why oBot uses a home-screen
web app over Tailscale with Web Push. Researched November 2026 from the public sources linked below.

## What similar apps do

| App | Phone surface | How the phone reaches the computer | Notifications |
|---|---|---|---|
| OpenClaw | Native iOS app (a "node" plus chat client) and chat apps (Telegram, WhatsApp, iMessage, …) | Recommends Tailscale Serve to a gateway on 127.0.0.1; pairs by QR or setup code | App Store builds go through a hosted push relay |
| Hermes Agent | Chat apps (Telegram, Discord, Slack, Signal, …); an official mobile app is a waitlist beta | The gateway polls Telegram (outbound only); unknown users are denied by default | Results arrive as chat messages |
| Claude Code Remote Control | Claude's iOS/Android app or the web | Outbound HTTPS only, paired by QR code or link | Push when a task finishes or needs a decision (opt-in) |
| Home Assistant | Native companion apps | Nabu Casa's relay or Tailscale | Its own push relay; actionable notifications with buttons |
| Open WebUI, LibreChat | Home-screen web app only | Whatever the user sets up (HTTPS required) | Open WebUI: webhooks; LibreChat: none found |

Sources: [OpenClaw iOS docs](https://docs.openclaw.ai/platforms/ios), [OpenClaw pairing](https://docs.openclaw.ai/channels/pairing),
[Hermes messaging](https://hermes-agent.nousresearch.com/docs/user-guide/messaging/), [Hermes mobile waitlist](https://hermes-agent.nousresearch.com/mobile),
[Claude Code Remote Control](https://code.claude.com/docs/en/remote-control), [Home Assistant notifications](https://companion.home-assistant.io/docs/notifications/actionable-notifications/),
[Open WebUI as an app](https://docs.openwebui.com/getting-started/open-webui-as-app).

## Options for oBot

* **Native app (Tauri 2 for iOS).** Needs the paid Apple Developer Program: free signing expires after 7 days
  and can't use push notifications ([Apple memberships](https://developer.apple.com/support/compare-memberships/),
  [capabilities](https://developer.apple.com/help/account/reference/supported-capabilities-ios)). Tauri's own
  notification plugin is local only ([docs](https://v2.tauri.app/plugin/notification/)).
* **Chat bridge (Telegram).** No app to install and long polling needs no open ports, but Telegram's servers
  see every message, anyone can message a bot, and a leaked bot token controls it
  ([Bot API](https://core.telegram.org/bots/api), [FAQ](https://telegram.org/faq)). OpenClaw's docs note that
  allowlists don't stop prompt injection from untrusted content.
* **Home-screen web app (chosen).** oBot already serves its UI over Tailscale Serve HTTPS. Since iOS 16.4 a
  web app added to the Home Screen can get Web Push, with no developer account
  ([WebKit](https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/)). The computer sends the push
  out to Apple, so it works even though the computer is only reachable on the tailnet, and the message is
  encrypted for the phone ([Apple](https://developer.apple.com/documentation/usernotifications/sending-web-push-notifications-in-web-apps-and-browsers)).

## Limits of the web app on iPhone

* Every push must show a notification; silent pushes get the subscription revoked
  ([WebKit](https://webkit.org/blog/12945/meet-web-push/)).
* No action buttons on notifications, so approving means tap, open, then Allow/Deny
  ([MDN compat data](https://github.com/mdn/browser-compat-data/blob/main/api/ServiceWorkerRegistration.json)).
* No share sheet target ([MDN compat data](https://github.com/mdn/browser-compat-data/blob/main/manifests/webapp/share_target.json)).
* The page is paused in the background and its WebSocket dies quietly, so the app reconnects when it comes back.
* Opening the app needs Tailscale on; the notification itself arrives without it.

## Security lessons applied

* OpenClaw's CVE-2026-25253: its web UI took a server address from the link and sent its token there
  ([advisory](https://github.com/advisories/GHSA-g8p2-7wf7-98mq)). oBot never takes a server address from a
  link, and keeps tokens out of web addresses (headers and WebSocket subprotocols; a file-only key for images).
* Exposed instances: tens of thousands of OpenClaw gateways were found on the open internet
  ([Infosecurity](https://www.infosecurity-magazine.com/news/researchers-40000-exposed-openclaw)). oBot listens
  on 127.0.0.1 and is reached only through Tailscale Serve.
* Pairing codes are single use, expire in 10 minutes and are rate limited; the QR link keeps the code after `#`,
  which the browser never sends.
