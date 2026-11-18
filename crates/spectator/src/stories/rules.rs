//! The rules of the stories: which events belong to a kind of story, and
//! which thread an event names.

use serde_json::Value;

use crate::config::StoryKind;
use crate::wire::FeedEvent;

/// The thread of an event: the list of `threads` that matched, and the
/// values of its fields as text. Events with the same thread are one
/// story.
pub(super) type Thread = (usize, Vec<String>);

/// Writes a value of a thread field as text. A value that is a list or an
/// object names no thread.
fn thread_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// Returns true when an event belongs to a kind of story.
pub(super) fn matches(rule: &StoryKind, event: &FeedEvent) -> bool {
    if !rule.prefixes.iter().any(|prefix| event.kind.starts_with(prefix.as_str())) {
        return false;
    }
    rule.only.iter().all(|(field, allowed)| match event.data.get(field) {
        None | Some(Value::Null) => true,
        Some(value) => thread_text(value).is_some_and(|text| allowed.contains(&text)),
    })
}

/// Returns the thread that an event names: the first list of fields that
/// the data has completely.
pub(super) fn thread_of(rule: &StoryKind, event: &FeedEvent) -> Option<Thread> {
    rule.threads.iter().enumerate().find_map(|(at, fields)| {
        let values: Option<Vec<String>> =
            fields.iter().map(|field| event.data.get(field).and_then(thread_text)).collect();
        values.map(|values| (at, values))
    })
}

/// Returns the names of the slots of a headline template: the text
/// between `{` and `}`.
pub(crate) fn headline_slots(template: &str) -> Vec<&str> {
    let mut slots = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                slots.push(&after[..close]);
                rest = &after[close + 1..];
            }
            None => {
                // A brace that does not close is an unknown slot.
                slots.push(after);
                break;
            }
        }
    }
    slots
}

/// Writes a headline: `{lead}`, `{latest}` and `{count}` get their values.
pub(super) fn headline(template: &str, lead: &str, latest: &str, count: u64) -> String {
    let mut out = String::new();
    let mut rest = template;
    // One pass, so that a line with a brace in it is not read as a slot.
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        match &after[..close] {
            "lead" => out.push_str(lead),
            "latest" => out.push_str(latest),
            "count" => out.push_str(&count.to_string()),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::Config;
    use crate::facts::test_event;

    fn rule(kind: &str) -> StoryKind {
        let config = Config::default();
        config.story_kinds.into_iter().find(|rule| rule.kind == kind).expect("a built-in rule")
    }

    #[test]
    fn an_event_belongs_to_a_kind_by_the_start_of_its_name() {
        let crime = rule("crime");
        assert!(matches(&crime, &test_event(1, 0, "crime.theft_committed", 50, None, json!({}))));
        assert!(!matches(&crime, &test_event(1, 0, "crimes.x", 50, None, json!({}))));
        assert!(!matches(&crime, &test_event(1, 0, "economy.crime.x", 50, None, json!({}))));
    }

    #[test]
    fn a_rule_can_ask_for_a_value() {
        let feud = rule("feud");
        let opinion = |verdict: Value| {
            test_event(
                1,
                0,
                "society.opinion_given",
                12,
                Some(1),
                json!({ "about": 2, "verdict": verdict }),
            )
        };
        assert!(matches(&feud, &opinion(json!("condemn"))));
        assert!(!matches(&feud, &opinion(json!("praise"))));
        assert!(!matches(&feud, &opinion(json!(["condemn"]))));
        // An event without the field passes.
        assert!(matches(
            &feud,
            &test_event(1, 0, "society.contract_breached", 40, None, json!({}))
        ));
    }

    #[test]
    fn the_first_complete_list_of_fields_names_the_thread() {
        let election = rule("election");
        assert_eq!(election.threads, [vec!["election"], vec!["city", "office"], vec!["city"]]);
        let event = |data: Value| test_event(1, 0, "politics.election_decided", 85, None, data);
        assert_eq!(
            thread_of(&election, &event(json!({ "election": 12, "city": 1 }))),
            Some((0, vec!["12".to_string()]))
        );
        assert_eq!(
            thread_of(&election, &event(json!({ "city": 1, "office": "mayor" }))),
            Some((1, vec!["1".to_string(), "mayor".to_string()]))
        );
        assert_eq!(
            thread_of(&election, &event(json!({ "city": 0 }))),
            Some((2, vec!["0".to_string()]))
        );
        assert_eq!(thread_of(&election, &event(json!({ "office": "mayor" }))), None);
        assert_eq!(thread_of(&election, &event(json!({ "city": null }))), None);
        assert_eq!(thread_of(&election, &event(json!({ "city": [1] }))), None);
    }

    #[test]
    fn a_headline_takes_the_lines_of_its_story() {
        assert_eq!(headline("{lead}", "A", "B", 3), "A");
        assert_eq!(headline("{latest} ({count} events)", "A", "B", 3), "B (3 events)");
        // A line with a brace in it is text.
        assert_eq!(
            headline("{lead} / {latest}", "say {latest}", "{count}", 3),
            "say {latest} / {count}"
        );
        assert_eq!(headline_slots("{lead}: {latest} and {more"), ["lead", "latest", "more"]);
        assert!(headline_slots("no slots").is_empty());
    }
}
