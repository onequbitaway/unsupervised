//! Plain text for lines: one line, no hidden characters, a bounded length.
//!
//! Names and speech are text that a resident chose. The service puts them
//! into lines for viewers. A viewer still shows a line only as text.

/// Returns true for a character that has no width or that changes the
/// direction of text.
fn hidden(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}'
    )
}

/// Returns `text` as one line of plain characters, at most `max`
/// characters long. Text that was cut ends in `…`.
pub(crate) fn clean(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut chars = 0;
    let mut gap = false;
    // A bound on the work for a very long input.
    for c in text.chars().take(max.saturating_mul(8)) {
        if hidden(c) {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            gap = chars > 0;
            continue;
        }
        if gap {
            out.push(' ');
            chars += 1;
            gap = false;
        }
        out.push(c);
        chars += 1;
        if chars > max {
            break;
        }
    }
    if chars <= max {
        return out;
    }
    let mut cut: String = out.chars().take(max - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// Writes cents as money: 4500 gives `$45.00`.
pub(crate) fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.unsigned_abs();
    let digits = (cents / 100).to_string();
    let mut whole = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            whole.push(',');
        }
        whole.push(digit);
    }
    format!("{sign}${whole}.{:02}", cents % 100)
}

/// Turns the kind of an event into words: `crime.theft_committed` gives
/// `Theft committed`. The words have at most `max` characters.
pub(crate) fn humanize(kind: &str, max: usize) -> String {
    let what = kind.split_once('.').map_or(kind, |(_, what)| what);
    let words = clean(&what.replace(['_', '.'], " "), max);
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Something happened".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_is_one_plain_line() {
        assert_eq!(clean("  line one\nline two\t\tend  ", 40), "line one line two end");
        assert_eq!(
            clean("left\u{202e}thgir \u{200b}hidden\u{0007} bell", 40),
            "leftthgir hidden bell"
        );
        assert_eq!(clean("", 40), "");
        assert_eq!(clean("\u{202e}\n", 40), "");
    }

    #[test]
    fn long_text_is_cut_with_a_mark() {
        assert_eq!(clean("abcdefghij", 10), "abcdefghij");
        assert_eq!(clean("abcdefghijk", 10), "abcdefghi…");
        // The cut does not leave a space before the mark.
        assert_eq!(clean("abcdefgh ijk", 10), "abcdefgh…");
        assert_eq!(clean("你好你好你好", 4), "你好你…");
        assert_eq!(clean("abc", 0), "");
        let long = "x".repeat(100_000);
        assert_eq!(clean(&long, 5), "xxxx…");
    }

    #[test]
    fn money_has_two_decimals_and_groups() {
        assert_eq!(money(4500), "$45.00");
        assert_eq!(money(5), "$0.05");
        assert_eq!(money(123_456_789), "$1,234,567.89");
        assert_eq!(money(100_000), "$1,000.00");
        assert_eq!(money(-250), "-$2.50");
        assert_eq!(money(0), "$0.00");
    }

    #[test]
    fn a_kind_becomes_words() {
        assert_eq!(humanize("crime.theft_committed", 40), "Theft committed");
        assert_eq!(humanize("politics.law.passed", 40), "Law passed");
        assert_eq!(humanize("odd", 40), "Odd");
        assert_eq!(humanize("x.", 40), "Something happened");
        assert_eq!(humanize("crime.theft_committed", 6), "Theft…");
    }
}
