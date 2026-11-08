# Releasing oBot

A release puts a new version on [github.com/bayshores/oBot/releases](https://github.com/bayshores/oBot/releases).
The apps find it by themselves: the Mac app and the always-on computer check every few hours, show
**oBot X is available · Update** above the message box, and update from Settings › Updates.

## Make a release

1. Raise `version` in the root `Cargo.toml` (for example `0.2.0` → `0.2.1`). Everything takes its
   version from there.
2. Commit, then tag and push the tag:
   ```bash
   git tag v0.2.1 && git push origin main v0.2.1
   ```
3. `.github/workflows/release.yml` builds, signs and publishes it (about 20 minutes). Watch it in the
   **Actions** tab.

The release has:

| File | What it is |
|---|---|
| `oBot-mac-apple-silicon.dmg` | The Mac app for new installs, signed with the Developer ID certificate and notarized by Apple. |
| `oBot-mac-apple-silicon.app.tar.gz` + `latest.json` | What the Mac app's updater downloads. `latest.json` names the version and carries the file's signature. |
| `obot-server.exe` + `obot-server.exe.sig` | The Windows server. A computer installs it from Settings › Updates only if the signature matches. |
| `install-host.ps1` | The Windows installer (first install). |

## How updates stay safe

Every file the app installs is signed with oBot's **update key**. The public half is `PUBLIC_KEY` in
`backend/core/src/update.rs` (and the same value in `frontend/desktop/tauri.conf.json`); the private
half lives only in the `TAURI_SIGNING_PRIVATE_KEY` repository secret and in a backup on the
maintainer's Mac (`~/.tauri/obot-updater.key`). A download whose signature doesn't match is refused.
If the private key is lost, installed apps can't update any more and need one manual reinstall with a
new key, so keep the backup.

On Windows the server runs as `obot-server --supervise` (set up by `install-host.ps1`): after an
update it moves the old program aside, puts the new one in its place and starts it again within a
few seconds. A computer installed before updates existed needs `install-host.ps1` run once more.

## Repository secrets

Settings › Secrets and variables › Actions on GitHub:

| Secret | What it is |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | The update key (`cargo tauri signer generate -w ~/.tauri/obot-updater.key`, then the contents of that file). |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The update key's password (empty if it has none). |
| `APPLE_CERTIFICATE` | The Developer ID Application certificate, exported from Keychain Access as a .p12 file and base64-encoded. |
| `APPLE_CERTIFICATE_PASSWORD` | The password chosen when exporting that .p12. |
| `APPLE_ID` · `APPLE_TEAM_ID` | The Apple developer account's email and team ID. |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID (appleid.apple.com › Sign-In and Security). |
| `OPENBOT_GOOGLE_CLIENT_ID` and the others | The sign-in apps, see [connections-setup.md](connections-setup.md). |
