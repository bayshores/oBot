//! Phone notifications (Web Push). A phone that added oBot to its home screen subscribes in
//! Settings › General › Notifications. When a Bot finishes or needs you and no oBot window is in
//! use anywhere, oBot sends the phone a notification through its browser's push service (Apple's
//! for iPhones). The message is encrypted for that phone, so the push service can't read it, and it
//! goes out from this computer: the phone doesn't need to reach it for the notification to arrive.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};
use web_push_native::{p256, Auth, WebPushBuilder};

use crate::db::Message;
use crate::state::Shared;
use crate::util::{now_ms, preview, truncate_chars};

/// Push services want to know who sends (RFC 8292). This names the app, not the user.
const CONTACT: &str = "https://github.com/bayshores/oBot";
/// Most phones a user would subscribe; old ones are dropped first.
const MAX_SUBS: usize = 20;
/// How long a push service keeps a notification for a phone that's off.
const KEEP_FOR: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PushConfig {
    /// This computer's VAPID private key (base64url). Made on first use; saved encrypted.
    #[serde(with = "crate::vault::sealed")]
    pub vapid: String,
    pub subs: Vec<Subscription>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Subscription {
    /// Where to send (the phone's push service address).
    pub endpoint: String,
    /// The phone's public key and secret for encrypting the message (base64url).
    pub p256dh: String,
    pub auth: String,
    /// The paired device that subscribed (None = this computer's own token).
    pub device: Option<String>,
    pub created_at: i64,
}

/// This computer's VAPID key, made and saved the first time.
async fn key(st: &Shared) -> Result<ES256KeyPair, String> {
    {
        let cfg = st.config.read().await;
        if !cfg.push.vapid.is_empty() {
            let raw = B64.decode(cfg.push.vapid.trim()).map_err(|e| e.to_string())?;
            return ES256KeyPair::from_bytes(&raw).map_err(|e| e.to_string());
        }
    }
    let kp = ES256KeyPair::generate();
    st.config.write().await.push.vapid = B64.encode(kp.to_bytes());
    st.save_config().await.map_err(|e| e.to_string())?;
    Ok(kp)
}

/// The public key a phone subscribes with (`applicationServerKey`).
pub async fn public_key(st: &Shared) -> Result<String, String> {
    let kp = key(st).await?;
    Ok(B64.encode(kp.public_key().public_key().to_bytes_uncompressed()))
}

pub async fn subscribe(st: &Shared, device: Option<String>, sub: &Value) -> Result<(), String> {
    let endpoint = sub["endpoint"].as_str().unwrap_or("").trim().to_string();
    let p256dh = sub["keys"]["p256dh"].as_str().unwrap_or("").trim().to_string();
    let auth = sub["keys"]["auth"].as_str().unwrap_or("").trim().to_string();
    if !endpoint.starts_with("https://") || endpoint.len() > 2000 || endpoint.parse::<http::Uri>().is_err() {
        return Err("That isn't a push service address".into());
    }
    parse_keys(&p256dh, &auth)?;
    key(st).await?;
    {
        let mut cfg = st.config.write().await;
        let subs = &mut cfg.push.subs;
        subs.retain(|s| s.endpoint != endpoint);
        subs.push(Subscription { endpoint, p256dh, auth, device, created_at: now_ms() });
        if subs.len() > MAX_SUBS {
            let extra = subs.len() - MAX_SUBS;
            subs.drain(..extra);
        }
    }
    st.save_config().await.map_err(|e| e.to_string())
}

pub async fn unsubscribe(st: &Shared, endpoint: &str) -> Result<(), String> {
    st.config.write().await.push.subs.retain(|s| s.endpoint != endpoint);
    st.save_config().await.map_err(|e| e.to_string())
}

/// A removed device gets no more notifications.
pub fn forget_device(cfg: &mut crate::config::Config, device: &str) {
    cfg.push.subs.retain(|s| s.device.as_deref() != Some(device));
}

fn parse_keys(p256dh: &str, auth: &str) -> Result<(p256::PublicKey, Auth), String> {
    let pk = B64.decode(p256dh).ok().and_then(|b| p256::PublicKey::from_sec1_bytes(&b).ok()).ok_or("The phone's key isn't valid")?;
    let a = B64.decode(auth).map_err(|_| "The phone's secret isn't valid")?;
    if a.len() != 16 {
        return Err("The phone's secret isn't valid".into());
    }
    Ok((pk, Auth::clone_from_slice(&a)))
}

/// What a notification says. `bot` is the chat to open when it's tapped.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub bot: String,
    /// Approvals and questions are delivered right away; replies can wait a little.
    pub urgent: bool,
}

/// Build the encrypted request for one phone (also used by the tests).
fn request(kp: &ES256KeyPair, sub: &Subscription, payload: &[u8], topic: &str, urgent: bool) -> Result<http::Request<Vec<u8>>, String> {
    let (pk, auth) = parse_keys(&sub.p256dh, &sub.auth)?;
    let endpoint = sub.endpoint.parse().map_err(|_| "Bad push address".to_string())?;
    let mut req = WebPushBuilder::new(endpoint, pk, auth)
        .with_valid_duration(KEEP_FOR)
        .with_vapid(kp, CONTACT)
        .build(payload.to_vec())
        .map_err(|e| e.to_string())?;
    let h = req.headers_mut();
    h.insert("Urgency", http::HeaderValue::from_static(if urgent { "high" } else { "normal" }));
    // A newer notification for the same chat replaces one the phone hasn't picked up yet.
    if let Ok(v) = http::HeaderValue::from_str(topic) {
        h.insert("Topic", v);
    }
    Ok(req)
}

/// What sending to one phone did.
enum Sent {
    Ok,
    /// The phone unsubscribed or the subscription expired: forget it.
    Gone,
    Failed(String),
}

fn send_one(req: http::Request<Vec<u8>>) -> Sent {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let mut r = agent.request(req.method().as_str(), &req.uri().to_string());
    for (k, v) in req.headers() {
        if let Ok(v) = v.to_str() {
            r = r.set(k.as_str(), v);
        }
    }
    match r.send_bytes(req.body()) {
        Ok(_) => Sent::Ok,
        Err(ureq::Error::Status(404 | 410, _)) => Sent::Gone,
        Err(ureq::Error::Status(code, resp)) => {
            let why = resp.into_string().unwrap_or_default();
            Sent::Failed(format!("the push service answered {code} {}", truncate_chars(why.trim(), 200)))
        }
        Err(e) => Sent::Failed(e.to_string()),
    }
}

/// Send `note` to these phones (all of them if `only` is None). Returns why it failed for any phone.
pub async fn send(st: &Shared, note: &Note, only: Option<&str>) -> Result<(), String> {
    let kp = key(st).await?;
    let subs: Vec<Subscription> = st
        .config
        .read()
        .await
        .push
        .subs
        .iter()
        .filter(|s| only.is_none_or(|e| e == s.endpoint))
        .cloned()
        .collect();
    if subs.is_empty() {
        return Err("This device isn't subscribed".into());
    }
    let payload = json!({
        "title": note.title,
        "body": note.body,
        "bot": note.bot,
        "tag": note.bot,
        "badge": unread_total(st),
    })
    .to_string();
    // Topic: at most 32 URL-safe characters.
    let topic: String = note.bot.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
    let mut errors = vec![];
    let mut gone = vec![];
    for sub in subs {
        let req = match request(&kp, &sub, payload.as_bytes(), &topic, note.urgent) {
            Ok(r) => r,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        match tokio::task::spawn_blocking(move || send_one(req)).await {
            Ok(Sent::Ok) => {}
            Ok(Sent::Gone) => gone.push(sub.endpoint),
            Ok(Sent::Failed(e)) => errors.push(e),
            Err(e) => errors.push(e.to_string()),
        }
    }
    if !gone.is_empty() {
        st.config.write().await.push.subs.retain(|s| !gone.contains(&s.endpoint));
        let _ = st.save_config().await;
        if only.is_some() {
            return Err("This device's notifications were turned off in its settings. Turn them on again here.".into());
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// The number on the app icon: unread messages in chats that are shown.
fn unread_total(st: &Shared) -> i64 {
    let bots = st.db.list_bots().unwrap_or_default();
    let groups = st.db.list_groups().unwrap_or_default();
    bots.iter().chain(groups.iter()).filter(|b| !b.hidden).map(|b| b.unread.max(0)).sum()
}

// ---------------------------------------------------------------- when to notify

/// Open oBot windows (event connections) → whether someone is using that window right now:
/// it's in front, focused, and was touched in the last couple of minutes.
static ACTIVE: Mutex<Option<HashMap<u64, bool>>> = Mutex::new(None);
/// Notifications already sent (message id + kind), so an edited message doesn't notify again.
static SENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Last "finished" notification per chat (unix ms): at most one every few seconds.
static LAST_DONE: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

pub fn set_active(conn: u64, active: bool) {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(conn, active);
}
pub fn closed(conn: u64) {
    if let Some(m) = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&conn);
    }
}
fn someone_looking() -> bool {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|m| m.values().any(|&a| a))
}

/// Is this message something to tell a phone about? The same cases as the desktop app's
/// notifications: a finished reply, a file or page, an approval, a question or a form.
/// Returns (key, urgent, text).
fn worth_telling(created: bool, m: &Message) -> Option<(String, bool, String)> {
    let meta = |k: &str| m.meta.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let or = |a: String, b: &str| if a.trim().is_empty() { b.to_string() } else { a };
    match m.role.as_str() {
        "bot" if m.status == "done" && !m.text.trim().is_empty() => Some((format!("d:{}", m.id), false, m.text.clone())),
        "attachment" if created => Some((format!("d:{}", m.id), false, or(m.text.clone(), &format!("Sent {}", or(meta("name"), "a file"))))),
        "visualization" if created => Some((format!("d:{}", m.id), false, or(meta("title"), "Made a page"))),
        "approval" if m.status == "pending" => Some((format!("a:{}", m.id), true, or(meta("summary"), &m.text))),
        "widget" if m.meta.get("answer").is_none_or(Value::is_null) && m.meta.get("question").is_some() => {
            Some((format!("w:{}", m.id), true, or(meta("question"), &m.text)))
        }
        "form" if m.status == "pending" => Some((format!("f:{}", m.id), true, or(meta("title"), &m.text))),
        _ => None,
    }
}

/// Called for every message oBot shows. Cheap when there's nothing to send.
pub fn on_message(st: &Shared, created: bool, m: &Message) {
    let Some((key, urgent, text)) = worth_telling(created, m) else { return };
    // Older messages being edited (a reaction, say) aren't news.
    if now_ms() - m.created_at > 30 * 60_000 {
        return;
    }
    {
        let mut sent = SENT.lock().unwrap_or_else(|e| e.into_inner());
        if sent.contains(&key) {
            return;
        }
        sent.push_back(key);
        if sent.len() > 500 {
            sent.pop_front();
        }
    }
    let (st, m) = (st.clone(), m.clone());
    tokio::spawn(async move {
        if st.config.read().await.push.subs.is_empty() || someone_looking() {
            return;
        }
        let Some(chat) = st.db.get_bot(&m.bot_id).ok().flatten() else { return };
        if chat.hidden || !chat.notifications {
            return;
        }
        if !urgent {
            let mut last = LAST_DONE.lock().unwrap_or_else(|e| e.into_inner());
            let last = last.get_or_insert_with(HashMap::new);
            if now_ms() - last.get(&chat.id).copied().unwrap_or(0) < 5000 {
                return;
            }
            last.insert(chat.id.clone(), now_ms());
        }
        // In a group: "School Bot: …" under the group's name.
        let speaker = m.meta.get("speaker").and_then(Value::as_str).filter(|s| *s != chat.id);
        let who = speaker.and_then(|s| st.db.get_bot(s).ok().flatten()).map(|b| format!("{}: ", b.name)).unwrap_or_default();
        let note = Note {
            title: if urgent { format!("{} is waiting for you", chat.name) } else { chat.name.clone() },
            body: format!("{who}{}", truncate_chars(&preview(&text), 140)),
            bot: chat.id.clone(),
            urgent,
        };
        match send(&st, &note, None).await {
            Ok(()) => tracing::info!("sent a phone notification ({})", if urgent { "needs you" } else { "finished" }),
            Err(e) => tracing::warn!("couldn't send a phone notification: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, status: &str, meta: Value) -> Message {
        Message { id: "m1".into(), bot_id: "b1".into(), role: role.into(), status: status.into(), text: "Hello there".into(), html: String::new(), meta, created_at: now_ms() }
    }

    #[test]
    fn tells_about_the_same_things_as_the_desktop_app() {
        assert_eq!(worth_telling(false, &msg("bot", "done", json!({}))).map(|x| x.1), Some(false));
        assert!(worth_telling(false, &msg("bot", "streaming", json!({}))).is_none());
        assert!(worth_telling(true, &msg("tool", "ok", json!({}))).is_none());
        assert_eq!(worth_telling(true, &msg("approval", "pending", json!({"summary": "Run npm test"}))).unwrap().2, "Run npm test");
        assert!(worth_telling(false, &msg("approval", "allowed", json!({}))).is_none());
        assert!(worth_telling(true, &msg("widget", "done", json!({"question": "Which day?"}))).unwrap().1);
        assert!(worth_telling(true, &msg("widget", "done", json!({"question": "Which day?", "answer": "Monday"}))).is_none());
        assert!(worth_telling(false, &msg("attachment", "done", json!({}))).is_none());
    }

    #[test]
    fn the_phone_can_read_what_was_sent() {
        // A pretend phone: its key pair and secret, as a browser makes them when it subscribes.
        let phone = p256::SecretKey::random(&mut aes_gcm::aead::OsRng);
        let auth = [7u8; 16];
        let sub = Subscription {
            endpoint: "https://web.push.apple.com/abc".into(),
            p256dh: B64.encode(phone.public_key().to_sec1_bytes()),
            auth: B64.encode(auth),
            device: None,
            created_at: 0,
        };
        let kp = ES256KeyPair::generate();
        let req = request(&kp, &sub, br#"{"title":"Food Bot"}"#, "b1", true).unwrap();
        assert_eq!(req.headers()["Urgency"], "high");
        assert_eq!(req.headers()["Topic"], "b1");
        assert!(req.headers()["Authorization"].to_str().unwrap().starts_with("vapid t="));
        let plain = web_push_native::decrypt(req.body().clone(), &phone, &Auth::clone_from_slice(&auth)).unwrap();
        assert_eq!(plain, br#"{"title":"Food Bot"}"#);
    }
}
