//! Home AI: the talkable natural-language layer at the home screen.
//!
//! Bare input that the command grammar does not claim falls back to a
//! fuzzy app search; before that, [`interpret`] recognizes every-day
//! phrases ("remind me to stretch at 18:30", "wake me at 9am",
//! "add a note buy milk", "open terminal", "what time is it") and turns
//! them into the same [`Command`] values the grammar would produce —
//! reminders schedule through the alarm pipeline, so a phrase can also
//! address another app with an `@app-id` (cross-app triggering through
//! the IPC broker, as discussed for the home assistant).
//!
//! Unrecognized phrases return `None` and keep fuzzy launch behavior.

use crate::command::{search_slots, Command};

const TIME_PHRASES: &[&str] = &[
    "what time is it",
    "what time is it now",
    "what's the time",
    "whats the time",
    "tell me the time",
];

const ALARM_LIST_PHRASES: &[&str] = &[
    "what alarms",
    "my alarms",
    "show alarms",
    "show my alarms",
    "list alarms",
    "what alarms do i have",
];

/// First words a dictation must start with to be read as a reminder
/// (`alarm` and `schedule` heads are already claimed by the grammar).
const REMINDER_PREFIXES: &[&str] = &["remind me", "reminder", "wake me", "timer"];

/// Absolute time in a phrase: `at 18:30`, `at 6pm`, `6 pm`, `9:15am`, or
/// a bare `HH:MM`. Returns `(canonical_hhmm, rest_of_phrase)`.
fn parse_at_time(body: &str) -> Option<(String, String)> {
    let trimmed = body.trim();
    let t = trimmed
        .strip_prefix("at ")
        .map(str::trim)
        .unwrap_or(trimmed);
    // Single token ("6pm", "18:30") or first word; a bare "at" falls out
    // later in the clock validation.
    let (token, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    if token.is_empty() {
        return None;
    }
    let tok = token.to_ascii_lowercase();
    // `6pm`/`9:15am` inline (suffix on the token), `6 pm` as its own word,
    // else the clock alone (`18:30`, `9:15`). A bare hour is only trusted
    // next to an am/pm marker, not on its own ("remind me 6 the dishes").
    let (clock, is_pm, has_ampm, rest) = if let Some(c) = tok.strip_suffix("pm") {
        (c.to_string(), true, true, rest.to_string())
    } else if let Some(c) = tok.strip_suffix("am") {
        (c.to_string(), false, true, rest.to_string())
    } else {
        let lowered_rest = rest.to_ascii_lowercase();
        match lowered_rest.split_once(char::is_whitespace) {
            Some((w, _r)) if w == "am" || w == "pm" => {
                let consumed = rest.split_once(char::is_whitespace).unwrap().1;
                (tok, w == "pm", true, consumed.to_string())
            }
            _ => (tok, false, false, rest.to_string()),
        }
    };
    let (h_str, m_val) = match clock.split_once(':') {
        Some((h, m))
            if h.len() <= 2
                && m.len() == 2
                && h.bytes().all(|c| c.is_ascii_digit())
                && m.bytes().all(|c| c.is_ascii_digit()) =>
        {
            (h.to_string(), m.parse::<u32>().ok()?)
        }
        Some(_) => return None,
        // Bare `18:30` is a time; a bare digit is only trusted next to an
        // am/pm marker (`6pm`), not on its own ("remind me 6 the dishes").
        None if !clock.is_empty() && (clock.contains(':') || has_ampm) => {
            let (h, m) = clock.split_once(':').unwrap_or((clock.as_str(), "0"));
            (h.to_string(), m.parse::<u32>().ok()?)
        }
        _ => return None,
    };
    let mut hour: u32 = h_str.parse().ok()?;
    if is_pm && hour < 12 {
        hour += 12;
    }
    if hour > 23 || m_val > 59 {
        return None;
    }
    Some((format!("{hour:02}:{m_val:02}"), rest))
}

/// Leading `in 20 minutes …` in a lowercased body: `(seconds, remainder)`.
/// A week is the sane ceiling for talk-to-the-shell times.
pub fn parse_in_seconds(s: &str) -> Option<(u64, &str)> {
    let after = s.trim().strip_prefix("in ").map(str::trim)?;
    let (num_w, rest) = after.split_once(char::is_whitespace)?;
    let n: u64 = num_w.trim().parse().ok()?;
    let (unit_w, text) = match rest.split_once(char::is_whitespace) {
        Some((w, t)) => (w, t.trim()),
        None => (rest, ""),
    };
    let secs = match unit_w.trim_end_matches(',') {
        "s" | "sec" | "secs" | "second" | "seconds" => n,
        "m" | "min" | "mins" | "minute" | "minutes" => n * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => n * 3600,
        "d" | "day" | "days" => n * 86400,
        _ => return None,
    };
    if secs == 0 || secs > 7 * 86400 {
        return None;
    }
    Some((secs, text))
}

/// `in 20 min`, `in 1 h` — how a relative alarm reports itself.
pub fn fmt_in(secs: u64) -> String {
    if secs % 3600 == 0 {
        format!("in {} h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("in {} min", secs / 60)
    } else {
        format!("in {secs} s")
    }
}

/// Text after the time: drop dictation glue like a leading `to` or comma.
fn reminder_text(rest: &str) -> String {
    let t = rest.trim().trim_start_matches(", ").trim();
    t.strip_prefix("to ").unwrap_or(t).trim().to_string()
}

/// Turn a bare phrase into a command, or `None` to keep the regular
/// fuzzy app search.
pub fn interpret(input: &str) -> Option<Command> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let lowered = s.trim_end_matches(['?', '.', '!']).to_ascii_lowercase();
    if TIME_PHRASES.contains(&lowered.as_str()) {
        return Some(Command::Time);
    }
    if ALARM_LIST_PHRASES.contains(&lowered.as_str()) {
        return Some(Command::Alarm(String::new()));
    }
    if let Some(n) = lowered
        .strip_prefix("cancel alarm")
        .map(str::trim)
        .and_then(|n| n.parse::<usize>().ok())
    {
        return Some(Command::Alarm(format!("cancel {n}")));
    }
    for (prefix, is_task) in [
        ("add task", true),
        ("add a task", true),
        ("add note", false),
        ("add a note", false),
    ] {
        if let Some(t) = lowered.strip_prefix(prefix) {
            let t = t.trim();
            if !t.is_empty() {
                return Some(if is_task {
                    Command::Tasks(format!("add {t}"))
                } else {
                    Command::Note(t.to_string())
                });
            }
        }
    }
    for lead in ["open", "launch", "start"] {
        let Some(t) = lowered.strip_prefix(lead) else {
            continue;
        };
        let t = t.trim();
        if t.is_empty() {
            return None;
        }
        return search_slots(t, 1).first().map(|s| Command::Launch(s.id));
    }
    for lead in REMINDER_PREFIXES {
        if !lowered.starts_with(lead) {
            continue;
        }
        let body_orig = &s[lead.len()..];
        let lowered_body = &lowered[lead.len()..];
        // Relative: `in 20 minutes (to) check the oven`; a bare `timer N
        // minutes` reads the same. The alarm grammar parses the same
        // `in N unit` shape, so hand it the verbatim tail.
        let relative_probe = if *lead == "timer" {
            format!("in {lowered_body}")
        } else {
            lowered_body.to_string()
        };
        if let Some((_secs, remainder)) = parse_in_seconds(&relative_probe) {
            // The alarm grammar's `in` branch re-parses this exact tail.
            let mut alarm_body = if *lead == "timer" {
                format!("in {}", body_orig.trim())
            } else {
                body_orig.trim().to_string()
            };
            if *lead == "timer" && remainder.trim().is_empty() {
                alarm_body.push_str(" timer done");
            }
            return Some(Command::Alarm(alarm_body));
        }
        // Absolute: `at 18:30 make coffee`, `at 6pm …`, `9:15am`, or a time
        // mid-phrase (`remind me to stretch at 6pm`).
        let parsed = parse_at_time(body_orig).or_else(|| {
            lowered_body.find(" at ").and_then(|pos| {
                parse_at_time(&body_orig[pos + 1..]).map(|(hhmm, rest)| {
                    let before = reminder_text(&body_orig[..pos]);
                    let text = [before, rest.trim().to_string()]
                        .into_iter()
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    (hhmm, text)
                })
            })
        });
        let Some((hhmm, rest)) = parsed else {
            return None;
        };
        let mut text = reminder_text(&rest);
        if text.is_empty() {
            match *lead {
                "wake me" => text = "wake up".into(),
                "timer" => text = "timer done".into(),
                _ => return None, // `remind me at 18:30` with no text: unclear
            }
        }
        return Some(Command::Alarm(format!("{hhmm} {text}")));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_and_alarm_questions() {
        assert_eq!(interpret("what time is it"), Some(Command::Time));
        assert_eq!(interpret("What's the time?"), Some(Command::Time));
        assert_eq!(interpret("my alarms"), Some(Command::Alarm(String::new())));
        assert_eq!(
            interpret("cancel alarm 2"),
            Some(Command::Alarm("cancel 2".into()))
        );
    }

    #[test]
    fn reminders_go_to_the_alarm_grammar() {
        assert_eq!(
            interpret("remind me at 18:30 make coffee"),
            Some(Command::Alarm("18:30 make coffee".into()))
        );
        assert_eq!(
            interpret("remind me to stretch at 6pm"),
            Some(Command::Alarm("18:00 stretch".into()))
        );
        assert_eq!(
            interpret("wake me at 9:15am"),
            Some(Command::Alarm("09:15 wake up".into()))
        );
        // Cross-app: the time address a module through the broker.
        assert_eq!(
            interpret("remind me at 07:00 @app-weather rain check"),
            Some(Command::Alarm("07:00 @app-weather rain check".into()))
        );
    }

    #[test]
    fn relative_durations() {
        assert_eq!(
            interpret("remind me in 20 minutes check the oven"),
            Some(Command::Alarm("in 20 minutes check the oven".into()))
        );
        assert_eq!(
            interpret("timer 5 minutes"),
            Some(Command::Alarm("in 5 minutes timer done".into()))
        );
        assert_eq!(
            parse_in_seconds("in 20 minutes check the oven"),
            Some((1200, "check the oven"))
        );
        assert_eq!(parse_in_seconds("in 1 hour"), Some((3600, "")));
        assert_eq!(parse_in_seconds("in 90"), None);
        assert_eq!(parse_in_seconds("in 0 min"), None);
        assert_eq!(fmt_in(1200), "in 20 min");
        assert_eq!(fmt_in(3600), "in 1 h");
    }

    #[test]
    fn dictations_and_app_opens() {
        assert_eq!(
            interpret("add a note buy milk"),
            Some(Command::Note("buy milk".into()))
        );
        assert_eq!(
            interpret("add task ship the shell"),
            Some(Command::Tasks("add ship the shell".into()))
        );
        assert_eq!(
            interpret("open terminal"),
            Some(Command::Launch("app-terminal"))
        );
    }

    #[test]
    fn unknown_phrases_stay_fuzzy() {
        assert_eq!(interpret("term"), None);
        assert_eq!(interpret("rgb"), None);
        assert_eq!(interpret("remind me"), None);
        assert_eq!(interpret("open"), None);
        assert_eq!(interpret("set brightness"), None);
    }
}
