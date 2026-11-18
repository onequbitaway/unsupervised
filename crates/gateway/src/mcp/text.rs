//! Tool results as compact text.
//!
//! A language model reads the result of a tool as text and pays for every
//! token. Pretty JSON spends many tokens on quotes and brackets; one long
//! line of JSON is hard to read. So the server writes an outline: one
//! `key: value` on each line, with indentation for depth, and small
//! values in braces on one line.
//!
//! Text that a resident wrote stays inside quotes, with every line break
//! and quote escaped, and inside its object with `untrusted: true`. It can
//! never look like a line that the server wrote.

use std::fmt::Write;

use serde_json::Value;
use unsup_protocol::Observation;

/// Spaces for one level of depth.
const INDENT: usize = 2;

/// Returns true when a string can stand without quotes: it is a plain
/// word that cannot be taken for a number, a keyword or structure.
fn is_plain(text: &str) -> bool {
    let mut chars = text.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
        && !matches!(text, "true" | "false" | "null")
}

/// Writes a string: plain when it is a simple word, else as a JSON
/// string.
fn write_text(text: &str, out: &mut String) {
    if is_plain(text) {
        out.push_str(text);
    } else {
        // A string always turns into JSON.
        out.push_str(&serde_json::to_string(text).unwrap_or_default());
    }
}

/// Writes a value on one line.
fn write_inline(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => write_text(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_inline(item, out);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            for (index, (key, item)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_text(key, out);
                out.push_str(": ");
                write_inline(item, out);
            }
            out.push('}');
        }
        // Null, booleans and numbers look the same as in JSON.
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

fn inline(value: &Value) -> String {
    let mut out = String::new();
    write_inline(value, &mut out);
    out
}

fn is_leaf(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        _ => true,
    }
}

/// Writes `label` and a value. The value goes on the same line when it
/// fits in `width`, else on indented lines below.
fn write_entry(label: &str, value: &Value, depth: usize, width: usize, out: &mut String) {
    let pad = depth * INDENT;
    let one_line = inline(value);
    if is_leaf(value) || pad + label.len() + 1 + one_line.len() <= width {
        let _ = writeln!(out, "{:pad$}{label} {one_line}", "");
        return;
    }
    let _ = writeln!(out, "{:pad$}{label}", "");
    write_block(value, depth + 1, width, out);
}

/// Writes one item of a list after a dash. An item that does not fit on
/// the line continues below, in line with its first part.
fn write_item(item: &Value, depth: usize, width: usize, out: &mut String) {
    let pad = depth * INDENT;
    let one_line = inline(item);
    if is_leaf(item) || pad + INDENT + one_line.len() <= width {
        let _ = writeln!(out, "{:pad$}- {one_line}", "");
        return;
    }
    let mut block = String::new();
    write_block(item, depth + 1, width, &mut block);
    // The block starts one level deeper. The dash takes that place.
    let _ = write!(out, "{:pad$}- {}", "", &block[pad + INDENT..]);
}

/// Writes the parts of a list or an object, one on each line.
fn write_block(value: &Value, depth: usize, width: usize, out: &mut String) {
    match value {
        Value::Object(fields) => {
            for (key, item) in fields {
                let mut label = String::new();
                write_text(key, &mut label);
                label.push(':');
                write_entry(&label, item, depth, width, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                write_item(item, depth, width, out);
            }
        }
        other => {
            let _ = writeln!(out, "{:pad$}{}", "", inline(other), pad = depth * INDENT);
        }
    }
}

/// Writes a value as an outline. Lines are at most `width` characters
/// long where the data permits.
pub(crate) fn outline(value: &Value, width: usize) -> String {
    let mut out = String::new();
    write_block(value, 0, width, &mut out);
    out
}

/// Writes one labelled value as an outline, for example `ack: {...}`.
pub(crate) fn entry(label: &str, value: &Value, width: usize) -> String {
    let mut out = String::new();
    write_entry(&format!("{label}:"), value, 0, width, &mut out);
    out
}

/// Writes an observation: the time, each section, then the news.
pub(crate) fn observation(observation: &Observation, width: usize) -> String {
    let clock = &observation.clock;
    let mut out = format!(
        "time: year {}, day {}, {:02}:{:02} (tick {})\n",
        clock.year, clock.day, clock.hour, clock.minute, observation.tick.0
    );
    for (name, section) in &observation.sections {
        // Reports are news. They go below, and only when there are some.
        if name != "reports" {
            write_entry(&format!("{name}:"), section, 0, width, &mut out);
        }
    }
    let reports = observation.sections.get("reports").filter(|reports| !is_leaf(reports));
    if observation.notices.is_empty() && reports.is_none() {
        out.push_str("news: none\n");
        return out;
    }
    if let Some(reports) = reports {
        write_entry("reports:", reports, 0, width, &mut out);
    }
    if !observation.notices.is_empty() {
        // The notices are plain data, so they always turn into JSON.
        let notices = serde_json::to_value(&observation.notices).unwrap_or_default();
        write_entry("notices:", &notices, 0, width, &mut out);
    }
    out
}
