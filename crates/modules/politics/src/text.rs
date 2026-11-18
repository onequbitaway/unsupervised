//! Checks for text that a resident wrote.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_must_have_content_and_fit() {
        assert!(check_text("Lower taxes", 11, "platform").is_ok());
        assert!(check_text("one\ntwo", 20, "platform").is_ok());
        let error = |text: &str, max| check_text(text, max, "platform").unwrap_err().message;
        assert_eq!(error("", 5), "The platform is empty.");
        assert_eq!(error(" \n ", 5), "The platform is empty.");
        assert_eq!(error("abcdef", 5), "The platform has more than 5 bytes.");
        assert_eq!(error("a\u{7}b", 5), "The platform has a control character.");
        // The limit counts bytes, not characters.
        assert_eq!(error("ééé", 5), "The platform has more than 5 bytes.");
    }

    #[test]
    fn wrapped_text_is_marked() {
        let value = untrusted(ResidentId(7), "Vote for me");
        assert_eq!(
            value,
            unsup_kernel::serde_json::json!({ "untrusted": true, "from": 7, "text": "Vote for me" })
        );
    }
}
