# oBot repo rules

These rules keep the repo clean. They apply to people and to AI coding assistants (Claude Code reads this file automatically).

## Where things go

| Folder | Put here | Never put here |
|---|---|---|
| `frontend/ui/` | The web interface the app shows: `index.html`, `app.js`, `app.css`, `avatar.js`. Third-party code it loads goes in `frontend/ui/vendor/<name>/` with its license file. | Test pages, mockups, screenshots. |
| `frontend/desktop/` | The Tauri desktop app (window, menus, OS notifications). | Server logic; it belongs in `backend/core`. |
| `frontend/assets/` | Source art (the app icon SVG/PNG). | Generated icon sizes; those live in `frontend/desktop/icons/`. |
| `backend/core/` | All server logic: API, Claude Code runner, storage, security. | UI code. |
| `backend/server/` | Only the `obot-server` command-line entry point. | Logic that the desktop app also needs (put it in `core`). |
| `backend/install/<os>/` | Scripts that set up a computer to host oBot. | One-off debugging scripts. |
| `docs/` | Documentation. Add every new doc to `docs/README.md`. | Notes copied from other companies' apps (see below). |
| `docs/research/` | Research write-ups (how other apps do something, options we compared). One topic per file, with sources. | Raw dumps, unsourced claims. |

The repo root holds only: `README.md`, `CLAUDE.md`, `LICENSE`, `Cargo.toml`, `Cargo.lock`, `.gitignore`, `.github/`, `frontend/`, `backend/`, `docs/`. Don't add new top-level files or folders without updating this table.

## Keep it tidy

1. **No scratch files in the repo.** Screenshots, test data, throwaway scripts and experiment pages go in a temp folder outside the repo (or a gitignored folder), not in a tracked folder. Delete them when done.
2. **No dead code.** When a feature is replaced, delete the old version in the same commit. Don't keep `old_`, `_v2` or commented-out copies; git history keeps them.
3. **Every file has one job** and a short comment at the top saying what it is.
4. **Moving or renaming** a file: update every path that points to it (Cargo.toml, tauri.conf.json, build.rs, the CI workflow, install scripts, README, docs) in the same commit, then run `cargo check --workspace`.
5. **Docs stay true.** When you change an API route, a setting or a folder, update `docs/API.md` / `README.md` in the same commit.
6. **Keep private notes private.** `docs/private/` is gitignored; never commit it or paste chunks of it into tracked files. Research docs use public sources only.
7. **No secrets** (tokens, API keys, pairing codes, recovery keys) in any tracked file.
8. **Commits:** one topic per commit, plain-English message. No AI attribution lines.

## Before you commit

```bash
cargo check --workspace
node --check frontend/ui/app.js && node --check frontend/ui/avatar.js
git status   # nothing unexpected (no screenshots, no temp files)
```
