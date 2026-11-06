//! The permission helper.
//!
//! Claude Code is started with `--permission-prompt-tool mcp__openbot__approve`. When it wants to
//! do something that needs approval, it calls this tiny MCP server (this same program started with
//! the `mcp-approve` argument). We forward the request to the openbot server, which shows an
//! Allow / Deny card in the chat and waits for the user's click.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

fn send(out: &Arc<Mutex<std::io::Stdout>>, msg: Value) {
    let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writeln!(o, "{msg}");
    let _ = o.flush();
}

fn ask_server(url: &str, token: &str, bot_id: &str, args: &Value) -> Value {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .build(); // no read timeout: the user may take a while to answer
    let body = json!({
        "bot_id": bot_id,
        "tool_name": args.get("tool_name").cloned().unwrap_or(json!("")),
        "input": args.get("input").cloned().unwrap_or(json!({})),
        "tool_use_id": args.get("tool_use_id").cloned().unwrap_or(Value::Null),
    });
    match agent
        .post(&format!("{url}/api/internal/approve"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
    {
        Ok(resp) => resp
            .into_json::<Value>()
            .unwrap_or_else(|_| json!({"behavior": "deny", "message": "openbot sent an unreadable answer."})),
        Err(e) => json!({"behavior": "deny", "message": format!("openbot could not ask the user: {e}")}),
    }
}

/// Returns (text, is_error, optional (base64 image, mime type)).
fn call_tool(url: &str, token: &str, bot_id: &str, name: &str, args: &Value) -> (String, bool, Option<(String, String)>) {
    // These wait for the user's Allow / Deny, which can take a while.
    let agent = if name == "delete_bot" || crate::accounts::waits_for_user(name) || crate::jobs::waits_for_user(name) {
        ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).build()
    } else {
        ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build()
    };
    match agent
        .post(&format!("{url}/api/internal/tool"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(json!({"bot_id": bot_id, "name": name, "arguments": args}))
    {
        Ok(resp) => match resp.into_json::<Value>() {
            Ok(v) => (
                v.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                v.get("image").and_then(|i| Some((i.get("data")?.as_str()?.to_string(), i.get("mime")?.as_str()?.to_string()))),
            ),
            Err(e) => (format!("openbot sent an unreadable answer: {e}"), true, None),
        },
        Err(e) => (format!("openbot couldn't run the tool: {e}"), true, None),
    }
}

/// Run the MCP server on stdin/stdout until Claude Code closes it.
pub fn run_approval_mcp() -> anyhow::Result<()> {
    let url = std::env::var("OPENBOT_URL").unwrap_or_else(|_| "http://127.0.0.1:7777".into());
    let token = std::env::var("OPENBOT_TOKEN").unwrap_or_default();
    let bot_id = std::env::var("OPENBOT_BOT_ID").unwrap_or_default();
    let out = Arc::new(Mutex::new(std::io::stdout()));

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let Some(id) = id else { continue }; // notifications need no answer

        match method {
            "initialize" => {
                let pv = req
                    .pointer("/params/protocolVersion")
                    .cloned()
                    .unwrap_or(json!("2025-06-18"));
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": pv,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "openbot", "version": crate::VERSION}
                }}));
            }
            "ping" => send(&out, json!({"jsonrpc": "2.0", "id": id, "result": {}})),
            "tools/list" => {
                let mut tools = vec![json!({
                    "name": "approve",
                    "description": "Internal: openbot uses this to ask the user for permission. Don't call it yourself.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tool_name": {"type": "string"},
                            "input": {"type": "object"},
                            "tool_use_id": {"type": "string"}
                        },
                        "required": ["tool_name", "input"]
                    }
                })];
                if let Some(more) = crate::tools::definitions().as_array() {
                    tools.extend(more.iter().cloned());
                }
                send(&out, json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}}));
            }
            "tools/call" => {
                let name = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("").to_string();
                let args = req.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let (out, url, token, bot_id) = (out.clone(), url.clone(), token.clone(), bot_id.clone());
                // Answer on another thread so pings keep working while the user decides.
                std::thread::spawn(move || {
                    let result = if name == "approve" {
                        let decision = ask_server(&url, &token, &bot_id, &args);
                        json!({"content": [{"type": "text", "text": decision.to_string()}]})
                    } else {
                        let (text, is_error, image) = call_tool(&url, &token, &bot_id, &name, &args);
                        let mut content = vec![json!({"type": "text", "text": text})];
                        if let Some((data, mime)) = image {
                            content.push(json!({"type": "image", "data": data, "mimeType": mime}));
                        }
                        json!({"content": content, "isError": is_error})
                    };
                    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
                });
            }
            _ => send(&out, json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {method}")}})),
        }
    }
    Ok(())
}
