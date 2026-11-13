//! Pages a Bot shows in the chat (the show_page tool): oBot's own look for every page, the rules a
//! page must pass before it is shown, and what Bots are told about both.
//!
//! A Bot gives only the content of a page. The look lives in frontend/ui/page.css, the icons in
//! frontend/ui/vendor/lucide/icons.json and the script that draws them in frontend/ui/page.js. They
//! are added when the page is served, so a change to the look also reaches pages made earlier.
//! A page that is a whole document of its own (an existing .html file, or a page from before oBot had
//! its own look) is shown as it is.
//!
//! Pictures: every `<img src>` (a web address or a file path) is saved next to the page when the page
//! is made, in `<data>/pages/<page id>/`, so it loads on every device, needs no outside website and
//! no sign-in (a page can't sign in), and stays the same if the website changes.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use sha2::{Digest, Sha256};

/// At most this many different pictures in one page.
const MAX_PICTURES: usize = 40;

fn ui_text(name: &str) -> String {
    crate::api::ui_file(name).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// The built-in icons: name -> the inside of a 24 x 24 SVG.
fn icons() -> BTreeMap<String, String> {
    serde_json::from_str(&ui_text("vendor/lucide/icons.json")).unwrap_or_default()
}

/// What the show_page tool tells a Bot (its description). Claude Code hands a Bot an oBot tool's
/// description only when the Bot looks the tool up, so this adds nothing to messages that make no page
/// (measured with real runs: 0 tokens there, about 1,800 when a page is made). Keep it tight all the
/// same. The class names are the ones in frontend/ui/page.css.
pub fn guide() -> String {
    format!(
        "Show a page inside the chat: a visual summary, plan, comparison, timeline, diagram, chart, table or small app. \
         Give the content as HTML (or the path of an .html file).\n\
         oBot adds the look (dark, the app's font, colors, spacing), so write only content: no <html>, <head>, <body>, \
         fonts, backgrounds or colors of your own. The title shows above the page; don't repeat it.\n\
         Rules:\n\
         1. No emoji. Use an icon, a real picture, or words.\n\
         2. Mostly one color: white and grey on dark. Color only where it carries meaning (classes ok, warn, bad, info). \
         No gradients, shadows or decoration.\n\
         3. Short labels in sentence case. Exact numbers with units.\n\
         4. Show real pictures of things that have one (products, places, food): <img src> takes a web address or a \
         file path, and oBot saves a copy, so each must load now. Never an icon or drawing standing in for the thing.\n\
         5. It must read well about 700px wide, and on a phone.\n\
         Classes:\n\
         - .grid > .card: cards side by side. Inside: <h2>, <small>, <h3> (small grey label), rows.\n\
         - .head: a title line; optional <span class=\"mark\"> (a square with one icon or 1-2 initials) first.\n\
         - .stats > .stat: <div class=\"stat\"><small>Total</small><b>$1,302</b></div>\n\
         - .row: value on the right: <div class=\"row\"><div>Name<small>detail</small></div><span>$84.95</span></div>. \
         <img class=\"thumb\"> first for a small picture; class total for a sum.\n\
         - .gallery > figure: bigger pictures: <figure><img src=\"...\"><figcaption>Name<small>detail</small></figcaption></figure>. \
         Class wide for 2 or 3 large ones.\n\
         - <table>: comparisons; class num on number cells.\n\
         - .tag: a small label; add ok, warn, bad or info.\n\
         - <ol class=\"steps\">: a timeline or how-to: <li><b>Title</b>detail</li>; class done when finished.\n\
         - .flow: a process; each child is a box, arrows are drawn between them.\n\
         - .bar: <div class=\"bar\" style=\"--v:72\"><span>Label</span><b>72%</b></div> (--v is 0 to 100); .meter is the bar alone.\n\
         - .note: a side note. .sub, .muted: grey text.\n\
         - <i data-icon=\"name\"></i>: use few; the same kind of thing gets the same icon; none when no name fits. Names: {icons}.\n\
         Other diagrams and charts: inline SVG, stroke-width 1.5, stroke and fill from var(--text), var(--text-2), \
         var(--text-3), var(--line), var(--surface), series colors var(--c1) to var(--c5), labels as <text>. Add a \
         <style> only for layout these don't cover. Scripts run sandboxed, with no access to the user's data.",
        icons = icons().keys().cloned().collect::<Vec<_>>().join(", ")
    )
}

/// True for characters that show as colorful emoji pictures. Plain marks that fonts draw as text
/// (arrows, bullets, ✓ ✕ ★ ☆) are fine.
fn is_emoji(c: char) -> bool {
    matches!(
        c as u32,
        0x1F000..=0x1FAFF          // pictures, faces, flags, transport, food...
            | 0xFE0F               // "show the character before me as an emoji"
            | 0x231A | 0x231B | 0x2328 | 0x23CF | 0x23E9..=0x23F3 | 0x23F8..=0x23FA
            | 0x2B05..=0x2B07 | 0x2B1B | 0x2B1C | 0x2B50 | 0x2B55
    ) || (matches!(c as u32, 0x2600..=0x27BF) && !matches!(c as u32, 0x2605 | 0x2606 | 0x2713 | 0x2715 | 0x2717))
}

/// The names in `data-icon="..."` attributes, in the order they appear.
fn icons_used(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("data-icon") {
        rest = &rest[i + "data-icon".len()..];
        let t = rest.trim_start();
        let Some(t) = t.strip_prefix('=') else { continue };
        let name: String = t
            .trim_start()
            .trim_start_matches(['"', '\''])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// True when the page is a whole HTML document, not just content.
fn own_document(html: &str) -> bool {
    let t = html.trim_start();
    ["<!doctype", "<html"].iter().any(|p| t.get(..p.len()).is_some_and(|s| s.eq_ignore_ascii_case(p)))
}

/// Check a page (and its title) before it is shown. The error is written for the Bot, so it can fix
/// the page and call show_page again. `written` is true for HTML the Bot wrote in the tool call
/// (false for an .html file it is showing).
pub fn check(title: &str, html: &str, written: bool, emoji_ok: bool) -> Result<(), String> {
    if written && own_document(html) {
        return Err("Not shown: give only the content, starting at the first thing the user should see (no <!doctype>, \
                    <html>, <head> or <body>, and no fonts or colors of your own). oBot adds the page's look. To show an \
                    existing .html file as it is, give its path instead."
            .into());
    }
    if !emoji_ok {
        let mut found: Vec<char> = Vec::new();
        for c in title.chars().chain(html.chars()) {
            if is_emoji(c) && c as u32 != 0xFE0F && !found.contains(&c) {
                found.push(c);
            }
        }
        if !found.is_empty() {
            let shown: String = found.iter().take(8).map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
            return Err(format!(
                "Not shown: pages don't use emoji (found {shown}). Replace each one with a built-in icon \
                 (<i data-icon=\"name\"></i>), a real picture, or plain words, then call show_page again."
            ));
        }
    }
    let known = icons();
    let unknown: Vec<String> = icons_used(html).into_iter().filter(|n| !known.contains_key(n)).collect();
    if !unknown.is_empty() {
        return Err(format!(
            "Not shown: there is no icon named {}. Use one of these names, or no icon: {}.",
            unknown.join(", "),
            known.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}

/// Where the value of each `<img src>` sits in a page. Scripts, styles and comments are skipped.
fn img_srcs(html: &str) -> Vec<Range<usize>> {
    let b = html.as_bytes();
    let lower = html.to_ascii_lowercase(); // same length, so positions match
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(j) = lower[i..].find('<') {
        let at = i + j;
        let rest = &lower[at..];
        let skip_to = [("<script", "</script"), ("<style", "</style"), ("<!--", "-->")].into_iter().find(|(open, _)| rest.starts_with(open));
        if let Some((_, close)) = skip_to {
            match rest.find(close) {
                Some(k) => i = at + k + close.len(),
                None => break,
            }
            continue;
        }
        if !(rest.starts_with("<img") && b.get(at + 4).is_some_and(|c| c.is_ascii_whitespace() || *c == b'/')) {
            i = at + 1;
            continue;
        }
        // Walk the tag's attributes: name, then an optional = value (quoted or not).
        let mut k = at + 4;
        loop {
            while k < b.len() && (b[k].is_ascii_whitespace() || b[k] == b'/') {
                k += 1;
            }
            if k >= b.len() || b[k] == b'>' {
                break;
            }
            let name_start = k;
            while k < b.len() && !b[k].is_ascii_whitespace() && !matches!(b[k], b'=' | b'>' | b'/') {
                k += 1;
            }
            let name = &lower[name_start..k];
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k >= b.len() || b[k] != b'=' {
                continue;
            }
            k += 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k >= b.len() {
                break;
            }
            let (start, end);
            if b[k] == b'"' || b[k] == b'\'' {
                let quote = b[k];
                start = k + 1;
                end = b[start..].iter().position(|c| *c == quote).map_or(b.len(), |n| start + n);
                k = (end + 1).min(b.len());
            } else {
                start = k;
                while k < b.len() && !b[k].is_ascii_whitespace() && b[k] != b'>' {
                    k += 1;
                }
                end = k;
            }
            if name == "src" && end > start {
                out.push(start..end);
            }
        }
        i = k.max(at + 1);
    }
    out
}

/// The address a picture was written with, cleaned up ("&amp;" is how "&" is written in HTML).
fn src_of(html: &str, r: &Range<usize>) -> String {
    html[r.clone()].trim().replace("&amp;", "&")
}

/// Saves the pictures of a new page in `<pages_dir>/<id>/` and points each `<img src>` at oBot's copy.
/// File paths start in `base`. If a picture can't be loaded, the error tells the Bot which one and why.
pub async fn save_pictures(pages_dir: &Path, id: &str, base: &Path, html: &str) -> Result<String, String> {
    let spots = img_srcs(html);
    let mut srcs: Vec<String> = Vec::new();
    for r in &spots {
        let s = src_of(html, r);
        if !s.starts_with("data:") && !srcs.contains(&s) {
            srcs.push(s);
        }
    }
    if srcs.is_empty() {
        return Ok(html.to_string());
    }
    if srcs.len() > MAX_PICTURES {
        return Err(format!("Not shown: {} pictures is too many for one page ({MAX_PICTURES} at most). Keep the ones that matter.", srcs.len()));
    }
    let dir = pages_dir.join(id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut jobs = tokio::task::JoinSet::new();
    for (i, src) in srcs.iter().enumerate() {
        let (src, base, dir) = (src.clone(), base.to_path_buf(), dir.clone());
        jobs.spawn(async move {
            let saved = async {
                let (bytes, ext) = crate::media::load(&base, &src).await?;
                let name = format!("{}.{ext}", &crate::media::hex(&Sha256::digest(src.as_bytes()))[..16]);
                std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
                Ok::<String, String>(name)
            }
            .await;
            (i, saved)
        });
    }
    let mut saved: Vec<Result<String, String>> = vec![Err("it wasn't loaded".into()); srcs.len()];
    while let Some(Ok((i, r))) = jobs.join_next().await {
        saved[i] = r;
    }
    let failed: Vec<String> = srcs
        .iter()
        .zip(&saved)
        .filter_map(|(src, r)| r.as_ref().err().map(|e| format!("{} ({e})", src.chars().take(160).collect::<String>())))
        .collect();
    if !failed.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "Not shown: {} of the {} pictures couldn't be loaded: {}. For each one, use an address that opens the \
             picture itself (not a web page), or a picture file you saved, or leave that picture out. Then call \
             show_page again.",
            failed.len(),
            srcs.len(),
            failed.join("; ")
        ));
    }
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    for r in &spots {
        out.push_str(&html[at..r.start]);
        match srcs.iter().position(|s| *s == src_of(html, r)) {
            Some(n) => out.push_str(&format!("/pages/{id}/{}", saved[n].as_deref().unwrap_or_default())),
            None => out.push_str(&html[r.clone()]),
        }
        at = r.end;
    }
    out.push_str(&html[at..]);
    Ok(out)
}

/// A saved page, ready to send to the chat. Content gets oBot's look underneath its own styles; every
/// page gets the icons and the script that reports its height.
pub fn wrap(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 48_000);
    if own_document(html) {
        out.push_str(html);
        // `:where(html)` has no weight, so a document that sets its own colors keeps them.
        out.push_str("\n<style>:where(html){color-scheme:dark;background:#0b0b0c;color:#ececec}</style>");
    } else {
        out.push_str("<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
        out.push_str(html);
        // Styles in a layer lose to any style outside one, so a page's own styles always win.
        out.push_str("\n<style>@layer obot{");
        out.push_str(&ui_text("page.css"));
        out.push_str("}</style>");
    }
    out.push_str("\n<script>window.OBOT_ICONS=");
    out.push_str(&serde_json::to_string(&icons()).unwrap_or_else(|_| "{}".into()).replace("</", "<\\/"));
    out.push_str(";\n");
    out.push_str(&ui_text("page.js"));
    out.push_str("</script>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_are_refused_but_plain_marks_pass() {
        assert!(check("Plan", "<h2>Pants</h2><p>3 → 5, ok ✓ • 4.5 ★ ©</p>", true, false).is_ok());
        let e = check("Plan", "<h1>🛍️ Wardrobe</h1><p>👖 Pants ✅</p>", true, false).unwrap_err();
        assert!(e.contains("🛍") && e.contains("👖") && e.contains("✅"), "{e}");
        assert!(check("🛍 Plan", "<p>Hi</p>", true, false).is_err());
        assert!(check("Plan", "<p>⚠️ careful</p>", true, false).is_err());
        assert!(check("Plan", "<p>🇺🇸</p>", true, false).is_err());
        assert!(check("Emoji chart", "<p>😀 is the most used</p>", true, true).is_ok());
    }

    #[test]
    fn unknown_icons_are_refused() {
        assert!(check("Plan", "<i data-icon=\"shirt\"></i><i data-icon='check'></i><i data-icon=tag></i>", true, false).is_ok());
        let e = check("Plan", "<i data-icon=\"pants\"></i>", true, false).unwrap_err();
        assert!(e.contains("no icon named pants") && e.contains("shirt"), "{e}");
    }

    #[test]
    fn content_becomes_a_document_with_the_look() {
        let h = wrap("<h2>Hello</h2><i data-icon=\"check\"></i>");
        assert!(h.starts_with("<!doctype html>"));
        assert!(h.contains("@layer obot{") && h.contains(".card") && h.contains("OBOT_ICONS") && h.contains("obot:height"));
        // A whole document is shown as it is: no look of ours that could clash with its own class names.
        let h = wrap("<!DOCTYPE html><html><body><p>Hi</p></body></html>");
        assert!(h.starts_with("<!DOCTYPE html>") && !h.contains("@layer obot{") && h.contains("OBOT_ICONS") && h.contains("obot:height"));
    }

    #[test]
    fn finds_the_pictures_in_a_page() {
        let h = "<!-- <img src=\"no.png\"> --><IMG class=thumb alt=\"a > b\" SRC=\"https://x.com/a.jpg?w=1&amp;h=2\"><img data-src='skip.png' src='shots/one.png'/>\
                 <img src=bare.webp><script>var s = '<img src=\"js.png\">';</script><image src=\"not-img.png\"><p>src=\"text.png\"</p>";
        let got: Vec<String> = img_srcs(h).iter().map(|r| src_of(h, r)).collect();
        assert_eq!(got, ["https://x.com/a.jpg?w=1&h=2", "shots/one.png", "bare.webp"]);
        assert!(img_srcs("<img src=\"unclosed").len() == 1 && img_srcs("<img").is_empty() && img_srcs("<img src=>").is_empty());
    }

    #[tokio::test]
    async fn pictures_are_saved_next_to_the_page() {
        let tmp = std::env::temp_dir().join(format!("obot-pages-test-{}", crate::util::new_id()));
        let (pages, work) = (tmp.join("pages"), tmp.join("work"));
        std::fs::create_dir_all(work.join("shots")).unwrap();
        std::fs::write(work.join("shots/one.png"), b"\x89PNG fake").unwrap();
        let html = "<img class=\"thumb\" src=\"shots/one.png\"><p>Hi</p><img src='shots/one.png'><img src=\"data:image/png;base64,AAAA\">";
        let out = save_pictures(&pages, "abc", &work, html).await.unwrap();
        let name = format!("{}.png", &crate::media::hex(&Sha256::digest(b"shots/one.png"))[..16]);
        assert_eq!(out, format!("<img class=\"thumb\" src=\"/pages/abc/{name}\"><p>Hi</p><img src='/pages/abc/{name}'><img src=\"data:image/png;base64,AAAA\">"));
        assert_eq!(std::fs::read(pages.join("abc").join(&name)).unwrap(), b"\x89PNG fake");
        // A picture that can't be loaded stops the page, and nothing half-saved is left behind.
        let e = save_pictures(&pages, "def", &work, "<img src=\"shots/one.png\"><img src=\"shots/missing.png\">").await.unwrap_err();
        assert!(e.contains("1 of the 2 pictures") && e.contains("shots/missing.png (no such file)"), "{e}");
        assert!(!pages.join("def").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_bot_writes_content_not_a_whole_document() {
        assert!(check("Plan", "<!doctype html><html><body>Hi</body></html>", true, false).is_err());
        assert!(check("Plan", "  <HTML><body>Hi</body></html>", true, false).is_err());
        assert!(check("Report", "<!doctype html><html><body>Hi</body></html>", false, false).is_ok());
    }

    #[test]
    fn the_guide_names_every_icon() {
        let g = guide();
        assert!(icons().len() > 50);
        for name in icons().keys() {
            assert!(g.contains(name.as_str()), "{name}");
        }
    }
}
