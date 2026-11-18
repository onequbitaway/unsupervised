//! Finds every text in the arguments of an action request.

use serde_json::Value;
use unsup_protocol::ActionRequest;

use super::rules::ModerationConfig;

/// The built-in verb whose arguments hold other action requests.
const SET_ROUTINES: &str = "set_routines";

/// One text in the arguments of an action request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found<'a> {
    /// The verb whose argument the text is. For a text in a routine it is
    /// the verb of the routine.
    pub(crate) verb: &'a str,
    /// Where the text is, for an error message: `args.text`.
    pub(crate) path: String,
    /// The text.
    pub(crate) text: &'a str,
    /// True for text that other residents or viewers get.
    pub(crate) speech: bool,
}

/// Returns every text in the arguments of a request: each string value
/// and each name of a field inside a value. The names of the arguments
/// themselves are not text of the resident: the verb defines them.
pub(crate) fn collect<'a>(config: &ModerationConfig, request: &'a ActionRequest) -> Vec<Found<'a>> {
    let mut out = Vec::new();
    arguments(config, &request.verb, &request.args, "args", &mut out);
    out
}

/// Walks the arguments of one verb.
fn arguments<'a>(
    config: &ModerationConfig,
    verb: &'a str,
    args: &'a Value,
    path: &str,
    out: &mut Vec<Found<'a>>,
) {
    let Value::Object(fields) = args else {
        return;
    };
    for (name, value) in fields {
        let here = format!("{path}.{name}");
        if verb == SET_ROUTINES
            && name == "routines"
            && let Value::Array(routines) = value
        {
            for (index, routine) in routines.iter().enumerate() {
                self::routine(config, verb, routine, &format!("{here}[{index}]"), out);
            }
        } else {
            let speech = config.is_speech(verb, name);
            walk(verb, value, &here, speech, out);
        }
    }
}

/// Walks one routine. Its `then` is an action request of its own: the
/// text in it counts for the verb of that request.
fn routine<'a>(
    config: &ModerationConfig,
    verb: &'a str,
    routine: &'a Value,
    path: &str,
    out: &mut Vec<Found<'a>>,
) {
    let Value::Object(fields) = routine else {
        return walk(verb, routine, path, false, out);
    };
    for (name, value) in fields {
        let here = format!("{path}.{name}");
        match (name.as_str(), value) {
            ("then", Value::Object(then)) => {
                let inner = then.get("verb").and_then(Value::as_str).unwrap_or_default();
                if let Some(args) = then.get("args") {
                    arguments(config, inner, args, &format!("{here}.args"), out);
                }
            }
            _ => walk(verb, value, &here, false, out),
        }
    }
}

/// Collects the strings of a value and the names of the fields in it.
fn walk<'a>(verb: &'a str, value: &'a Value, path: &str, speech: bool, out: &mut Vec<Found<'a>>) {
    match value {
        Value::String(text) => out.push(Found { verb, path: path.to_string(), text, speech }),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(verb, item, &format!("{path}[{index}]"), speech, out);
            }
        }
        Value::Object(fields) => {
            for (name, item) in fields {
                // The name is text of the resident too. The message does
                // not repeat it.
                out.push(Found {
                    verb,
                    path: format!("a field name in {path}"),
                    text: name,
                    speech,
                });
                walk(verb, item, &format!("{path}.{name}"), speech, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use unsup_protocol::ActionRequest;

    use super::{Found, collect};
    use crate::config::GatewayConfig;

    fn found<'a>(request: &'a ActionRequest, toml: &str) -> Vec<(String, &'a str, &'a str, bool)> {
        let config = GatewayConfig::from_toml(toml).expect("a configuration").moderation;
        collect(&config, request)
            .into_iter()
            .map(|Found { verb, path, text, speech }| (path, verb, text, speech))
            .collect()
    }

    const RULES: &str = "[moderation]\ntext_verbs = [\"org_create\"]\ntext_args = [\"text\"]";

    #[test]
    fn every_string_is_found_and_speech_is_marked() {
        let say = ActionRequest::new("say", json!({ "text": "hello", "volume": 3 }));
        assert_eq!(found(&say, RULES), [("args.text".to_string(), "say", "hello", true)]);

        // A verb of the list: each of its strings is speech.
        let org = ActionRequest::new("org_create", json!({ "name": "The Guild", "open": true }));
        assert_eq!(
            found(&org, RULES),
            [("args.name".to_string(), "org_create", "The Guild", true)]
        );

        // The name of an item is text, but not speech.
        let buy = ActionRequest::new("buy", json!({ "item": "bread", "qty": 2 }));
        assert_eq!(found(&buy, RULES), [("args.item".to_string(), "buy", "bread", false)]);

        // No arguments, or arguments that are not an object: no text.
        assert_eq!(found(&ActionRequest::new("wait", json!(null)), RULES), []);
        assert_eq!(found(&ActionRequest::new("wait", json!({ "ticks": 5 })), RULES), []);
    }

    #[test]
    fn nested_values_and_the_names_of_their_fields_are_text() {
        let offer = ActionRequest::new(
            "contract_offer",
            json!({ "clauses": [{ "event": "crime.arrest_made", "fields": { "who": "ada" } }] }),
        );
        assert_eq!(
            found(&offer, RULES),
            [
                ("a field name in args.clauses[0]".to_string(), "contract_offer", "event", false),
                ("args.clauses[0].event".to_string(), "contract_offer", "crime.arrest_made", false),
                ("a field name in args.clauses[0]".to_string(), "contract_offer", "fields", false),
                (
                    "a field name in args.clauses[0].fields".to_string(),
                    "contract_offer",
                    "who",
                    false
                ),
                ("args.clauses[0].fields.who".to_string(), "contract_offer", "ada", false),
            ]
        );
    }

    #[test]
    fn the_text_of_a_routine_counts_for_the_verb_of_the_routine() {
        let routines = ActionRequest::new(
            "set_routines",
            json!({ "routines": [
                { "name": "eat", "when": { "type": "attr_below", "attr": "need.food", "value": 300 },
                  "then": { "verb": "eat", "args": {} } },
                { "name": "greet", "when": { "type": "always" },
                  "then": { "id": "g1", "verb": "say", "args": { "text": "hi all" } } },
            ] }),
        );
        let all = found(&routines, RULES);
        let speech: Vec<_> = all.iter().filter(|entry| entry.3).collect();
        assert_eq!(
            speech,
            [&("args.routines[1].then.args.text".to_string(), "say", "hi all", true)]
        );
        // The label of a routine, and the names and the values in its
        // condition, are text of the resident without being speech.
        let other: Vec<&str> = all.iter().filter(|entry| !entry.3).map(|entry| entry.2).collect();
        assert_eq!(
            other,
            ["eat", "attr", "need.food", "type", "attr_below", "value", "greet", "type", "always"]
        );
    }
}
