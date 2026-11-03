//! HTTP + WebSocket API (see docs/API.md) and the built-in web UI.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tower_http::cors::CorsLayer;

use crate::computer::computer_json;
use crate::config::{default_data_dir, Config, Connection, Secret};
use crate::db::{Bot, Db, Message};
use crate::runner;
use crate::state::{AppState, PendingApproval, Shared};
use crate::util::{new_id, now_ms, preview, truncate_chars};

// ---------------------------------------------------------------- startup

pub struct ServeOptions {
    /// "127.0.0.1" = only this computer; "0.0.0.0" = reachable from other computers.
    pub host: String,
    /// 0 = pick any free port.
    pub port: u16,
    pub data_dir: Option<PathBuf>,
}

pub struct StartedServer {
    pub addr: SocketAddr,
    pub token: String,
    pub state: Shared,
}

pub async fn start(opts: ServeOptions) -> anyhow::Result<StartedServer> {
    let data_dir = opts.data_dir.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let config_path = data_dir.join("config.json");
    let config = Config::load_or_create(&config_path)?;
    let db = Db::open(&data_dir.join("openbot.db"))?;
    db.close_dangling()?;
    if !crate::util::set_timezone(&config.timezone) {
        tracing::warn!("unknown timezone {:?}; using this computer's timezone", config.timezone);
    }

    if db.list_bots()?.is_empty() {
        let nb = crate::bots::NewBot { name: "General Bot".into(), color: "magenta".into(), shape: "blob".into(), ..Default::default() };
        let b = crate::bots::build(&db, &config, &nb)?;
        db.upsert_bot(&b)?;
        db.kv_set("main_bot", &b.id);
    }
    // Make sure there is a Main Bot (older data, or the main one was removed).
    let bots = db.list_bots()?;
    let main_ok = db.kv_get("main_bot").map_or(false, |m| bots.iter().any(|b| b.id == m));
    if !main_ok {
        if let Some(oldest) = bots.iter().min_by_key(|b| b.created_at) {
            db.kv_set("main_bot", &oldest.id);
        }
    }

    let listener = tokio::net::TcpListener::bind((opts.host.as_str(), opts.port)).await?;
    let addr = listener.local_addr()?;
    let (tx, _) = broadcast::channel(1024);
    let token = config.token.clone();

    let state: Shared = Arc::new(AppState {
        db,
        config: RwLock::new(config),
        config_path,
        data_dir,
        events: tx,
        queues: Mutex::new(HashMap::new()),
        runs: Mutex::new(HashMap::new()),
        approvals: Mutex::new(HashMap::new()),
        activity: std::sync::Mutex::new(HashMap::new()),
        port: addr.port(),
        listen: addr.to_string(),
        self_exe: std::env::current_exe()?,
        limiter: crate::security::RateLimiter::default(),
        device_seen: std::sync::Mutex::new(HashMap::new()),
        pair_lock: Mutex::new(()),
        screens: Default::default(),
    });

    // Look up Claude Code in the background so the first screen loads fast.
    let claude_path = state.config.read().await.claude_path.clone();
    tokio::spawn(async move {
        crate::computer::claude_info(&claude_path).await;
    });

    // Hide saved secret values in chats saved before they were in the Vault.
    spawn_redact_history(state.clone());

    crate::routines::spawn_scheduler(state.clone());
    crate::screen::Screens::spawn_reaper(state.clone());

    let app = router(state.clone());
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await {
            tracing::error!("server stopped: {e}");
        }
    });
    Ok(StartedServer { addr, token, state })
}

fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/api/connection", get(get_connection).put(put_connection))
        .route("/api/connection/test", post(test_connection))
        .route("/api/prefs", get(get_prefs).put(put_prefs))
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/bots", get(list_bots).post(create_bot))
        .route("/api/bots/{id}", axum::routing::patch(patch_bot).delete(delete_bot))
        .route("/api/bots/{id}/reset", post(reset_bot))
        .route("/api/bots/{id}/read", post(read_bot))
        .route("/api/bots/{id}/unread", post(unread_bot))
        .route("/api/bots/{id}/messages", get(list_messages).post(send_message))
        .route("/api/bots/{id}/stop", post(stop_bot))
        .route(
            "/api/bots/{id}/upload",
            post(upload).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route("/api/bots/{id}/files/{*path}", get(get_file))
        .route("/api/approvals/{id}", post(decide_approval))
        .route("/api/internal/approve", post(internal_approve))
        .route("/api/internal/tool", post(internal_tool))
        .route("/api/screens/status", get(screens_status))
        .route("/api/screens/setup", post(screens_setup))
        .route("/api/bots/{id}/screen", get(bot_screen))
        .route("/api/bots/{id}/screen/start", post(bot_screen_start))
        .route("/api/bots/{id}/screen/stop", post(bot_screen_stop))
        .route("/api/bots/{id}/screen/ws", get(bot_screen_ws))
        .route("/api/messages/{id}/answer", post(answer_widget))
        .route("/api/bots/{id}/routines", get(list_routines))
        .route("/api/routines/{id}", axum::routing::patch(patch_routine).delete(delete_routine))
        .route("/api/routines/{id}/run", post(run_routine_now))
        .route("/api/bots/{id}/memories", get(list_memories))
        .route("/api/memories/import", post(import_memories))
        .route("/api/bots/{id}/make-main", post(make_main))
        .route("/api/bots/{id}/skills", get(list_skills))
        .route("/api/bots/{id}/skills/{name}", axum::routing::put(put_skill).delete(delete_skill))
        .route("/api/bots/{id}/library", get(list_library))
        .route("/api/mcp-servers", post(add_mcp_server))
        .route("/api/mcp-servers/{name}", delete(remove_mcp_server))
        .route("/api/claude/update", post(update_claude))
        .route("/api/claude/logout", post(logout_claude))
        .route("/api/memories/{id}", delete(delete_memory))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/vault", post(add_secret))
        .route("/api/vault/{name}", delete(remove_secret))
        .route("/api/computer", get(get_computer))
        .route("/api/usage", get(get_usage))
        .route("/api/search", get(search))
        .route("/api/open-folder", post(open_folder))
        .route("/api/events", get(events))
        .route("/api/pair/code", post(new_pair_code))
        .route("/api/devices", get(list_devices))
        .route("/api/devices/{id}", delete(revoke_device))
        .route("/api/connection/pair", post(pair_with_computer))
        .route("/api/tailscale", get(get_tailscale))
        .route("/api/discover", post(discover_computers))
        .layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/", get(ui_index))
        .route("/ui/{*path}", get(ui_static))
        .route("/vendor/{*path}", get(|UrlPath(p): UrlPath<String>| async move { ui_response(&format!("vendor/{p}")) }))
        .route("/app.css", get(|| async { ui_response("app.css") }))
        .route("/app.js", get(|| async { ui_response("app.js") }))
        .route("/avatar.js", get(|| async { ui_response("avatar.js") }))
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .route("/pages/{id}", get(get_page))
        // No token needed: a new device trades a pairing code for its own token here.
        .route("/api/pair", post(pair_device))
        // No token needed: lets the setup screen recognize an openbot computer (name + version only).
        .route("/api/hello", get(hello))
        .merge(api)
        .layer(cors())
        .layer(middleware::from_fn(guard_origin))
        .with_state(state)
}

// ---------------------------------------------------------------- errors + auth

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

fn bad(msg: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}
fn not_found(msg: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, msg.into())
}

type ApiResult<T> = Result<T, ApiError>;

use crate::security::{client_key, constant_eq, sha256_hex};

/// Which credential a request used: None = the owner token, Some(id) = a paired device.
#[derive(Clone)]
pub struct AuthDevice(pub Option<String>);

fn peer_of(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
}

fn token_of(req: &Request) -> Option<String> {
    let from_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    from_header.or_else(|| {
        req.uri().query().and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("token="))
                .map(percent_decode)
        })
    })
}

fn too_many(secs: i64) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, secs.to_string())],
        Json(json!({ "error": format!("Too many wrong tries. Wait {} minute(s) and try again.", (secs + 59) / 60) })),
    )
        .into_response()
}

async fn require_token(State(st): State<Shared>, mut req: Request, next: Next) -> Response {
    if req.method() == axum::http::Method::OPTIONS {
        return next.run(req).await;
    }
    let key = client_key(peer_of(&req), req.headers());
    if let Some(secs) = st.limiter.blocked(&key) {
        return too_many(secs);
    }
    let who = match token_of(&req) {
        Some(t) if !t.is_empty() => {
            let cfg = st.config.read().await;
            if constant_eq(&t, &cfg.token) {
                Some(AuthDevice(None))
            } else {
                let h = sha256_hex(&t);
                cfg.devices.iter().find(|d| constant_eq(&d.token_hash, &h)).map(|d| AuthDevice(Some(d.id.clone())))
            }
        }
        _ => None,
    };
    let Some(who) = who else {
        st.limiter.fail(&key);
        return ApiError(StatusCode::UNAUTHORIZED, "Wrong or missing token".into()).into_response();
    };
    if let Some(id) = &who.0 {
        st.device_seen.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), now_ms());
    }
    req.extensions_mut().insert(who);
    next.run(req).await
}

/// Refuse requests from web pages that aren't openbot's own (stops a malicious site from
/// talking to openbot in the background), and requests for unexpected host names
/// (DNS rebinding).
async fn guard_origin(req: Request, next: Next) -> Response {
    if !crate::security::host_allowed(req.headers()) {
        return ApiError(StatusCode::FORBIDDEN, "Unknown host name. Set OPENBOT_ALLOWED_HOSTS to allow it.".into()).into_response();
    }
    let path = req.uri().path();
    if path.starts_with("/api/") {
        if let Some(o) = req.headers().get(header::ORIGIN) {
            if !crate::security::origin_allowed(o, req.headers()) {
                tracing::warn!("blocked a request from web page origin {:?}", o);
                return ApiError(StatusCode::FORBIDDEN, "This web page isn't allowed to use openbot".into()).into_response();
            }
        }
    }
    next.run(req).await
}

fn cors() -> CorsLayer {
    use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin};
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|o, parts| crate::security::origin_allowed(o, &parts.headers)))
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
        .max_age(Duration::from_secs(600))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode_path(s: &str) -> String {
    let mut out = String::new();
    for c in s.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(c as char),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------- web UI

mod ui_assets {
    include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));
}

/// Serve a UI file. OPENBOT_UI_DIR=/path/to/ui reads from disk instead (handy while editing the UI).
fn ui_response(name: &str) -> Response {
    // Subfolders are allowed (vendor/novnc/core/rfb.js); climbing out isn't.
    if name.is_empty() || name.contains("..") || name.contains('\\') || name.starts_with('/') || name.contains("//") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let bytes: Option<Vec<u8>> = std::env::var("OPENBOT_UI_DIR")
        .ok()
        .and_then(|dir| std::fs::read(Path::new(&dir).join(name)).ok())
        .or_else(|| {
            ui_assets::UI_FILES
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, b)| b.to_vec())
        });
    match bytes {
        Some(b) => {
            let mime = mime_guess::from_path(name).first_or_octet_stream();
            let ct = if mime.type_() == "text" || mime.subtype() == "javascript" {
                format!("{mime}; charset=utf-8")
            } else {
                mime.to_string()
            };
            ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "no-store".to_string())], b).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn ui_index() -> Response {
    ui_response("index.html")
}

async fn ui_static(UrlPath(name): UrlPath<String>) -> Response {
    ui_response(&name)
}

// ---------------------------------------------------------------- connection + prefs (home server)

async fn get_connection(State(st): State<Shared>) -> Json<Connection> {
    Json(st.config.read().await.connection.clone())
}

async fn put_connection(State(st): State<Shared>, Json(mut c): Json<Connection>) -> ApiResult<Json<Connection>> {
    if c.mode != "remote" {
        c.mode = "local".into();
    }
    c.url = c.url.trim().trim_end_matches('/').to_string();
    if c.mode == "remote" && !(c.url.starts_with("http://") || c.url.starts_with("https://")) {
        return Err(bad("The address must start with http:// or https://"));
    }
    st.config.write().await.connection = c.clone();
    st.save_config().await?;
    Ok(Json(c))
}

#[derive(Deserialize)]
struct TestBody {
    url: String,
    token: String,
}

async fn test_connection(Json(b): Json<TestBody>) -> Json<Value> {
    let url = b.url.trim().trim_end_matches('/').to_string();
    let token = b.token.trim().to_string();
    let res = tokio::task::spawn_blocking(move || {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(8)).build();
        agent
            .get(&format!("{url}/api/computer"))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
    })
    .await;
    Json(match res {
        Ok(Ok(r)) => match r.into_json::<Value>() {
            Ok(c) => json!({ "ok": true, "computer": c }),
            Err(e) => json!({ "ok": false, "error": format!("Unexpected answer: {e}") }),
        },
        Ok(Err(ureq::Error::Status(401, _))) => json!({ "ok": false, "error": "The computer rejected the token." }),
        Ok(Err(e)) => json!({ "ok": false, "error": format!("Couldn't reach the computer: {e}") }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    })
}

// ---------------------------------------------------------------- pairing + devices

#[derive(Deserialize)]
struct PairBody {
    code: String,
    #[serde(default)]
    device_name: String,
}

/// A new device trades a 6-digit pairing code for its own token. No token needed.
async fn pair_device(
    State(st): State<Shared>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(b): Json<PairBody>,
) -> Response {
    let key = client_key(Some(peer), &headers);
    if let Some(secs) = st.limiter.blocked(&key) {
        return too_many(secs);
    }
    let _guard = st.pair_lock.lock().await;
    match crate::security::check_pairing_code(&st.data_dir, &b.code) {
        crate::security::PairCheck::Ok => {}
        crate::security::PairCheck::Wrong { tries_left } => {
            st.limiter.fail(&key);
            let msg = if tries_left == 0 {
                "Wrong code. That code no longer works: make a new one on the host computer.".to_string()
            } else {
                format!("Wrong code. {tries_left} tries left.")
            };
            return ApiError(StatusCode::UNAUTHORIZED, msg).into_response();
        }
        crate::security::PairCheck::NoCode => {
            st.limiter.fail(&key);
            return ApiError(
                StatusCode::UNAUTHORIZED,
                "No pairing code is active. On the host computer run `openbot-server pair` to get a new one.".into(),
            )
            .into_response();
        }
    }
    let token = crate::security::new_device_token();
    let name = truncate_chars(b.device_name.trim(), 60);
    let device = crate::security::Device {
        id: new_id(),
        name: if name.is_empty() { "Device".into() } else { name },
        token_hash: sha256_hex(&token),
        created_at: now_ms(),
        last_seen: now_ms(),
    };
    st.config.write().await.devices.push(device.clone());
    if let Err(e) = st.save_config().await {
        return ApiError::from(e).into_response();
    }
    tracing::info!("paired a new device: {}", device.name);
    st.emit("devices.updated", json!({}));
    let computer = computer_json(&st).await;
    Json(json!({ "token": token, "device_id": device.id, "computer": computer })).into_response()
}

/// Settings › "Pair a new device": make a pairing code on this computer.
async fn new_pair_code(State(st): State<Shared>) -> ApiResult<Json<Value>> {
    let _guard = st.pair_lock.lock().await;
    let (code, expires_at) = crate::security::new_pairing_code(&st.data_dir)?;
    Ok(Json(json!({ "code": code, "expires_at": expires_at })))
}

async fn list_devices(State(st): State<Shared>, axum::Extension(who): axum::Extension<AuthDevice>) -> ApiResult<Json<Value>> {
    let seen = st.device_seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut changed = false;
    let list = {
        let mut cfg = st.config.write().await;
        for d in cfg.devices.iter_mut() {
            if let Some(&t) = seen.get(&d.id) {
                if t > d.last_seen + 60_000 {
                    d.last_seen = t;
                    changed = true;
                }
            }
        }
        cfg.devices
            .iter()
            .map(|d| {
                json!({
                    "id": d.id, "name": d.name, "created_at": d.created_at,
                    "last_seen": seen.get(&d.id).copied().unwrap_or(d.last_seen).max(d.last_seen),
                    "current": who.0.as_deref() == Some(d.id.as_str()),
                })
            })
            .collect::<Vec<_>>()
    };
    if changed {
        st.save_config().await?;
    }
    Ok(Json(json!(list)))
}

async fn revoke_device(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    {
        let mut cfg = st.config.write().await;
        let before = cfg.devices.len();
        cfg.devices.retain(|d| d.id != id);
        if cfg.devices.len() == before {
            return Err(not_found("No such device"));
        }
    }
    st.save_config().await?;
    st.device_seen.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    st.emit("devices.updated", json!({}));
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct PairWithBody {
    url: String,
    code: String,
}

async fn hello() -> Json<Value> {
    Json(json!({
        "app": "openbot",
        "version": crate::VERSION,
        "name": sysinfo::System::host_name().unwrap_or_default(),
    }))
}

/// Home server: is Tailscale installed and signed in here, and which of your devices are online.
async fn get_tailscale() -> Json<Value> {
    Json(crate::discover::tailscale_status().await)
}

/// Home server: find your devices (on Tailscale) that run openbot.
async fn discover_computers() -> Json<Value> {
    Json(crate::discover::discover().await)
}

/// Home server: pair this app with another computer (sends the code to its /api/pair).
async fn pair_with_computer(Json(b): Json<PairWithBody>) -> Json<Value> {
    let url = b.url.trim().trim_end_matches('/').to_string();
    let code = b.code.trim().to_string();
    let name = sysinfo::System::host_name().unwrap_or_else(|| "openbot app".into());
    let res = tokio::task::spawn_blocking(move || {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
        agent.post(&format!("{url}/api/pair")).send_json(json!({ "code": code, "device_name": name }))
    })
    .await;
    Json(match res {
        Ok(Ok(r)) => match r.into_json::<Value>() {
            Ok(v) if v.get("token").and_then(Value::as_str).is_some() => json!({ "ok": true, "token": v["token"], "computer": v["computer"] }),
            Ok(_) | Err(_) => json!({ "ok": false, "error": "That address answered, but it doesn't look like openbot." }),
        },
        Ok(Err(ureq::Error::Status(_, r))) => {
            let msg = r
                .into_json::<Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "The computer said no.".into());
            json!({ "ok": false, "error": msg })
        }
        Ok(Err(e)) => json!({ "ok": false, "error": format!("Couldn't reach that address: {e}") }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    })
}

async fn get_prefs(State(st): State<Shared>) -> Json<Value> {
    Json(st.config.read().await.prefs.clone())
}

async fn put_prefs(State(st): State<Shared>, Json(v): Json<Value>) -> ApiResult<Json<Value>> {
    if !v.is_object() {
        return Err(bad("prefs must be an object"));
    }
    st.config.write().await.prefs = v.clone();
    st.save_config().await?;
    Ok(Json(v))
}

// ---------------------------------------------------------------- bots

async fn all_bots(st: &AppState) -> ApiResult<Vec<Bot>> {
    let mut out = vec![];
    for b in st.db.list_bots()? {
        out.push(st.bot_view(b).await);
    }
    Ok(out)
}

async fn get_bot(st: &AppState, id: &str) -> ApiResult<Bot> {
    st.db.get_bot(id)?.ok_or_else(|| not_found("No such bot"))
}

pub fn usage_json(st: &AppState) -> Value {
    let rate_limit: Value = st
        .db
        .kv_get("rate_limit")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let week_start = now_ms() - 7 * 24 * 3600 * 1000;
    let (week, total, per) = st.db.usage_summary(week_start).unwrap_or((0.0, 0.0, vec![]));
    json!({
        "rate_limit": rate_limit,
        "week_cost_usd": week,
        "total_cost_usd": total,
        "per_bot": per.into_iter().map(|(b, c, t)| json!({"bot_id": b, "cost_usd": c, "turns": t})).collect::<Vec<_>>(),
    })
}

async fn bootstrap(State(st): State<Shared>) -> ApiResult<Json<Value>> {
    let bots = all_bots(&st).await?;
    let settings = st.config.read().await.settings_json();
    let computer = computer_json(&st).await;
    Ok(Json(json!({
        "bots": bots,
        "settings": settings,
        "computer": computer,
        "usage": usage_json(&st),
        "version": crate::VERSION,
    })))
}

async fn list_bots(State(st): State<Shared>) -> ApiResult<Json<Vec<Bot>>> {
    Ok(Json(all_bots(&st).await?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BotBody {
    name: Option<String>,
    color: Option<String>,
    shape: Option<String>,
    instructions: Option<String>,
    model: Option<String>,
    hidden: Option<bool>,
    label: Option<String>,
    pinned: Option<bool>,
    notifications: Option<bool>,
}

async fn create_bot(State(st): State<Shared>, Json(b): Json<BotBody>) -> ApiResult<Json<Bot>> {
    let name = b.name.unwrap_or_default();
    if name.trim().is_empty() {
        return Err(bad("Give your bot a name"));
    }
    let nb = crate::bots::NewBot {
        name,
        color: b.color.unwrap_or_default(),
        shape: b.shape.unwrap_or_default(),
        instructions: b.instructions.unwrap_or_default(),
        model: b.model.unwrap_or_default(),
        label: b.label.unwrap_or_default(),
    };
    let bot = crate::bots::create(&st, &nb).await?;
    crate::bots::say_hello(&st, &bot, "").await;
    Ok(Json(bot))
}

async fn patch_bot(State(st): State<Shared>, UrlPath(id): UrlPath<String>, Json(b): Json<BotBody>) -> ApiResult<Json<Bot>> {
    let mut bot = get_bot(&st, &id).await?;
    if let Some(n) = b.name {
        if n.trim().is_empty() {
            return Err(bad("Give your bot a name"));
        }
        bot.name = n.trim().to_string();
    }
    if let Some(v) = b.color { bot.color = v; }
    if let Some(v) = b.shape { bot.shape = v; }
    if let Some(v) = b.instructions { bot.instructions = v; }
    if let Some(v) = b.model { bot.model = v; }
    if let Some(v) = b.hidden { bot.hidden = v; }
    if let Some(v) = b.label { bot.label = crate::util::truncate_chars(v.trim(), 60); }
    if let Some(v) = b.pinned { bot.pinned = v; }
    if let Some(v) = b.notifications { bot.notifications = v; }
    st.db.upsert_bot(&bot)?;
    let bot = st.bot_view(bot).await;
    st.emit("bot.updated", json!({ "bot": bot }));
    Ok(Json(bot))
}

async fn delete_bot(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    get_bot(&st, &id).await?;
    crate::bots::delete(&st, &id).await.map_err(|e| bad(&e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn reset_bot(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let mut bot = get_bot(&st, &id).await?;
    runner::stop(&st, &id).await;
    st.db.clear_messages(&id)?;
    bot.session_id.clear();
    bot.last_preview.clear();
    bot.unread = 0;
    st.db.upsert_bot(&bot)?;
    st.emit("messages.cleared", json!({ "bot_id": id }));
    st.emit_bot(&id).await;
    Ok(Json(json!({ "ok": true })))
}

async fn read_bot(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let mut bot = get_bot(&st, &id).await?;
    st.db.mark_routines_read(&id);
    if bot.unread != 0 {
        bot.unread = 0;
        st.db.upsert_bot(&bot)?;
        st.emit_bot(&id).await;
    }
    Ok(Json(json!({ "ok": true })))
}

/// "Mark as Unread" in the sidebar menu.
async fn unread_bot(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let mut bot = get_bot(&st, &id).await?;
    if bot.unread == 0 {
        bot.unread = 1;
        st.db.upsert_bot(&bot)?;
        st.emit_bot(&id).await;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn list_messages(
    State(st): State<Shared>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<Message>>> {
    let before = q.get("before").and_then(|s| s.parse().ok());
    let limit = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(200).clamp(1, 1000);
    Ok(Json(st.db.list_messages(&id, before, limit)?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SendBody {
    text: String,
    attachments: Vec<Value>,
}

async fn send_message(State(st): State<Shared>, UrlPath(id): UrlPath<String>, Json(b): Json<SendBody>) -> ApiResult<Json<Message>> {
    Ok(Json(post_user_message(&st, &id, b).await?))
}

async fn post_user_message(st: &Shared, id: &str, b: SendBody) -> ApiResult<Message> {
    let st = st.clone();
    let id = id.to_string();
    let mut bot = get_bot(&st, &id).await?;
    let text = b.text.trim().to_string();
    if text.is_empty() && b.attachments.is_empty() {
        return Err(bad("Type a message first"));
    }
    let m = Message {
        id: new_id(),
        bot_id: id.clone(),
        role: "user".into(),
        text: text.clone(),
        html: String::new(),
        status: "done".into(),
        meta: json!({ "attachments": b.attachments }),
        created_at: now_ms(),
    };
    st.db.upsert_message(&m)?;
    st.emit_message(true, &m);

    let mut prompt = text.clone();
    let files: Vec<String> = b
        .attachments
        .iter()
        .filter_map(|a| a.get("path").and_then(Value::as_str).map(str::to_string))
        .collect();
    if !files.is_empty() {
        prompt.push_str("\n\n[The user attached these files (paths are inside your working folder):]\n");
        for f in &files {
            prompt.push_str(&format!("- {f}\n"));
        }
    }

    bot.updated_at = now_ms();
    bot.last_preview = preview(if text.is_empty() { "Sent a file" } else { &text });
    bot.unread = 0;
    st.db.upsert_bot(&bot)?;
    st.emit_bot(&id).await;

    runner::enqueue(&st, &id, prompt).await;
    Ok(m)
}

async fn stop_bot(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    runner::stop(&st, &id).await;
    Ok(Json(json!({ "ok": true })))
}

fn safe_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let s: String = base
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() { "file".into() } else { s }
}

async fn upload(State(st): State<Shared>, UrlPath(id): UrlPath<String>, mut mp: Multipart) -> ApiResult<Json<Vec<Value>>> {
    let bot = get_bot(&st, &id).await?;
    let dir = PathBuf::from(&bot.workdir).join("uploads");
    tokio::fs::create_dir_all(&dir).await?;
    let mut out = vec![];
    while let Some(field) = mp.next_field().await.map_err(|e| bad(&e.to_string()))? {
        let Some(fname) = field.file_name().map(safe_file_name) else { continue };
        let data = field.bytes().await.map_err(|e| bad(&e.to_string()))?;
        let (stem, ext) = match fname.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (fname.clone(), String::new()),
        };
        let mut name = fname.clone();
        let mut n = 2;
        while dir.join(&name).exists() {
            name = format!("{stem}-{n}{ext}");
            n += 1;
        }
        tokio::fs::write(dir.join(&name), &data).await?;
        let rel = format!("uploads/{name}");
        let mime = mime_guess::from_path(&name).first_or_octet_stream().to_string();
        out.push(json!({
            "name": name,
            "path": rel,
            "mime": mime,
            "size": data.len(),
            "url": format!("/api/bots/{id}/files/{}", percent_encode_path(&rel)),
        }));
    }
    Ok(Json(out))
}

async fn get_file(State(st): State<Shared>, UrlPath((id, path)): UrlPath<(String, String)>) -> ApiResult<Response> {
    let bot = get_bot(&st, &id).await?;
    let root = std::fs::canonicalize(&bot.workdir)?;
    let full = std::fs::canonicalize(root.join(&path)).map_err(|_| not_found("No such file"))?;
    if !full.starts_with(&root) || !full.is_file() {
        return Err(not_found("No such file"));
    }
    let bytes = tokio::fs::read(&full).await?;
    let mime = mime_guess::from_path(&full).first_or_octet_stream().to_string();
    Ok(([(header::CONTENT_TYPE, mime)], bytes).into_response())
}

// ---------------------------------------------------------------- approvals

#[derive(Deserialize)]
struct InternalApprove {
    bot_id: String,
    tool_name: String,
    #[serde(default)]
    input: Value,
}

/// Shorten huge strings (e.g. a whole file being written) before showing them in the chat.
fn trim_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(truncate_chars(s, 1500)),
        Value::Array(a) => Value::Array(a.iter().take(50).map(trim_value).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), trim_value(v))).collect()),
        other => other.clone(),
    }
}

fn rule_for(tool: &str, input: &Value) -> String {
    match tool {
        "Bash" | "PowerShell" => {
            let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
            format!("{tool}({cmd})")
        }
        "WebFetch" => {
            let url = input.get("url").and_then(Value::as_str).unwrap_or("");
            let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("");
            if host.is_empty() { tool.into() } else { format!("WebFetch(domain:{host})") }
        }
        _ => tool.to_string(),
    }
}

/// Called by the MCP helper. Shows an approval card and waits for the user's answer.
async fn internal_approve(State(st): State<Shared>, Json(b): Json<InternalApprove>) -> ApiResult<Json<Value>> {
    let summary = runner::tool_summary(&b.tool_name, &b.input, false);

    // Auto-review: let a fast model wave through routine actions so the user isn't pestered.
    let cfg = st.config.read().await.clone();
    let mut verdict = None;
    if cfg.auto_review {
        if let Ok(Some(bot)) = st.db.get_bot(&b.bot_id) {
            runner::set_activity(&st, &b.bot_id, Some(("thinking", "Checking the action"))).await;
            let last = st.db.last_user_text(&b.bot_id).unwrap_or_default();
            verdict = crate::review::review(&cfg, &st.data_dir, &bot.name, &bot.workdir, &last, &b.tool_name, &b.input).await;
            if let Some(v) = &verdict {
                if v.allow {
                    tracing::info!("auto-review allowed {} for {}: {}", b.tool_name, bot.name, v.reason);
                    return Ok(Json(json!({ "behavior": "allow", "updatedInput": b.input })));
                }
            }
        }
    }
    let proposed_rule = verdict
        .as_ref()
        .map(|v| v.proposed_rule.clone())
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| crate::review::fallback_rule(&b.tool_name, &b.input));
    let reason = verdict.as_ref().map(|v| v.reason.clone()).unwrap_or_default();
    let allowed = ask_approval(&st, &b.bot_id, &b.tool_name, &summary, &b.input, &proposed_rule, &reason).await?;
    Ok(Json(if allowed {
        json!({ "behavior": "allow", "updatedInput": b.input })
    } else {
        json!({ "behavior": "deny", "message": "The user said no to this action. Don't retry it; ask them in the chat if you're unsure what to do." })
    }))
}

/// Show an Allow / Deny card in the bot's chat and wait for the user's answer.
/// An empty `proposed_rule` hides "Always allow". Returns false if the run ends first.
pub(crate) async fn ask_approval(
    st: &Shared,
    bot_id: &str,
    tool_name: &str,
    summary: &str,
    input: &Value,
    proposed_rule: &str,
    reason: &str,
) -> anyhow::Result<bool> {
    let approval_id = new_id();
    let m = Message {
        id: new_id(),
        bot_id: bot_id.to_string(),
        role: "approval".into(),
        text: summary.to_string(),
        html: String::new(),
        status: "pending".into(),
        meta: json!({
            "approval_id": approval_id,
            "tool_name": tool_name,
            "summary": summary,
            "input": trim_value(input),
            "proposed_rule": proposed_rule,
            "reason": reason,
        }),
        created_at: now_ms(),
    };
    st.db.upsert_message(&m)?;
    st.emit_message(true, &m);
    if let Ok(Some(mut bot)) = st.db.get_bot(bot_id) {
        bot.last_preview = format!("Needs your OK: {}", preview(summary));
        bot.updated_at = now_ms();
        bot.unread += 1;
        let _ = st.db.upsert_bot(&bot);
        st.emit_bot(bot_id).await;
    }
    if st.is_running(bot_id).await {
        runner::set_activity(st, bot_id, Some(("waiting_for_you", "Waiting for your OK"))).await;
    }

    let (tx, rx) = oneshot::channel();
    st.approvals.lock().await.insert(
        approval_id.clone(),
        PendingApproval {
            bot_id: bot_id.to_string(),
            message_id: m.id.clone(),
            tool_name: tool_name.to_string(),
            input: input.clone(),
            reply: tx,
        },
    );
    Ok(rx.await.unwrap_or(false))
}

#[derive(Deserialize)]
struct Decision {
    decision: String,
}

async fn decide_approval(State(st): State<Shared>, UrlPath(id): UrlPath<String>, Json(d): Json<Decision>) -> ApiResult<Json<Value>> {
    let p = st
        .approvals
        .lock()
        .await
        .remove(&id)
        .ok_or_else(|| not_found("This request already ended"))?;
    let allow = d.decision == "allow" || d.decision == "always";
    if d.decision == "always" && !p.tool_name.starts_with("mcp__openbot__") {
        let rule = rule_for(&p.tool_name, &p.input);
        let proposed = st
            .db
            .get_message(&p.message_id)
            .ok()
            .flatten()
            .and_then(|m| m.meta.get("proposed_rule").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        {
            let mut cfg = st.config.write().await;
            if !cfg.allow_rules.contains(&rule) {
                cfg.allow_rules.push(rule);
            }
            let proposed = proposed.trim().to_string();
            if !proposed.is_empty() && !cfg.review_allow.contains(&proposed) && cfg.review_allow.len() < 20 {
                cfg.review_allow.push(proposed);
            }
        }
        st.save_config().await?;
        let settings = st.config.read().await.settings_json();
        st.emit("settings.updated", json!({ "settings": settings }));
    }
    if let Ok(Some(mut m)) = st.db.get_message(&p.message_id) {
        m.status = if allow { "allowed" } else { "denied" }.into();
        // "allow" | "always" | "deny", so the card can show "Always allowed" after a reload.
        if let Some(o) = m.meta.as_object_mut() {
            o.insert("decision".into(), json!(d.decision));
        }
        st.db.upsert_message(&m)?;
        st.emit_message(false, &m);
    }
    if st.is_running(&p.bot_id).await {
        let label = if allow { "Working" } else { "Thinking" };
        runner::set_activity(&st, &p.bot_id, Some((if allow { "working" } else { "thinking" }, label))).await;
    }
    let _ = p.reply.send(allow);
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- openbot tools, widgets, routines, memories, pages

#[derive(Deserialize)]
struct ToolCall {
    bot_id: String,
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn internal_tool(State(st): State<Shared>, Json(c): Json<ToolCall>) -> Json<Value> {
    let r = crate::tools::call(&st, &c.bot_id, &c.name, &c.arguments).await;
    let image = r.image.map(|(data, mime)| json!({ "data": data, "mime": mime }));
    Json(json!({ "text": r.text, "is_error": r.is_error, "image": image }))
}

// ---------------------------------------------------------------- bot screens (screen.rs)

static SCREEN_SETUP_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Are bot screens possible here, and is everything installed?
async fn screens_status(State(st): State<Shared>) -> Json<Value> {
    let cfg = st.config.read().await.clone();
    let mut v = crate::screen::status(&cfg).await;
    v["setup_running"] = json!(SCREEN_SETUP_RUNNING.load(std::sync::atomic::Ordering::SeqCst));
    v["running"] = json!(st.screens.running_ids().await);
    Json(v)
}

/// Install what screens need (takes a few minutes). Progress arrives as "screens.setup" events.
async fn screens_setup(State(st): State<Shared>) -> ApiResult<Json<Value>> {
    crate::screen::platform_ok().map_err(|e| bad(&e))?;
    if SCREEN_SETUP_RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Ok(Json(json!({ "started": false, "running": true })));
    }
    st.emit("screens.setup", json!({ "state": "running" }));
    let st2 = st.clone();
    tokio::spawn(async move {
        let cfg = st2.config.read().await.clone();
        let result = crate::screen::setup(&cfg).await;
        SCREEN_SETUP_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
        match result {
            Ok(log) => st2.emit("screens.setup", json!({ "state": "done", "log": log })),
            Err(e) => st2.emit("screens.setup", json!({ "state": "error", "error": e })),
        }
    });
    Ok(Json(json!({ "started": true })))
}

async fn bot_screen(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    st.db.get_bot(&id)?.ok_or_else(|| not_found("No such bot"))?;
    let supported = crate::screen::platform_ok();
    Ok(Json(json!({
        "supported": supported.is_ok(),
        "reason": supported.err(),
        "running": st.screens.is_running(&id).await,
        "width": crate::screen::WIDTH,
        "height": crate::screen::HEIGHT,
    })))
}

/// Start (or reuse) the bot's screen. The password is what the viewer needs to connect.
async fn bot_screen_start(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    st.db.get_bot(&id)?.ok_or_else(|| not_found("No such bot"))?;
    let info = st.screens.ensure(&st, &id).await.map_err(|e| bad(&e))?;
    Ok(Json(json!({
        "running": true,
        "password": info.password,
        "width": crate::screen::WIDTH,
        "height": crate::screen::HEIGHT,
    })))
}

async fn bot_screen_stop(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    st.screens.stop(&st, &id).await;
    Ok(Json(json!({ "running": false })))
}

/// Live view: the app's noVNC viewer connects here; we forward to the screen's VNC server.
async fn bot_screen_ws(State(st): State<Shared>, UrlPath(id): UrlPath<String>, ws: WebSocketUpgrade) -> ApiResult<Response> {
    st.db.get_bot(&id)?.ok_or_else(|| not_found("No such bot"))?;
    let info = st.screens.ensure(&st, &id).await.map_err(|e| bad(&e))?;
    Ok(ws
        .protocols(["binary"])
        .max_message_size(64 * 1024 * 1024)
        .on_upgrade(move |socket| crate::screen::proxy(st, id, socket, info.port)))
}

#[derive(Deserialize)]
struct Answer {
    value: String,
}

/// The user tapped a quick-reply button (or typed a custom answer) on an ask_user card.
async fn answer_widget(State(st): State<Shared>, UrlPath(id): UrlPath<String>, Json(a): Json<Answer>) -> ApiResult<Json<Message>> {
    let mut m = st.db.get_message(&id)?.ok_or_else(|| not_found("No such question"))?;
    if m.role != "widget" {
        return Err(bad("That message isn't a question"));
    }
    if !m.meta["answer"].is_null() {
        return Err(bad("Already answered"));
    }
    let value = a.value.trim().to_string();
    if value.is_empty() {
        return Err(bad("Pick an answer"));
    }
    m.meta["answer"] = json!(value);
    st.db.upsert_message(&m)?;
    st.emit_message(false, &m);
    let user = post_user_message(&st, &m.bot_id, SendBody { text: value, attachments: vec![] }).await?;
    Ok(Json(user))
}

async fn list_routines(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Vec<Value>>> {
    Ok(Json(st.db.routines_for(Some(&id))?.iter().map(crate::routines::routine_json).collect()))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RoutinePatch {
    name: Option<String>,
    prompt: Option<String>,
    schedule: Option<Value>,
    enabled: Option<bool>,
}

async fn patch_routine(State(st): State<Shared>, UrlPath(id): UrlPath<String>, Json(p): Json<RoutinePatch>) -> ApiResult<Json<Value>> {
    let mut r = st.db.get_routine(&id)?.ok_or_else(|| not_found("No such routine"))?;
    if let Some(v) = p.name { r.name = v.trim().to_string(); }
    if let Some(v) = p.prompt { r.prompt = v.trim().to_string(); }
    if let Some(v) = p.schedule { r.schedule = crate::routines::validate(&v).map_err(|e| bad(&e))?; }
    if let Some(v) = p.enabled { r.enabled = v; r.unread_runs = 0; }
    r.next_run_at = if r.enabled { crate::routines::next_run(&r.schedule, now_ms()) } else { None };
    st.db.upsert_routine(&r)?;
    st.emit("routines.updated", json!({ "bot_id": r.bot_id }));
    Ok(Json(crate::routines::routine_json(&r)))
}

/// "Test routine": run it once right now.
async fn run_routine_now(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let r = st.db.get_routine(&id)?.ok_or_else(|| not_found("No such routine"))?;
    runner::add_event(&st, &r.bot_id, &format!("Testing routine: {}", r.name), json!({"kind": "routine-run", "routine_id": r.id}));
    let prompt = format!(
        "[openbot] The user asked you to test your routine \"{}\" now. Task: {}\nDo it, then send the user what's worth their attention.",
        r.name, r.prompt
    );
    runner::enqueue(&st, &r.bot_id, prompt).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_routine(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    if let Some(r) = st.db.get_routine(&id)? {
        st.db.delete_routine(&id)?;
        st.emit("routines.updated", json!({ "bot_id": r.bot_id }));
    }
    Ok(Json(json!({ "ok": true })))
}

async fn list_memories(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    Ok(Json(json!(st.db.memories_for(&id)?)))
}

async fn delete_memory(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    st.db.delete_memory(&id)?;
    st.emit("memories.updated", json!({}));
    Ok(Json(json!({ "ok": true })))
}

/// Pages made by bots (show_page). The random id is the key; the page runs sandboxed in its own origin.
async fn get_page(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(st.data_dir.join("pages").join(format!("{id}.html"))).await {
        Ok(mut bytes) => {
            // Tell the chat how tall the page is, so the frame fits it (the UI listens for this).
            bytes.extend_from_slice(PAGE_HEIGHT_SCRIPT.as_bytes());
            (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts allow-popups allow-forms"),
                (header::CACHE_CONTROL, "private, max-age=3600"),
            ],
            bytes,
        )
            .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// `:where(html)` has no weight, so a page that sets its own colors keeps them.
const PAGE_HEIGHT_SCRIPT: &str = "\n<style>:where(html){color-scheme:dark;background:#0b0b0c;color:#ececec}</style><script>(function(){function s(){try{parent.postMessage({type:'openbot:height',height:document.documentElement.scrollHeight},'*')}catch(e){}}addEventListener('load',s);addEventListener('resize',s);if(window.ResizeObserver){new ResizeObserver(s).observe(document.documentElement)}setTimeout(s,0)})();</script>\n";

async fn make_main(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let bot = get_bot(&st, &id).await?;
    let old = st.db.kv_get("main_bot");
    st.db.kv_set("main_bot", &bot.id);
    if let Some(o) = old {
        st.emit_bot(&o).await;
    }
    st.emit_bot(&id).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ImportBody {
    text: String,
}

/// "Import memory": paste what another assistant knows about you; one fact per line.
async fn import_memories(State(st): State<Shared>, Json(b): Json<ImportBody>) -> ApiResult<Json<Value>> {
    let mut n = 0;
    for line in b.text.lines() {
        let fact = line.trim().trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_ascii_digit() || c == '.' || c == ')').trim();
        if fact.len() < 3 {
            continue;
        }
        st.db.add_memory(&crate::db::Memory {
            id: new_id()[..8].to_string(),
            bot_id: None,
            text: crate::util::truncate_chars(fact, 500),
            created_at: now_ms(),
        })?;
        n += 1;
        if n >= 300 {
            break;
        }
    }
    st.emit("memories.updated", json!({}));
    Ok(Json(json!({ "imported": n })))
}

fn skill_dir(bot: &Bot) -> PathBuf {
    PathBuf::from(&bot.workdir).join(".claude").join("skills")
}

fn valid_skill_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Skills are Claude Code skills in the bot's folder: .claude/skills/<name>/SKILL.md
async fn list_skills(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Vec<Value>>> {
    let bot = get_bot(&st, &id).await?;
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(skill_dir(&bot)) {
        for e in rd.flatten() {
            let text = std::fs::read_to_string(e.path().join("SKILL.md")).unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            let (mut desc, mut body) = (String::new(), text.clone());
            if let Some(rest) = text.strip_prefix("---") {
                if let Some(end) = rest.find("\n---") {
                    for l in rest[..end].lines() {
                        if let Some(d) = l.trim().strip_prefix("description:") {
                            desc = d.trim().trim_matches('"').to_string();
                        }
                    }
                    body = rest[end + 4..].trim_start().to_string();
                }
            }
            out.push(json!({ "name": e.file_name().to_string_lossy(), "description": desc, "body": body }));
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(Json(out))
}

#[derive(Deserialize)]
struct SkillBody {
    #[serde(default)]
    description: String,
    body: String,
}

async fn put_skill(
    State(st): State<Shared>,
    UrlPath((id, name)): UrlPath<(String, String)>,
    Json(b): Json<SkillBody>,
) -> ApiResult<Json<Value>> {
    let bot = get_bot(&st, &id).await?;
    if !valid_skill_name(&name) {
        return Err(bad("Use lowercase letters, numbers and dashes, like weekly-report"));
    }
    if b.body.trim().is_empty() {
        return Err(bad("Write the skill's instructions"));
    }
    let dir = skill_dir(&bot).join(&name);
    std::fs::create_dir_all(&dir)?;
    let desc = b.description.replace(['\n', '"'], " ");
    std::fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\ndescription: \"{}\"\n---\n\n{}\n", desc.trim(), b.body.trim()))?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_skill(State(st): State<Shared>, UrlPath((id, name)): UrlPath<(String, String)>) -> ApiResult<Json<Value>> {
    let bot = get_bot(&st, &id).await?;
    if valid_skill_name(&name) {
        let _ = std::fs::remove_dir_all(skill_dir(&bot).join(&name));
    }
    Ok(Json(json!({ "ok": true })))
}

/// Files library: what's in the bot's folder (newest first, skips hidden folders).
async fn list_library(State(st): State<Shared>, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Vec<Value>>> {
    let bot = get_bot(&st, &id).await?;
    let root = PathBuf::from(&bot.workdir);
    let mut out = vec![];
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                if stack.len() < 200 {
                    stack.push(e.path());
                }
                continue;
            }
            let rel = e.path().strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let modified = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            out.push(json!({
                "name": name, "path": rel, "size": md.len(), "modified": modified,
                "mime": mime_guess::from_path(&name).first_or_octet_stream().to_string(),
                "url": format!("/api/bots/{id}/files/{}", percent_encode_path(&rel)),
            }));
            if out.len() >= 2000 {
                break;
            }
        }
    }
    out.sort_by(|a, b| b["modified"].as_i64().cmp(&a["modified"].as_i64()));
    Ok(Json(out))
}

/// Add or replace one of the user's MCP servers.
async fn add_mcp_server(State(st): State<Shared>, Json(v): Json<Value>) -> ApiResult<Json<Value>> {
    let name = v.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if name.is_empty() || name == "openbot" || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(bad("Give the server a name with letters, numbers, - or _"));
    }
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("http");
    let entry = if kind == "http" {
        let url = v.get("url").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(bad("Enter the server's URL (https://...)"));
        }
        json!({"name": name, "type": "http", "url": url, "headers": v.get("headers").cloned().unwrap_or(json!({}))})
    } else {
        let command = v.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if command.is_empty() {
            return Err(bad("Enter the command that starts the server"));
        }
        json!({"name": name, "type": "stdio", "command": command,
               "args": v.get("args").cloned().unwrap_or(json!([])), "env": v.get("env").cloned().unwrap_or(json!({}))})
    };
    {
        let mut cfg = st.config.write().await;
        cfg.mcp_servers.retain(|s| s.get("name").and_then(Value::as_str) != Some(name.as_str()));
        cfg.mcp_servers.push(entry);
    }
    st.save_config().await?;
    let settings = st.config.read().await.settings_json();
    st.emit("settings.updated", json!({ "settings": settings }));
    Ok(Json(settings))
}

async fn remove_mcp_server(State(st): State<Shared>, UrlPath(name): UrlPath<String>) -> ApiResult<Json<Value>> {
    st.config
        .write()
        .await
        .mcp_servers
        .retain(|s| s.get("name").and_then(Value::as_str) != Some(name.as_str()));
    st.save_config().await?;
    let settings = st.config.read().await.settings_json();
    st.emit("settings.updated", json!({ "settings": settings }));
    Ok(Json(settings))
}

/// Updates › "Update Claude Code": runs `claude update` on the bots' computer.
async fn update_claude(State(st): State<Shared>) -> ApiResult<Json<Value>> {
    let path = st.config.read().await.claude_path.clone();
    let claude = crate::computer::find_claude(&path).ok_or_else(|| bad("Claude Code isn't installed on this computer"))?;
    let mut cmd = tokio::process::Command::new(claude);
    cmd.arg("update").stdin(std::process::Stdio::null()).kill_on_drop(true);
    crate::util::hide_console(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| bad("The update took too long"))??;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok(Json(json!({ "ok": out.status.success(), "output": crate::util::truncate_chars(text.trim(), 2000) })))
}

/// Settings › General › Sign Out: signs Claude Code out on this computer.
async fn logout_claude(State(st): State<Shared>) -> ApiResult<Json<Value>> {
    let path = st.config.read().await.claude_path.clone();
    let claude = crate::computer::find_claude(&path).ok_or_else(|| bad("Claude Code isn't installed on this computer"))?;
    let mut cmd = tokio::process::Command::new(claude);
    cmd.args(["auth", "logout"]).stdin(std::process::Stdio::null()).kill_on_drop(true);
    crate::util::hide_console(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| bad("Signing out took too long"))??;
    crate::computer::forget_claude_info();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err(bad(&format!("Couldn't sign out: {}", crate::util::truncate_chars(text.trim(), 300))));
    }
    Ok(Json(json!({ "ok": true, "output": crate::util::truncate_chars(text.trim(), 2000) })))
}

// ---------------------------------------------------------------- settings + vault

async fn get_settings(State(st): State<Shared>) -> Json<Value> {
    Json(st.config.read().await.settings_json())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SettingsBody {
    auth_mode: Option<String>,
    api_key: Option<String>,
    permission_mode: Option<String>,
    allow_rules: Option<Vec<String>>,
    default_model: Option<String>,
    workspace_root: Option<String>,
    claude_path: Option<String>,
    runner: Option<String>,
    wsl_distro: Option<String>,
    auto_review: Option<bool>,
    review_allow: Option<Vec<String>>,
    review_ask: Option<Vec<String>>,
    timezone: Option<String>,
}

fn clean_rules(v: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = vec![];
    for r in v {
        let r = r.trim().to_string();
        if r.is_empty() || out.contains(&r) {
            continue;
        }
        if r.chars().count() > 1000 {
            return Err(bad("Rules can be at most 1000 characters"));
        }
        out.push(r);
    }
    if out.len() > 20 {
        return Err(bad("You can have at most 20 rules in each list"));
    }
    Ok(out)
}

async fn put_settings(State(st): State<Shared>, Json(b): Json<SettingsBody>) -> ApiResult<Json<Value>> {
    let mut tz_changed = false;
    {
        let mut cfg = st.config.write().await;
        if let Some(v) = b.auth_mode {
            if v != "subscription" && v != "api_key" {
                return Err(bad("auth_mode must be subscription or api_key"));
            }
            cfg.auth_mode = v;
        }
        if let Some(v) = b.api_key {
            let v = v.trim().to_string();
            if !v.is_empty() && !v.starts_with("sk-ant-") {
                return Err(bad("That doesn't look like an Anthropic API key (they start with sk-ant-)"));
            }
            if v.is_empty() && cfg.auth_mode == "api_key" {
                cfg.auth_mode = "subscription".into();
            }
            cfg.api_key = v;
        }
        if cfg.auth_mode == "api_key" && cfg.api_key.is_empty() {
            return Err(bad("Add an API key first"));
        }
        if let Some(v) = b.permission_mode {
            if !["ask", "auto", "accept_edits", "allow_all", "never"].contains(&v.as_str()) {
                return Err(bad("Unknown permission mode"));
            }
            cfg.permission_mode = v;
        }
        if let Some(v) = b.allow_rules {
            cfg.allow_rules = v.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        if let Some(v) = b.default_model {
            cfg.default_model = v;
        }
        if let Some(v) = b.workspace_root {
            let v = v.trim().to_string();
            if v.is_empty() {
                return Err(bad("Pick a folder"));
            }
            std::fs::create_dir_all(&v).map_err(|e| bad(&format!("Can't use that folder: {e}")))?;
            cfg.workspace_root = v;
        }
        if let Some(v) = b.claude_path {
            cfg.claude_path = v.trim().to_string();
        }
        if let Some(v) = b.runner {
            if !["native", "wsl"].contains(&v.as_str()) {
                return Err(bad("runner must be \"native\" or \"wsl\""));
            }
            if v == "wsl" && !cfg!(windows) {
                return Err(bad("Running bots in WSL only works on a Windows computer"));
            }
            cfg.runner = v;
        }
        if let Some(v) = b.wsl_distro {
            let v = v.trim();
            if v.is_empty() || v.len() > 64 || !v.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
                return Err(bad("That isn't a valid WSL distro name"));
            }
            cfg.wsl_distro = v.to_string();
        }
        if let Some(v) = b.auto_review {
            cfg.auto_review = v;
        }
        if let Some(v) = b.review_allow {
            cfg.review_allow = clean_rules(v)?;
        }
        if let Some(v) = b.review_ask {
            cfg.review_ask = clean_rules(v)?;
        }
        if let Some(v) = b.timezone {
            let v = v.trim().to_string();
            if !crate::util::set_timezone(&v) {
                return Err(bad("Unknown timezone"));
            }
            if v != cfg.timezone {
                cfg.timezone = v;
                tz_changed = true;
            }
        }
    }
    if tz_changed {
        // Routine times are wall-clock times, so their next run moves with the timezone.
        let now = now_ms();
        for mut r in st.db.routines_for(None).unwrap_or_default() {
            if r.enabled && r.schedule.get("kind").and_then(Value::as_str) != Some("once") {
                r.next_run_at = crate::routines::next_run(&r.schedule, now);
                let _ = st.db.upsert_routine(&r);
                st.emit("routines.updated", json!({ "bot_id": r.bot_id }));
            }
        }
    }
    st.save_config().await?;
    let settings = st.config.read().await.settings_json();
    st.emit("settings.updated", json!({ "settings": settings }));
    Ok(Json(settings))
}

/// Hide saved secret values in earlier chats and memories (runs in the background).
fn spawn_redact_history(st: Shared) {
    tokio::task::spawn_blocking(move || match st.db.redact_history() {
        Ok(0) => {}
        Ok(n) => tracing::info!("vault: hid secret values in {n} saved messages/memories"),
        Err(e) => tracing::warn!("vault: couldn't check saved chats for secret values: {e}"),
    });
}

/// What the app sends to add or edit a secret.
#[derive(Deserialize)]
struct SecretIn {
    name: String,
    /// Empty when editing = keep the saved value.
    #[serde(default)]
    value: String,
    #[serde(default)]
    description: String,
    /// Bot ids that may use it; null or missing = every Bot.
    #[serde(default)]
    bots: Option<Vec<String>>,
}

async fn add_secret(State(st): State<Shared>, Json(s): Json<SecretIn>) -> ApiResult<Json<Value>> {
    let name = s.name.trim().to_string();
    if let Some(p) = crate::vault::name_problem(&name) {
        return Err(bad(&p));
    }
    if s.value.len() > 4096 {
        return Err(bad("Too large. Maximum size is 4 KiB."));
    }
    if s.description.chars().count() > 300 {
        return Err(bad("Keep “What it is for” under 300 characters"));
    }
    let bots = match s.bots {
        None => None,
        Some(ids) => {
            let known: Vec<String> = st.db.list_bots()?.into_iter().map(|b| b.id).collect();
            let mut ids: Vec<String> = ids.into_iter().filter(|i| known.contains(i)).collect();
            ids.dedup();
            if ids.is_empty() {
                return Err(bad("Pick at least one Bot, or choose All Bots"));
            }
            Some(ids)
        }
    };
    let new_value = !s.value.is_empty();
    {
        let mut cfg = st.config.write().await;
        let value = if new_value {
            s.value
        } else {
            // Editing only the description or the Bots keeps the saved value.
            let old = cfg.vault.iter().find(|x| x.name == name).map(|x| x.value.clone());
            let Some(value) = old else { return Err(bad("Enter the secret's value")) };
            if crate::vault::is_locked(&value) {
                return Err(bad("This secret is locked on this computer. Enter its value again."));
            }
            value
        };
        let pos = cfg.vault.iter().position(|x| x.name == name);
        let secret = Secret { name, value, description: s.description.trim().to_string(), bots };
        match pos {
            Some(i) => cfg.vault[i] = secret, // keep its place in the list
            None => cfg.vault.push(secret),
        }
    }
    st.save_config().await?;
    if new_value {
        spawn_redact_history(st.clone());
    }
    let settings = st.config.read().await.settings_json();
    st.emit("settings.updated", json!({ "settings": settings }));
    Ok(Json(settings))
}

async fn remove_secret(State(st): State<Shared>, UrlPath(name): UrlPath<String>) -> ApiResult<Json<Value>> {
    st.config.write().await.vault.retain(|x| x.name != name);
    st.save_config().await?;
    let settings = st.config.read().await.settings_json();
    st.emit("settings.updated", json!({ "settings": settings }));
    Ok(Json(settings))
}

// ---------------------------------------------------------------- misc

/// `?refresh=1` re-checks Claude Code right away (the setup screen's "Check again").
async fn get_computer(State(st): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    if q.get("refresh").map(String::as_str) == Some("1") {
        crate::computer::forget_claude_info();
    }
    Json(computer_json(&st).await)
}

async fn get_usage(State(st): State<Shared>) -> Json<Value> {
    Json(usage_json(&st))
}

async fn search(State(st): State<Shared>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    let q = q.get("q").map(|s| s.trim().to_string()).unwrap_or_default();
    if q.is_empty() {
        return Ok(Json(json!({ "bots": [], "messages": [] })));
    }
    let lower = q.to_lowercase();
    let bots: Vec<Bot> = all_bots(&st)
        .await?
        .into_iter()
        .filter(|b| b.name.to_lowercase().contains(&lower))
        .collect();
    let messages = st.db.search_messages(&q, 50)?;
    Ok(Json(json!({ "bots": bots, "messages": messages })))
}

#[derive(Deserialize)]
struct FolderBody {
    path: String,
}

async fn open_folder(Json(b): Json<FolderBody>) -> ApiResult<Json<Value>> {
    let p = PathBuf::from(&b.path);
    if !p.is_dir() {
        return Err(bad("That folder doesn't exist"));
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(&p).spawn()?;
    Ok(Json(json!({ "ok": true })))
}

async fn events(State(st): State<Shared>, ws: WebSocketUpgrade, _h: HeaderMap) -> Response {
    ws.on_upgrade(move |socket| ws_loop(st, socket))
}

async fn ws_loop(st: Shared, mut socket: WebSocket) {
    let mut rx = st.events.subscribe();
    let hello = json!({ "type": "hello", "version": crate::VERSION }).to_string();
    if socket.send(WsMessage::Text(hello.into())).await.is_err() {
        return;
    }
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(text) => {
                    if socket.send(WsMessage::Text(text.into())).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Too far behind: ask the UI to reload everything.
                    let msg = json!({ "type": "resync" }).to_string();
                    if socket.send(WsMessage::Text(msg.into())).await.is_err() { break; }
                }
                Err(_) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
            _ = ping.tick() => {
                if socket.send(WsMessage::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
}
