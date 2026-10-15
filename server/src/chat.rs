//! Chat rules: what makes a valid nickname or message, and how taken nicknames get a number.

use std::collections::HashSet;

pub const MAX_NICK_CHARS: usize = 24;
pub const MAX_TEXT_CHARS: usize = 200;
/// How many recent messages new viewers (and a reconnecting AI host) are sent.
pub const HISTORY: usize = 50;

/// Trims, collapses whitespace and drops invisible characters. `None` if nothing
/// is left or it is longer than 24 characters.
pub fn clean_nick(raw: &str) -> Option<String> {
    clean(raw, MAX_NICK_CHARS)
}

/// Same as [`clean_nick`], up to 200 characters.
pub fn clean_text(raw: &str) -> Option<String> {
    clean(raw, MAX_TEXT_CHARS)
}

/// Trims, collapses whitespace and drops invisible characters; `None` if the
/// result is empty or longer than `max_chars`.
pub fn clean(raw: &str, max_chars: usize) -> Option<String> {
    let visible: String = raw.chars().filter(|&c| !is_invisible(c)).collect();
    let cleaned = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = cleaned.chars().count();
    (1..=max_chars).contains(&chars).then_some(cleaned)
}

/// Control characters, zero-width characters and direction overrides, which
/// could otherwise hide text or make one nickname pass for another.
fn is_invisible(c: char) -> bool {
    (c.is_control() && !c.is_whitespace())
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Nicknames in use, compared without case.
#[derive(Default)]
pub struct Nicknames {
    taken: HashSet<String>,
}

impl Nicknames {
    /// Gives out `wanted`, or `wanted` with the lowest free number after it
    /// (shortened if needed to stay within 24 characters).
    pub fn claim(&mut self, wanted: &str) -> String {
        let mut candidate = wanted.to_owned();
        let mut n = 2;
        while self.taken.contains(&candidate.to_lowercase()) {
            let suffix = n.to_string();
            let room = MAX_NICK_CHARS - suffix.len();
            candidate = wanted.chars().take(room).collect::<String>() + &suffix;
            n += 1;
        }
        self.taken.insert(candidate.to_lowercase());
        candidate
    }

    pub fn release(&mut self, nick: &str) {
        self.taken.remove(&nick.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_trims_collapses_and_strips_invisible_characters() {
        assert_eq!(clean_text("  hi\n\n  there\t "), Some("hi there".into()));
        assert_eq!(clean_text("hi\u{7} there"), Some("hi there".into()));
        assert_eq!(clean_nick("An\u{200B}a\u{202E}"), Some("Ana".into()));
        assert_eq!(clean_text("Cât câștigi?"), Some("Cât câștigi?".into()));
    }

    #[test]
    fn empty_or_long_input_is_rejected() {
        assert_eq!(clean_nick("   "), None);
        assert_eq!(clean_nick("\u{200B}"), None);
        assert_eq!(clean_nick(&"a".repeat(24)), Some("a".repeat(24)));
        assert_eq!(clean_nick(&"a".repeat(25)), None);
        // Characters, not bytes: 24 two-byte letters are fine.
        assert!(clean_nick(&"ă".repeat(24)).is_some());
        assert!(clean_text(&"x".repeat(200)).is_some());
        assert_eq!(clean_text(&"x".repeat(201)), None);
    }

    #[test]
    fn taken_nicknames_get_a_number() {
        let mut nicks = Nicknames::default();
        assert_eq!(nicks.claim("Ana"), "Ana");
        assert_eq!(nicks.claim("ana"), "ana2");
        assert_eq!(nicks.claim("ANA"), "ANA3");
        nicks.release("ana2");
        assert_eq!(nicks.claim("Ana"), "Ana2");
    }

    #[test]
    fn numbered_nicknames_stay_within_the_limit() {
        let mut nicks = Nicknames::default();
        let long = "a".repeat(24);
        assert_eq!(nicks.claim(&long), long);
        let second = nicks.claim(&long);
        assert_eq!(second, format!("{}2", "a".repeat(23)));
        assert_eq!(second.chars().count(), 24);
    }
}
