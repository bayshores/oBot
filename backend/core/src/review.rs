//! Auto-review: before an action that needs approval reaches the user, a fast
//! Claude model checks it against built-in safety guidance and the user's plain-language rules.
//! It answers "allow" (go ahead silently) or "ask" (show the Allow / Deny card).

use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::computer::find_claude;
use crate::config::Config;
use crate::util::{hide_console, truncate_chars};

pub struct Verdict {
    pub allow: bool,
    pub reason: String,
    /// A short rule the user could save with "Always allow", e.g. "Run git commands in my projects".
    pub proposed_rule: String,
}

const GUIDE: &str = "You review actions an AI assistant (a \"bot\") wants to take on the user's computer.\n\
Decide if the action can run without interrupting the user.\n\
ALLOW routine, low-risk actions that clearly serve the user's latest request: reading files, searching, \
running ordinary commands (builds, tests, git status/diff/log, package lists), creating or editing files inside \
the bot's working folder, web searches and page reads.\n\
ASK for anything risky or hard to undo: deleting or overwriting data outside the working folder, sending \
messages/emails/posts to other people, purchases or payments, logging in or handling passwords and secrets, \
changing system settings, installing system-wide software, sudo/admin rights, force-pushing or publishing, \
anything that touches other people's accounts, or anything that doesn't match what the user asked for.\n\
The user's own rules override your judgment. \"Always Ask\" rules win over \"Allow without Asking\" rules.\n\
Reply with ONLY a JSON object: {\"decision\":\"allow\"|\"ask\",\"reason\":\"<max 12 words>\",\
\"proposed_rule\":\"<short plain-language rule that would allow this kind of action, max 8 words>\"}";

fn parse(text: &str) -> Option<Verdict> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let v: Value = serde_json::from_str(&text[start..=end]).ok()?;
    let decision = v.get("decision")?.as_str()?.to_lowercase();
    Some(Verdict {
        allow: decision == "allow",
        reason: v.get("reason").and_then(Value::as_str).unwrap_or("").to_string(),
        proposed_rule: v.get("proposed_rule").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

/// Returns None when the reviewer couldn't decide (then the user is asked).
pub async fn review(
    cfg: &Config,
    data_dir: &std::path::Path,
    bot_name: &str,
    workdir: &str,
    last_request: &str,
    tool: &str,
    input: &Value,
) -> Option<Verdict> {
    let claude = find_claude(&cfg.claude_path)?;
    let rules = |list: &Vec<String>| {
        if list.is_empty() {
            "(none)".to_string()
        } else {
            list.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n")
        }
    };
    let prompt = format!(
        "{GUIDE}\n\nUser's \"Always Ask\" rules:\n{}\n\nUser's \"Allow without Asking\" rules:\n{}\n\n\
         Bot: {bot_name}\nBot's working folder: {workdir}\nUser's latest request to the bot: {}\n\n\
         Action the bot wants to take:\nTool: {tool}\nInput: {}\n",
        rules(&cfg.review_ask),
        rules(&cfg.review_allow),
        truncate_chars(last_request, 1500),
        truncate_chars(&input.to_string(), 4000),
    );

    let mut cmd = Command::new(&claude);
    cmd.args([
        "-p",
        "--output-format",
        "json",
        "--model",
        "haiku",
        "--max-turns",
        "1",
        "--strict-mcp-config",
        // No personal Claude Code settings, hooks, plugins or CLAUDE.md: the safety check sees only this prompt.
        "--setting-sources",
        "",
        "--permission-mode",
        "dontAsk",
        "--disallowedTools",
        "Bash",
        "Edit",
        "Write",
        "WebFetch",
        "WebSearch",
        "Task",
    ])
    .current_dir(data_dir)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .env("OPENBOT", "1");
    if cfg.auth_mode == "api_key" && !cfg.api_key.is_empty() {
        cmd.env("ANTHROPIC_API_KEY", &cfg.api_key);
    } else {
        cmd.env_remove("ANTHROPIC_API_KEY");
    }
    hide_console(&mut cmd);

    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await.ok()?;
        drop(stdin);
    }
    let out = tokio::time::timeout(Duration::from_secs(45), child.wait_with_output()).await.ok()?.ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let text = v.get("result").and_then(Value::as_str)?;
    parse(text)
}

/// For the approval card's "Always allow": a suggested plain-language rule.
pub fn fallback_rule(tool: &str, input: &Value) -> String {
    let summary = crate::runner::tool_summary(tool, input, false);
    truncate_chars(&summary, 80)
}

pub fn verdict_json(v: &Verdict) -> Value {
    json!({"allow": v.allow, "reason": v.reason, "proposed_rule": v.proposed_rule})
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_reviewer_json() {
        let v = parse("Sure: {\"decision\":\"allow\",\"reason\":\"read only\",\"proposed_rule\":\"Read files\"}").unwrap();
        assert!(v.allow);
        assert_eq!(v.proposed_rule, "Read files");
        assert!(!parse("{\"decision\":\"ask\"}").unwrap().allow);
        assert!(parse("no json").is_none());
    }
}
