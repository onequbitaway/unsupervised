//! Checks of what a client sends, before the server uses it.

use serde_json::Value;
use unsup_protocol::{ActionRequest, Routine};

use crate::error::ApiError;

/// Greatest length of a display name and of a model name, in bytes.
const NAME_MAX: usize = 64;
/// Greatest length of a request id, in bytes.
const REQUEST_ID_MAX: usize = 64;
/// Greatest length of a verb name, in bytes.
const VERB_MAX: usize = 64;
/// Greatest length of the name of a section or of a notice kind.
const LIST_NAME_MAX: usize = 64;
/// The fields of an action request.
const ACTION_FIELDS: [&str; 3] = ["id", "verb", "args"];
/// The fields of a routine.
const ROUTINE_FIELDS: [&str; 3] = ["name", "when", "then"];

/// Checks a display name: 1 to 64 bytes without control characters.
/// Returns the name without the spaces around it.
pub(crate) fn name(field: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::invalid(format!("`{field}` must not be empty.")));
    }
    if trimmed.len() > NAME_MAX {
        return Err(ApiError::invalid(format!("`{field}` has at most {NAME_MAX} bytes.")));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ApiError::invalid(format!("`{field}` must not hold control characters.")));
    }
    Ok(trimmed.to_string())
}

/// Checks an optional name, such as the model. An empty text counts as
/// absent.
pub(crate) fn optional_name(field: &str, value: Option<&str>) -> Result<Option<String>, ApiError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => name(field, text).map(Some),
    }
}

/// Reads a list of names from a query value: names with commas between
/// them. `extra` holds the characters that a name can have in addition to
/// lowercase letters, digits and `_`.
pub(crate) fn name_list(
    field: &str,
    value: &str,
    extra: &str,
    max: usize,
) -> Result<Vec<String>, ApiError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let names: Vec<String> = value.split(',').map(|part| part.trim().to_string()).collect();
    check_names(field, &names, extra, max)?;
    Ok(names)
}

/// Checks a list of names: see [`name_list`].
pub(crate) fn check_names(
    field: &str,
    names: &[String],
    extra: &str,
    max: usize,
) -> Result<(), ApiError> {
    if names.len() > max {
        return Err(ApiError::invalid(format!("`{field}` has at most {max} names.")));
    }
    for name in names {
        let good = !name.is_empty()
            && name.len() <= LIST_NAME_MAX
            && name.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || extra.contains(c)
            });
        if !good {
            return Err(ApiError::invalid(format!(
                "`{field}` holds a name that is not valid. A name has 1 to {LIST_NAME_MAX} \
                 lowercase letters, digits or `_{extra}`."
            )));
        }
    }
    Ok(())
}

/// Reads one action request from JSON and checks it. `place` names the
/// value in an error, for example `plan[2]`.
pub(crate) fn action_from_value(place: &str, value: Value) -> Result<ActionRequest, String> {
    let Value::Object(fields) = &value else {
        return Err(format!("{place} must be an object with `verb` and `args`."));
    };
    if let Some(unknown) = fields.keys().find(|key| !ACTION_FIELDS.contains(&key.as_str())) {
        return Err(format!(
            "{place} has the unknown field `{unknown}`. An action has `id`, `verb` and `args`."
        ));
    }
    let request: ActionRequest =
        serde_json::from_value(value).map_err(|error| format!("{place} is not valid: {error}."))?;
    action(place, &request)?;
    Ok(request)
}

/// Checks an action request that was read already.
pub(crate) fn action(place: &str, request: &ActionRequest) -> Result<(), String> {
    if let Some(id) = &request.id
        && (id.is_empty() || id.len() > REQUEST_ID_MAX)
    {
        return Err(format!("{place}: `id` has 1 to {REQUEST_ID_MAX} bytes."));
    }
    let verb = &request.verb;
    let good_verb = !verb.is_empty()
        && verb.len() <= VERB_MAX
        && verb.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !good_verb {
        return Err(format!(
            "{place}: `verb` has 1 to {VERB_MAX} lowercase letters, digits or `_`."
        ));
    }
    if !(request.args.is_null() || request.args.is_object()) {
        return Err(format!("{place}: `args` must be an object."));
    }
    Ok(())
}

/// Reads the body of `POST /v1/act`: one action request, or
/// `{ "plan": [...] }`. Returns the requests and true for a plan.
pub(crate) fn act_body(
    value: Value,
    max_plan: usize,
) -> Result<(Vec<ActionRequest>, bool), String> {
    let Value::Object(mut fields) = value else {
        return Err("The body must be an action request or `{ \"plan\": [...] }`.".into());
    };
    let Some(plan) = fields.remove("plan") else {
        return action_from_value("The request", Value::Object(fields)).map(|r| (vec![r], false));
    };
    if let Some(extra) = fields.keys().next() {
        return Err(format!("A plan body has only the field `plan`, not `{extra}`."));
    }
    let Value::Array(items) = plan else {
        return Err("`plan` must be a list of action requests.".into());
    };
    plan_from_values("plan", items, max_plan).map(|requests| (requests, true))
}

/// Reads the requests of a plan. `field` names the list in an error.
pub(crate) fn plan_from_values(
    field: &str,
    items: Vec<Value>,
    max_plan: usize,
) -> Result<Vec<ActionRequest>, String> {
    if items.is_empty() {
        return Err(format!("`{field}` must hold at least one action request."));
    }
    if items.len() > max_plan {
        return Err(format!("`{field}` holds at most {max_plan} action requests."));
    }
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| action_from_value(&format!("{field}[{index}]"), item))
        .collect()
}

/// Reads a list of routines. Each routine has only the fields `name`,
/// `when` and `then`, and `then` is an action request. The simulation
/// checks the rest: the verb, the arguments, the condition and the
/// number of routines. `field` names the list in an error.
pub(crate) fn routines_from_values(field: &str, items: Vec<Value>) -> Result<Vec<Routine>, String> {
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let place = format!("{field}[{index}]");
            let Value::Object(fields) = &item else {
                return Err(format!("{place} must be an object with `name`, `when` and `then`."));
            };
            if let Some(unknown) = fields.keys().find(|key| !ROUTINE_FIELDS.contains(&key.as_str()))
            {
                return Err(format!(
                    "{place} has the unknown field `{unknown}`. A routine has `name`, `when` \
                     and `then`."
                ));
            }
            if let Some(then) = fields.get("then") {
                action_from_value(&format!("{place}.then"), then.clone())?;
            }
            serde_json::from_value(item).map_err(|error| format!("{place} is not valid: {error}."))
        })
        .collect()
}
