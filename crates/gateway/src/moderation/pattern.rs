//! Patterns with wildcards, for the pattern list of the rule filter.

/// One part of a pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// `*`: any number of characters, also none.
    Run,
    /// `?`: one character.
    One,
    /// `#`: one digit.
    Digit,
    /// This character.
    Is(char),
}

impl Part {
    fn takes(self, c: char) -> bool {
        match self {
            Part::Run => false,
            Part::One => true,
            Part::Digit => c.is_ascii_digit(),
            Part::Is(wanted) => wanted == c,
        }
    }
}

/// A pattern that is looked for anywhere in a text.
///
/// `*` stands for any number of characters, `?` for one character and
/// `#` for one digit. A backslash before one of them, or before a
/// backslash, takes the character as it is. Every other character stands
/// for itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pattern {
    /// The parts, with a run at each end so that the pattern can be
    /// anywhere in the text.
    parts: Vec<Part>,
}

impl Pattern {
    /// Reads a pattern. The text is the pattern as the rules compare it:
    /// in lower case.
    ///
    /// # Errors
    ///
    /// Fails for a pattern that is empty or that every text matches.
    pub(crate) fn parse(text: &str) -> Result<Pattern, String> {
        let mut parts = vec![Part::Run];
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            let part = match c {
                '*' => Part::Run,
                '?' => Part::One,
                '#' => Part::Digit,
                '\\' => Part::Is(chars.next().unwrap_or('\\')),
                other => Part::Is(other),
            };
            // Two runs in a row say the same as one.
            if !(part == Part::Run && parts.last() == Some(&Part::Run)) {
                parts.push(part);
            }
        }
        if parts.last() != Some(&Part::Run) {
            parts.push(Part::Run);
        }
        if parts.iter().all(|part| *part == Part::Run) {
            return Err(format!("the pattern `{text}` matches every text"));
        }
        Ok(Pattern { parts })
    }

    /// Returns true when the pattern is somewhere in `text`.
    pub(crate) fn found_in(&self, text: &[char]) -> bool {
        let parts = &self.parts;
        let (mut t, mut p) = (0, 0);
        // Where the last run is, and how much of the text it has taken.
        let mut run: Option<(usize, usize)> = None;
        while t < text.len() {
            match parts.get(p) {
                Some(Part::Run) => {
                    run = Some((p, t));
                    p += 1;
                }
                Some(part) if part.takes(text[t]) => {
                    t += 1;
                    p += 1;
                }
                _ => match run {
                    // The last run takes one more character.
                    Some((at, taken)) => {
                        run = Some((at, taken + 1));
                        p = at + 1;
                        t = taken + 1;
                    }
                    None => return false,
                },
            }
        }
        parts[p..].iter().all(|part| *part == Part::Run)
    }
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn found(pattern: &str, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        Pattern::parse(pattern).expect("a pattern").found_in(&chars)
    }

    #[test]
    fn a_pattern_is_looked_for_anywhere_in_the_text() {
        assert!(found("free money", "get free money now"));
        assert!(found("free money", "free money"));
        assert!(!found("free money", "free the money"));
        assert!(!found("free money", ""));
    }

    #[test]
    fn wildcards_stand_for_a_run_one_character_and_one_digit() {
        assert!(found("buy * now", "please buy this thing now"));
        assert!(found("buy * now", "buy  now"));
        assert!(!found("buy * now", "buy now"));
        assert!(found("b?d", "a bad day") && found("b?d", "a bed") && !found("b?d", "a bd"));
        assert!(found("call ###-####", "call 555-0199 today"));
        assert!(!found("call ###-####", "call 55a-0199"));
        // A run goes back when its first try fails.
        assert!(found("a*b*c", "xxaxxbxxaxxbxxcxx"));
        assert!(!found("a*b*c", "c b a"));
    }

    #[test]
    fn a_backslash_takes_a_wildcard_as_it_is() {
        assert!(found(r"100\# sure", "i am 100# sure"));
        assert!(!found(r"100\# sure", "i am 1005 sure"));
        assert!(found(r"what\?", "what?") && !found(r"what\?", "whats"));
        assert!(found(r"a\\b", r"a\b"));
    }

    #[test]
    fn a_pattern_that_says_nothing_is_refused() {
        for empty in ["", "*", "***"] {
            let error = Pattern::parse(empty).expect_err("no pattern");
            assert_eq!(error, format!("the pattern `{empty}` matches every text"));
        }
        // One character is a pattern.
        assert!(found("?", "x") && !found("?", ""));
    }
}
