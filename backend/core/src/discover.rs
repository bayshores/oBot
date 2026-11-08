//! Finding computers that run oBot, for the "Connect to another computer" setup.
//!
//! oBot only answers on Tailscale (see README), so discovery asks this computer's Tailscale
//! for the list of your devices and checks each online one for oBot at
//! `https://<device>.<tailnet>.ts.net/api/hello`.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};

use crate::util::hide_console;

/// Where the `tailscale` command lives on this computer, if it's installed.
pub fn find_tailscale() -> Option<PathBuf> {
    if let Ok(p) = which::which("tailscale") {
        return Some(p);
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
            "/usr/local/bin/tailscale",
            "/opt/homebrew/bin/tailscale",
        ]
    } else if cfg!(windows) {
        &[r"C:\Program Files\Tailscale\tailscale.exe", r"C:\Program Files (x86)\Tailscale\tailscale.exe"]
    } else {
        &["/usr/bin/tailscale", "/usr/local/bin/tailscale"]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

fn clean_dns(v: &Value) -> String {
    v.as_str().unwrap_or("").trim_end_matches('.').to_string()
}

fn device(v: &Value) -> Value {
    json!({
        "name": v.get("HostName").and_then(Value::as_str).unwrap_or(""),
        "dns": clean_dns(v.get("DNSName").unwrap_or(&Value::Null)),
        "os": v.get("OS").and_then(Value::as_str).unwrap_or(""),
        "online": v.get("Online").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// `{installed, running, needs_login, self, peers:[{name, dns, os, online}]}`
pub async fn tailscale_status() -> Value {
    let Some(ts) = find_tailscale() else {
        return json!({ "installed": false, "running": false, "needs_login": false, "self": null, "peers": [] });
    };
    let mut cmd = tokio::process::Command::new(&ts);
    cmd.args(["status", "--json"]).stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    hide_console(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(6), cmd.output()).await;
    let v: Value = match out {
        Ok(Ok(o)) => serde_json::from_slice(&o.stdout).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let state = v.get("BackendState").and_then(Value::as_str).unwrap_or("");
    let running = state == "Running";
    let mut peers: Vec<Value> = v
        .get("Peer")
        .and_then(Value::as_object)
        .map(|m| m.values().map(device).collect())
        .unwrap_or_default();
    // Online first, then by name.
    peers.sort_by(|a, b| {
        let ao = a["online"].as_bool().unwrap_or(false);
        let bo = b["online"].as_bool().unwrap_or(false);
        bo.cmp(&ao).then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    json!({
        "installed": true,
        "running": running,
        // "NeedsLogin" / "NoState" / "Stopped" (app open but switched off)
        "needs_login": state == "NeedsLogin" || state == "NoState",
        "state": state,
        "self": v.get("Self").map(device),
        "peers": peers,
    })
}

/// Ask one address whether oBot runs there. Returns its /api/hello answer.
pub fn probe(url: &str) -> Option<Value> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(3500)).build();
    let v: Value = agent.get(&format!("{}/api/hello", url.trim_end_matches('/'))).call().ok()?.into_json().ok()?;
    // "openbot" is what computers answer before they update to oBot.
    matches!(v.get("app").and_then(Value::as_str), Some("oBot" | "openbot")).then_some(v)
}

/// Check every online Tailscale device for oBot (in parallel). `[{name, dns, os, url, version, computer_name}]`
pub async fn discover() -> Value {
    let status = tailscale_status().await;
    let peers: Vec<Value> = status["peers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p["online"].as_bool().unwrap_or(false) && !p["dns"].as_str().unwrap_or("").is_empty())
        .collect();
    let mut tasks = vec![];
    for p in peers {
        tasks.push(tokio::task::spawn_blocking(move || {
            let url = format!("https://{}", p["dns"].as_str().unwrap_or(""));
            probe(&url).map(|hello| {
                json!({
                    "name": p["name"], "dns": p["dns"], "os": p["os"], "url": url,
                    "version": hello.get("version"), "computer_name": hello.get("name"),
                })
            })
        }));
    }
    let mut found = vec![];
    for t in tasks {
        if let Ok(Some(v)) = t.await {
            found.push(v);
        }
    }
    json!({ "tailscale": status, "found": found })
}
