//! Calendars for Bots: list calendars, read, add, change and delete events, reply to invitations and
//! find free time, on connected Google and Outlook accounts (Settings › Connections). Inviting people,
//! changing or deleting an event people are invited to, deleting any event and replying to an
//! invitation show the user a card first. Events a Bot adds or changes without asking show as a
//! calendar card in the chat.

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Weekday};
use serde_json::{json, Value};

use crate::accounts::{api, choose, confirm, enc, fmt_ms, Account};
use crate::db::{Bot, Message};
use crate::state::Shared;
use crate::util::{new_id, now_ms, truncate_chars};

/// Google scopes for seeing the list of calendars and other people's free time (accounts connected
/// before these were added don't have them and show "Update permissions").
pub const LIST_SCOPE: &str = "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
pub const FREEBUSY_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.freebusy";
/// Google Calendar's event colors, by colorId 1-11.
const COLORS: [&str; 11] = ["lavender", "sage", "grape", "flamingo", "banana", "tangerine", "peacock", "graphite", "blueberry", "basil", "tomato"];
const MAX_GUESTS: usize = 100;
const DAY_MS: i64 = 24 * 3600_000;

/// Did this Google account allow the calendar list and free time? (Outlook always can.)
pub fn has_all_scopes(a: &Account) -> bool {
    a.provider != "google" || [LIST_SCOPE, FREEBUSY_SCOPE].iter().all(|s| a.scopes.split_whitespace().any(|x| x == *s))
}

pub fn tool_definitions() -> Vec<Value> {
    let account = json!({"type": "string", "description": "The account's email address (see list_accounts). Can be left out when you have only one account."});
    let calendar = json!({"type": "string", "description": "Which calendar: a name or id from calendar_list. Leave out for the main calendar."});
    let people = json!({"type": "array", "items": {"type": "string"}, "description": "Email addresses"});
    let event_fields = json!({
        "account": account, "calendar": calendar,
        "title": {"type": "string"},
        "start": {"type": "string", "description": "The user's local time like 2026-11-05T14:00 (a date like 2026-11-05 for all_day)"},
        "end": {"type": "string", "description": "Like start. For all_day events, the last day (a one-day event has start = end)"},
        "all_day": {"type": "boolean"},
        "time_zone": {"type": "string", "description": "Only when start and end are in another timezone: an IANA name like Asia/Tokyo"},
        "location": {"type": "string"},
        "description": {"type": "string"},
        "attendees": {"type": "array", "items": {"type": "string"}, "description": "Email addresses to invite. On calendar_update this replaces the whole guest list (use add_attendees / remove_attendees to change it)"},
        "optional_attendees": {"type": "array", "items": {"type": "string"}, "description": "Email addresses invited as optional"},
        "repeat": {"type": "array", "items": {"type": "string"}, "description": "Repeat rules (RFC 5545) like [\"RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10\"] or [\"RRULE:FREQ=MONTHLY;BYMONTHDAY=1;UNTIL=20270601\"]. [] stops repeating"},
        "reminders": {"type": "array", "items": {"type": "object", "properties": {"minutes": {"type": "integer"}, "method": {"type": "string", "enum": ["popup", "email"]}}, "required": ["minutes"]},
            "description": "Reminders before the start, like [{\"minutes\": 30}] (popup) or [{\"minutes\": 1440, \"method\": \"email\"}]. [] = no reminders. Leave out for the calendar's usual reminders. Outlook keeps only the first one"},
        "video_call": {"type": "boolean", "description": "Add a Google Meet link (Google) or a Teams link (Outlook work/school accounts)"},
        "color": {"type": "string", "description": "Google only: lavender, sage, grape, flamingo, banana, tangerine, peacock, graphite, blueberry, basil or tomato"},
        "show_as": {"type": "string", "enum": ["busy", "free"]},
        "visibility": {"type": "string", "enum": ["default", "private", "public"]},
        "notify": {"type": "string", "enum": ["all", "external", "none"], "description": "Who gets an email about the invitation or change: all guests (default), only guests outside the user's organization, or nobody. Google only; Outlook always emails guests"}
    });
    let mut create = event_fields.clone();
    create["attendees"]["description"] = json!("Email addresses to invite");
    let mut update = event_fields;
    update["id"] = json!({"type": "string", "description": "The event's id"});
    update["add_attendees"] = people.clone();
    update["remove_attendees"] = people.clone();
    update["repeat"]["description"] = json!("Change how the series repeats (give the id of the series, not of one date). [] stops repeating");
    vec![
        json!({"name": "calendar_list", "description": "List the user's calendars in this account (names, ids, which is the main one, which you can change).",
               "inputSchema": {"type": "object", "properties": {"account": account}}}),
        json!({"name": "calendar_events", "description": "List events between two times (default: the next 7 days), soonest first. Repeating events show each date. Times are the user's local time like 2026-11-05T09:00. calendar \"all\" lists every calendar the user shows.",
               "inputSchema": {"type": "object", "properties": {"account": account, "calendar": calendar, "from": {"type": "string"}, "to": {"type": "string"},
                   "query": {"type": "string", "description": "Only events matching these words (title, description, place, guests)"}, "max": {"type": "integer", "description": "1-100, default 25"}}}}),
        json!({"name": "calendar_get", "description": "All details of one event: description, guests and their replies, reminders, how it repeats, video call link and the link to open it.",
               "inputSchema": {"type": "object", "properties": {"account": account, "calendar": calendar, "id": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "calendar_free_time", "description": "Find free times when the user (and the people you list) are all available, between two times (default: the next 7 days), within working hours.",
               "inputSchema": {"type": "object", "properties": {"account": account,
                   "people": {"type": "array", "items": {"type": "string"}, "description": "Other people's email addresses (the user is always included). Google can see other people's free time when they share it; Outlook only in work or school accounts"},
                   "from": {"type": "string"}, "to": {"type": "string"},
                   "minutes": {"type": "integer", "description": "How long the free time must be, default 30"},
                   "day_start": {"type": "string", "description": "Earliest time each day, default 09:00"},
                   "day_end": {"type": "string", "description": "Latest time each day, default 18:00"},
                   "weekends": {"type": "boolean", "description": "Include Saturdays and Sundays (default false)"},
                   "max": {"type": "integer", "description": "How many free times to return, 1-20, default 8"}}}}),
        json!({"name": "calendar_create", "description": "Add an event. If you invite people they get an email, so the user sees the event on a card and sends it themselves; this waits for their answer. Otherwise it's added right away and shown in the chat.",
               "inputSchema": {"type": "object", "properties": create, "required": ["title", "start", "end"]}}),
        json!({"name": "calendar_update", "description": "Change an event (only the fields you give change; moving only the start keeps its length). If people are invited they're told, so the user sees the change on a card first. For a repeating event, an id from calendar_events changes that one date; calendar_get shows the series id.",
               "inputSchema": {"type": "object", "properties": update, "required": ["id"]}}),
        json!({"name": "calendar_delete", "description": "Delete an event (the user sees an Allow / Deny card first; guests are told it's cancelled). An id from calendar_events deletes that one date of a repeating event; the series id deletes them all.",
               "inputSchema": {"type": "object", "properties": {"account": account, "calendar": calendar, "id": {"type": "string"},
                   "notify": {"type": "string", "enum": ["all", "external", "none"], "description": "Google only: who is told (default all)"}}, "required": ["id"]}}),
        json!({"name": "calendar_respond", "description": "Reply to an invitation: yes, maybe or no, with an optional note to the organizer. The user sees the reply on a card first.",
               "inputSchema": {"type": "object", "properties": {"account": account, "calendar": calendar, "id": {"type": "string"},
                   "response": {"type": "string", "enum": ["yes", "maybe", "no"]}, "comment": {"type": "string"}}, "required": ["id", "response"]}}),
    ]
}

/// Tools that may wait for the user's answer on a card.
pub fn waits_for_user(name: &str) -> bool {
    matches!(name, "calendar_create" | "calendar_update" | "calendar_delete" | "calendar_respond")
}

pub fn is_tool(name: &str) -> bool {
    name.starts_with("calendar_")
}

// ------------------------------------------------------------------ the shared event model

/// One guest of an event.
#[derive(Clone, Debug, PartialEq)]
pub struct Guest {
    pub email: String,
    pub name: String,
    /// "yes" | "no" | "maybe" | "waiting"
    pub response: String,
    pub optional: bool,
    pub is_self: bool,
    pub organizer: bool,
}

/// An event as both providers return it.
#[derive(Clone, Debug, Default)]
pub struct Event {
    pub id: String,
    pub series_id: String,
    pub calendar: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    /// All-day events: first day and the day after the last.
    pub days: Option<(NaiveDate, NaiveDate)>,
    pub location: String,
    pub description: String,
    pub organizer: String,
    pub guests: Vec<Guest>,
    /// RFC 5545 lines (Google) or a description (Outlook).
    pub repeat: Vec<String>,
    pub link: String,
    pub video: String,
    pub my_response: String,
    pub cancelled: bool,
    pub color: String,
    pub show_as: String,
    pub visibility: String,
    pub reminders: Vec<(String, i64)>,
    pub default_reminders: bool,
}

/// One calendar.
pub struct Cal {
    pub id: String,
    pub name: String,
    pub main: bool,
    pub can_edit: bool,
    pub shown: bool,
    pub time_zone: String,
}

/// What a create/update call asks for.
#[derive(Default)]
pub struct Change {
    pub title: Option<String>,
    pub times: Option<Times>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// The whole guest list, when it changes: (email, optional).
    pub guests: Option<Vec<(String, bool)>>,
    pub repeat: Option<Vec<String>>,
    /// None = keep; Some([]) = none; Some(list) = these (method, minutes).
    pub reminders: Option<Vec<(String, i64)>>,
    pub video_call: bool,
    pub color: Option<String>,
    pub show_as: Option<String>,
    pub visibility: Option<String>,
    /// "all" | "externalOnly" | "none"
    pub notify: String,
    /// The timezone the times were given in (IANA name), if not the user's.
    pub time_zone: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Times {
    Timed(i64, i64),
    /// First day, day after the last.
    AllDay(NaiveDate, NaiveDate),
}

// ------------------------------------------------------------------ small helpers

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn opt(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(|x| x.trim().to_string())
}

fn emails(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect(),
        Some(Value::String(t)) => t.split([',', ';']).map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    }
}

/// "2026-11-05T14:00" in the user's timezone (or `tz`), or a full RFC 3339 time -> unix ms.
fn parse_time(t: &str, tz: Option<chrono_tz::Tz>) -> Option<i64> {
    let t = t.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(d.timestamp_millis());
    }
    let local = |n: NaiveDateTime| match tz {
        Some(z) => z.from_local_datetime(&n).earliest().map(|d| d.timestamp_millis()),
        None => crate::util::from_local(n),
    };
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(t, f) {
            return local(n);
        }
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d").ok().and_then(|d| local(d.and_hms_opt(0, 0, 0)?))
}

fn parse_date(t: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(t.trim().get(..10)?, "%Y-%m-%d").ok()
}

fn utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default()
}

fn local_date(ms: i64) -> NaiveDate {
    crate::util::to_local(ms).map(|d| d.date()).unwrap_or_default()
}

fn day_start_ms(d: NaiveDate) -> i64 {
    crate::util::from_local(d.and_hms_opt(0, 0, 0).unwrap_or_default()).unwrap_or(0)
}

fn range(args: &Value) -> Result<(i64, i64), String> {
    let from = match s(args, "from").as_str() {
        "" => now_ms(),
        t => parse_time(t, None).ok_or_else(|| format!("Couldn't read the time “{t}” (use 2026-11-05T09:00)"))?,
    };
    let to = match s(args, "to").as_str() {
        "" => from + 7 * DAY_MS,
        t => parse_time(t, None).ok_or_else(|| format!("Couldn't read the time “{t}” (use 2026-11-05T18:00)"))?,
    };
    if to <= from {
        return Err("“to” must be after “from”".into());
    }
    if to - from > 400 * DAY_MS {
        return Err("Look at most about a year at a time".into());
    }
    Ok((from, to))
}

/// "Thu Nov 5, 2:00 PM – 3:00 PM" style text for the Bot.
fn when_text(e: &Event) -> String {
    match e.days {
        Some((a, b)) => {
            let last = b.pred_opt().unwrap_or(b);
            if last <= a { format!("{a} (all day)") } else { format!("{a} – {last} (all day)") }
        }
        None => format!("{} – {}", fmt_ms(e.start), fmt_ms(e.end)),
    }
}

/// A short, readable version of a repeat rule, like "every week on Mon, Wed (10 times)".
pub fn repeat_text(lines: &[String]) -> String {
    let Some(rule) = lines.iter().find_map(|l| l.strip_prefix("RRULE:")) else {
        return lines.join("; ");
    };
    let mut parts = std::collections::HashMap::new();
    for kv in rule.split(';') {
        if let Some((k, v)) = kv.split_once('=') {
            parts.insert(k.to_ascii_uppercase(), v.to_string());
        }
    }
    let n: u32 = parts.get("INTERVAL").and_then(|x| x.parse().ok()).unwrap_or(1);
    let unit = match parts.get("FREQ").map(String::as_str) {
        Some("DAILY") => "day",
        Some("WEEKLY") => "week",
        Some("MONTHLY") => "month",
        Some("YEARLY") => "year",
        _ => return rule.to_string(),
    };
    let mut t = if n == 1 { format!("every {unit}") } else { format!("every {n} {unit}s") };
    if let Some(days) = parts.get("BYDAY") {
        let names: Vec<String> = days
            .split(',')
            .map(|d| {
                let code = &d[d.len().saturating_sub(2)..];
                let nth = &d[..d.len().saturating_sub(2)];
                let name = match code {
                    "MO" => "Mon", "TU" => "Tue", "WE" => "Wed", "TH" => "Thu", "FR" => "Fri", "SA" => "Sat", "SU" => "Sun", other => other,
                };
                match nth {
                    "" => name.to_string(),
                    "-1" => format!("last {name}"),
                    k => format!("{} {name}", ordinal(k.parse().unwrap_or(1))),
                }
            })
            .collect();
        t.push_str(&format!(" on {}", names.join(", ")));
    } else if let Some(d) = parts.get("BYMONTHDAY") {
        t.push_str(&format!(" on day {d}"));
    }
    if let Some(c) = parts.get("COUNT") {
        t.push_str(&format!(" ({c} times)"));
    } else if let Some(u) = parts.get("UNTIL") {
        let d = u.get(..8).and_then(|x| NaiveDate::parse_from_str(x, "%Y%m%d").ok()).map(|d| d.to_string()).unwrap_or_else(|| u.clone());
        t.push_str(&format!(" until {d}"));
    }
    t
}

fn ordinal(n: i64) -> &'static str {
    match n {
        1 => "1st",
        2 => "2nd",
        3 => "3rd",
        4 => "4th",
        5 => "5th",
        _ => "nth",
    }
}

/// Read create/update arguments.
fn change_in(args: &Value, creating: bool, existing: Option<&Event>) -> Result<Change, String> {
    let tz = match s(args, "time_zone").as_str() {
        "" => None,
        z => Some(z.parse::<chrono_tz::Tz>().map_err(|_| format!("“{z}” isn't a timezone name (use one like Asia/Tokyo)"))?),
    };
    let (start, end) = (s(args, "start"), s(args, "end"));
    let all_day = args.get("all_day").and_then(Value::as_bool);
    let times = if start.is_empty() && end.is_empty() {
        if creating {
            return Err("Give start and end".into());
        }
        None
    } else if all_day.unwrap_or(false) || (all_day.is_none() && start.len() == 10 && (end.is_empty() || end.len() == 10)) {
        let a = parse_date(&start).ok_or("Couldn't read the start date (use 2026-11-05)")?;
        let last = if end.is_empty() { a } else { parse_date(&end).ok_or("Couldn't read the end date (use 2026-11-06)")? };
        if last < a {
            return Err("The end must be on or after the start".into());
        }
        Some(Times::AllDay(a, last.succ_opt().unwrap_or(last)))
    } else {
        let a = if start.is_empty() { None } else { Some(parse_time(&start, tz).ok_or_else(|| format!("Couldn't read the start “{start}” (use 2026-11-05T14:00)"))?) };
        let b = if end.is_empty() { None } else { Some(parse_time(&end, tz).ok_or_else(|| format!("Couldn't read the end “{end}” (use 2026-11-05T15:00)"))?) };
        let (a, b) = match (a, b, existing) {
            (Some(a), Some(b), _) => (a, b),
            // Moving only the start keeps the length; changing only the end keeps the start.
            (Some(a), None, Some(e)) if e.days.is_none() => (a, a + (e.end - e.start)),
            (None, Some(b), Some(e)) if e.days.is_none() => (e.start, b),
            _ => return Err("Give both start and end".into()),
        };
        if b <= a {
            return Err("The end must be after the start".into());
        }
        Some(Times::Timed(a, b))
    };
    // Guests: a whole new list, or additions/removals to the current one.
    let mut guests: Option<Vec<(String, bool)>> = None;
    if args.get("attendees").is_some() || args.get("optional_attendees").is_some() {
        let mut g: Vec<(String, bool)> = emails(args, "attendees").into_iter().map(|e| (e, false)).collect();
        if args.get("attendees").is_none() {
            if let Some(e) = existing {
                g = e.guests.iter().filter(|x| !x.is_self && !x.organizer).map(|x| (x.email.to_lowercase(), x.optional)).collect();
            }
        }
        for e in emails(args, "optional_attendees") {
            g.retain(|x| x.0 != e);
            g.push((e, true));
        }
        guests = Some(g);
    }
    let (add, remove) = (emails(args, "add_attendees"), emails(args, "remove_attendees"));
    if !add.is_empty() || !remove.is_empty() {
        let mut g = guests.take().unwrap_or_else(|| {
            existing.map(|e| e.guests.iter().filter(|x| !x.is_self && !x.organizer).map(|x| (x.email.to_lowercase(), x.optional)).collect()).unwrap_or_default()
        });
        for e in add {
            if !g.iter().any(|x| x.0 == e) {
                g.push((e, false));
            }
        }
        g.retain(|x| !remove.contains(&x.0));
        guests = Some(g);
    }
    if let Some(g) = &guests {
        if let Some(bad) = g.iter().find(|x| !x.0.contains('@') || x.0.contains(char::is_whitespace)) {
            return Err(format!("“{}” isn't an email address", bad.0));
        }
        if g.len() > MAX_GUESTS {
            return Err(format!("At most {MAX_GUESTS} guests"));
        }
    }
    let repeat = match args.get("repeat") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(vec![t.trim().to_string()]),
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()),
        _ => return Err("repeat must be a list of rules like RRULE:FREQ=WEEKLY;BYDAY=MO".into()),
    };
    if let Some(r) = &repeat {
        if let Some(bad) = r.iter().find(|l| !["RRULE:", "EXRULE:", "RDATE", "EXDATE"].iter().any(|p| l.to_ascii_uppercase().starts_with(p))) {
            return Err(format!("“{bad}” isn't a repeat rule. Use one like RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10"));
        }
    }
    let reminders = match args.get("reminders") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => {
            let mut out = vec![];
            for r in a {
                let (method, minutes) = match r {
                    Value::Number(n) => ("popup".to_string(), n.as_i64().unwrap_or(-1)),
                    Value::Object(_) => (r.get("method").and_then(Value::as_str).unwrap_or("popup").to_string(), r.get("minutes").and_then(Value::as_i64).unwrap_or(-1)),
                    _ => ("popup".into(), -1),
                };
                if !(0..=40320).contains(&minutes) {
                    return Err("Reminder minutes must be between 0 and 40320 (4 weeks)".into());
                }
                if method != "popup" && method != "email" {
                    return Err("A reminder's method is popup or email".into());
                }
                out.push((method, minutes));
            }
            if out.len() > 5 {
                return Err("At most 5 reminders".into());
            }
            Some(out)
        }
        _ => return Err("reminders must be a list like [{\"minutes\": 30}]".into()),
    };
    let color = match s(args, "color").to_lowercase().as_str() {
        "" => None,
        c => match COLORS.iter().position(|x| *x == c) {
            Some(i) => Some((i + 1).to_string()),
            None if c.parse::<usize>().map(|n| (1..=11).contains(&n)).unwrap_or(false) => Some(c.to_string()),
            None => return Err(format!("“{c}” isn't a calendar color. Use one of: {}", COLORS.join(", "))),
        },
    };
    let notify = match s(args, "notify").as_str() {
        "" | "all" => "all",
        "external" => "externalOnly",
        "none" => "none",
        o => return Err(format!("notify is all, external or none, not “{o}”")),
    };
    let title = opt(args, "title");
    if creating && title.as_deref().unwrap_or("").is_empty() {
        return Err("Give the event a title".into());
    }
    let pick = |k: &str, allowed: &[&str]| -> Result<Option<String>, String> {
        match s(args, k).as_str() {
            "" => Ok(None),
            v if allowed.contains(&v) => Ok(Some(v.to_string())),
            v => Err(format!("{k} is {}, not “{v}”", allowed.join(" or "))),
        }
    };
    Ok(Change {
        title,
        times,
        location: opt(args, "location"),
        description: opt(args, "description"),
        guests,
        repeat,
        reminders,
        video_call: args.get("video_call").and_then(Value::as_bool).unwrap_or(false),
        color,
        show_as: pick("show_as", &["busy", "free"])?,
        visibility: pick("visibility", &["default", "private", "public"])?,
        notify: notify.into(),
        time_zone: tz.map(|z| z.name().to_string()),
    })
}

// ------------------------------------------------------------------ the tools

pub async fn tool(st: &Shared, bot: &Bot, name: &str, args: &Value) -> Result<String, String> {
    let cfg = st.config.read().await.clone();
    let acct = choose(&cfg, bot, args)?;
    let google = acct.provider == "google";
    let want = s(args, "calendar");
    match name {
        "calendar_list" => {
            let cals = calendars(st, &acct).await?;
            Ok(cals
                .iter()
                .map(|c| {
                    format!(
                        "- {} · id: {}{}{}{}",
                        c.name,
                        c.id,
                        if c.main { " · main calendar" } else { "" },
                        if c.can_edit { "" } else { " · read only" },
                        if c.time_zone.is_empty() { String::new() } else { format!(" · {}", c.time_zone) }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "calendar_events" => {
            let (from, to) = range(args)?;
            let max = args.get("max").and_then(Value::as_i64).unwrap_or(25).clamp(1, 100) as usize;
            let q = s(args, "query");
            let targets: Vec<(String, String)> = if want.eq_ignore_ascii_case("all") {
                calendars(st, &acct).await?.into_iter().filter(|c| c.shown || c.main).take(15).map(|c| (c.id, c.name)).collect()
            } else {
                let (id, name) = resolve(st, &acct, &want).await?;
                vec![(id, name)]
            };
            let mut events = vec![];
            for (id, cname) in &targets {
                let mut list = if google { gcal::list(st, &acct, id, from, to, &q, max).await? } else { mscal::list(st, &acct, id, from, to, &q, max).await? };
                if targets.len() > 1 {
                    for e in list.iter_mut() {
                        e.calendar = cname.clone();
                    }
                }
                events.extend(list);
            }
            events.sort_by_key(|e| e.start);
            events.truncate(max);
            if events.is_empty() {
                return Ok(format!("No events between {} and {}.", fmt_ms(from), fmt_ms(to)));
            }
            Ok(format!(
                "{} event(s) between {} and {} (times in {}):\n{}",
                events.len(),
                fmt_ms(from),
                fmt_ms(to),
                crate::util::timezone_label(),
                events.iter().map(|e| line(e, &acct)).collect::<Vec<_>>().join("\n")
            ))
        }
        "calendar_get" => {
            let (cal, _) = resolve(st, &acct, &want).await?;
            let e = get(st, &acct, &cal, &need_id(args)?).await?;
            Ok(details(&e, &acct))
        }
        "calendar_free_time" => free_time(st, &acct, args).await,
        "calendar_create" | "calendar_update" => write(st, bot, &acct, &want, name == "calendar_create", args).await,
        "calendar_delete" => {
            let (cal, cal_name) = resolve(st, &acct, &want).await?;
            let id = need_id(args)?;
            let e = get(st, &acct, &cal, &id).await?;
            let notify = match s(args, "notify").as_str() {
                "external" => "externalOnly",
                "none" => "none",
                _ => "all",
            };
            let mut card = card_json(&e, &acct, &cal_name, "delete");
            card["kind"] = json!("event");
            let others: Vec<&Guest> = e.guests.iter().filter(|g| !g.is_self && !g.organizer).collect();
            let summary = if others.is_empty() {
                format!("Delete “{}” from {}'s calendar", e.title, acct.email)
            } else {
                format!("Delete “{}” from {}'s calendar ({} will be told it's cancelled)", e.title, acct.email, others.iter().map(|g| g.email.as_str()).collect::<Vec<_>>().join(", "))
            };
            if !confirm(st, bot, name, &summary, card.clone(), "").await? {
                return Ok("The user said no, so the event was not deleted.".into());
            }
            if google { gcal::delete(st, &acct, &cal, &id, notify).await? } else { mscal::delete(st, &acct, &id).await? }
            Ok(format!("Deleted “{}”.", e.title))
        }
        "calendar_respond" => {
            let (cal, cal_name) = resolve(st, &acct, &want).await?;
            let id = need_id(args)?;
            let resp = s(args, "response");
            if !["yes", "maybe", "no"].contains(&resp.as_str()) {
                return Err("response is yes, maybe or no".into());
            }
            let comment = s(args, "comment");
            let e = get(st, &acct, &cal, &id).await?;
            if !e.guests.iter().any(|g| g.is_self && !g.organizer) {
                return Err("The user isn't a guest of this event (they organize it, or nobody was invited), so there's nothing to reply to.".into());
            }
            let word = match resp.as_str() { "yes" => "Yes", "maybe" => "Maybe", _ => "No" };
            let mut card = card_json(&e, &acct, &cal_name, "respond");
            card["kind"] = json!("event");
            card["response"] = json!(resp);
            card["comment"] = json!(comment);
            let summary = format!("Reply {word} to “{}”{}", e.title, if e.organizer.is_empty() { String::new() } else { format!(" from {}", e.organizer) });
            if !confirm(st, bot, name, &summary, card.clone(), "").await? {
                return Ok("The user said no, so no reply was sent.".into());
            }
            if google { gcal::respond(st, &acct, &cal, &e, &resp, &comment).await? } else { mscal::respond(st, &acct, &id, &resp, &comment).await? }
            Ok(format!("Replied {word} to “{}”.", e.title))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

fn need_id(args: &Value) -> Result<String, String> {
    match s(args, "id") {
        id if id.is_empty() => Err("Give the event's id (from calendar_events)".into()),
        id => Ok(id),
    }
}

async fn calendars(st: &Shared, acct: &Account) -> Result<Vec<Cal>, String> {
    if acct.provider == "google" {
        if !acct.scopes.split_whitespace().any(|x| x == LIST_SCOPE) {
            return Err(format!(
                "{} hasn't allowed openbot to see its list of calendars yet. Only the main calendar works until the user clicks Update permissions for it in Settings › Connections.",
                acct.email
            ));
        }
        gcal::calendars(st, acct).await
    } else {
        mscal::calendars(st, acct).await
    }
}

/// A calendar name or id -> (id, name). "" = the main calendar.
async fn resolve(st: &Shared, acct: &Account, want: &str) -> Result<(String, String), String> {
    let w = want.trim();
    if w.is_empty() || w.eq_ignore_ascii_case("primary") || w.eq_ignore_ascii_case("main") {
        return Ok((String::new(), String::new()));
    }
    // A Google calendar id is an email-like address; trust it without looking it up.
    if acct.provider == "google" && w.contains('@') && !acct.scopes.split_whitespace().any(|x| x == LIST_SCOPE) {
        return Ok((w.to_string(), w.to_string()));
    }
    let cals = calendars(st, acct).await?;
    if let Some(c) = cals.iter().find(|c| c.id == w).or_else(|| cals.iter().find(|c| c.name.eq_ignore_ascii_case(w))) {
        return Ok((if c.main { String::new() } else { c.id.clone() }, c.name.clone()));
    }
    let close: Vec<&Cal> = cals.iter().filter(|c| c.name.to_lowercase().contains(&w.to_lowercase())).collect();
    if close.len() == 1 {
        let c = close[0];
        return Ok((if c.main { String::new() } else { c.id.clone() }, c.name.clone()));
    }
    Err(format!("There's no calendar called “{w}”. The calendars are: {}", cals.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")))
}

async fn get(st: &Shared, acct: &Account, cal: &str, id: &str) -> Result<Event, String> {
    if acct.provider == "google" { gcal::get(st, acct, cal, id).await } else { mscal::get(st, acct, id).await }
}

/// One line per event in a list.
fn line(e: &Event, acct: &Account) -> String {
    let others: Vec<&str> = e.guests.iter().filter(|g| !g.is_self).map(|g| g.email.as_str()).collect();
    let mut t = format!("- id: {} · {} · {}", e.id, when_text(e), if e.title.is_empty() { "(No title)" } else { &e.title });
    if !e.calendar.is_empty() {
        t.push_str(&format!(" · calendar: {}", e.calendar));
    }
    if !e.location.is_empty() {
        t.push_str(&format!(" · at {}", truncate_chars(&e.location, 80)));
    }
    if !others.is_empty() {
        let shown: Vec<&str> = others.iter().take(6).copied().collect();
        t.push_str(&format!(" · with {}{}", shown.join(", "), if others.len() > 6 { format!(" and {} more", others.len() - 6) } else { String::new() }));
    }
    if !e.my_response.is_empty() && e.my_response != "organizer" && !e.organizer.eq_ignore_ascii_case(&acct.email) {
        t.push_str(&format!(" · your reply: {}", e.my_response));
    }
    if !e.series_id.is_empty() {
        t.push_str(" · repeats");
    }
    if !e.video.is_empty() {
        t.push_str(&format!(" · video call: {}", e.video));
    }
    if e.cancelled {
        t.push_str(" · cancelled");
    }
    t
}

/// Everything about one event, for calendar_get.
fn details(e: &Event, acct: &Account) -> String {
    let mut t = vec![format!("{} (id {})", if e.title.is_empty() { "(No title)" } else { &e.title }, e.id), format!("When: {} (times in {})", when_text(e), crate::util::timezone_label())];
    if !e.series_id.is_empty() && e.series_id != e.id {
        t.push(format!("Part of a repeating series (series id {})", e.series_id));
    }
    if !e.repeat.is_empty() {
        t.push(format!("Repeats: {} ({})", repeat_text(&e.repeat), e.repeat.join(" ")));
    }
    if !e.location.is_empty() {
        t.push(format!("Where: {}", e.location));
    }
    if !e.organizer.is_empty() {
        t.push(format!("Organizer: {}", e.organizer));
    }
    if !e.guests.is_empty() {
        t.push(format!(
            "Guests: {}",
            e.guests
                .iter()
                .map(|g| format!("{}{} ({}{})", if g.name.is_empty() { &g.email } else { &g.name }, if g.is_self { " [the user]" } else { "" }, g.response, if g.optional { ", optional" } else { "" }))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !e.my_response.is_empty() && !e.organizer.eq_ignore_ascii_case(&acct.email) {
        t.push(format!("The user's reply: {}", e.my_response));
    }
    if !e.video.is_empty() {
        t.push(format!("Video call: {}", e.video));
    }
    if e.default_reminders {
        t.push("Reminders: the calendar's usual ones".into());
    } else if !e.reminders.is_empty() {
        t.push(format!("Reminders: {}", e.reminders.iter().map(|(m, n)| format!("{n} min before ({m})")).collect::<Vec<_>>().join(", ")));
    }
    if !e.show_as.is_empty() {
        t.push(format!("Shows as: {}", e.show_as));
    }
    if !e.visibility.is_empty() && e.visibility != "default" {
        t.push(format!("Visibility: {}", e.visibility));
    }
    if !e.color.is_empty() {
        t.push(format!("Color: {}", e.color));
    }
    if e.cancelled {
        t.push("Status: cancelled".into());
    }
    if !e.link.is_empty() {
        t.push(format!("Open it: {}", e.link));
    }
    if !e.description.is_empty() {
        t.push(format!("Description:\n{}", truncate_chars(&crate::accounts::html_to_text(&e.description), 4000)));
    }
    t.join("\n")
}

/// The event as the chat's calendar card shows it.
fn card_json(e: &Event, acct: &Account, cal_name: &str, action: &str) -> Value {
    json!({
        "action": action,
        "account": acct.email,
        "provider": acct.provider,
        "calendar": cal_name,
        "id": e.id,
        "title": e.title,
        "start": e.start,
        "end": e.end,
        "all_day": e.days.is_some(),
        "first_day": e.days.map(|d| d.0.to_string()),
        "last_day": e.days.map(|d| d.1.pred_opt().unwrap_or(d.1).to_string()),
        "when": when_text(e),
        "location": e.location,
        "organizer": e.organizer,
        "guests": e.guests.iter().map(|g| json!({"email": g.email, "name": g.name, "response": g.response, "optional": g.optional, "self": g.is_self, "organizer": g.organizer})).collect::<Vec<_>>(),
        "repeat": if e.repeat.is_empty() { String::new() } else { repeat_text(&e.repeat) },
        "video": e.video,
        "link": e.link,
        "description": truncate_chars(&crate::accounts::html_to_text(&e.description), 1500),
    })
}

/// The user's other events on the same day (for the card's day view). Errors are ignored.
async fn day_events(st: &Shared, acct: &Account, cal: &str, e: &Event) -> Vec<Value> {
    let day = local_date(e.start);
    let (from, to) = (day_start_ms(day), day_start_ms(day.succ_opt().unwrap_or(day)));
    let list = if acct.provider == "google" { gcal::list(st, acct, cal, from, to, "", 30).await } else { mscal::list(st, acct, cal, from, to, "", 30).await };
    list.unwrap_or_default()
        .into_iter()
        // Leave out the event itself (a repeating event comes back as its dated copies).
        .filter(|x| x.id != e.id && (e.id.is_empty() || x.series_id != e.id) && !(x.start == e.start && x.title == e.title) && !x.cancelled)
        .map(|x| json!({"title": x.title, "start": x.start, "end": x.end, "all_day": x.days.is_some()}))
        .collect()
}

fn post_card(st: &Shared, bot: &Bot, text: &str, card: Value) {
    let m = Message { id: new_id(), bot_id: bot.id.clone(), role: "calendar".into(), text: text.into(), html: String::new(), status: "done".into(), meta: card, created_at: now_ms() };
    let _ = st.db.upsert_message(&m);
    st.emit_message(true, &m);
}

async fn write(st: &Shared, bot: &Bot, acct: &Account, want: &str, creating: bool, args: &Value) -> Result<String, String> {
    let google = acct.provider == "google";
    let (cal, cal_name) = resolve(st, acct, want).await?;
    let id = if creating { String::new() } else { need_id(args)? };
    let existing = if creating { None } else { Some(get(st, acct, &cal, &id).await?) };
    let ch = change_in(args, creating, existing.as_ref())?;
    if !google && ch.color.is_some() {
        return Err("Outlook events don't have colors here. Leave out color.".into());
    }
    // What the event will look like, for the card and for deciding who gets told.
    let mut preview = existing.clone().unwrap_or_default();
    if let Some(t) = &ch.title {
        preview.title = t.clone();
    }
    match ch.times {
        Some(Times::Timed(a, b)) => {
            preview.start = a;
            preview.end = b;
            preview.days = None;
        }
        Some(Times::AllDay(a, b)) => {
            preview.start = day_start_ms(a);
            preview.end = day_start_ms(b);
            preview.days = Some((a, b));
        }
        None => {}
    }
    if let Some(l) = &ch.location {
        preview.location = l.clone();
    }
    if let Some(d) = &ch.description {
        preview.description = d.clone();
    }
    if let Some(r) = &ch.repeat {
        preview.repeat = r.clone();
    }
    if let Some(g) = &ch.guests {
        let old = existing.as_ref().map(|e| e.guests.clone()).unwrap_or_default();
        let mut list: Vec<Guest> = old.iter().filter(|x| x.is_self || x.organizer).cloned().collect();
        for (email, optional) in g {
            let prev = old.iter().find(|x| x.email.eq_ignore_ascii_case(email));
            list.push(Guest {
                email: email.clone(),
                name: prev.map(|p| p.name.clone()).unwrap_or_default(),
                response: prev.map(|p| p.response.clone()).unwrap_or_else(|| "waiting".into()),
                optional: *optional,
                is_self: false,
                organizer: false,
            });
        }
        preview.guests = list;
    }
    if creating && preview.organizer.is_empty() {
        preview.organizer = acct.email.clone();
    }
    // People who get an email: the guests, unless notify is "none" (Outlook always emails them).
    let people: Vec<String> = preview.guests.iter().filter(|g| !g.is_self && !g.organizer).map(|g| g.email.clone()).collect();
    let removed: Vec<String> = existing
        .as_ref()
        .map(|e| e.guests.iter().filter(|g| !g.is_self && !g.organizer && !people.iter().any(|p| p.eq_ignore_ascii_case(&g.email))).map(|g| g.email.clone()).collect())
        .unwrap_or_default();
    // Ask first when guests are added or taken off (they see the event even without an email), or
    // when guests get an email about a change.
    let old_people: Vec<String> = existing.as_ref().map(|e| e.guests.iter().filter(|g| !g.is_self && !g.organizer).map(|g| g.email.to_lowercase()).collect()).unwrap_or_default();
    let guests_changed = ch.guests.is_some() && (people.iter().any(|p| !old_people.contains(&p.to_lowercase())) || !removed.is_empty());
    let told = guests_changed || (!people.is_empty() && (ch.notify != "none" || !google));
    let action = if creating { "create" } else { "update" };
    let mut card = card_json(&preview, acct, &cal_name, action);
    if told {
        card["kind"] = json!("event");
        card["day_events"] = json!(day_events(st, acct, &cal, &preview).await);
        card["removed"] = json!(removed);
        let tool = if creating { "calendar_create" } else { "calendar_update" };
        let summary = if creating {
            format!("Add “{}” to {}'s calendar and invite {}", preview.title, acct.email, people.join(", "))
        } else {
            let mut who = people.clone();
            who.extend(removed.iter().cloned());
            format!("Change “{}” in {}'s calendar ({} will be told)", preview.title, acct.email, who.join(", "))
        };
        if !confirm(st, bot, tool, &summary, card.clone(), "").await? {
            return Ok("The user said no, so the calendar was not changed.".into());
        }
    }
    let saved = if google { gcal::write(st, acct, &cal, existing.as_ref(), &ch).await? } else { mscal::write(st, acct, &cal, existing.as_ref(), &ch).await? };
    let when = when_text(&saved);
    let mut note = String::new();
    if !google && ch.notify == "none" && !people.is_empty() {
        note.push_str(" Outlook emails guests anyway (it can't skip that).");
    }
    if ch.video_call && saved.video.is_empty() {
        note.push_str(" No video call link was added (the account may not allow it).");
    }
    if !google && ch.reminders.as_ref().map(|r| r.len() > 1).unwrap_or(false) {
        note.push_str(" Outlook keeps one reminder, so only the first was set.");
    }
    if !told {
        let mut c = card_json(&saved, acct, &cal_name, action);
        c["day_events"] = json!(day_events(st, acct, &cal, &saved).await);
        post_card(st, bot, &format!("{} “{}”", if creating { "Added" } else { "Updated" }, saved.title), c);
    }
    Ok(if creating {
        format!("Added “{}” · {when} (id {}).{note}{}", saved.title, saved.id, if saved.video.is_empty() { String::new() } else { format!(" Video call: {}", saved.video) })
    } else {
        format!("Updated “{}” · {when}.{note}", saved.title)
    })
}

// ------------------------------------------------------------------ free time

async fn free_time(st: &Shared, acct: &Account, args: &Value) -> Result<String, String> {
    let (from, to) = range(args)?;
    let minutes = args.get("minutes").and_then(Value::as_i64).unwrap_or(30).clamp(5, 24 * 60);
    let max = args.get("max").and_then(Value::as_i64).unwrap_or(8).clamp(1, 20) as usize;
    let hm = |k: &str, d: &str| -> Result<chrono::NaiveTime, String> {
        let v = match s(args, k) { x if x.is_empty() => d.to_string(), x => x };
        chrono::NaiveTime::parse_from_str(&v, "%H:%M").map_err(|_| format!("{k} must look like 09:00"))
    };
    let (day_start, day_end) = (hm("day_start", "09:00")?, hm("day_end", "18:00")?);
    if day_end <= day_start {
        return Err("day_end must be after day_start".into());
    }
    let weekends = args.get("weekends").and_then(Value::as_bool).unwrap_or(false);
    let mut who = vec![acct.email.to_lowercase()];
    for p in emails(args, "people") {
        if !who.contains(&p) {
            who.push(p);
        }
    }
    if who.len() > 20 {
        return Err("At most 20 people at once".into());
    }
    let busy = if acct.provider == "google" { gcal::busy(st, acct, &who, from, to).await? } else { mscal::busy(st, acct, &who, from, to).await? };
    let mut blocked: Vec<(i64, i64)> = vec![];
    let mut unknown = vec![];
    for (email, b) in &busy {
        match b {
            Ok(list) => blocked.extend(list.iter().copied()),
            Err(e) => unknown.push(format!("{email} ({e})")),
        }
    }
    let slots = free_slots(&blocked, from, to, day_start, day_end, weekends, minutes, max);
    let mut out = if slots.is_empty() {
        format!(
            "No free time of {minutes} minutes or more between {} and {}, {}–{}{}.",
            fmt_ms(from),
            fmt_ms(to),
            day_start.format("%H:%M"),
            day_end.format("%H:%M"),
            if weekends { "" } else { " on weekdays (pass weekends: true to include Saturdays and Sundays)" }
        )
    } else {
        format!(
            "Free times of at least {minutes} minutes for {} (times in {}):\n{}",
            who.join(", "),
            crate::util::timezone_label(),
            slots.iter().map(|(a, b)| format!("- {} – {}", fmt_ms(*a), crate::util::to_local(*b).map(|d| d.format("%H:%M").to_string()).unwrap_or_default())).collect::<Vec<_>>().join("\n")
        )
    };
    if !unknown.is_empty() {
        out.push_str(&format!("\nCouldn't see the calendars of: {}. Those people may be busy at these times.", unknown.join(", ")));
    }
    Ok(out)
}

/// Gaps of at least `minutes` inside working hours on each day, avoiding the busy blocks.
#[allow(clippy::too_many_arguments)]
fn free_slots(busy: &[(i64, i64)], from: i64, to: i64, day_start: chrono::NaiveTime, day_end: chrono::NaiveTime, weekends: bool, minutes: i64, max: usize) -> Vec<(i64, i64)> {
    let mut busy: Vec<(i64, i64)> = busy.iter().copied().filter(|(a, b)| b > a).collect();
    busy.sort();
    let need = minutes * 60_000;
    let mut out = vec![];
    let mut day = local_date(from);
    let last = local_date(to);
    while day <= last && out.len() < max {
        if weekends || !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            let a = crate::util::from_local(day.and_time(day_start)).unwrap_or(0).max(from);
            let b = crate::util::from_local(day.and_time(day_end)).unwrap_or(0).min(to);
            let mut cur = a;
            for (s1, e1) in &busy {
                if *e1 <= cur || *s1 >= b {
                    continue;
                }
                if *s1 - cur >= need {
                    out.push((cur, *s1));
                    if out.len() >= max {
                        return out;
                    }
                }
                cur = cur.max(*e1);
            }
            if b - cur >= need {
                out.push((cur, b));
            }
        }
        day = match day.succ_opt() {
            Some(d) => d,
            None => break,
        };
    }
    out.truncate(max);
    out
}

// ------------------------------------------------------------------ Google Calendar

mod gcal {
    use super::*;

    const BASE: &str = "https://www.googleapis.com/calendar/v3";

    fn cal_id(cal: &str) -> String {
        if cal.is_empty() { "primary".into() } else { enc(cal) }
    }

    fn when(v: &Value) -> (i64, Option<NaiveDate>) {
        if let Some(d) = v.get("dateTime").and_then(Value::as_str) {
            (chrono::DateTime::parse_from_rfc3339(d).map(|x| x.timestamp_millis()).unwrap_or(0), None)
        } else {
            let d = v.get("date").and_then(Value::as_str).and_then(parse_date);
            (d.map(day_start_ms).unwrap_or(0), d)
        }
    }

    pub fn event(e: &Value, me: &str) -> Event {
        let (start, sd) = when(e.get("start").unwrap_or(&Value::Null));
        let (end, ed) = when(e.get("end").unwrap_or(&Value::Null));
        let guests: Vec<Guest> = e
            .get("attendees")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter(|x| x.get("resource") != Some(&json!(true)))
                    .map(|x| Guest {
                        email: s(x, "email"),
                        name: s(x, "displayName"),
                        response: match s(x, "responseStatus").as_str() {
                            "accepted" => "yes",
                            "declined" => "no",
                            "tentative" => "maybe",
                            _ => "waiting",
                        }
                        .into(),
                        optional: x.get("optional") == Some(&json!(true)),
                        is_self: x.get("self") == Some(&json!(true)) || s(x, "email").eq_ignore_ascii_case(me),
                        organizer: x.get("organizer") == Some(&json!(true)),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let my_response = guests.iter().find(|g| g.is_self).map(|g| g.response.clone()).unwrap_or_default();
        let video = e
            .pointer("/conferenceData/entryPoints")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|p| s(p, "entryPointType") == "video"))
            .map(|p| s(p, "uri"))
            .or_else(|| e.get("hangoutLink").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let organizer = e.pointer("/organizer/email").and_then(Value::as_str).unwrap_or("").to_string();
        let color = e.get("colorId").and_then(Value::as_str).and_then(|c| c.parse::<usize>().ok()).and_then(|i| COLORS.get(i.wrapping_sub(1))).map(|c| c.to_string()).unwrap_or_default();
        Event {
            id: s(e, "id"),
            series_id: s(e, "recurringEventId"),
            calendar: String::new(),
            title: s(e, "summary"),
            start,
            end,
            days: match (sd, ed) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => None,
            },
            location: s(e, "location"),
            description: s(e, "description"),
            organizer: if organizer.eq_ignore_ascii_case(me) { me.to_string() } else { e.pointer("/organizer/displayName").and_then(Value::as_str).map(|n| format!("{n} <{organizer}>")).unwrap_or(organizer) },
            guests,
            repeat: e.get("recurrence").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
            link: s(e, "htmlLink"),
            video,
            my_response,
            cancelled: s(e, "status") == "cancelled",
            color,
            show_as: if s(e, "transparency") == "transparent" { "free".into() } else { "busy".into() },
            visibility: s(e, "visibility"),
            reminders: e
                .pointer("/reminders/overrides")
                .and_then(Value::as_array)
                .map(|l| l.iter().map(|r| (s(r, "method"), r.get("minutes").and_then(Value::as_i64).unwrap_or(0))).collect())
                .unwrap_or_default(),
            default_reminders: e.pointer("/reminders/useDefault") == Some(&json!(true)),
        }
    }

    pub async fn calendars(st: &Shared, a: &Account) -> Result<Vec<Cal>, String> {
        let v = api(st, a, "GET", &format!("{BASE}/users/me/calendarList?maxResults=250"), &[], None).await?;
        Ok(v.get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|c| Cal {
                id: s(c, "id"),
                name: match s(c, "summaryOverride") { n if n.is_empty() => s(c, "summary"), n => n },
                main: c.get("primary") == Some(&json!(true)),
                can_edit: matches!(s(c, "accessRole").as_str(), "owner" | "writer"),
                shown: c.get("selected") == Some(&json!(true)),
                time_zone: s(c, "timeZone"),
            })
            .collect())
    }

    pub async fn list(st: &Shared, a: &Account, cal: &str, from: i64, to: i64, q: &str, max: usize) -> Result<Vec<Event>, String> {
        let mut url = format!(
            "{BASE}/calendars/{}/events?singleEvents=true&orderBy=startTime&maxResults={}&timeMin={}&timeMax={}",
            cal_id(cal),
            max.min(250),
            enc(&utc(from)),
            enc(&utc(to))
        );
        if !q.is_empty() {
            url.push_str(&format!("&q={}", enc(q)));
        }
        let v = api(st, a, "GET", &url, &[], None).await?;
        Ok(v.get("items").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|e| event(e, &a.email)).collect())
    }

    pub async fn get(st: &Shared, a: &Account, cal: &str, id: &str) -> Result<Event, String> {
        let v = api(st, a, "GET", &format!("{BASE}/calendars/{}/events/{}", cal_id(cal), enc(id)), &[], None).await?;
        Ok(event(&v, &a.email))
    }

    pub async fn write(st: &Shared, a: &Account, cal: &str, existing: Option<&Event>, ch: &Change) -> Result<Event, String> {
        let mut body = json!({});
        if let Some(t) = &ch.title {
            body["summary"] = json!(t);
        }
        if let Some(l) = &ch.location {
            body["location"] = json!(l);
        }
        if let Some(d) = &ch.description {
            body["description"] = json!(d);
        }
        match ch.times {
            Some(Times::Timed(s1, e1)) => {
                // With a timezone name, so repeating events keep their clock time across daylight saving.
                let zone = ch.time_zone.clone().or_else(crate::util::timezone_name);
                let at = |ms: i64| match zone.as_deref().and_then(|z| z.parse::<chrono_tz::Tz>().ok()) {
                    Some(z) => json!({"dateTime": z.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default(), "timeZone": z.name(), "date": Value::Null}),
                    None => json!({"dateTime": utc(ms), "timeZone": "UTC", "date": Value::Null}),
                };
                body["start"] = at(s1);
                body["end"] = at(e1);
            }
            Some(Times::AllDay(d1, d2)) => {
                body["start"] = json!({"date": d1.to_string(), "dateTime": Value::Null});
                body["end"] = json!({"date": d2.to_string(), "dateTime": Value::Null});
            }
            None => {}
        }
        if let Some(g) = &ch.guests {
            // Keep the organizer's and the user's own entries (with their replies) and set the rest.
            let mut list: Vec<Value> = existing
                .map(|e| e.guests.iter().filter(|x| x.is_self || x.organizer).map(|x| json!({"email": x.email})).collect())
                .unwrap_or_default();
            for (email, optional) in g {
                let mut v = json!({"email": email});
                if *optional {
                    v["optional"] = json!(true);
                }
                if let Some(old) = existing.and_then(|e| e.guests.iter().find(|x| x.email.eq_ignore_ascii_case(email))) {
                    v["responseStatus"] = json!(match old.response.as_str() { "yes" => "accepted", "no" => "declined", "maybe" => "tentative", _ => "needsAction" });
                }
                list.push(v);
            }
            if g.is_empty() {
                list.clear();
            }
            body["attendees"] = json!(list);
        }
        if let Some(r) = &ch.repeat {
            body["recurrence"] = json!(r);
        }
        if let Some(r) = &ch.reminders {
            body["reminders"] = json!({"useDefault": false, "overrides": r.iter().map(|(m, n)| json!({"method": m, "minutes": n})).collect::<Vec<_>>()});
        }
        if ch.video_call && existing.map(|e| e.video.is_empty()).unwrap_or(true) {
            body["conferenceData"] = json!({"createRequest": {"requestId": new_id(), "conferenceSolutionKey": {"type": "hangoutsMeet"}}});
        }
        if let Some(c) = &ch.color {
            body["colorId"] = json!(c);
        }
        if let Some(sh) = &ch.show_as {
            body["transparency"] = json!(if sh == "free" { "transparent" } else { "opaque" });
        }
        if let Some(vis) = &ch.visibility {
            body["visibility"] = json!(vis);
        }
        let q = format!("sendUpdates={}&conferenceDataVersion=1", ch.notify);
        let v = match existing {
            None => api(st, a, "POST", &format!("{BASE}/calendars/{}/events?{q}", cal_id(cal)), &[], Some(body)).await?,
            Some(e) => api(st, a, "PATCH", &format!("{BASE}/calendars/{}/events/{}?{q}", cal_id(cal), enc(&e.id)), &[], Some(body)).await?,
        };
        Ok(event(&v, &a.email))
    }

    pub async fn delete(st: &Shared, a: &Account, cal: &str, id: &str, notify: &str) -> Result<(), String> {
        api(st, a, "DELETE", &format!("{BASE}/calendars/{}/events/{}?sendUpdates={notify}", cal_id(cal), enc(id)), &[], None).await.map(|_| ())
    }

    pub async fn respond(st: &Shared, a: &Account, cal: &str, e: &Event, resp: &str, comment: &str) -> Result<(), String> {
        // Change only the user's own entry; the others are sent back as they are.
        let v = api(st, a, "GET", &format!("{BASE}/calendars/{}/events/{}", cal_id(cal), enc(&e.id)), &[], None).await?;
        let mut list = v.get("attendees").and_then(Value::as_array).cloned().unwrap_or_default();
        for x in list.iter_mut() {
            if x.get("self") == Some(&json!(true)) || s(x, "email").eq_ignore_ascii_case(&a.email) {
                x["responseStatus"] = json!(match resp { "yes" => "accepted", "maybe" => "tentative", _ => "declined" });
                if !comment.is_empty() {
                    x["comment"] = json!(comment);
                }
            }
        }
        api(st, a, "PATCH", &format!("{BASE}/calendars/{}/events/{}?sendUpdates=all", cal_id(cal), enc(&e.id)), &[], Some(json!({"attendees": list}))).await.map(|_| ())
    }

    /// Busy blocks per person.
    pub async fn busy(st: &Shared, a: &Account, who: &[String], from: i64, to: i64) -> Result<Vec<(String, Result<Vec<(i64, i64)>, String>)>, String> {
        if !a.scopes.split_whitespace().any(|x| x == FREEBUSY_SCOPE) && who.len() > 1 {
            return Err(format!("{} hasn't allowed openbot to see free time yet. The user can click Update permissions for it in Settings › Connections.", a.email));
        }
        if !a.scopes.split_whitespace().any(|x| x == FREEBUSY_SCOPE) {
            // Only the user: their own events work with the old permission.
            let events = list(st, a, "", from, to, "", 250).await?;
            let b = events.iter().filter(|e| !e.cancelled && e.show_as != "free" && e.my_response != "no").map(|e| (e.start, e.end)).collect();
            return Ok(vec![(a.email.clone(), Ok(b))]);
        }
        let body = json!({"timeMin": utc(from), "timeMax": utc(to), "items": who.iter().map(|w| json!({"id": if w.eq_ignore_ascii_case(&a.email) { "primary".to_string() } else { w.clone() }})).collect::<Vec<_>>()});
        let v = api(st, a, "POST", &format!("{BASE}/freeBusy"), &[], Some(body)).await?;
        let cals = v.get("calendars").cloned().unwrap_or(json!({}));
        Ok(who
            .iter()
            .map(|w| {
                let key = if w.eq_ignore_ascii_case(&a.email) { "primary".to_string() } else { w.clone() };
                let c = cals.get(&key).or_else(|| cals.get(w)).cloned().unwrap_or(json!({}));
                if let Some(err) = c.get("errors").and_then(Value::as_array).and_then(|l| l.first()) {
                    let why = match s(err, "reason").as_str() {
                        "notFound" => "not shared with the user".to_string(),
                        r => r.to_string(),
                    };
                    return (w.clone(), Err(why));
                }
                let b = c
                    .get("busy")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|x| {
                                let a = chrono::DateTime::parse_from_rfc3339(&s(x, "start")).ok()?.timestamp_millis();
                                let b = chrono::DateTime::parse_from_rfc3339(&s(x, "end")).ok()?.timestamp_millis();
                                Some((a, b))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (w.clone(), Ok(b))
            })
            .collect())
    }
}

// ------------------------------------------------------------------ Outlook (Microsoft Graph)

mod mscal {
    use super::*;

    const BASE: &str = "https://graph.microsoft.com/v1.0/me";
    const UTC: (&str, &str) = ("Prefer", "outlook.timezone=\"UTC\"");
    const SELECT: &str = "id,subject,start,end,isAllDay,location,attendees,organizer,webLink,onlineMeeting,isOnlineMeeting,responseStatus,showAs,sensitivity,type,seriesMasterId,isCancelled,isReminderOn,reminderMinutesBeforeStart,recurrence,body";

    fn graph_time(v: &Value) -> i64 {
        let t = v.as_str().unwrap_or("");
        NaiveDateTime::parse_from_str(t.get(..19).unwrap_or(t), "%Y-%m-%dT%H:%M:%S").map(|n| n.and_utc().timestamp_millis()).unwrap_or(0)
    }

    fn events_path(cal: &str) -> String {
        if cal.is_empty() { format!("{BASE}/calendar") } else { format!("{BASE}/calendars/{}", enc(cal)) }
    }

    pub fn event(e: &Value, me: &str) -> Event {
        let start = graph_time(e.pointer("/start/dateTime").unwrap_or(&Value::Null));
        let end = graph_time(e.pointer("/end/dateTime").unwrap_or(&Value::Null));
        let all_day = e.get("isAllDay") == Some(&json!(true));
        let utc_date = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).map(|x| x.date_naive()).unwrap_or_default();
        let org = e.pointer("/organizer/emailAddress/address").and_then(Value::as_str).unwrap_or("").to_string();
        let mut guests: Vec<Guest> = e
            .get("attendees")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter(|x| s(x, "type") != "resource")
                    .map(|x| {
                        let email = x.pointer("/emailAddress/address").and_then(Value::as_str).unwrap_or("").to_string();
                        Guest {
                            name: x.pointer("/emailAddress/name").and_then(Value::as_str).unwrap_or("").to_string(),
                            response: match x.pointer("/status/response").and_then(Value::as_str).unwrap_or("") {
                                "accepted" | "organizer" => "yes",
                                "declined" => "no",
                                "tentativelyAccepted" => "maybe",
                                _ => "waiting",
                            }
                            .into(),
                            optional: s(x, "type") == "optional",
                            is_self: email.eq_ignore_ascii_case(me),
                            organizer: email.eq_ignore_ascii_case(&org),
                            email,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Outlook doesn't list the user among the guests of an invitation they received; add them so replies work.
        let mine = e.pointer("/responseStatus/response").and_then(Value::as_str).unwrap_or("");
        if !guests.is_empty() && !guests.iter().any(|g| g.is_self) && !org.eq_ignore_ascii_case(me) {
            guests.push(Guest { email: me.to_string(), name: String::new(), response: "waiting".into(), optional: false, is_self: true, organizer: false });
        }
        let my_response = match mine {
            "accepted" => "yes",
            "declined" => "no",
            "tentativelyAccepted" => "maybe",
            "notResponded" | "none" if !org.eq_ignore_ascii_case(me) && !guests.is_empty() => "waiting",
            _ => "",
        }
        .to_string();
        for g in guests.iter_mut().filter(|g| g.is_self) {
            if !my_response.is_empty() {
                g.response = my_response.clone();
            }
        }
        let repeat = e.get("recurrence").filter(|r| !r.is_null()).map(|r| vec![describe(r)]).unwrap_or_default();
        Event {
            id: s(e, "id"),
            series_id: s(e, "seriesMasterId"),
            calendar: String::new(),
            title: s(e, "subject"),
            start,
            end,
            days: if all_day { Some((utc_date(start), utc_date(end))) } else { None },
            location: e.pointer("/location/displayName").and_then(Value::as_str).unwrap_or("").to_string(),
            description: e.pointer("/body/content").and_then(Value::as_str).unwrap_or("").to_string(),
            organizer: if org.eq_ignore_ascii_case(me) { me.to_string() } else { e.pointer("/organizer/emailAddress/name").and_then(Value::as_str).map(|n| format!("{n} <{org}>")).unwrap_or(org.clone()) },
            guests,
            repeat,
            link: s(e, "webLink"),
            video: e.pointer("/onlineMeeting/joinUrl").and_then(Value::as_str).unwrap_or("").to_string(),
            my_response,
            cancelled: e.get("isCancelled") == Some(&json!(true)),
            color: String::new(),
            show_as: match s(e, "showAs").as_str() { "free" => "free", "" => "", _ => "busy" }.into(),
            visibility: match s(e, "sensitivity").as_str() { "private" | "confidential" => "private", _ => "default" }.into(),
            reminders: if e.get("isReminderOn") == Some(&json!(true)) { vec![("popup".into(), e.get("reminderMinutesBeforeStart").and_then(Value::as_i64).unwrap_or(15))] } else { vec![] },
            default_reminders: false,
        }
    }

    /// "every week on Mon, Wed until 2027-06-01" from Outlook's repeat pattern.
    fn describe(r: &Value) -> String {
        let p = r.get("pattern").cloned().unwrap_or(json!({}));
        let n = p.get("interval").and_then(Value::as_i64).unwrap_or(1);
        let unit = match s(&p, "type").as_str() {
            "daily" => "day",
            "weekly" => "week",
            "absoluteMonthly" | "relativeMonthly" => "month",
            _ => "year",
        };
        let mut t = if n == 1 { format!("every {unit}") } else { format!("every {n} {unit}s") };
        if let Some(days) = p.get("daysOfWeek").and_then(Value::as_array).filter(|d| !d.is_empty()) {
            t.push_str(&format!(" on {}", days.iter().filter_map(Value::as_str).map(|d| d.get(..3).unwrap_or(d)).collect::<Vec<_>>().join(", ")));
        }
        let range = r.get("range").cloned().unwrap_or(json!({}));
        match s(&range, "type").as_str() {
            "endDate" => t.push_str(&format!(" until {}", s(&range, "endDate"))),
            "numbered" => t.push_str(&format!(" ({} times)", range.get("numberOfOccurrences").and_then(Value::as_i64).unwrap_or(0))),
            _ => {}
        }
        t
    }

    /// An RFC 5545 RRULE -> Outlook's repeat pattern (the common cases).
    pub fn pattern(lines: &[String], first: NaiveDate) -> Result<Value, String> {
        let rule = lines.iter().find_map(|l| l.strip_prefix("RRULE:")).ok_or("Outlook needs a repeat rule that starts with RRULE:")?;
        let mut p = std::collections::HashMap::new();
        for kv in rule.split(';') {
            if let Some((k, v)) = kv.split_once('=') {
                p.insert(k.to_ascii_uppercase(), v.to_ascii_uppercase());
            }
        }
        let day = |c: &str| -> Option<&'static str> {
            Some(match c {
                "MO" => "monday", "TU" => "tuesday", "WE" => "wednesday", "TH" => "thursday", "FR" => "friday", "SA" => "saturday", "SU" => "sunday",
                _ => return None,
            })
        };
        let interval: i64 = p.get("INTERVAL").and_then(|x| x.parse().ok()).unwrap_or(1);
        let byday: Vec<String> = p.get("BYDAY").map(|d| d.split(',').map(str::to_string).collect()).unwrap_or_default();
        let weekday_of_first = match first.weekday() {
            Weekday::Mon => "monday", Weekday::Tue => "tuesday", Weekday::Wed => "wednesday", Weekday::Thu => "thursday", Weekday::Fri => "friday", Weekday::Sat => "saturday", Weekday::Sun => "sunday",
        };
        let pattern = match p.get("FREQ").map(String::as_str) {
            Some("DAILY") => json!({"type": "daily", "interval": interval}),
            Some("WEEKLY") => {
                let days: Vec<&str> = if byday.is_empty() { vec![weekday_of_first] } else { byday.iter().filter_map(|d| day(d)).collect() };
                json!({"type": "weekly", "interval": interval, "daysOfWeek": days, "firstDayOfWeek": "sunday"})
            }
            Some("MONTHLY") if !byday.is_empty() => {
                let d = &byday[0];
                let (nth, code) = d.split_at(d.len().saturating_sub(2));
                let index = match nth { "1" | "+1" => "first", "2" => "second", "3" => "third", "4" => "fourth", "-1" => "last", _ => "first" };
                json!({"type": "relativeMonthly", "interval": interval, "daysOfWeek": [day(code).unwrap_or(weekday_of_first)], "index": index})
            }
            Some("MONTHLY") => json!({"type": "absoluteMonthly", "interval": interval, "dayOfMonth": p.get("BYMONTHDAY").and_then(|x| x.parse::<i64>().ok()).unwrap_or(first.day() as i64)}),
            Some("YEARLY") => json!({"type": "absoluteYearly", "interval": interval, "month": first.month(), "dayOfMonth": first.day()}),
            _ => return Err("Outlook repeats daily, weekly, monthly or yearly (FREQ=DAILY|WEEKLY|MONTHLY|YEARLY)".into()),
        };
        let range = if let Some(c) = p.get("COUNT").and_then(|x| x.parse::<i64>().ok()) {
            json!({"type": "numbered", "startDate": first.to_string(), "numberOfOccurrences": c})
        } else if let Some(u) = p.get("UNTIL").and_then(|u| u.get(..8)).and_then(|u| NaiveDate::parse_from_str(u, "%Y%m%d").ok()) {
            json!({"type": "endDate", "startDate": first.to_string(), "endDate": u.to_string()})
        } else {
            json!({"type": "noEnd", "startDate": first.to_string()})
        };
        Ok(json!({"pattern": pattern, "range": range}))
    }

    pub async fn calendars(st: &Shared, a: &Account) -> Result<Vec<Cal>, String> {
        let v = api(st, a, "GET", &format!("{BASE}/calendars?$select=id,name,isDefaultCalendar,canEdit&$top=100"), &[], None).await?;
        Ok(v.get("value")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|c| Cal { id: s(c, "id"), name: s(c, "name"), main: c.get("isDefaultCalendar") == Some(&json!(true)), can_edit: c.get("canEdit") == Some(&json!(true)), shown: true, time_zone: String::new() })
            .collect())
    }

    pub async fn list(st: &Shared, a: &Account, cal: &str, from: i64, to: i64, q: &str, max: usize) -> Result<Vec<Event>, String> {
        let url = format!(
            "{}/calendarView?startDateTime={}&endDateTime={}&$top={}&$orderby=start/dateTime&$select={SELECT}",
            events_path(cal),
            enc(&utc(from)),
            enc(&utc(to)),
            if q.is_empty() { max.min(100) } else { 250 }
        );
        let v = api(st, a, "GET", &url, &[UTC, ("Prefer", "outlook.body-content-type=\"text\"")], None).await?;
        let words: Vec<String> = q.to_lowercase().split_whitespace().map(str::to_string).collect();
        Ok(v.get("value")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|e| event(e, &a.email))
            .filter(|e| {
                let hay = format!("{} {} {} {}", e.title, e.location, e.description, e.guests.iter().map(|g| g.email.as_str()).collect::<Vec<_>>().join(" ")).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .take(max)
            .collect())
    }

    pub async fn get(st: &Shared, a: &Account, id: &str) -> Result<Event, String> {
        let v = api(st, a, "GET", &format!("{BASE}/events/{}?$select={SELECT}", enc(id)), &[UTC, ("Prefer", "outlook.body-content-type=\"text\"")], None).await?;
        Ok(event(&v, &a.email))
    }

    pub async fn write(st: &Shared, a: &Account, cal: &str, existing: Option<&Event>, ch: &Change) -> Result<Event, String> {
        let mut body = json!({});
        if let Some(t) = &ch.title {
            body["subject"] = json!(t);
        }
        if let Some(l) = &ch.location {
            body["location"] = json!({"displayName": l});
        }
        if let Some(d) = &ch.description {
            body["body"] = json!({"contentType": "Text", "content": d});
        }
        if let Some(g) = &ch.guests {
            body["attendees"] = json!(g.iter().map(|(x, o)| json!({"emailAddress": {"address": x}, "type": if *o { "optional" } else { "required" }})).collect::<Vec<_>>());
        }
        let naive = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default();
        let mut first = existing.map(|e| e.days.map(|d| d.0).unwrap_or_else(|| local_date(e.start)));
        match ch.times {
            Some(Times::Timed(s1, e1)) => {
                body["isAllDay"] = json!(false);
                // Outlook accepts IANA timezone names; with one, a repeating event keeps its clock time.
                let zone = ch.time_zone.clone().or_else(crate::util::timezone_name).and_then(|z| z.parse::<chrono_tz::Tz>().ok());
                let at = |ms: i64| match zone {
                    Some(z) => json!({"dateTime": z.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default(), "timeZone": z.name()}),
                    None => json!({"dateTime": naive(ms), "timeZone": "UTC"}),
                };
                body["start"] = at(s1);
                body["end"] = at(e1);
                first = Some(local_date(s1));
            }
            Some(Times::AllDay(d1, d2)) => {
                body["isAllDay"] = json!(true);
                body["start"] = json!({"dateTime": format!("{d1}T00:00:00"), "timeZone": "UTC"});
                body["end"] = json!({"dateTime": format!("{d2}T00:00:00"), "timeZone": "UTC"});
                first = Some(d1);
            }
            None => {}
        }
        if let Some(r) = &ch.repeat {
            body["recurrence"] = if r.is_empty() { Value::Null } else { pattern(r, first.unwrap_or_else(|| local_date(now_ms())))? };
        }
        if let Some(r) = &ch.reminders {
            match r.first() {
                Some((_, n)) => {
                    body["isReminderOn"] = json!(true);
                    body["reminderMinutesBeforeStart"] = json!(n);
                }
                None => body["isReminderOn"] = json!(false),
            }
        }
        if ch.video_call {
            body["isOnlineMeeting"] = json!(true);
            body["onlineMeetingProvider"] = json!("teamsForBusiness");
        }
        if let Some(sh) = &ch.show_as {
            body["showAs"] = json!(sh);
        }
        if let Some(vis) = &ch.visibility {
            body["sensitivity"] = json!(if vis == "private" { "private" } else { "normal" });
        }
        let v = match existing {
            None => api(st, a, "POST", &format!("{}/events", events_path(cal)), &[UTC], Some(body)).await?,
            Some(e) => api(st, a, "PATCH", &format!("{BASE}/events/{}", enc(&e.id)), &[UTC], Some(body)).await?,
        };
        Ok(event(&v, &a.email))
    }

    pub async fn delete(st: &Shared, a: &Account, id: &str) -> Result<(), String> {
        api(st, a, "DELETE", &format!("{BASE}/events/{}", enc(id)), &[], None).await.map(|_| ())
    }

    pub async fn respond(st: &Shared, a: &Account, id: &str, resp: &str, comment: &str) -> Result<(), String> {
        let action = match resp { "yes" => "accept", "maybe" => "tentativelyAccept", _ => "decline" };
        api(st, a, "POST", &format!("{BASE}/events/{}/{action}", enc(id)), &[], Some(json!({"comment": comment, "sendResponse": true}))).await.map(|_| ())
    }

    /// Busy blocks per person. Other people's free time works only for work or school accounts.
    pub async fn busy(st: &Shared, a: &Account, who: &[String], from: i64, to: i64) -> Result<Vec<(String, Result<Vec<(i64, i64)>, String>)>, String> {
        let body = json!({
            "schedules": who, "availabilityViewInterval": 15,
            "startTime": {"dateTime": utc(from).trim_end_matches('Z'), "timeZone": "UTC"},
            "endTime": {"dateTime": utc(to).trim_end_matches('Z'), "timeZone": "UTC"},
        });
        match api(st, a, "POST", &format!("{BASE}/calendar/getSchedule"), &[UTC], Some(body)).await {
            Ok(v) => Ok(v
                .get("value")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|x| {
                    let id = s(x, "scheduleId");
                    if let Some(err) = x.get("error").filter(|e| !e.is_null()) {
                        return (id, Err(s(err, "message")));
                    }
                    let b = x
                        .get("scheduleItems")
                        .and_then(Value::as_array)
                        .map(|l| l.iter().filter(|i| s(i, "status") != "free").map(|i| (graph_time(i.pointer("/start/dateTime").unwrap_or(&Value::Null)), graph_time(i.pointer("/end/dateTime").unwrap_or(&Value::Null)))).collect())
                        .unwrap_or_default();
                    (id, Ok(b))
                })
                .collect()),
            // Personal Microsoft accounts can't use getSchedule: read the user's own calendar instead.
            Err(_) => {
                let events = list(st, a, "", from, to, "", 250).await?;
                let mine = events.iter().filter(|e| !e.cancelled && e.show_as != "free" && e.my_response != "no").map(|e| (e.start, e.end)).collect();
                let mut out = vec![(a.email.clone(), Ok(mine))];
                for w in who.iter().skip(1) {
                    out.push((w.clone(), Err("personal Microsoft accounts can't see other people's free time".into())));
                }
                Ok(out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_changes() {
        let c = change_in(&json!({"title": "x", "start": "2026-11-05", "end": "2026-11-05", "all_day": true}), true, None).unwrap();
        assert_eq!(c.times, Some(Times::AllDay(NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(), NaiveDate::from_ymd_opt(2026, 11, 6).unwrap())));
        assert!(change_in(&json!({"title": "x", "start": "2026-11-05T10:00", "end": "2026-11-05T09:00"}), true, None).is_err());
        assert!(change_in(&json!({"start": "2026-11-05T10:00", "end": "2026-11-05T11:00"}), true, None).is_err());
        assert!(change_in(&json!({"title": "x", "start": "2026-11-05T10:00", "end": "2026-11-05T11:00", "attendees": ["nope"]}), true, None).is_err());
        assert!(change_in(&json!({"title": "x", "start": "2026-11-05T10:00", "end": "2026-11-05T11:00", "repeat": ["every monday"]}), true, None).is_err());
        let c = change_in(&json!({"title": "x", "start": "2026-11-05T10:00", "end": "2026-11-05T11:00", "color": "Tomato", "reminders": [{"minutes": 10}, 60], "notify": "none"}), true, None).unwrap();
        assert_eq!(c.color.as_deref(), Some("11"));
        assert_eq!(c.reminders, Some(vec![("popup".into(), 10), ("popup".into(), 60)]));
        assert_eq!(c.notify, "none");
    }

    #[test]
    fn guest_changes_keep_the_rest() {
        let old = Event {
            guests: vec![
                Guest { email: "me@x.com".into(), name: String::new(), response: "yes".into(), optional: false, is_self: true, organizer: true },
                Guest { email: "a@x.com".into(), name: String::new(), response: "yes".into(), optional: false, is_self: false, organizer: false },
            ],
            ..Default::default()
        };
        let c = change_in(&json!({"id": "1", "add_attendees": ["B@x.com"], "remove_attendees": ["a@x.com"]}), false, Some(&old)).unwrap();
        assert_eq!(c.guests, Some(vec![("b@x.com".into(), false)]));
    }

    #[test]
    fn repeat_rules() {
        assert_eq!(repeat_text(&["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10".into()]), "every week on Mon, Wed (10 times)");
        assert_eq!(repeat_text(&["RRULE:FREQ=MONTHLY;INTERVAL=2;BYDAY=-1FR;UNTIL=20270601T000000Z".into()]), "every 2 months on last Fri until 2027-06-01");
        let first = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let p = mscal::pattern(&["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20270101".into()], first).unwrap();
        assert_eq!(p["pattern"]["daysOfWeek"], json!(["monday", "wednesday"]));
        assert_eq!(p["range"]["endDate"], json!("2027-01-01"));
        let p = mscal::pattern(&["RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=5".into()], first).unwrap();
        assert_eq!((p["pattern"]["type"].clone(), p["pattern"]["index"].clone(), p["range"]["numberOfOccurrences"].clone()), (json!("relativeMonthly"), json!("second"), json!(5)));
    }

    #[test]
    fn finds_free_slots() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap(); // a Monday
        let at = |h: u32, m: u32| crate::util::from_local(day.and_hms_opt(h, m, 0).unwrap()).unwrap();
        let t = |h: u32| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let busy = [(at(10, 0), at(11, 0)), (at(11, 15), at(12, 0))];
        let slots = free_slots(&busy, at(0, 0), at(23, 0), t(9), t(13), false, 30, 10);
        assert_eq!(slots, vec![(at(9, 0), at(10, 0)), (at(12, 0), at(13, 0))]);
    }
}
