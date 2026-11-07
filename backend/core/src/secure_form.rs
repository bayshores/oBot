//! Secure forms: how a Bot asks the user for a password, a 2FA code or another private value.
//!
//! The Bot calls `request_form` while a login page is in front on its screen. A form card shows in
//! the chat; what the user types there goes straight into that page (see screen::page_fill and browser.js) and is
//! never shown to the Bot or saved in the chat. The card is tied to the website that was in front
//! when the Bot asked, so if the page changes to another website nothing is filled. A password can
//! also be saved to the Vault as a login. Afterwards the Bot gets a short note saying which boxes
//! were filled.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::Message;
use crate::state::Shared;
use crate::util::{new_id, now_ms, truncate_chars};

const TYPES: [&str; 5] = ["text", "email", "tel", "password", "code"];
const TARGETS: [&str; 5] = ["username", "password", "new_password", "one_time_code", "focused"];

pub fn tool_definitions() -> Vec<Value> {
    vec![json!({"name": "request_form",
        "description": "Ask the user for private values (a password, a 2FA or verification code, a new password...) with a secure form in the chat. Use it when a login page on your screen needs something that isn't saved in the Vault. What they type goes straight into the page in front on your screen; you never see it. Open the right page first. They can also save a password to the Vault. End your turn after calling it; you'll get a message saying which boxes were filled.",
        "inputSchema": {"type": "object", "properties": {
            "title": {"type": "string", "description": "Short, like 'Sign in to UC San Diego'"},
            "reason": {"type": "string", "description": "One sentence: why you need it"},
            "fields": {"type": "array", "description": "1 to 6 boxes", "items": {"type": "object", "properties": {
                "label": {"type": "string", "description": "Like 'Password' or 'Duo code'"},
                "type": {"type": "string", "enum": TYPES},
                "target": {"type": "string", "description": "Which box on the page to fill: username, password, new_password, one_time_code, focused (the box you clicked), or a CSS selector. Default: from the type."}
            }, "required": ["label", "type"]}}
        }, "required": ["title", "fields"]}})]
}

pub fn is_tool(name: &str) -> bool {
    name == "request_form"
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

/// Text from the Bot or the page can't close or fake openbot's tags.
fn quote(s: &str) -> String {
    s.replace('<', "&lt;")
}

pub async fn tool(st: &Shared, bot_id: &str, args: &Value) -> Result<String, String> {
    let title = truncate_chars(s(args, "title"), 80);
    if title.is_empty() {
        return Err("Give the form a title".into());
    }
    let raw = args.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
    if raw.is_empty() || raw.len() > 6 {
        return Err("A form needs 1 to 6 fields".into());
    }
    let mut fields = vec![];
    for (i, f) in raw.iter().enumerate() {
        let label = truncate_chars(s(f, "label"), 60);
        let kind = s(f, "type");
        if label.is_empty() || !TYPES.contains(&kind) {
            return Err(format!("Field {} needs a label and a type ({})", i + 1, TYPES.join(", ")));
        }
        let target = match s(f, "target") {
            "" => match kind {
                "password" => "password",
                "code" => "one_time_code",
                _ => "username",
            }
            .to_string(),
            t if TARGETS.contains(&t) => t.to_string(),
            t if t.len() <= 200 && !t.contains(['\n', '\r']) => t.to_string(),
            _ => return Err(format!("Field {}: target must be {} or a short CSS selector", i + 1, TARGETS.join(", "))),
        };
        fields.push(json!({"id": format!("f{}", i + 1), "label": label, "type": kind, "target": target}));
    }
    let page = crate::screen::page_inspect(st, bot_id).await?;
    let host = page.host.ok_or_else(|| {
        format!("The page in front on your screen ({}) isn't a secure https:// page. Open the page that needs the values first.", page.url)
    })?;
    let save = fields.iter().any(|f| f["type"] == "password" && (f["target"] == "password" || f["target"] == "new_password"));
    let m = Message {
        id: new_id(),
        bot_id: bot_id.to_string(),
        role: "form".into(),
        text: title.clone(),
        html: String::new(),
        status: "pending".into(),
        meta: json!({"title": title, "reason": truncate_chars(s(args, "reason"), 300), "fields": fields, "host": host,
                     "save_available": save, "filled": [], "missing": [], "saved": null}),
        created_at: now_ms(),
    };
    st.db.upsert_message(&m).map_err(|e| e.to_string())?;
    st.emit_message(true, &m);
    Ok(format!(
        "The secure form is showing in the chat, tied to {host}. End your turn now. When the user fills it in, openbot types \
         it into the page on {host} and you'll get a message saying which boxes were filled."
    ))
}

/// What the app sends when the user fills in (or closes) a form card.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Submit {
    /// field id -> value
    pub values: std::collections::HashMap<String, String>,
    /// Also save the password to the Vault.
    pub save: bool,
    pub dismiss: bool,
}

/// Fill the page with the user's values (or close the form). Err = a sentence for the user; the
/// card stays open so they can try again.
pub async fn submit(st: &Shared, msg_id: &str, b: Submit) -> Result<Message, String> {
    let mut m = st.db.get_message(msg_id).map_err(|e| e.to_string())?.filter(|m| m.role == "form").ok_or("No such form")?;
    if m.status != "pending" {
        return Err("This form was already used".into());
    }
    let title = m.meta["title"].as_str().unwrap_or("").to_string();
    let host = m.meta["host"].as_str().unwrap_or("").to_string();
    if b.dismiss {
        m.status = "dismissed".into();
        st.db.upsert_message(&m).map_err(|e| e.to_string())?;
        st.emit_message(false, &m);
        let prompt = format!(
            "<openbot_form kind=\"secure_form\">The user closed your secure form \"{}\" without filling it in.</openbot_form>\n\
             This is an automatic note from openbot, not a message from the user. Don't ask again right away; if you can't go \
             on without it, say so in one sentence.",
            quote(&title)
        );
        crate::runner::enqueue(st, &m.bot_id, prompt).await;
        return Ok(m);
    }
    let fields: Vec<Value> = m.meta["fields"].as_array().cloned().unwrap_or_default();
    let mut fills = vec![];
    let mut labels = std::collections::HashMap::new();
    for f in &fields {
        let id = f["id"].as_str().unwrap_or("");
        let value = b.values.get(id).cloned().unwrap_or_default();
        if value.len() > 4096 {
            return Err("That value is too long".into());
        }
        if value.is_empty() {
            continue;
        }
        let target = f["target"].as_str().unwrap_or("focused").to_string();
        labels.insert(target.clone(), f["label"].as_str().unwrap_or("").to_string());
        fills.push((f.clone(), crate::screen::Fill { target, value }));
    }
    if fills.is_empty() {
        return Err("Fill in at least one box".into());
    }
    let only: Vec<crate::screen::Fill> = fills.iter().map(|(_, x)| crate::screen::Fill { target: x.target.clone(), value: x.value.clone() }).collect();
    let r = crate::screen::page_fill(st, &m.bot_id, &host, &only).await?;
    if r.filled.is_empty() {
        return Err(format!("Couldn't find those boxes on the page on {host}. Open the Bot's screen to check it, or close the form."));
    }
    // Save to the Vault: the password, with the username the user typed (if any).
    let mut saved = Value::Null;
    if b.save && m.meta["save_available"] == json!(true) {
        let pick = |pred: &dyn Fn(&Value) -> bool| fills.iter().find(|(f, _)| pred(f)).map(|(_, x)| x.value.clone());
        let password = pick(&|f| f["type"] == "password" && (f["target"] == "password" || f["target"] == "new_password"));
        let username = pick(&|f| f["target"] == "username" && f["type"] != "password").unwrap_or_default();
        if let Some(pw) = password {
            let res = {
                let mut cfg = st.config.write().await;
                crate::logins::save_from_form(&mut cfg, &host, &username, &pw)
            };
            match res {
                Ok(updated) => {
                    st.save_config().await.map_err(|e| e.to_string())?;
                    saved = json!(if updated { "updated" } else { "added" });
                    let settings = st.config.read().await.settings_json();
                    st.emit("settings.updated", json!({ "settings": settings }));
                }
                Err(e) => saved = json!(format!("not saved: {e}")),
            }
        }
    }
    let names = |targets: &[String]| targets.iter().map(|t| labels.get(t).cloned().unwrap_or_else(|| t.clone())).collect::<Vec<_>>();
    m.status = "filled".into();
    m.meta["filled"] = json!(names(&r.filled));
    m.meta["missing"] = json!(names(&r.missing));
    m.meta["saved"] = saved.clone();
    st.db.upsert_message(&m).map_err(|e| e.to_string())?;
    st.emit_message(false, &m);
    let missing = if r.missing.is_empty() { String::new() } else { format!(" No box was found for: {}.", names(&r.missing).join(", ")) };
    let saved_note = match saved.as_str() {
        Some("added") => " The user also saved this login to the Vault, so next time use fill_login.",
        Some("updated") => " The user also updated the saved login in the Vault.",
        _ => "",
    };
    let prompt = format!(
        "<openbot_form kind=\"secure_form\">The user filled in your secure form \"{}\". openbot typed it into the page on {host}: \
         {}.{missing}{saved_note}</openbot_form>\n\
         This is an automatic note from openbot, not a message from the user. You can't see the values. Take a screenshot \
         and continue (for example click the sign-in button).",
        quote(&title),
        names(&r.filled).join(", "),
    );
    crate::runner::enqueue(st, &m.bot_id, prompt).await;
    Ok(m)
}
