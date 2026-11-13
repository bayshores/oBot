//! Markdown -> safe HTML for bot messages (done in Rust so the UI stays small and fast).

use pulldown_cmark::{html, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, TextMergeStream};

/// Change this when `render` makes different HTML, so saved messages are rendered again (db.rs).
pub const VERSION: &str = "3";

fn safe_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:") || u.starts_with('#')
}

pub fn render(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    // Inside a link, an image or a code block, web addresses stay plain text.
    let mut no_links = 0usize;
    let mut events: Vec<Event> = Vec::new();
    for ev in TextMergeStream::new(Parser::new_ext(md, opts)) {
        match ev {
            // Never pass raw HTML through: show it as text instead.
            Event::Html(s) | Event::InlineHtml(s) => events.push(Event::Text(s)),
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                no_links += 1;
                let dest = if safe_url(&dest_url) { dest_url } else { CowStr::from("#") };
                events.push(Event::Start(Tag::Link { link_type, dest_url: dest, title, id }));
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                no_links += 1;
                // Pictures a Bot showed are saved in its folder (media.rs) and served by oBot.
                let ours = dest_url.starts_with("/api/bots/") && !dest_url.contains("..");
                let dest = if ours || safe_url(&dest_url) { dest_url } else { CowStr::from("#") };
                events.push(Event::Start(Tag::Image { link_type, dest_url: dest, title, id }));
            }
            Event::Start(t @ Tag::CodeBlock(_)) => {
                no_links += 1;
                events.push(Event::Start(t));
            }
            Event::End(t @ (TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock)) => {
                no_links = no_links.saturating_sub(1);
                events.push(Event::End(t));
            }
            Event::Text(s) if no_links == 0 => linkify(&s, &mut events),
            other => events.push(other),
        }
    }

    let mut out = String::with_capacity(md.len() * 2);
    html::push_html(&mut out, events.into_iter());
    out
}

/// Turns web addresses written as plain text ("https://…", "www.…") into links. The link shows a
/// short version of the address; the full one appears when you point at it.
fn linkify(text: &str, out: &mut Vec<Event<'_>>) {
    let mut rest = 0;
    for (start, end) in find_urls(text) {
        if start > rest {
            out.push(Event::Text(CowStr::from(text[rest..start].to_string())));
        }
        let shown = &text[start..end];
        let dest = if shown.to_ascii_lowercase().starts_with("www.") { format!("https://{shown}") } else { shown.to_string() };
        out.push(Event::Start(Tag::Link {
            link_type: LinkType::Autolink,
            dest_url: CowStr::from(dest.clone()),
            title: CowStr::from(dest),
            id: CowStr::from(""),
        }));
        out.push(Event::Text(CowStr::from(short_url(shown))));
        out.push(Event::End(TagEnd::Link));
        rest = end;
    }
    if rest < text.len() {
        out.push(Event::Text(CowStr::from(text[rest..].to_string())));
    }
}

/// Byte ranges of the web addresses in `text`.
fn find_urls(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let at = &lower[i..];
        let prefix = ["https://", "http://", "www."].into_iter().find(|p| at.starts_with(p));
        let after_word = text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '/' || c == '@');
        let Some(prefix) = prefix.filter(|_| !after_word) else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let mut end = i + text[i..].find(|c: char| c.is_whitespace() || "<>\"`".contains(c)).unwrap_or(text.len() - i);
        // Leave sentence punctuation and unmatched closing brackets outside the link.
        loop {
            let url = &text[i..end];
            let Some(last) = url.chars().next_back() else { break };
            let unmatched = |open: char, close: char| last == close && url.matches(close).count() > url.matches(open).count();
            if ".,;:!?'*_~".contains(last) || unmatched('(', ')') || unmatched('[', ']') {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        let host = text[i + prefix.len()..end].split(['/', '?', '#']).next().unwrap_or("");
        if host.contains('.') || host.starts_with("localhost") {
            found.push((i, end));
            i = end;
        } else {
            i += prefix.len();
        }
    }
    found
}

/// "https://www.example.com/a/very/long/path?x=1" -> "example.com/a/very/long/pa…"
fn short_url(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let mut s = url;
    for p in ["https://", "http://", "www."] {
        if lower[url.len() - s.len()..].starts_with(p) {
            s = &s[p.len()..];
        }
    }
    let s = s.strip_suffix('/').unwrap_or(s);
    const MAX: usize = 48;
    if s.chars().count() > MAX {
        format!("{}…", s.chars().take(MAX - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn escapes_html_and_bad_links() {
        let h = render("<script>alert(1)</script>\n\nhi <img src=x onerror=alert(1)> [x](javascript:alert(1)) **b**");
        assert!(!h.contains("<script>"));
        assert!(!h.contains("<img"));
        assert!(!h.contains("href=\"javascript:"));
        assert!(h.contains("<strong>b</strong>"));
    }

    #[test]
    fn plain_web_addresses_become_links() {
        let h = render("Read https://ucsd.edu/about/principles.html. Then sign (https://example.com/a_(b)).");
        assert!(h.contains(r#"<a href="https://ucsd.edu/about/principles.html" title="https://ucsd.edu/about/principles.html">ucsd.edu/about/principles.html</a>. Then"#), "{h}");
        assert!(h.contains(r#"href="https://example.com/a_(b)""#), "{h}");
        assert!(h.contains("</a>).</p>"), "{h}");

        let long = render("**1. Oath**\nhttps://powerforms.docusign.net/42fc0e92-d004-4d1c-b51d-9b693d3cf594?env=na1&acct=6371e373");
        assert!(long.contains(r#"href="https://powerforms.docusign.net/42fc0e92-d004-4d1c-b51d-9b693d3cf594?env=na1&amp;acct=6371e373""#), "{long}");
        assert!(long.contains(">powerforms.docusign.net/42fc0e92-d004-4d1c-b51d…</a>"), "{long}");

        assert!(render("go to www.example.org/x").contains(r#"href="https://www.example.org/x""#));
    }

    #[test]
    fn addresses_in_links_and_code_stay_as_they_are() {
        let h = render("[the form](https://a.example.com/f) and `https://b.example.com` and\n\n```\ncurl https://c.example.com\n```");
        assert_eq!(h.matches("<a ").count(), 1, "{h}");
        assert!(h.contains(r#"<a href="https://a.example.com/f">the form</a>"#), "{h}");
        assert!(!render("email me at me@www.x.com or see https://nohost").contains("<a "));
    }
}
