//! Vault logins and contact info (Settings › Vault), and the Bot tools that use them.
//!
//! - A **login** is up to 10 websites, a username, a password and hints. Bots see the websites,
//!   usernames and hints (`list_logins`) but never the password: `fill_login` has oBot type it
//!   straight into the login page on the Bot's screen, and only when that page is `https://` on one
//!   of the login's websites or a subdomain (so a look-alike page on another website gets nothing).
//!   Passwords are saved encrypted (see vault.rs) and can't be read back, not even in the app.
//! - **Contact info** (name, address, phones, emails) is visible to Bots so they can fill forms.
//! - Logins can be imported from a password manager's CSV export (the app reads the file and sends
//!   the rows; `import` decides which are new).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::Config;
use crate::state::Shared;
use crate::util::{new_id, now_ms, truncate_chars};

pub const MAX_LOGINS: usize = 2000;
pub const MAX_CONTACTS: usize = 50;
const MAX_SITES: usize = 10;
const MAX_LIST: usize = 10;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Login {
    pub id: String,
    /// Hosts like "ucsd.edu" (also fills on its subdomains, like sso.ucsd.edu).
    pub sites: Vec<String>,
    pub username: String,
    /// Saved encrypted in config.json; plain text in memory. Never sent to the app or a Bot.
    #[serde(with = "crate::vault::sealed")]
    pub password: String,
    /// "Tell the Bot when to use this login".
    pub hints: String,
    /// Which Bots may use it (their ids). None = every Bot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bots: Option<Vec<String>>,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Contact {
    pub id: String,
    pub full_name: String,
    pub address1: String,
    pub address2: String,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    pub country: String,
    /// The first one is the primary number.
    pub phones: Vec<String>,
    /// The first one is the primary address.
    pub emails: Vec<String>,
    pub label: String,
    pub hints: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bots: Option<Vec<String>>,
    pub updated_at: i64,
}

// ------------------------------------------------------------------ websites

/// "https://SSO.ucsd.edu/login" or "*.ucsd.edu" -> "sso.ucsd.edu" / "ucsd.edu" ("www." is dropped).
/// Err = a sentence for the user.
pub fn normalize_site(input: &str) -> Result<String, String> {
    let s = input.trim().to_lowercase();
    let s = s.split_once("://").map(|x| x.1).unwrap_or(&s);
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map(|x| x.1).unwrap_or(host);
    let host = host.split(':').next().unwrap_or("").trim_start_matches("*.").trim_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host);
    let ok = host.contains('.')
        && host.len() <= 253
        && host.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if !ok {
        return Err(format!("“{}” isn't a website address. Use something like example.com", truncate_chars(input.trim(), 60)));
    }
    Ok(host.to_string())
}

/// The host of an https:// page address (None for anything else, including http://).
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map(|x| x.1).unwrap_or(host);
    let host = host.split(':').next()?.to_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Is `host` one of `sites` or a subdomain of one?
pub fn site_allowed(sites: &[String], host: &str) -> bool {
    sites.iter().any(|s| host == s || host.ends_with(&format!(".{s}")))
}

fn may_use(bots: &Option<Vec<String>>, bot_id: &str) -> bool {
    bots.as_ref().is_none_or(|ids| ids.iter().any(|i| i == bot_id))
}

/// Logins a Bot may use (ones whose password can be decrypted on this computer).
pub fn for_bot<'a>(cfg: &'a Config, bot_id: &str) -> Vec<&'a Login> {
    cfg.logins.iter().filter(|l| may_use(&l.bots, bot_id) && !crate::vault::is_locked(&l.password)).collect()
}

pub fn contacts_for<'a>(cfg: &'a Config, bot_id: &str) -> Vec<&'a Contact> {
    cfg.contacts.iter().filter(|c| may_use(&c.bots, bot_id)).collect()
}

/// What the app may see of a login: everything but the password.
pub fn view(l: &Login) -> Value {
    let locked = crate::vault::is_locked(&l.password);
    json!({"id": l.id, "sites": l.sites, "username": l.username, "hints": l.hints, "bots": l.bots,
           "has_password": !l.password.is_empty(), "locked": locked, "updated_at": l.updated_at})
}

// ------------------------------------------------------------------ saving

/// What the app sends to add or edit a login.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LoginIn {
    pub sites: Vec<String>,
    pub username: String,
    /// Empty when editing = keep the saved password.
    pub password: String,
    pub hints: String,
    pub bots: Option<Vec<String>>,
}

/// Check and tidy a login from the app. `old` = the login being edited.
pub fn clean_login(input: LoginIn, old: Option<&Login>, known_bots: &[String]) -> Result<Login, String> {
    let mut sites: Vec<String> = vec![];
    for s in input.sites.iter().flat_map(|s| s.split_whitespace()).filter(|s| !s.is_empty()) {
        let s = normalize_site(s)?;
        if !sites.contains(&s) {
            sites.push(s);
        }
    }
    if sites.is_empty() {
        return Err("Add the website this login is for".into());
    }
    if sites.len() > MAX_SITES {
        return Err(format!("A login can have at most {MAX_SITES} websites"));
    }
    if input.username.chars().count() > 300 {
        return Err("That username is too long".into());
    }
    if input.password.len() > 4096 {
        return Err("That password is too long".into());
    }
    if input.hints.chars().count() > 2000 {
        return Err("Keep the hints under 2000 characters".into());
    }
    let password = if input.password.is_empty() {
        match old {
            Some(o) if crate::vault::is_locked(&o.password) => return Err("This password can't be unlocked on this computer. Enter it again.".into()),
            Some(o) if !o.password.is_empty() => o.password.clone(),
            _ => return Err("Enter the password".into()),
        }
    } else {
        input.password
    };
    Ok(Login {
        id: old.map(|o| o.id.clone()).unwrap_or_else(new_id),
        sites,
        username: input.username.trim().to_string(),
        password,
        hints: input.hints.trim().to_string(),
        bots: clean_bots(input.bots, known_bots)?,
        updated_at: now_ms(),
    })
}

/// Check and tidy contact info from the app.
pub fn clean_contact(mut c: Contact, old: Option<&Contact>, known_bots: &[String]) -> Result<Contact, String> {
    let short = |v: &str, n: usize, what: &str| -> Result<String, String> {
        if v.trim().chars().count() > n {
            return Err(format!("{what} is too long"));
        }
        Ok(v.trim().to_string())
    };
    c.full_name = short(&c.full_name, 200, "The name")?;
    c.address1 = short(&c.address1, 300, "The address")?;
    c.address2 = short(&c.address2, 300, "Address line 2")?;
    c.city = short(&c.city, 120, "The city")?;
    c.region = short(&c.region, 120, "The state or region")?;
    c.postal_code = short(&c.postal_code, 40, "The ZIP or postal code")?;
    c.country = short(&c.country, 80, "The country")?;
    c.label = short(&c.label, 80, "The label")?;
    c.hints = short(&c.hints, 2000, "The hints")?;
    let mut phones: Vec<String> = vec![];
    for p in c.phones.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let digits = p.chars().filter(char::is_ascii_digit).count();
        if !(7..=15).contains(&digits) || !p.chars().all(|ch| ch.is_ascii_digit() || " +-().".contains(ch)) {
            return Err(format!("“{}” isn't a valid phone number", truncate_chars(p, 40)));
        }
        if !phones.iter().any(|x| x == p) {
            phones.push(p.to_string());
        }
    }
    let mut emails: Vec<String> = vec![];
    for e in c.emails.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        let ok = e.len() <= 254 && !e.contains(char::is_whitespace) && e.split_once('@').is_some_and(|(a, d)| !a.is_empty() && d.contains('.'));
        if !ok {
            return Err(format!("“{}” isn't a valid email address", truncate_chars(e, 60)));
        }
        if !emails.iter().any(|x| x.eq_ignore_ascii_case(e)) {
            emails.push(e.to_string());
        }
    }
    if phones.len() > 10 || emails.len() > 10 {
        return Err("Add at most 10 phone numbers and 10 emails".into());
    }
    c.phones = phones;
    c.emails = emails;
    let empty = [&c.full_name, &c.address1, &c.city, &c.postal_code, &c.label].iter().all(|v| v.is_empty()) && c.phones.is_empty() && c.emails.is_empty();
    if empty {
        return Err("Fill in at least a name, an address, a phone number or an email".into());
    }
    c.bots = clean_bots(c.bots, known_bots)?;
    c.id = old.map(|o| o.id.clone()).unwrap_or_else(new_id);
    c.updated_at = now_ms();
    Ok(c)
}

fn clean_bots(bots: Option<Vec<String>>, known: &[String]) -> Result<Option<Vec<String>>, String> {
    match bots {
        None => Ok(None),
        Some(ids) => {
            let mut ids: Vec<String> = ids.into_iter().filter(|i| known.contains(i)).collect();
            ids.dedup();
            if ids.is_empty() {
                return Err("Pick at least one Bot, or choose All Bots".into());
            }
            Ok(Some(ids))
        }
    }
}

/// Save a password the user typed into a secure form (see secure_form.rs): updates the login with
/// the same website and username, or adds one. Returns true if it updated an existing login.
pub fn save_from_form(cfg: &mut Config, host: &str, username: &str, password: &str) -> Result<bool, String> {
    let site = normalize_site(host)?;
    if let Some(l) = cfg.logins.iter_mut().find(|l| site_allowed(&l.sites, &site) && l.username == username) {
        l.password = password.to_string();
        l.updated_at = now_ms();
        return Ok(true);
    }
    if cfg.logins.len() >= MAX_LOGINS {
        return Err("The Vault is full".into());
    }
    cfg.logins.push(Login {
        id: new_id(),
        sites: vec![site],
        username: username.to_string(),
        password: password.to_string(),
        hints: String::new(),
        bots: None,
        updated_at: now_ms(),
    });
    Ok(false)
}

// ------------------------------------------------------------------ import

/// One row of a password export, as the app read it.
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct ImportRow {
    pub url: String,
    pub username: String,
    pub password: String,
    pub name: String,
}

/// Check rows of a password export. With `apply`, add the ready ones (up to the Vault's limit).
/// Returns {ready, duplicates, errors, skipped_full, items:[{row, title, username, status, error?}]}.
pub fn import(cfg: &mut Config, rows: &[ImportRow], apply: bool) -> Value {
    let mut seen: Vec<(String, String)> = cfg.logins.iter().flat_map(|l| l.sites.iter().map(move |s| (s.clone(), l.username.clone()))).collect();
    let (mut ready, mut dupes, mut errors, mut full) = (0, 0, 0, 0);
    let mut items = vec![];
    for (i, r) in rows.iter().enumerate() {
        let site = normalize_site(&r.url);
        let title = match &site {
            Ok(s) => s.clone(),
            Err(_) if !r.name.trim().is_empty() => truncate_chars(r.name.trim(), 80),
            Err(_) => truncate_chars(r.url.trim(), 80),
        };
        let username = truncate_chars(r.username.trim(), 300);
        let problem = match &site {
            _ if r.password.is_empty() => Some("Missing password"),
            _ if r.password.len() > 4096 => Some("Password too long"),
            Err(_) => Some("Missing website"),
            Ok(_) => None,
        };
        let status = if let Some(p) = problem {
            errors += 1;
            items.push(json!({"row": i, "title": title, "username": username, "status": "error", "error": p}));
            continue;
        } else if seen.iter().any(|(s, u)| site.as_ref().is_ok_and(|x| x == s) && *u == username) {
            dupes += 1;
            "duplicate"
        } else if cfg.logins.len() >= MAX_LOGINS {
            full += 1;
            "full"
        } else {
            ready += 1;
            let site = site.clone().unwrap_or_default();
            seen.push((site.clone(), username.clone()));
            if apply {
                cfg.logins.push(Login {
                    id: new_id(),
                    sites: vec![site],
                    username: username.clone(),
                    password: r.password.clone(),
                    hints: String::new(),
                    bots: None,
                    updated_at: now_ms(),
                });
            }
            "ready"
        };
        items.push(json!({"row": i, "title": title, "username": username, "status": status}));
    }
    json!({"ready": ready, "duplicates": dupes, "errors": errors, "skipped_full": full, "items": items})
}

// ------------------------------------------------------------------ Bot tools

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({"name": "list_logins",
               "description": "List the website logins the user saved for you in the Vault: id, websites, username and hints (never the password). Give \"site\" (like ucsd.edu) to see only logins for that website.",
               "inputSchema": {"type": "object", "properties": {"site": {"type": "string"}}}}),
        json!({"name": "fill_login",
               "description": "Sign in with a saved login: oBot types its username and password into the login page in front on your screen. You never see the password. Works only on https pages of that login's websites (or their subdomains). Open the login page first; on a two-step login (username first, password on the next page) call it again on each page. Then click the sign-in button yourself. Leave out \"id\" when only one saved login is for this website: oBot picks it.",
               "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "description": "The login's id from list_logins (needed only when several logins are for this website)"}}}}),
        json!({"name": "list_contact_info",
               "description": "The user's saved contact info from the Vault (name, address, phone numbers, emails), for filling in forms. Type it in yourself with computer_type.",
               "inputSchema": {"type": "object", "properties": {}}}),
    ]
}

pub fn is_tool(name: &str) -> bool {
    matches!(name, "list_logins" | "fill_login" | "list_contact_info")
}

fn describe(l: &Login) -> String {
    let user = if l.username.is_empty() { "(no username)".to_string() } else { l.username.clone() };
    let hints = if l.hints.is_empty() { String::new() } else { format!(" · hints: {}", truncate_chars(&l.hints, 300)) };
    format!("- [{}] {} · username: {user}{hints}", l.id, l.sites.join(", "))
}

pub async fn tool(st: &Shared, bot_id: &str, name: &str, args: &Value) -> Result<String, String> {
    let cfg = st.config.read().await.clone();
    match name {
        "list_logins" => {
            let site = args.get("site").and_then(Value::as_str).unwrap_or("").trim();
            let mine = for_bot(&cfg, bot_id);
            let found: Vec<&&Login> = if site.is_empty() {
                mine.iter().collect()
            } else {
                let host = normalize_site(site)?;
                mine.iter().filter(|l| site_allowed(&l.sites, &host) || l.sites.iter().any(|s| s.ends_with(&format!(".{host}")))).collect()
            };
            if found.is_empty() {
                let what = if site.is_empty() { String::new() } else { format!(" for {site}") };
                return Ok(format!(
                    "No saved logins{what}. To sign in, open the login page and call request_form to ask the user for it \
                     securely (they can save it to the Vault from there)."
                ));
            }
            let more = found.len().saturating_sub(MAX_LIST * 5);
            let mut out: Vec<String> = found.iter().take(MAX_LIST * 5).map(|l| describe(l)).collect();
            if more > 0 {
                out.push(format!("…and {more} more. Give \"site\" to narrow it down."));
            }
            Ok(out.join("\n"))
        }
        "fill_login" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
            let mine = for_bot(&cfg, bot_id);
            if mine.is_empty() {
                return Err("The user hasn't saved any logins for you. Open the login page and call request_form to ask for it securely.".into());
            }
            let page = crate::screen::page_inspect(st, bot_id).await?;
            let host = page.host.ok_or_else(|| format!("The page in front ({}) isn't a secure https:// page, so the login can't be filled there.", page.url))?;
            let (l, note) = pick(&mine, id, &host)?;
            let mut fields = vec![];
            if !l.username.is_empty() {
                fields.push(crate::screen::Fill { target: "username".into(), value: l.username.clone() });
            }
            fields.push(crate::screen::Fill { target: "password".into(), value: l.password.clone() });
            let r = crate::screen::page_fill(st, bot_id, &host, &fields).await?;
            if r.filled.is_empty() {
                return Err(format!("Couldn't find a username or password box on the page on {host}. Make sure the login form is showing."));
            }
            let missing = if r.missing.is_empty() { String::new() } else { format!(" No {} box on this page (normal for a two-step login: call fill_login again on the next page).", r.missing.join(" or ")) };
            Ok(format!("{note}Filled the {} on {host}.{missing} Now click the sign-in button and check the result with computer_screenshot.", r.filled.join(" and ")))
        }
        "list_contact_info" => {
            let mine = contacts_for(&cfg, bot_id);
            if mine.is_empty() {
                return Ok("The user hasn't saved contact info in the Vault. Ask them with ask_user if you need it.".into());
            }
            Ok(mine.iter().map(|c| contact_text(c)).collect::<Vec<_>>().join("\n\n"))
        }
        _ => Err(format!("Unknown tool {name}")),
    }
}

/// Which login `fill_login` uses on a page on `host`: the one with `id`, or, when the id is left out
/// or unknown (an old id remembered from before, say), the only login for that website. The note
/// tells the Bot which one it got when its id didn't match.
fn pick(mine: &[&Login], id: &str, host: &str) -> Result<(Login, String), String> {
    if let Some(l) = mine.iter().find(|l| !id.is_empty() && l.id == id) {
        if !site_allowed(&l.sites, host) {
            return Err(format!(
                "This login is for {}, but the page in front is on {host}, so nothing was filled. If {host} really is that \
                 website's login page, ask the user to add it to the login in Settings › Vault.",
                l.sites.join(", ")
            ));
        }
        return Ok(((*l).clone(), String::new()));
    }
    let fits: Vec<&&Login> = mine.iter().filter(|l| site_allowed(&l.sites, host)).collect();
    let unknown = if id.is_empty() { String::new() } else { format!("No saved login has the id {id}. ") };
    match fits.as_slice() {
        [] => Err(format!(
            "{unknown}No saved login is for {host}, so nothing was filled. Call request_form to ask the user for it securely."
        )),
        [l] => {
            let note = if id.is_empty() { String::new() } else { format!("{unknown}Used the login for {host} ({}, id {}). ", l.username, l.id) };
            Ok(((**l).clone(), note))
        }
        many => Err(format!(
            "{unknown}Several saved logins are for {host}; call fill_login again with one of these ids:\n{}",
            many.iter().map(|l| describe(l)).collect::<Vec<_>>().join("\n")
        )),
    }
}

fn contact_text(c: &Contact) -> String {
    let mut lines = vec![format!("[{}] {}", c.id, if c.label.is_empty() { "Contact info" } else { &c.label })];
    let mut add = |k: &str, v: &str| {
        if !v.is_empty() {
            lines.push(format!("{k}: {v}"));
        }
    };
    add("Name", &c.full_name);
    let place = [c.city.as_str(), c.region.as_str(), c.postal_code.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(", ");
    let address = [c.address1.as_str(), c.address2.as_str(), place.as_str(), c.country.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(", ");
    add("Address", &address);
    add("Phones", &c.phones.join(", "));
    add("Emails", &c.emails.join(", "));
    add("Hints", &c.hints);
    lines.join("\n")
}

/// The Vault part of a Bot's instructions.
pub fn persona(cfg: &Config, bot_id: &str) -> String {
    let logins = for_bot(cfg, bot_id).len();
    let contacts = contacts_for(cfg, bot_id).len();
    format!(
        "\nVault: {} and {} saved for you. To sign in to a website on your screen: open its login page, call \
         list_logins with the site, then fill_login (you never see passwords). If there's no saved login, or you need a \
         2FA code or a new password, call request_form so the user types it into a secure form in the chat; it goes \
         straight into the page. Never ask the user to type a password or code in the chat itself.\n",
        count(logins, "login", "logins"),
        count(contacts, "contact info entry", "contact info entries"),
    )
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_login_picks_the_login_for_the_page() {
        let mk = |id: &str, site: &str, user: &str| Login { id: id.into(), sites: vec![site.into()], username: user.into(), ..Default::default() };
        let (a, b, c) = (mk("a1", "ucsd.edu", "sesla"), mk("b2", "ucsd.edu", "seesla"), mk("c3", "shop.com", "me"));
        // The right id works, an id for another website doesn't.
        assert_eq!(pick(&[&a, &c], "a1", "a5.ucsd.edu").unwrap().0.id, "a1");
        assert!(pick(&[&a, &c], "c3", "a5.ucsd.edu").is_err());
        // No id, or an old one: the only login for the page is used, and the Bot is told which.
        let (l, note) = pick(&[&a, &c], "", "a5.ucsd.edu").unwrap();
        assert_eq!((l.id.as_str(), note.as_str()), ("a1", ""));
        let (l, note) = pick(&[&a, &c], "885758", "a5.ucsd.edu").unwrap();
        assert_eq!(l.id, "a1");
        assert!(note.contains("No saved login has the id 885758") && note.contains("id a1"));
        // Two logins for the page: it lists them instead of guessing.
        let e = pick(&[&a, &b, &c], "", "a5.ucsd.edu").unwrap_err();
        assert!(e.contains("[a1]") && e.contains("[b2]") && !e.contains("[c3]"));
        assert!(pick(&[&c], "", "a5.ucsd.edu").is_err());
    }

    #[test]
    fn sites_match_only_their_own_pages() {
        assert_eq!(normalize_site(" https://SSO.ucsd.edu/login?x=1 ").unwrap(), "sso.ucsd.edu");
        assert_eq!(normalize_site("*.ucsd.edu").unwrap(), "ucsd.edu");
        assert_eq!(normalize_site("https://www.amazon.com/").unwrap(), "amazon.com");
        assert!(normalize_site("edu").is_err() && normalize_site("not a site").is_err());
        let sites = vec!["ucsd.edu".to_string()];
        let ok = |u: &str| https_host(u).is_some_and(|h| site_allowed(&sites, &h));
        assert!(ok("https://sso.ucsd.edu/idp/login") && ok("https://ucsd.edu"));
        assert!(!ok("http://sso.ucsd.edu/login"), "never over plain http");
        assert!(!ok("https://ucsd.edu.evil.com/login") && !ok("https://evilucsd.edu/") && !ok("https://ucsd.edu@evil.com/"));
    }

    #[test]
    fn import_sorts_rows() {
        let mut cfg = Config::default();
        let row = |u: &str, n: &str, p: &str| ImportRow { url: u.into(), username: n.into(), password: p.into(), name: String::new() };
        let rows = vec![row("https://a.com/login", "me", "pw1"), row("https://a.com", "me", "pw2"), row("", "x", "pw"), row("https://b.com", "x", "")];
        let preview = import(&mut cfg, &rows, false);
        assert_eq!((preview["ready"].as_i64(), preview["duplicates"].as_i64(), preview["errors"].as_i64()), (Some(1), Some(1), Some(2)));
        assert!(cfg.logins.is_empty(), "a preview saves nothing");
        import(&mut cfg, &rows, true);
        assert_eq!(cfg.logins.len(), 1);
        let again = import(&mut cfg, &rows, false);
        assert_eq!(again["duplicates"].as_i64(), Some(2), "already in the vault");
    }

    #[test]
    fn editing_keeps_the_password() {
        let input = |pw: &str| LoginIn { sites: vec!["https://x.com/login https://y.com".into()], username: "me".into(), password: pw.into(), ..Default::default() };
        let l = clean_login(input("secret-1"), None, &[]).unwrap();
        assert_eq!(l.sites, vec!["x.com", "y.com"]);
        let edited = clean_login(input(""), Some(&l), &[]).unwrap();
        assert_eq!((edited.id.as_str(), edited.password.as_str()), (l.id.as_str(), "secret-1"));
        assert!(clean_login(input(""), None, &[]).is_err(), "a new login needs a password");
    }
}
