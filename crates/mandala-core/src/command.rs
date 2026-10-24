//! Fuzzy slot search and the command-line grammar of the main input field.

use crate::slots::{SlotDef, SLOTS};

/// Case-insensitive subsequence match. Higher is better; `None` = no match.
/// Rewards prefix hits, word-boundary hits and consecutive runs.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    let mut qi = 0;
    let mut score = 0i32;
    let mut prev_match: Option<usize> = None;
    for (ci, &c) in cand.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += 10;
            if ci == 0 {
                score += 15;
            } else if !cand[ci - 1].is_alphanumeric() {
                score += 8;
            }
            if prev_match == Some(ci.wrapping_sub(1)) {
                score += 6;
            }
            prev_match = Some(ci);
            qi += 1;
        }
    }
    if qi == q.len() {
        // Prefer shorter candidates for equal matches.
        Some(score - (cand.len() as i32 - q.len() as i32).max(0))
    } else {
        None
    }
}

/// Slots matching `query`, best first. Matches the label, the module id and
/// the `A1`-style tag.
pub fn search_slots(query: &str, limit: usize) -> Vec<&'static SlotDef> {
    let mut scored: Vec<(i32, &'static SlotDef)> = SLOTS
        .iter()
        .filter_map(|s| {
            let best = [
                fuzzy_score(query, s.label),
                fuzzy_score(query, s.id.trim_start_matches("app-")),
                fuzzy_score(query, &s.tag()),
            ]
            .into_iter()
            .flatten()
            .max();
            best.map(|b| (b, s))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(b.1.label)));
    scored.into_iter().take(limit).map(|(_, s)| s).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `= 2*(3+4)` -> calculator `eval`.
    Calc(String),
    /// `= save 2+2` -> calculator `save` (result is appended to Notes via IPC).
    CalcSave(String),
    /// `note buy milk` -> notes `append`.
    Note(String),
    /// `theme next` / `theme cel-shaded`.
    Theme(String),
    /// `monitor` opens the module process monitor.
    Monitor,
    /// Launch the slot with this id.
    Launch(&'static str),
    Empty,
    NoMatch(String),
}

pub fn parse_command(input: &str) -> Command {
    let s = input.trim();
    if s.is_empty() {
        return Command::Empty;
    }
    if let Some(rest) = s.strip_prefix('=') {
        let rest = rest.trim();
        if let Some(expr) = rest.strip_prefix("save ") {
            return Command::CalcSave(expr.trim().to_string());
        }
        return Command::Calc(rest.to_string());
    }
    let (head, tail) = match s.split_once(char::is_whitespace) {
        Some((h, t)) => (h, t.trim()),
        None => (s, ""),
    };
    match head.to_ascii_lowercase().as_str() {
        "note" | "n" if !tail.is_empty() => return Command::Note(tail.to_string()),
        "theme" if !tail.is_empty() => return Command::Theme(tail.to_string()),
        "monitor" | "ps" => return Command::Monitor,
        _ => {}
    }
    match search_slots(s, 1).first() {
        Some(slot) => Command::Launch(slot.id),
        None => Command::NoMatch(s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_basics() {
        assert!(fuzzy_score("trm", "Terminal").is_some());
        assert!(fuzzy_score("xyz", "Terminal").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert!(fuzzy_score("term", "Terminal") > fuzzy_score("term", "Determinant long name"));
    }

    #[test]
    fn search_ranks_prefix_first() {
        assert_eq!(search_slots("calc", 3)[0].id, "app-calculator");
        assert_eq!(search_slots("note", 1)[0].id, "app-notes");
        assert_eq!(search_slots("c4", 1)[0].id, "app-calculator");
        assert!(search_slots("zzzzqq", 5).is_empty());
    }

    #[test]
    fn command_grammar() {
        assert_eq!(parse_command(""), Command::Empty);
        assert_eq!(parse_command("= 2+2"), Command::Calc("2+2".into()));
        assert_eq!(parse_command("=save 2+2"), Command::CalcSave("2+2".into()));
        assert_eq!(parse_command("= save 2+2"), Command::CalcSave("2+2".into()));
        assert_eq!(parse_command("note buy milk"), Command::Note("buy milk".into()));
        assert_eq!(parse_command("n x"), Command::Note("x".into()));
        assert_eq!(parse_command("theme next"), Command::Theme("next".into()));
        assert_eq!(parse_command("monitor"), Command::Monitor);
        assert_eq!(parse_command("termin"), Command::Launch("app-terminal"));
        assert_eq!(parse_command("qqqqzz"), Command::NoMatch("qqqqzz".into()));
    }

    #[test]
    fn bare_note_word_launches_notes() {
        assert_eq!(parse_command("note"), Command::Launch("app-notes"));
    }
}
