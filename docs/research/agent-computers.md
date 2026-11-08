# How agent apps give a bot "its own computer" (and what oBot should do on Windows)

Researched November 2026. Every claim has a numbered source at the bottom. Items we could not confirm are marked **(unverified)**.

## The short answer

No popular agent app protects the user by running the agent as the user's own account. They do one of three things:

| Strategy | Who does it |
|---|---|
| **A computer in the cloud** (a Linux VM rented by the company) | Manus, ChatGPT agent |
| **A container or VM on the user's PC** (optional) | Hermes Agent (Docker backend), OpenClaw (sandbox mode) |
| **A separate, weaker Windows user account** | OpenAI Codex on Windows, Microsoft's Windows "Agent Workspace" (preview) |

oBot runs on your own Windows PC, so the cloud option is out. **Recommendation: give the bots their own locked-down Linux computer inside WSL2 on the PC**, with a separate low-privilege Windows account as a later add-on for Windows-only jobs. See [What oBot should do](#what-oBot-should-do).

## Hermes Agent (Nous Research)

You pick one "terminal backend" in the config: `local`, `docker`, `ssh`, `modal`, `daytona`, `vercel_sandbox` or `singularity` [1][2].

- **`local` is the default and has no isolation.** Commands run as you, with your file access [1].
- **`docker` is the locked-down option** [1]:
  - It drops almost all Linux permissions and caps the number of processes.
  - Default limits are 1 CPU and 5 GB of RAM.
  - It can turn the network off.
  - Your folders aren't visible inside unless you share them.
- **Persistence.** By default one long-lived container is shared by all sessions. Choose one container per session when you want conversations walled off from each other [1].
- **Windows.**
  - Hermes runs natively, with commands going through Git Bash. "Same strategy Claude Code uses", and it has no sandbox [3].
  - WSL2 is optional [3]. Its WSL guide warns that WSL shuts down when nothing is using it, and suggests a keep-alive task [4].

## OpenClaw (formerly Clawdbot / Moltbot)

- **Sandboxing is off by default.** The main program (the "gateway") always runs on your computer. When the sandbox is on, only the tools it runs (commands, file edits) move into a container. Their own docs call it "not a perfect security boundary" [5].
- **Setting:** `agents.defaults.sandbox.mode` takes one of three values [5][6]:
  - `off`: no sandbox.
  - `non-main`: sandbox group chats and channels, but not your main chat.
  - `all`: sandbox everything.
- **Other options** [5][6]:
  - Workspace access can be none, read-only or read-write.
  - Backends: Docker (the default), Podman, SSH and others.
  - `tools.elevated` is an escape hatch that runs a command outside the sandbox.
- **Windows.** WSL2 is the recommended runtime. Their Windows app sets up **its own WSL Linux install (`OpenClawGateway`) that the app owns**. That's the same idea we recommend below [7].
  - A known WSL bug (WSL 2.6.1+) can shut the Linux side down when it's idle. They document a workaround [7].
- **Security incidents** (why sandboxing matters):
  - A one-click bug (CVE-2026-25253) let a malicious link steal the gateway's token. The attacker could then turn off approvals and move commands out of the sandbox, even on installs only reachable from the PC itself [8][9].
  - Hundreds of malicious add-on "skills" were found in its skill store [10].
  - Tens of thousands of instances were exposed to the internet. Estimates vary [11].
  - **Lesson:** a sandbox is useless if a stolen login can switch it off.

## Other apps, for comparison

- **Claude Code's own sandbox** works on macOS, Linux and WSL2, but "on native Windows, Claude Code runs commands unsandboxed" [12].
  - For unattended runs, Anthropic says to use a container, a VM, or the sandbox inside WSL2 [13]. That fits oBot, which runs Claude Code unattended.
- **OpenAI Codex on Windows** creates dedicated lower-privilege Windows users plus firewall rules. Setting it up needs admin approval [14].
- **Manus** gives each task an isolated cloud VM. Its paid "Cloud Computer" is a persistent Ubuntu VM that runs 24/7 [15][16].
- **ChatGPT agent** (formerly Operator) works on "its own computer", a remote virtual machine you can take over [17]. Secondary source only.
- **Windows 11 "Agent Workspace"** (Microsoft) [18][19]:
  - Each agent gets its own Windows account and its own desktop in a separate session.
  - You choose per agent which of your folders it can see.
  - It's still a preview for Windows Insiders and is off by default.
  - We found no way for a third-party app like oBot to use it yet **(unverified)**. Worth checking again later.

## Ways to isolate a bot on a Windows 11 PC

| Option | Works on Windows Home? | Can oBot's installer set it up? | Keeps files? | Gives the bot a screen to use | Claude Code inside? | How strongly it walls off your files |
|---|---|---|---|---|---|---|
| Run as you (today) | Yes | Nothing to set up | Yes | Your real desktop | Yes, unsandboxed [12] | Not at all |
| Separate low-privilege Windows account | Yes | Yes, easily [14] | Yes | Only if that user is logged in **(unverified on Home)** | Yes | Medium (Windows file permissions) |
| AppContainer (restricted Windows process) | Yes | Hard, needs custom code [22] | Yes | No | Probably fragile **(unverified)** | Medium–high |
| **WSL2 Linux, owned by oBot and locked down** | **Yes** | **Yes** [7][23] | **Yes** | **Yes (a virtual Linux desktop, viewable from the Mac)** | **Yes, plus its sandbox** [12] | Medium–high, *if* access to `C:\` and Windows programs is turned off [24] |
| Docker (Desktop or inside WSL2) | Yes | Heavy install [25] | Yes (volumes) | Possible (noVNC) | Yes [13] | Medium (shares WSL's kernel) |
| Docker Sandboxes (`sbx`) | **(unverified)** | Yes, but needs a Docker account [26] | Yes [27] | **(unverified)** | Yes, built in [26] | High (own mini VM) |
| Hyper-V virtual machine | **No, Pro only** [28] | Possible but heavy | Yes | Full Windows or Linux desktop | Yes | Highest |
| Windows Sandbox | **No, Pro only** [29] | Partly; can't read command output [30] | **No, wiped when closed** [29] | Yes | Would need reinstalling every time | High, but useless for 24/7 |
| Windows Agent Workspace | **(unverified)** | Not open to other apps yet [18] | Yes | Yes | **(unverified)** | Medium |

## What oBot should do

**Plan: an app-owned WSL2 Linux computer called `oBot-computer`.**

1. **Install.** `install-host.ps1` turns on WSL if needed, then imports a ready-made Ubuntu image as a separate Linux install called `oBot-computer`. This is the same approach as OpenClaw's Windows app [7].
2. **Lock it down** in `/etc/wsl.conf` [24]:
   - Turn off automatic mounting of `C:\`, so the bots can't see your files.
   - Turn off Windows interop, so the bots can't launch Windows programs.
   - Share exactly one Windows folder, for example `C:\Users\<you>\oBot\Shared`.
3. **Run Claude Code inside it** as a normal (non-admin) Linux user `bot`, with Claude Code's sandbox turned on [12][13].
   - The oBot server stays on Windows and starts each job with `wsl.exe -d oBot-computer -u bot -- claude -p ...`.
   - This is the same split as OpenClaw: the controller on the PC, the work inside the sandbox [5].
4. **Give each bot a screen.** Run a virtual Linux desktop per bot inside the WSL computer (Xvfb, a small window manager and Chromium).
   - Stream it over VNC, reachable only through Tailscale.
   - The Mac app can then show a live "bot screen" with a **Use screen** button.
5. **Keep it running 24/7** [4][7][24]:
   - In `.wslconfig`, stop WSL from shutting down when idle and cap its RAM and CPU.
   - Add a keep-alive task that starts at boot.
6. **Later, for Windows-only jobs** (Office files, Windows apps): add a second runner as a dedicated standard Windows account (`oBot-agent`) [14].
   - It can only reach chosen folders.
   - You approve each job in the app.
7. **Security rules learned from OpenClaw** [8][10]:
   - Nothing reachable over the network may turn off approvals or the sandbox.
   - Keep the server Tailscale-only, as it is today.
   - Never auto-install third-party add-ons.

**Why this one:**
- It works on Windows 11 Home (Hyper-V and Windows Sandbox need Pro) [28][29].
- It keeps files.
- One script can set it up.
- It's officially supported by Claude Code's sandbox.
- It gives the bots a desktop you can watch, at a moderate RAM cost.

**Downsides:**
- The bots can't use Windows desktop apps unless we add step 6 or a Hyper-V VM.
- The isolation is only as good as the `wsl.conf` lockdown, so that file needs a test that checks it.

## Sources

1. https://hermes-agent.nousresearch.com/docs/user-guide/configuration
2. https://github.com/NousResearch/hermes-agent/issues/1855
3. https://hermes-agent.nousresearch.com/docs/user-guide/windows-native
4. https://hermes-agent.nousresearch.com/docs/user-guide/windows-wsl-quickstart
5. https://docs.openclaw.ai/gateway/sandboxing
6. https://openclawlab.com/en/docs/gateway/sandbox-vs-tool-policy-vs-elevated/ (mirror of OpenClaw docs)
7. https://docs.openclaw.ai/platforms/windows.md
8. https://thehackernews.com/2026/02/openclaw-bug-enables-one-click-remote.html
9. https://advisories.gitlab.com/pkg/npm/clawdbot/CVE-2026-25253/
10. https://thehackernews.com/2026/02/researchers-find-341-malicious-clawhub.html
11. https://blog.rankiteo.com/ope1770630718-openclaw-vulnerability-february-2026/
12. https://code.claude.com/docs/en/sandboxing
13. https://code.claude.com/docs/en/sandbox-environments
14. https://learn.chatgpt.com/docs/windows/windows-sandbox.md (redirect target of developers.openai.com/codex/windows.md)
15. https://www.manus.im/blog/manus-sandbox
16. https://help.manus.im/en/articles/15392111-what-is-the-cloud-computer
17. https://www.neowin.net/news/openai-introduces-chatgpt-agent-that-can-complete-tasks-using-its-own-computer/
18. https://support.microsoft.com/en-us/windows/experimental-agentic-features-a25ede8a-e4c2-4841-85a8-44839191dfb3
19. https://blogs.windows.com/windowsexperience/2025/10/16/securing-ai-agents-on-windows/
20. https://www.windowslatest.com/2026/04/18/microsoft-confirms-ai-agents-are-still-coming-to-the-windows-11-taskbar-as-it-prepares-for-public-rollout/
21. https://learn.microsoft.com/en-us/windows/ai/mcp/overview
22. https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation
23. https://learn.microsoft.com/en-us/windows/wsl/about
24. https://learn.microsoft.com/en-us/windows/wsl/wsl-config
25. https://docs.docker.com/subscription/desktop-license/
26. https://docs.docker.com/ai/sandboxes/get-started.md
27. https://docs.docker.com/ai/sandboxes/get-started/
28. https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/get-started/install-hyper-v
29. https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/
30. https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-cli
