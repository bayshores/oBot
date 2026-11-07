//! Weekly memory tidy-up.
//!
//! Bots save facts all the time (see `remember` in tools.rs). Once a week, at night, each Bot with
//! enough saved facts gets a short housekeeping turn: merge duplicates, fix outdated facts, keep the
//! always-loaded "profile" small and move lasting facts from the "log" into it. The main Bot also
//! tidies the facts about the user that every Bot shares; other Bots only touch their own.

use std::time::Duration;

use serde_json::json;

use crate::state::Shared;
use crate::util::now_ms;

const DAY_MS: i64 = 24 * 3600 * 1000;
const WEEK_MS: i64 = 7 * DAY_MS;
/// A computer that's never on at night still gets a tidy-up after this long.
const OVERDUE_MS: i64 = 9 * DAY_MS;
/// Bots with fewer facts than this don't need tidying.
const MIN_FACTS: usize = 12;
/// kv key: when the shared facts were last imported (a tidy-up is then due soon).
pub const IMPORTED_KEY: &str = "memory_imported_at";

fn last_key(bot_id: &str) -> String {
    format!("memory_tidied:{bot_id}")
}

/// Starts the background check (once an hour).
pub fn spawn(state: Shared) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            check(&state).await;
        }
    });
}

async fn check(state: &Shared) {
    let now = now_ms();
    let hour = chrono::Timelike::hour(&crate::util::now_local());
    let imported: i64 = state.db.kv_get(IMPORTED_KEY).and_then(|v| v.parse().ok()).unwrap_or(0);
    let Ok(bots) = state.db.list_bots() else { return };
    let main_id = state.db.kv_get("main_bot").unwrap_or_default();
    let busy: Vec<String> = state.runs.lock().await.keys().cloned().collect();
    for b in bots {
        if busy.contains(&b.id) {
            continue;
        }
        let last: i64 = state.db.kv_get(&last_key(&b.id)).and_then(|v| v.parse().ok()).unwrap_or(0);
        // An import makes the main Bot's tidy-up due right away; otherwise once a week, at night.
        let main = b.id == main_id;
        let import_due = main && imported > last;
        let weekly_due = (now - last >= WEEK_MS && (2..6).contains(&hour)) || now - last >= OVERDUE_MS;
        if !import_due && !weekly_due {
            continue;
        }
        let Ok(mems) = state.db.memories_for(&b.id) else { continue };
        let own = mems.iter().filter(|m| m.bot_id.is_some()).count();
        let count = if main { mems.len() } else { own };
        state.db.kv_set(&last_key(&b.id), &now.to_string());
        if count < MIN_FACTS && !import_due {
            continue;
        }
        // A copy first, in case the Bot deletes something it shouldn't.
        if let Err(e) = backup(state, &b.id, &mems) {
            tracing::warn!("memory backup for {} failed, skipping the tidy-up: {e}", b.name);
            continue;
        }
        crate::runner::add_event(state, &b.id, "Tidying up memories", json!({"kind": "memory-tidy"}));
        crate::runner::enqueue(state, &b.id, prompt(main)).await;
        // One Bot per hour, so several tidy-ups don't run at once.
        return;
    }
}

/// Save the memories as `memory-backup.json` in the Bot's folder (the last one before each tidy-up).
fn backup(state: &Shared, bot_id: &str, mems: &[crate::db::Memory]) -> anyhow::Result<()> {
    let dir = state.data_dir.join("bots").join(bot_id);
    std::fs::create_dir_all(&dir)?;
    let rows: Vec<_> = mems
        .iter()
        .map(|m| json!({"id": m.id, "about": if m.bot_id.is_some() { "me" } else { "user" }, "tier": m.tier, "text": m.text, "created_at": m.created_at}))
        .collect();
    std::fs::write(dir.join("memory-backup.json"), serde_json::to_vec_pretty(&json!({"saved_at": now_ms(), "memories": rows}))?)?;
    Ok(())
}

/// What the Bot is told (the chat only shows "Tidying up memories").
fn prompt(main: bool) -> String {
    let scope = if main {
        "your own facts (about:'me') and the shared facts about the user (about:'user'), which every Bot sees"
    } else {
        "only your own facts (about:'me'). Leave the shared facts about the user alone; the main Bot tidies those"
    };
    format!(
        "<openbot_task kind=\"memory_tidy\">\n\
         This is openbot's weekly memory housekeeping, not a message from the user.\n\
         Scope: {scope}.\n\
         </openbot_task>\n\
         <instructions>\n\
         1. Call recall with no query to see your profile and newest log entries; search with recall for more.\n\
         2. Merge duplicates into one fact (update_memory the best one, forget the rest).\n\
         3. Fix or forget facts that are outdated or contradicted by newer ones.\n\
         4. The profile is only for standing instructions and core identity (name, location, role, family, strong \
            preferences, how the user wants you to work, and rules like \"always\", \"never\" or \"sign in to this site this \
            way\"). Move such facts from the log into the profile with \
            update_memory tier:'profile'. Move anything else in the profile to the log.\n\
         5. Keep the profile under about 40 short facts. Never delete something the user explicitly asked you to remember.\n\
         6. Don't message the user about this unless something needs their decision. End with one short line saying \
            what changed, like \"Memory tidied: merged 3 duplicates, moved 2 facts to your profile.\"\n\
         </instructions>"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_scopes() {
        assert!(super::prompt(true).contains("shared facts about the user (about:'user'), which every Bot sees"));
        assert!(super::prompt(false).contains("Leave the shared facts about the user alone"));
    }
}
