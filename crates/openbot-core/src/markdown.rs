//! Markdown -> safe HTML for bot messages (done in Rust so the UI stays small and fast).

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag};

fn safe_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:") || u.starts_with('#')
}

pub fn render(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, opts).map(|ev| match ev {
        // Never pass raw HTML through: show it as text instead.
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
            let dest = if safe_url(&dest_url) { dest_url } else { CowStr::from("#") };
            Event::Start(Tag::Link { link_type, dest_url: dest, title, id })
        }
        Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
            let dest = if safe_url(&dest_url) { dest_url } else { CowStr::from("#") };
            Event::Start(Tag::Image { link_type, dest_url: dest, title, id })
        }
        other => other,
    });

    let mut out = String::with_capacity(md.len() * 2);
    html::push_html(&mut out, parser);
    out
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
}
