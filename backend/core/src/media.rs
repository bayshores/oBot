//! Pictures in Bot replies. A Bot shows a picture by writing it into its reply as markdown:
//! `![what it shows](https://… or a file path)`. Before the reply is shown, each picture is saved
//! in the Bot's folder under "shown/" (web pictures are downloaded, files are copied), so it loads
//! on every device, needs no outside website, and stays the same if the website changes.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use sha2::{Digest, Sha256};

use crate::state::Shared;

/// At most this many pictures per reply; the rest become plain links.
const MAX_PICTURES: usize = 12;
const MAX_BYTES: u64 = 15 * 1024 * 1024;

/// One picture written in a reply.
struct Picture {
    range: Range<usize>,
    alt: String,
    src: String,
}

/// Saves the pictures in `text` into the Bot's folder and points them at oBot's own copy.
/// Pictures that can't be saved become a link (web) or their description (missing file).
pub async fn save_pictures(state: &Shared, bot_id: &str, text: &str) -> String {
    if !text.contains("![") {
        return text.to_string();
    }
    let Some(bot) = state.db.get_bot(bot_id).ok().flatten() else { return text.to_string() };
    let pics = find_pictures(text);
    if pics.is_empty() {
        return text.to_string();
    }
    let workdir = PathBuf::from(&bot.workdir);
    let mut jobs = tokio::task::JoinSet::new();
    for (i, p) in pics.iter().enumerate() {
        let (src, workdir, bot_id) = (p.src.clone(), workdir.clone(), bot_id.to_string());
        let allowed = i < MAX_PICTURES;
        jobs.spawn(async move {
            if !allowed {
                return (i, Err(()));
            }
            let r = save_one(&workdir, &bot_id, &src).await;
            if let Err(e) = &r {
                let shown: String = src.chars().take(120).collect();
                eprintln!("Couldn't show a picture from {shown}: {e}");
            }
            (i, r.map_err(|_| ()))
        });
    }
    let mut saved = vec![Err(()); pics.len()];
    while let Some(Ok((i, r))) = jobs.join_next().await {
        saved[i] = r;
    }

    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (p, r) in pics.iter().zip(saved) {
        out.push_str(&text[at..p.range.start]);
        let alt = escape_alt(&p.alt);
        match r {
            Ok(url) => out.push_str(&format!("![{alt}]({url})")),
            Err(()) if is_web(&p.src) => out.push_str(&format!("[{}](<{}>)", if alt.is_empty() { "Picture" } else { &alt }, p.src)),
            Err(()) => out.push_str(&alt),
        }
        at = p.range.end;
    }
    out.push_str(&text[at..]);
    out
}

fn find_pictures(text: &str) -> Vec<Picture> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut pics: Vec<Picture> = Vec::new();
    let mut open: Option<Picture> = None;
    for (ev, range) in Parser::new_ext(text, opts).into_offset_iter() {
        match ev {
            Event::Start(Tag::Image { dest_url, .. }) if open.is_none() => {
                open = Some(Picture { range, alt: String::new(), src: dest_url.trim().to_string() });
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some(p) = open.as_mut() {
                    p.alt.push_str(&t);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some(p) = open.take() {
                    // Already one of oBot's own copies (a reply shown before).
                    if !p.src.starts_with("/api/bots/") {
                        pics.push(p);
                    }
                }
            }
            _ => {}
        }
    }
    pics
}

fn is_web(src: &str) -> bool {
    let s = src.to_ascii_lowercase();
    s.starts_with("https://") || s.starts_with("http://")
}

fn escape_alt(alt: &str) -> String {
    alt.split_whitespace().collect::<Vec<_>>().join(" ").replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]")
}

/// Saves one picture into `<workdir>/shown/` and returns its address in the chat.
async fn save_one(workdir: &Path, bot_id: &str, src: &str) -> Result<String, String> {
    let root = std::fs::canonicalize(workdir).map_err(|e| format!("the Bot's folder: {e}"))?;
    let rel = if is_web(src) {
        let (bytes, mime) = download(src.to_string()).await?;
        let ext = picture_ext(&mime, src).ok_or_else(|| format!("it isn't a picture ({mime})"))?;
        let name = format!("{}.{ext}", &hex(&Sha256::digest(src.as_bytes()))[..16]);
        let dir = root.join("shown");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
        format!("shown/{name}")
    } else {
        let real = local_picture(&root, src)?;
        if real.starts_with(&root) {
            real.strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/")
        } else {
            // Files outside the Bot's folder are copied in so the chat can show them.
            let ext = real.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let name = format!("{}.{ext}", &hex(&Sha256::digest(real.to_string_lossy().as_bytes()))[..16]);
            let dir = root.join("shown");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::copy(&real, dir.join(&name)).map_err(|e| e.to_string())?;
            format!("shown/{name}")
        }
    };
    Ok(format!("/api/bots/{bot_id}/files/{}", crate::util::url_path(&rel)))
}

/// The picture file a Bot named by its path (a relative path starts in `root`).
fn local_picture(root: &Path, src: &str) -> Result<PathBuf, String> {
    let p = src.strip_prefix("file://").unwrap_or(src);
    // "file:///C:/Users/…" -> "C:/Users/…"
    let p = if p.len() > 2 && p.starts_with('/') && p.as_bytes()[2] == b':' { &p[1..] } else { p };
    let mut path = crate::util::from_wsl_path(p);
    if path.is_relative() {
        path = root.join(path);
    }
    let real = std::fs::canonicalize(&path).map_err(|_| "no such file".to_string())?;
    let meta = std::fs::metadata(&real).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_BYTES || picture_ext(&mime_of(&real), "").is_none() {
        return Err("not a picture file (or bigger than 15 MB)".into());
    }
    Ok(real)
}

/// A picture's bytes and file extension, from a web address (downloaded) or a file path (a relative
/// path starts in `root`). Used for the pictures in a page (pages.rs).
pub(crate) async fn load(root: &Path, src: &str) -> Result<(Vec<u8>, &'static str), String> {
    if is_web(src) {
        let (bytes, mime) = download(src.to_string()).await?;
        let ext = picture_ext(&mime, src).ok_or_else(|| format!("it isn't a picture ({mime})"))?;
        return Ok((bytes, ext));
    }
    let real = local_picture(root, src)?;
    let ext = picture_ext(&mime_of(&real), "").ok_or("not a picture file")?;
    Ok((std::fs::read(&real).map_err(|e| e.to_string())?, ext))
}

fn mime_of(p: &Path) -> String {
    mime_guess::from_path(p).first_or_octet_stream().to_string()
}

/// The file extension for a picture, or None when it isn't a picture a browser shows.
fn picture_ext(mime: &str, url: &str) -> Option<&'static str> {
    let mime = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let from_mime = match mime.as_str() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/avif" => Some("avif"),
        "image/svg+xml" => Some("svg"),
        "image/bmp" => Some("bmp"),
        _ => None,
    };
    from_mime.or_else(|| {
        // Some picture servers say "application/octet-stream": trust the address's extension then.
        let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
        ["jpg", "jpeg", "png", "gif", "webp", "avif"].into_iter().find(|e| path.ends_with(&format!(".{e}"))).map(|e| if e == "jpeg" { "jpg" } else { e })
    })
}

/// Downloads a picture. A busy or briefly failing server gets two more tries.
async fn download(url: String) -> Result<(Vec<u8>, String), String> {
    let mut last = String::new();
    for wait in [0, 1500, 4000] {
        tokio::time::sleep(Duration::from_millis(wait)).await;
        let u = url.clone();
        match tokio::task::spawn_blocking(move || fetch(&u)).await.map_err(|e| e.to_string())? {
            Ok(got) => return Ok(got),
            Err((again, e)) => {
                last = e;
                if !again {
                    break;
                }
            }
        }
    }
    Err(last)
}

/// One download. The error says whether trying again could help.
fn fetch(url: &str) -> Result<(Vec<u8>, String), (bool, String)> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout(Duration::from_secs(20))
        // Some picture servers refuse requests that don't look like a browser.
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .build();
    let resp = match agent.get(url).set("Accept", "image/avif,image/webp,image/png,image/*;q=0.8").call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err((code == 429 || code >= 500, format!("the website answered {code}"))),
        Err(e) => return Err((true, e.to_string())),
    };
    let mime = resp.content_type().to_string();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), MAX_BYTES + 1), &mut bytes).map_err(|e| (true, e.to_string()))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err((false, "empty, or bigger than 15 MB".into()));
    }
    Ok((bytes, mime))
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_pictures_and_their_descriptions() {
        let t = "Two picks:\n\n![Gap *baggy* jeans](https://cdn.example.com/a.jpg) and ![](<C:\\Users\\me\\shot one.png>)\n\n`![not](a picture)`\n![again](/api/bots/b1/files/shown/x.jpg)";
        let p = find_pictures(t);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].alt, "Gap baggy jeans");
        assert_eq!(p[0].src, "https://cdn.example.com/a.jpg");
        assert_eq!(&t[p[0].range.clone()], "![Gap *baggy* jeans](https://cdn.example.com/a.jpg)");
        assert_eq!(p[1].src, "C:\\Users\\me\\shot one.png");
    }

    #[test]
    fn picture_types() {
        assert_eq!(picture_ext("image/jpeg; charset=binary", ""), Some("jpg"));
        assert_eq!(picture_ext("application/octet-stream", "https://x.com/p/1.WEBP?w=400"), Some("webp"));
        assert_eq!(picture_ext("text/html", "https://x.com/page"), None);
        assert_eq!(escape_alt("a [b]\n c"), "a \\[b\\] c");
    }
}
