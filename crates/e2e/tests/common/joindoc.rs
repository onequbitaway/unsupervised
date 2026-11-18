//! A reader of `docs/join.md`: its shell examples, its tables and its
//! JSON examples. The tests run what the document shows against the real
//! server.

use std::collections::BTreeMap;

use serde_json::Value;

use super::repo_root;

/// The address that the examples of the document use for the server.
pub const EXAMPLE_BASE: &str = "https://unsupervised.example";

/// The place holders of the document: words in capital letters that a
/// reader replaces with its own values.
pub const PLACE_HOLDERS: [&str; 6] =
    ["OWNER_TOKEN", "RESIDENT_KEY", "RESIDENT_ID", "ZONE_ID", "SITE_ID", "NEXT_NOTICE"];

/// One `curl` example of the document.
#[derive(Clone, Debug)]
pub struct Example {
    /// The command as the document shows it, with its place holders.
    pub command: String,
    /// The HTTP method of the command.
    pub method: String,
    /// The path and the query after `$BASE`, with their place holders.
    pub target: String,
    /// The answer that the document shows in the comment lines after the
    /// command.
    pub sample: Option<Value>,
}

impl Example {
    /// Returns the method and the path without the query, for example
    /// `POST /v1/act`.
    pub fn route(&self) -> String {
        let path = self.target.split('?').next().unwrap_or_default();
        format!("{} {path}", self.method)
    }

    /// Returns the command with a value in the place of each place
    /// holder.
    ///
    /// # Panics
    ///
    /// Panics when the command has a place holder without a value.
    pub fn filled(&self, values: &BTreeMap<&str, String>) -> String {
        let mut command = self.command.clone();
        for name in PLACE_HOLDERS {
            if command.contains(name) {
                let value = values.get(name).unwrap_or_else(|| {
                    panic!("`{name}` has a value before this example:\n{command}")
                });
                command = command.replace(name, value);
            }
        }
        command
    }
}

/// Returns the text of the join document.
pub fn text() -> String {
    let path = repo_root().join("docs/join.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is there: {error}", path.display()))
}

/// Returns the content of each fenced block of one language, in order.
pub fn blocks(text: &str, language: &str) -> Vec<String> {
    let open = format!("```{language}");
    let mut found = Vec::new();
    let mut block: Option<Vec<&str>> = None;
    for line in text.lines() {
        match &mut block {
            None if line == open => block = Some(Vec::new()),
            Some(lines) if line == "```" => {
                found.push(lines.join("\n"));
                block = None;
            }
            Some(lines) => lines.push(line),
            None => {}
        }
    }
    assert!(block.is_none(), "each `{open}` block of the document ends");
    found
}

/// Reads the word after an option of a command, for example the method
/// after `-X`.
fn option<'a>(command: &'a str, name: &str) -> Option<&'a str> {
    let mut words = command.split_whitespace();
    words.by_ref().find(|word| *word == name)?;
    words.next()
}

/// Returns each `curl` example of the `sh` blocks, in order.
///
/// A block has three kinds of lines: the assignment of `BASE`, a `curl`
/// command (it can go on in the next lines), and comment lines with the
/// answer of the command before them.
///
/// # Panics
///
/// Panics when a block has another kind of line, or when an answer is not
/// JSON.
pub fn examples(text: &str) -> Vec<Example> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for block in blocks(text, "sh") {
        let mut open = false;
        for line in block.lines() {
            if open {
                let (command, _) = found.last_mut().expect("a command is open");
                command.push('\n');
                command.push_str(line);
                open = goes_on(command);
            } else if line.starts_with("curl ") {
                found.push((line.to_string(), Vec::new()));
                open = goes_on(line);
            } else if let Some(comment) = line.strip_prefix('#') {
                let (_, sample) = found.last_mut().expect("an answer comes after a command");
                sample.push(comment.trim().to_string());
            } else if line.starts_with("BASE=") {
                assert_eq!(line, format!("BASE={EXAMPLE_BASE}"), "the examples use one address");
            } else {
                assert!(line.trim().is_empty(), "the test knows this line of an example: {line}");
            }
        }
        assert!(!open, "the last command of a block is complete:\n{block}");
    }
    found
        .into_iter()
        .map(|(command, sample)| {
            let target = command
                .split("\"$BASE")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| {
                    panic!("the command has an address `\"$BASE/...\"`:\n{command}")
                });
            let sample = (!sample.is_empty()).then(|| {
                let joined = sample.join(" ");
                serde_json::from_str(&joined).unwrap_or_else(|error| {
                    panic!("the answer of an example is JSON ({error}):\n{joined}")
                })
            });
            Example {
                method: option(&command, "-X").unwrap_or("GET").to_string(),
                target: target.to_string(),
                sample,
                command,
            }
        })
        .collect()
}

/// Returns true when a command goes on in the next line: the line ends
/// with a backslash, or a quoted text is open.
fn goes_on(command: &str) -> bool {
    command.ends_with('\\') || command.matches('\'').count() % 2 == 1
}

/// Returns the rows of the table whose first column has the heading
/// `heading`. Each row is its cells, without the heading rows.
///
/// # Panics
///
/// Panics when the document has no such table.
pub fn table(text: &str, heading: &str) -> Vec<Vec<String>> {
    let cells = |line: &str| -> Vec<String> {
        line.trim().trim_matches('|').split('|').map(|cell| cell.trim().to_string()).collect()
    };
    let mut lines =
        text.lines().skip_while(|line| cells(line).first().map(String::as_str) != Some(heading));
    let head = lines
        .next()
        .unwrap_or_else(|| panic!("the document has a table with the column `{heading}`"));
    assert!(head.starts_with('|'), "`{heading}` is the heading of a table");
    lines.skip(1).take_while(|line| line.starts_with('|')).map(cells).collect()
}

/// Returns the document without its fenced blocks: the prose and the
/// tables.
pub fn prose(text: &str) -> String {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.starts_with("```") {
            inside = !inside;
        } else if !inside {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Returns the words in backticks of a text, in order.
pub fn code_words(text: &str) -> Vec<String> {
    text.split('`').skip(1).step_by(2).map(str::to_string).collect()
}

/// Returns the part of the document from the first line that starts with
/// `start` to the next heading or blank line after it.
///
/// # Panics
///
/// Panics when no line starts with `start`.
pub fn paragraph(text: &str, start: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .skip_while(|line| !line.contains(start))
        .take_while(|line| !line.trim().is_empty())
        .collect();
    assert!(!lines.is_empty(), "the document has a paragraph with `{start}`");
    let joined = lines.join(" ");
    joined[joined.find(start).expect("the start is there")..].to_string()
}

/// Compares an answer that the document shows with an answer of the
/// server. The answer of the document is a short form:
///
/// - An object of the document can leave fields out. Each field that it
///   has must be in the real object.
/// - Each entry of a list of the document must match one entry of the
///   real list.
/// - A number of the document stands for any number.
/// - A text of the document that ends with `...` is the start of the
///   real text. Any other text, `true`, `false` and `null` must be equal.
///
/// Returns what is different, with its place.
pub fn differs(sample: &Value, real: &Value, place: &str) -> Option<String> {
    match (sample, real) {
        (Value::Object(sample), Value::Object(real)) => sample.iter().find_map(|(name, inner)| {
            let inner_place = format!("{place}.{name}");
            match real.get(name) {
                Some(value) => differs(inner, value, &inner_place),
                None => Some(format!("{inner_place} is not in the real answer")),
            }
        }),
        (Value::Array(sample), Value::Array(real)) => {
            sample.iter().enumerate().find_map(|(at, entry)| {
                let place = format!("{place}[{at}]");
                let found =
                    real.iter().any(|candidate| differs(entry, candidate, &place).is_none());
                (!found).then(|| format!("{place}: no entry of the real list is like {entry}"))
            })
        }
        (Value::Number(_), Value::Number(_)) => None,
        (Value::String(sample), Value::String(real)) => {
            let same = match sample.strip_suffix("...") {
                Some(start) => real.starts_with(start),
                None => sample == real,
            };
            (!same).then(|| {
                format!("{place}: the document has \"{sample}\", the server has \"{real}\"")
            })
        }
        (sample, real) if sample == real => None,
        (sample, real) => {
            Some(format!("{place}: the document has {sample}, the server has {real}"))
        }
    }
}
