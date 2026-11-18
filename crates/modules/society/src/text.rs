//! Checks for text and names that a resident wrote.

use unsup_kernel::{ActionError, ResidentId, Untrusted, Value};

/// Checks text that a resident wrote: it is not blank, it has at most `max`
/// bytes and it has no control character except a line break.
pub(crate) fn check_text(text: &str, max: i64, what: &str) -> Result<(), ActionError> {
    if text.trim().is_empty() {
        return Err(ActionError::invalid_args(format!("The {what} is empty.")));
    }
    if i64::try_from(text.len()).unwrap_or(i64::MAX) > max {
        return Err(ActionError::invalid_args(format!("The {what} has more than {max} bytes.")));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ActionError::invalid_args(format!("The {what} has a control character.")));
    }
    Ok(())
}

/// Wraps text of a resident for a notice, an event or an observation.
pub(crate) fn untrusted(from: ResidentId, text: &str) -> Value {
    Untrusted::new(from, text).to_value()
}

/// Returns true for a name made of lower-case letters, digits and `_` that
/// starts with a letter and has at most `max` bytes. Such a name cannot
/// carry a sentence, so it is safe in a plain string field.
pub(crate) fn is_ident(name: &str, max: usize) -> bool {
    !name.is_empty()
        && name.len() <= max
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Returns true for the kind of an event: `<module>.<what_happened>`, two
/// identifiers with a dot between them, at most `max` bytes in all.
pub(crate) fn is_event_kind(kind: &str, max: usize) -> bool {
    match kind.split_once('.') {
        Some((module, what)) => kind.len() <= max && is_ident(module, max) && is_ident(what, max),
        None => false,
    }
}

/// Returns true for a short value without spaces: letters, digits and the
/// characters `_`, `.`, `-` and `:`. Event data use such values for names
/// of items, kinds and causes.
pub(crate) fn is_token(value: &str, max: usize) -> bool {
    value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_must_have_content_and_fit() {
        assert!(check_text("hello", 5, "text").is_ok());
        assert!(check_text("line\nbreak", 20, "text").is_ok());
        let error = |text: &str, max| check_text(text, max, "text").unwrap_err().message;
        assert_eq!(error("", 5), "The text is empty.");
        assert_eq!(error(" \n ", 5), "The text is empty.");
        assert_eq!(error("hello!", 5), "The text has more than 5 bytes.");
        assert_eq!(error("a\u{7}b", 5), "The text has a control character.");
        // The limit counts bytes, not characters.
        assert_eq!(error("ééé", 5), "The text has more than 5 bytes.");
    }

    #[test]
    fn identifiers_kinds_and_tokens() {
        assert!(is_ident("treasurer_2", 24));
        for bad in ["", "Treasurer", "2nd", "a b", "a.b", "é"] {
            assert!(!is_ident(bad, 24), "{bad}");
        }
        assert!(!is_ident("abcdef", 5));
        assert!(is_event_kind("crime.arrest_made", 17));
        assert!(!is_event_kind("crime.arrest_made", 16));
        for bad in ["crime", "crime.", ".made", "crime.arrest made", "a.b.c", "Crime.made"] {
            assert!(!is_event_kind(bad, 64), "{bad}");
        }
        assert!(is_token("bread-2.x:y_Z", 20));
        assert!(is_token("", 20));
        assert!(!is_token("two words", 20));
        assert!(!is_token("toolong", 3));
    }
}
