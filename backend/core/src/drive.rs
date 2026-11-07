//! Google Drive for Bots: find, read, download, upload and organize files in a connected Google
//! account's Drive (Settings › Connections). The account must have allowed Google Drive (accounts
//! connected before Drive was added show "Add Google Drive"). If the user runs Google Drive for
//! desktop, files in their My Drive also appear on their own computer, so this is how a Bot puts
//! files there. Moving to the trash asks the user first; everything else is undoable in Drive.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::accounts::{api, api_raw, enc, has_drive, Account};
use crate::db::Bot;
use crate::state::Shared;
use crate::util::truncate_chars;

const API: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER: &str = "application/vnd.google-apps.folder";
const SHORTCUT: &str = "application/vnd.google-apps.shortcut";
const FIELDS: &str = "id,name,mimeType,modifiedTime,size,parents,webViewLink,shortcutDetails";
/// Biggest file a Bot can download or upload.
const MAX_BYTES: u64 = 100 * 1024 * 1024;
/// Files in shared drives too.
const ALL_DRIVES: &str = "supportsAllDrives=true&includeItemsFromAllDrives=true";

pub fn tool_definitions() -> Vec<Value> {
    let account = json!({"type": "string", "description": "The Google account's email address. Can be left out when only one has Drive."});
    let folder = json!({"type": "string", "description": "A folder id, or a path in My Drive like \"School/CSE 21\" (\"\" or \"My Drive\" = the top)"});
    vec![
        json!({"name": "drive_search",
            "description": "Find files in Google Drive by name or text inside them (newest first). Can limit to one folder, or to files others shared with the user.",
            "inputSchema": {"type": "object", "properties": {
                "account": account, "query": {"type": "string", "description": "Words to look for"}, "folder": folder,
                "shared_with_me": {"type": "boolean", "description": "Only files others shared with the user"},
                "max": {"type": "integer", "description": "1-50, default 20"}}}}),
        json!({"name": "drive_list",
            "description": "List what's in a Google Drive folder (folders first).",
            "inputSchema": {"type": "object", "properties": {"account": account, "folder": folder}}}),
        json!({"name": "drive_download",
            "description": "Save a Drive file into your folder so you can open it. Google Docs, Sheets and Slides are converted (default PDF; or format docx, xlsx, pptx, csv, txt, md). Up to 100 MB.",
            "inputSchema": {"type": "object", "properties": {
                "account": account, "id": {"type": "string"},
                "to": {"type": "string", "description": "Where in your folder (default: drive/<name>)"},
                "format": {"type": "string", "enum": ["pdf", "docx", "xlsx", "pptx", "csv", "txt", "md", "png"]}}, "required": ["id"]}}),
        json!({"name": "drive_upload",
            "description": "Upload a file from your folder into a Google Drive folder (up to 100 MB). If a file with that name is already there it stops, unless replace is true (Drive keeps the old version in its history).",
            "inputSchema": {"type": "object", "properties": {
                "account": account, "path": {"type": "string", "description": "The file on your computer"}, "folder": folder,
                "name": {"type": "string", "description": "Name in Drive (default: the file's name)"},
                "replace": {"type": "boolean"}}, "required": ["path"]}}),
        json!({"name": "drive_create_folder",
            "description": "Make a folder in Google Drive (or return it if it's already there). Give a path to make several levels.",
            "inputSchema": {"type": "object", "properties": {
                "account": account, "path": {"type": "string", "description": "Like \"School/CSE 21/Homework\", inside parent"},
                "parent": folder}, "required": ["path"]}}),
        json!({"name": "drive_move",
            "description": "Move a Drive file or folder to another folder, and/or rename it.",
            "inputSchema": {"type": "object", "properties": {
                "account": account, "id": {"type": "string"}, "folder": folder, "name": {"type": "string", "description": "New name"}}, "required": ["id"]}}),
        json!({"name": "drive_copy",
            "description": "Copy a Drive file (for example one someone shared) into a folder of the user's own Drive. Folders can't be copied; copy their files one by one.",
            "inputSchema": {"type": "object", "properties": {
                "account": account, "id": {"type": "string"}, "folder": folder, "name": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "drive_trash",
            "description": "Move a Drive file or folder to the trash (the user sees an Allow / Deny card first; it can be restored from the trash for 30 days).",
            "inputSchema": {"type": "object", "properties": {"account": account, "id": {"type": "string"}}, "required": ["id"]}}),
    ]
}

pub fn is_tool(name: &str) -> bool {
    name.starts_with("drive_")
}

/// drive_trash waits for the user's Allow / Deny.
pub fn waits_for_user(name: &str) -> bool {
    name == "drive_trash"
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

/// A Drive search string: 'it''s' -> 'it\'s'.
fn quoted(t: &str) -> String {
    format!("'{}'", t.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The Google account a Drive tool call is about.
fn choose(st_cfg: &crate::config::Config, bot: &Bot, args: &Value) -> Result<Account, String> {
    let google: Vec<&Account> = crate::accounts::for_bot(st_cfg, &bot.id).into_iter().filter(|a| a.provider == "google").collect();
    if google.is_empty() {
        return Err("No Google account is connected for you. The user can connect one in Settings › Connections (and pick you there).".into());
    }
    let want = s(args, "account");
    let pick = if want.is_empty() {
        let with_drive: Vec<&&Account> = google.iter().filter(|a| has_drive(a)).collect();
        match with_drive.len() {
            1 => (*with_drive[0]).clone(),
            0 if google.len() == 1 => google[0].clone(),
            0 => {
                let names: Vec<_> = google.iter().map(|a| a.email.as_str()).collect();
                return Err(format!(
                    "None of your Google accounts ({}) has allowed Google Drive yet. Ask the user to open Settings › Connections and click Add Google Drive next to the account they want (they sign in once more and allow Drive).",
                    names.join(", ")
                ));
            }
            _ => {
                let names: Vec<_> = with_drive.iter().map(|a| a.email.as_str()).collect();
                return Err(format!("Say which account (\"account\"): {}", names.join(", ")));
            }
        }
    } else {
        google
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(&want) || a.id == want)
            .map(|a| (*a).clone())
            .ok_or_else(|| format!("{want} isn't one of your Google accounts"))?
    };
    if !has_drive(&pick) {
        return Err(missing(&pick.email));
    }
    Ok(pick)
}

fn missing(email: &str) -> String {
    format!(
        "{email} hasn't allowed Google Drive yet. Ask the user to open Settings › Connections and click Add Google Drive next to the account (they sign in once more and allow Drive)."
    )
}

fn size_text(v: &Value) -> String {
    let n: u64 = s(v, "size").parse().unwrap_or(0);
    match n {
        0 => String::new(),
        n if n < 1024 * 1024 => format!(" · {} KB", n / 1024 + 1),
        n => format!(" · {:.1} MB", n as f64 / (1024.0 * 1024.0)),
    }
}

fn kind_text(mime: &str) -> &str {
    match mime {
        FOLDER => "folder",
        SHORTCUT => "shortcut",
        "application/vnd.google-apps.document" => "Google Doc",
        "application/vnd.google-apps.spreadsheet" => "Google Sheet",
        "application/vnd.google-apps.presentation" => "Google Slides",
        "application/pdf" => "PDF",
        m if m.starts_with("image/") => "image",
        m if m.starts_with("video/") => "video",
        _ => "file",
    }
}

fn line(f: &Value) -> String {
    let mime = s(f, "mimeType");
    let modified = s(f, "modifiedTime");
    let modified = modified.get(..10).unwrap_or("");
    format!("- {}{} · {}{} · modified {modified} · id {}", s(f, "name"), if mime == FOLDER { "/" } else { "" }, kind_text(&mime), size_text(f), s(f, "id"))
}

/// All results of a files.list query (up to `max`).
async fn list_files(st: &Shared, a: &Account, q: &str, order: &str, max: usize) -> Result<Vec<Value>, String> {
    let mut out = vec![];
    let mut page = String::new();
    while out.len() < max {
        let mut url = format!("{API}?q={}&pageSize={}&fields=nextPageToken,files({FIELDS})&{ALL_DRIVES}", enc(q), (max - out.len()).min(200));
        if !order.is_empty() {
            url.push_str(&format!("&orderBy={}", enc(order)));
        }
        if !page.is_empty() {
            url.push_str(&format!("&pageToken={}", enc(&page)));
        }
        let v = api(st, a, "GET", &url, &[], None).await?;
        out.extend(v.get("files").and_then(Value::as_array).cloned().unwrap_or_default());
        page = s(&v, "nextPageToken");
        if page.is_empty() {
            break;
        }
    }
    Ok(out)
}

async fn get_file(st: &Shared, a: &Account, id: &str) -> Result<Value, String> {
    if id.is_empty() {
        return Err("Give the file's id (from drive_search or drive_list)".into());
    }
    api(st, a, "GET", &format!("{API}/{}?fields={FIELDS},trashed&supportsAllDrives=true", enc(id)), &[], None)
        .await
        .map_err(|e| if e.contains("HTTP 404") { format!("There's no Drive file with id {id}") } else { e })
}

/// Child folders of `parent` named `name` (any case).
async fn child_folder(st: &Shared, a: &Account, parent: &str, name: &str) -> Result<Option<Value>, String> {
    let q = format!("{} in parents and mimeType = '{FOLDER}' and trashed = false", quoted(parent));
    let kids = list_files(st, a, &q, "name", 1000).await?;
    Ok(kids.iter().find(|k| s(k, "name") == name).or_else(|| kids.iter().find(|k| s(k, "name").eq_ignore_ascii_case(name))).cloned())
}

fn parts(path: &str) -> Vec<String> {
    path.split('/').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

fn looks_like_id(t: &str) -> bool {
    t.len() >= 15 && !t.contains(['/', ' ']) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A folder from an id or a My Drive path. Returns (id, readable name).
async fn folder(st: &Shared, a: &Account, f: &str) -> Result<(String, String), String> {
    let f = f.trim();
    let path = f.strip_prefix("My Drive").map(|r| r.trim_start_matches('/')).unwrap_or(f);
    if path.is_empty() || path.eq_ignore_ascii_case("root") {
        return Ok(("root".into(), "My Drive".into()));
    }
    if looks_like_id(path) {
        if let Ok(v) = get_file(st, a, path).await {
            if s(&v, "mimeType") != FOLDER {
                return Err(format!("{} is a file, not a folder", s(&v, "name")));
            }
            return Ok((s(&v, "id"), s(&v, "name")));
        }
    }
    let mut id = "root".to_string();
    let mut walked = vec![];
    for p in parts(path) {
        match child_folder(st, a, &id, &p).await? {
            Some(k) => id = s(&k, "id"),
            None => {
                let at = if walked.is_empty() { "My Drive".to_string() } else { walked.join("/") };
                return Err(format!("There's no folder \"{p}\" in {at}. Use drive_list to look, or drive_create_folder."));
            }
        }
        walked.push(p);
    }
    Ok((id, walked.join("/")))
}

/// A path inside the Bot's folder.
fn in_workdir(workdir: &str, rel: &str) -> Result<PathBuf, String> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err("\"to\" must be a path inside your folder, like drive/notes.pdf".into());
    }
    Ok(Path::new(workdir).join(p))
}

fn safe_name(n: &str) -> String {
    let t: String = n.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let t = t.trim().trim_start_matches('.').to_string();
    if t.is_empty() { "file".into() } else { truncate_chars(&t, 150) }
}

/// Google Docs/Sheets/Slides can't be downloaded as they are: (export type, file extension).
fn export_type(mime: &str, format: &str) -> Result<(&'static str, &'static str), String> {
    let f = if format.is_empty() { "pdf" } else { format };
    let pick = match (mime, f) {
        (_, "pdf") => ("application/pdf", "pdf"),
        ("application/vnd.google-apps.document", "docx") => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
        ("application/vnd.google-apps.document", "txt") => ("text/plain", "txt"),
        ("application/vnd.google-apps.document", "md") => ("text/markdown", "md"),
        ("application/vnd.google-apps.spreadsheet", "xlsx") => ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "xlsx"),
        ("application/vnd.google-apps.spreadsheet", "csv") => ("text/csv", "csv"),
        ("application/vnd.google-apps.presentation", "pptx") => ("application/vnd.openxmlformats-officedocument.presentationml.presentation", "pptx"),
        ("application/vnd.google-apps.presentation", "txt") => ("text/plain", "txt"),
        ("application/vnd.google-apps.drawing", "png") => ("image/png", "png"),
        _ => return Err(format!("A {} can't be saved as {f}", kind_text(mime))),
    };
    Ok(pick)
}

pub async fn tool(st: &Shared, bot: &Bot, name: &str, args: &Value) -> Result<String, String> {
    let cfg = st.config.read().await.clone();
    let a = choose(&cfg, bot, args)?;
    match name {
        "drive_search" => {
            let mut q = vec!["trashed = false".to_string()];
            let words = s(args, "query");
            if !words.is_empty() {
                q.push(format!("(name contains {0} or fullText contains {0})", quoted(&words)));
            }
            let mut place = String::new();
            if !s(args, "folder").is_empty() {
                let (id, n) = folder(st, &a, &s(args, "folder")).await?;
                q.push(format!("{} in parents", quoted(&id)));
                place = format!(" in {n}");
            }
            if args.get("shared_with_me").and_then(Value::as_bool) == Some(true) {
                q.push("sharedWithMe = true".into());
                place.push_str(" (shared with the user)");
            }
            let max = args.get("max").and_then(Value::as_u64).unwrap_or(20).clamp(1, 50) as usize;
            // Drive can't sort full-text searches, so those come back by relevance.
            let order = if words.is_empty() { "modifiedTime desc" } else { "" };
            let files = list_files(st, &a, &q.join(" and "), order, max).await?;
            if files.is_empty() {
                return Ok(format!("Nothing found{place}."));
            }
            Ok(format!("{} result(s){place}:\n{}", files.len(), files.iter().map(line).collect::<Vec<_>>().join("\n")))
        }
        "drive_list" => {
            let (id, n) = folder(st, &a, &s(args, "folder")).await?;
            let q = format!("{} in parents and trashed = false", quoted(&id));
            let files = list_files(st, &a, &q, "folder,name", 500).await?;
            if files.is_empty() {
                return Ok(format!("{n} is empty."));
            }
            let more = if files.len() >= 500 { " (first 500)" } else { "" };
            Ok(format!("{n}: {} item(s){more}\n{}", files.len(), files.iter().map(line).collect::<Vec<_>>().join("\n")))
        }
        "drive_download" => {
            let mut f = get_file(st, &a, &s(args, "id")).await?;
            if s(&f, "mimeType") == SHORTCUT {
                let target = f.pointer("/shortcutDetails/targetId").and_then(Value::as_str).unwrap_or("").to_string();
                f = get_file(st, &a, &target).await?;
            }
            let mime = s(&f, "mimeType");
            if mime == FOLDER {
                return Err("That's a folder. Use drive_list, then download its files one by one.".into());
            }
            let id = s(&f, "id");
            let mut fname = safe_name(&s(&f, "name"));
            let url = if mime.starts_with("application/vnd.google-apps.") {
                let (ty, ext) = export_type(&mime, &s(args, "format"))?;
                if !fname.to_lowercase().ends_with(&format!(".{ext}")) {
                    fname = format!("{fname}.{ext}");
                }
                format!("{API}/{}/export?mimeType={}", enc(&id), enc(ty))
            } else {
                format!("{API}/{}?alt=media&supportsAllDrives=true", enc(&id))
            };
            let dest = match s(args, "to") {
                t if !t.is_empty() => in_workdir(&bot.workdir, &t)?,
                _ => {
                    let dir = Path::new(&bot.workdir).join("drive");
                    let mut p = dir.join(&fname);
                    let mut n = 2;
                    while p.exists() {
                        p = dir.join(format!("{n}-{fname}"));
                        n += 1;
                    }
                    p
                }
            };
            let (bytes, _) = api_raw(st, &a, "GET", &url, &[], None, MAX_BYTES).await?;
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
            Ok(format!("Saved {} ({} KB) to {}", s(&f, "name"), bytes.len() / 1024 + 1, dest.display()))
        }
        "drive_upload" => {
            let path = s(args, "path");
            let local = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { Path::new(&bot.workdir).join(&path) };
            let meta = std::fs::metadata(&local).map_err(|_| format!("There's no file at {}", local.display()))?;
            if !meta.is_file() {
                return Err(format!("{} is a folder; upload one file at a time (or zip it first)", local.display()));
            }
            if meta.len() > MAX_BYTES {
                return Err("That file is over 100 MB, too big to upload".into());
            }
            let dname = match s(args, "name") {
                n if !n.is_empty() => n,
                _ => local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into()),
            };
            let (fid, fname) = folder(st, &a, &s(args, "folder")).await?;
            let q = format!("{} in parents and name = {} and trashed = false and mimeType != '{FOLDER}'", quoted(&fid), quoted(&dname));
            let existing = list_files(st, &a, &q, "", 1).await?.into_iter().next();
            let replace = args.get("replace").and_then(Value::as_bool).unwrap_or(false);
            if let (Some(e), false) = (&existing, replace) {
                return Err(format!(
                    "{fname} already has a file named {dname} (id {}). Pass replace: true to update it, or pick another name.",
                    s(e, "id")
                ));
            }
            let ctype = mime_guess::from_path(&local).first_or_octet_stream().to_string();
            let bytes = Arc::new(std::fs::read(&local).map_err(|e| e.to_string())?);
            let len = bytes.len().to_string();
            let hs = [("Content-Type", "application/json; charset=UTF-8"), ("X-Upload-Content-Type", ctype.as_str()), ("X-Upload-Content-Length", len.as_str())];
            // A resumable upload: first say what's coming, then send the bytes to the address Drive gives back.
            let (start_url, method, meta_body) = match &existing {
                Some(e) => (format!("{UPLOAD}/{}?uploadType=resumable&supportsAllDrives=true", enc(&s(e, "id"))), "PATCH", json!({})),
                None => (format!("{UPLOAD}?uploadType=resumable&supportsAllDrives=true"), "POST", json!({"name": dname, "parents": [fid]})),
            };
            let meta_bytes = Arc::new(meta_body.to_string().into_bytes());
            let (_, location) = api_raw(st, &a, method, &start_url, &hs, Some(meta_bytes), 1024 * 1024).await?;
            let location = location.ok_or("Drive didn't accept the upload (no upload address)")?;
            let (out, _) = api_raw(st, &a, "PUT", &location, &[("Content-Type", ctype.as_str())], Some(bytes), 1024 * 1024).await?;
            let v: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
            let verb = if existing.is_some() { "Updated" } else { "Uploaded" };
            Ok(format!("{verb} {dname} in {fname} (id {}).", s(&v, "id")))
        }
        "drive_create_folder" => {
            let (mut id, mut at) = folder(st, &a, &s(args, "parent")).await?;
            let ps = parts(&s(args, "path"));
            if ps.is_empty() {
                return Err("Give the folder's name or path".into());
            }
            let mut made = vec![];
            for p in ps {
                match child_folder(st, &a, &id, &p).await? {
                    Some(k) => id = s(&k, "id"),
                    None => {
                        let v = api(st, &a, "POST", &format!("{API}?fields=id&supportsAllDrives=true"), &[], Some(json!({"name": p, "mimeType": FOLDER, "parents": [id]}))).await?;
                        id = s(&v, "id");
                        made.push(p.clone());
                    }
                }
                at = if at == "My Drive" { p } else { format!("{at}/{p}") };
            }
            if made.is_empty() {
                return Ok(format!("{at} is already there (id {id})."));
            }
            Ok(format!("Made {at} (id {id})."))
        }
        "drive_move" => {
            let f = get_file(st, &a, &s(args, "id")).await?;
            let id = s(&f, "id");
            let new_name = s(args, "name");
            let mut url = format!("{API}/{}?fields={FIELDS}&supportsAllDrives=true", enc(&id));
            let mut to = String::new();
            if !s(args, "folder").is_empty() {
                let (fid, fname) = folder(st, &a, &s(args, "folder")).await?;
                if fid == id {
                    return Err("A folder can't go inside itself".into());
                }
                let old: Vec<String> = f.get("parents").and_then(Value::as_array).map(|p| p.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
                url.push_str(&format!("&addParents={}", enc(&fid)));
                if !old.is_empty() {
                    url.push_str(&format!("&removeParents={}", enc(&old.join(","))));
                }
                to = format!(" to {fname}");
            }
            if new_name.is_empty() && to.is_empty() {
                return Err("Give a folder to move it to, a new name, or both".into());
            }
            let body = if new_name.is_empty() { json!({}) } else { json!({"name": new_name}) };
            let v = api(st, &a, "PATCH", &url, &[], Some(body)).await?;
            let renamed = if new_name.is_empty() { String::new() } else { format!(" as {}", s(&v, "name")) };
            Ok(format!("Moved {}{to}{renamed}.", s(&f, "name")))
        }
        "drive_copy" => {
            let f = get_file(st, &a, &s(args, "id")).await?;
            if s(&f, "mimeType") == FOLDER {
                return Err("Folders can't be copied. Make a folder with drive_create_folder and copy the files into it.".into());
            }
            let (fid, fname) = folder(st, &a, &s(args, "folder")).await?;
            let name = match s(args, "name") {
                n if !n.is_empty() => n,
                _ => s(&f, "name"),
            };
            let v = api(st, &a, "POST", &format!("{API}/{}/copy?fields={FIELDS}&supportsAllDrives=true", enc(&s(&f, "id"))), &[], Some(json!({"name": name, "parents": [fid]}))).await?;
            Ok(format!("Copied {} to {fname} as {} (id {}).", s(&f, "name"), s(&v, "name"), s(&v, "id")))
        }
        "drive_trash" => {
            let f = get_file(st, &a, &s(args, "id")).await?;
            let what = if s(&f, "mimeType") == FOLDER { "folder" } else { "file" };
            let summary = format!("Move the {what} “{}” in {}'s Google Drive to the trash", s(&f, "name"), a.email);
            let input = json!({"kind": "drive", "action": "trash", "account": a.email, "name": s(&f, "name"), "id": s(&f, "id")});
            let ok = crate::api::ask_approval(st, &bot.id, "mcp__openbot__drive_trash", &summary, &input, "", "")
                .await
                .map_err(|e| e.to_string())?;
            if !ok {
                return Ok(format!("The user said no, so {} was not moved to the trash.", s(&f, "name")));
            }
            api(st, &a, "PATCH", &format!("{API}/{}?supportsAllDrives=true", enc(&s(&f, "id"))), &[], Some(json!({"trashed": true}))).await?;
            Ok(format!("Moved {} to the trash (it can be restored from Drive's trash for 30 days).", s(&f, "name")))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(quoted("it's"), "'it\\'s'");
        assert_eq!(parts("/School//CSE 21/ "), vec!["School", "CSE 21"]);
        assert!(looks_like_id("1AbC-def_ghIJKLmnopQ"));
        assert!(!looks_like_id("School/CSE 21"));
        assert_eq!(safe_name("a/b:c.pdf"), "a_b_c.pdf");
        assert_eq!(export_type("application/vnd.google-apps.document", "").unwrap().1, "pdf");
        assert_eq!(export_type("application/vnd.google-apps.spreadsheet", "csv").unwrap().1, "csv");
        assert!(export_type("application/vnd.google-apps.document", "xlsx").is_err());
        assert!(in_workdir("/bots/a", "../x").is_err());
        assert_eq!(in_workdir("/bots/a", "drive/x.pdf").unwrap(), PathBuf::from("/bots/a/drive/x.pdf"));
    }
}
