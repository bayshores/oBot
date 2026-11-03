//! Routines: tasks a bot runs on a schedule, plus one-time
//! "check back later" wakeups. A background loop checks every 20 seconds for anything due.
//!
//! Schedule JSON (times are the bot computer's local time, "HH:MM", 24-hour):
//!   {"kind":"once","at":<unix ms>}
//!   {"kind":"interval","minutes":30}
//!   {"kind":"daily","time":"08:00"}
//!   {"kind":"weekdays","time":"08:00"}                (Monday–Friday)
//!   {"kind":"weekly","day":"monday","time":"09:30"}
//!   {"kind":"monthly","day":1,"time":"09:00"}

use std::time::Duration;

use chrono::{Datelike, Local, NaiveTime, TimeZone, Weekday};
use serde_json::{json, Value};

use crate::db::Routine;
use crate::runner;
use crate::state::Shared;
use crate::util::now_ms;

/// After this many runs in a row that nobody looked at, the routine pauses itself.
pub const MAX_UNREAD_RUNS: i64 = 5;

fn parse_time(v: &Value) -> Option<NaiveTime> {
    let s = v.get("time").and_then(Value::as_str).unwrap_or("09:00");
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .ok()
        .or_else(|| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S").ok())
}

fn parse_weekday(v: &Value) -> Option<Weekday> {
    match v.get("day") {
        Some(Value::Number(n)) => match n.as_u64()? % 7 {
            0 => Some(Weekday::Sun),
            1 => Some(Weekday::Mon),
            2 => Some(Weekday::Tue),
            3 => Some(Weekday::Wed),
            4 => Some(Weekday::Thu),
            5 => Some(Weekday::Fri),
            _ => Some(Weekday::Sat),
        },
        Some(Value::String(s)) => s.trim().parse::<Weekday>().ok(),
        _ => None,
    }
}

fn local_ms(date: chrono::NaiveDate, t: NaiveTime) -> Option<i64> {
    Local
        .from_local_datetime(&date.and_time(t))
        .earliest()
        .map(|d| d.timestamp_millis())
}

/// Check a schedule and return a clean copy, or an error message for the bot.
pub fn validate(s: &Value) -> Result<Value, String> {
    let kind = s.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "once" => {
            let at = s.get("at").and_then(Value::as_i64).ok_or("once needs \"at\" (unix ms)")?;
            Ok(json!({"kind": "once", "at": at}))
        }
        "interval" => {
            let m = s.get("minutes").and_then(Value::as_i64).ok_or("interval needs \"minutes\"")?;
            if m < 5 {
                return Err("The shortest interval is 5 minutes".into());
            }
            Ok(json!({"kind": "interval", "minutes": m}))
        }
        "daily" | "weekdays" => {
            let t = parse_time(s).ok_or("time must look like \"08:30\"")?;
            Ok(json!({"kind": kind, "time": t.format("%H:%M").to_string()}))
        }
        "weekly" => {
            let t = parse_time(s).ok_or("time must look like \"08:30\"")?;
            let d = parse_weekday(s).ok_or("weekly needs \"day\" like \"monday\"")?;
            Ok(json!({"kind": "weekly", "day": d.to_string().to_lowercase(), "time": t.format("%H:%M").to_string()}))
        }
        "monthly" => {
            let t = parse_time(s).ok_or("time must look like \"08:30\"")?;
            let d = s.get("day").and_then(Value::as_u64).filter(|d| (1..=31).contains(d)).ok_or("monthly needs \"day\" 1-31")?;
            Ok(json!({"kind": "monthly", "day": d, "time": t.format("%H:%M").to_string()}))
        }
        _ => Err("kind must be once, interval, daily, weekdays, weekly or monthly".into()),
    }
}

/// Next time (unix ms) strictly after `after_ms`, or None if it won't run again.
pub fn next_run(s: &Value, after_ms: i64) -> Option<i64> {
    let after = Local.timestamp_millis_opt(after_ms).single()?;
    let kind = s.get("kind").and_then(Value::as_str)?;
    match kind {
        "once" => {
            let at = s.get("at").and_then(Value::as_i64)?;
            (at > after_ms).then_some(at)
        }
        "interval" => {
            let m = s.get("minutes").and_then(Value::as_i64)?.max(5);
            Some(after_ms + m * 60_000)
        }
        "daily" | "weekdays" | "weekly" | "monthly" => {
            let t = parse_time(s)?;
            let wd = parse_weekday(s);
            let mday = s.get("day").and_then(Value::as_u64).unwrap_or(1) as u32;
            let mut date = after.date_naive();
            for _ in 0..400 {
                let ok = match kind {
                    "daily" => true,
                    "weekdays" => !matches!(date.weekday(), Weekday::Sat | Weekday::Sun),
                    "weekly" => Some(date.weekday()) == wd,
                    _ => date.day() == mday,
                };
                if ok {
                    if let Some(ms) = local_ms(date, t) {
                        if ms > after_ms {
                            return Some(ms);
                        }
                    }
                }
                date = date.succ_opt()?;
            }
            None
        }
        _ => None,
    }
}

/// Human description, e.g. "Every weekday at 08:00".
pub fn describe(s: &Value) -> String {
    let time = s.get("time").and_then(Value::as_str).unwrap_or("");
    match s.get("kind").and_then(Value::as_str).unwrap_or("") {
        "once" => {
            let at = s.get("at").and_then(Value::as_i64).unwrap_or(0);
            match Local.timestamp_millis_opt(at).single() {
                Some(d) => format!("Once, {}", d.format("%b %-d at %H:%M")),
                None => "Once".into(),
            }
        }
        "interval" => {
            let m = s.get("minutes").and_then(Value::as_i64).unwrap_or(0);
            if m % 1440 == 0 {
                format!("Every {} day(s)", m / 1440)
            } else if m % 60 == 0 {
                format!("Every {} hour(s)", m / 60)
            } else {
                format!("Every {m} minutes")
            }
        }
        "daily" => format!("Every day at {time}"),
        "weekdays" => format!("Every weekday at {time}"),
        "weekly" => {
            let d = s.get("day").and_then(Value::as_str).unwrap_or("");
            let mut c = d.chars();
            let d = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
            format!("Every {d} at {time}")
        }
        "monthly" => format!("Monthly on day {} at {time}", s.get("day").and_then(Value::as_u64).unwrap_or(1)),
        _ => "Custom".into(),
    }
}

pub fn routine_json(r: &Routine) -> Value {
    json!({
        "id": r.id, "bot_id": r.bot_id, "name": r.name, "prompt": r.prompt, "schedule": r.schedule,
        "schedule_text": describe(&r.schedule), "enabled": r.enabled, "next_run_at": r.next_run_at,
        "last_run_at": r.last_run_at, "paused_for_unread": r.unread_runs >= MAX_UNREAD_RUNS,
    })
}

/// Background loop that starts due routines.
pub fn spawn_scheduler(state: Shared) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(20));
        loop {
            tick.tick().await;
            let now = now_ms();
            let Ok(due) = state.db.due_routines(now) else { continue };
            for mut r in due {
                let is_wakeup = r.schedule.get("kind").and_then(Value::as_str) == Some("once");
                r.last_run_at = Some(now);
                r.next_run_at = next_run(&r.schedule, now);
                if r.next_run_at.is_none() {
                    r.enabled = false;
                }
                r.unread_runs += 1;
                if r.unread_runs > MAX_UNREAD_RUNS && !is_wakeup {
                    // Nobody has looked at the results for a while: pause instead of wasting usage.
                    r.enabled = false;
                    let _ = state.db.upsert_routine(&r);
                    runner::add_event(
                        &state,
                        &r.bot_id,
                        &format!("Paused the routine “{}” because nobody has read its results. Turn it back on in the bot's settings.", r.name),
                        json!({"kind": "routine-paused", "routine_id": r.id}),
                    );
                    state.emit("routines.updated", json!({ "bot_id": r.bot_id }));
                    continue;
                }
                let _ = state.db.upsert_routine(&r);
                state.emit("routines.updated", json!({ "bot_id": r.bot_id }));

                let label = if is_wakeup { "Checking back in" } else { "Running routine" };
                runner::add_event(
                    &state,
                    &r.bot_id,
                    &format!("{label}: {}", r.name),
                    json!({"kind": "routine-run", "routine_id": r.id}),
                );
                let prompt = if is_wakeup {
                    format!(
                        "[openbot] It's time for the check-in you scheduled earlier (\"{}\"). Your note to yourself: {}\n\
                         Do what you planned, then tell the user what they need to know.",
                        r.name, r.prompt
                    )
                } else {
                    format!(
                        "[openbot] Your routine \"{}\" ({}) is due now. Task: {}\n\
                         Do it, then send the user only what's worth their attention.",
                        r.name,
                        describe(&r.schedule),
                        r.prompt
                    )
                };
                runner::enqueue(&state, &r.bot_id, prompt).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone, Timelike};

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
    }

    #[test]
    fn daily_and_weekdays() {
        // Friday 2026-11-06 10:00
        let now = at(2026, 11, 6, 10, 0);
        let daily = next_run(&json!({"kind":"daily","time":"08:00"}), now).unwrap();
        assert_eq!(daily, at(2026, 11, 7, 8, 0));
        let wk = next_run(&json!({"kind":"weekdays","time":"08:00"}), now).unwrap();
        assert_eq!(wk, at(2026, 11, 9, 8, 0)); // skips the weekend
        let later_today = next_run(&json!({"kind":"daily","time":"18:30"}), now).unwrap();
        assert_eq!(Local.timestamp_millis_opt(later_today).unwrap().hour(), 18);
    }

    #[test]
    fn weekly_monthly_once_interval() {
        let now = at(2026, 11, 6, 10, 0);
        assert_eq!(next_run(&json!({"kind":"weekly","day":"monday","time":"09:30"}), now).unwrap(), at(2026, 11, 9, 9, 30));
        assert_eq!(next_run(&json!({"kind":"monthly","day":1,"time":"09:00"}), now).unwrap(), at(2026, 12, 1, 9, 0));
        assert_eq!(next_run(&json!({"kind":"once","at": now - 1}), now), None);
        assert_eq!(next_run(&json!({"kind":"interval","minutes":30}), now).unwrap(), now + 30 * 60_000);
        assert!(validate(&json!({"kind":"interval","minutes":1})).is_err());
        assert_eq!(describe(&json!({"kind":"weekly","day":"monday","time":"09:30"})), "Every Monday at 09:30");
    }
}
