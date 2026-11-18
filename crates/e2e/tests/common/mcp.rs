//! A small MCP client: JSON-RPC 2.0 over `POST /mcp`, as a client of a
//! language model sends it.

use reqwest::{RequestBuilder, StatusCode};
use serde_json::{Value, json};

use super::World;

/// The protocol version without a session.
pub const STATELESS: &str = "2026-07-28";
/// The newest protocol version with a session.
pub const SESSION: &str = "2025-11-25";

/// The answer of one HTTP request to the MCP endpoint.
#[derive(Debug)]
pub struct Answer {
    /// The HTTP status.
    pub status: StatusCode,
    /// The header `Mcp-Session-Id` of the answer.
    pub session: Option<String>,
    /// The JSON body. `null` when the answer has none.
    pub body: Value,
}

/// The result of a tool call.
#[derive(Debug)]
pub struct ToolResult {
    /// The text for the model.
    pub text: String,
    /// True when the call did not do what was asked.
    pub is_error: bool,
}

/// How the client speaks to the server.
#[derive(Clone, Debug)]
enum Mode {
    /// A session of a handshake version, with its id.
    Session(String),
    /// The version without a session: each request names the version.
    Stateless,
}

/// An MCP client of one resident.
pub struct Mcp {
    http: reqwest::Client,
    url: String,
    key: String,
    mode: Mode,
    next_id: u64,
}

impl Mcp {
    /// Opens a session: sends `initialize` and the notification
    /// `notifications/initialized`. Returns the client and the result of
    /// `initialize`.
    pub async fn initialize(world: &World, key: &str) -> (Mcp, Value) {
        let mut mcp = Mcp {
            http: world.http.clone(),
            url: world.url("/mcp"),
            key: key.to_string(),
            mode: Mode::Stateless,
            next_id: 1,
        };
        let params = json!({
            "protocolVersion": SESSION,
            "capabilities": {},
            "clientInfo": { "name": "e2e-test", "version": "1" },
        });
        let message =
            json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": params });
        let answer = mcp.send(mcp.post().json(&message)).await;
        assert_eq!(answer.status, StatusCode::OK, "initialize: {}", answer.body);
        let session = answer.session.expect("`initialize` gives a session id");
        mcp.mode = Mode::Session(session);

        let ready = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let accepted = mcp.send(mcp.session_request().json(&ready)).await;
        assert_eq!(accepted.status, StatusCode::ACCEPTED, "a notification is accepted");
        (mcp, answer.body["result"].clone())
    }

    /// Makes a client of the version without a session. It sends nothing.
    pub fn stateless(world: &World, key: &str) -> Mcp {
        Mcp {
            http: world.http.clone(),
            url: world.url("/mcp"),
            key: key.to_string(),
            mode: Mode::Stateless,
            next_id: 1,
        }
    }

    fn post(&self) -> RequestBuilder {
        self.http
            .post(&self.url)
            .bearer_auth(&self.key)
            .header("Accept", "application/json, text/event-stream")
    }

    /// A request with the headers of the session.
    fn session_request(&self) -> RequestBuilder {
        let Mode::Session(session) = &self.mode else {
            panic!("the client has no session");
        };
        self.post().header("Mcp-Session-Id", session).header("MCP-Protocol-Version", SESSION)
    }

    async fn send(&self, request: RequestBuilder) -> Answer {
        let response = request.send().await.expect("the MCP endpoint answers");
        let status = response.status();
        let session = response
            .headers()
            .get("mcp-session-id")
            .map(|value| value.to_str().expect("a session id is text").to_string());
        let bytes = response.bytes().await.expect("the body arrives");
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("the MCP endpoint answers with JSON")
        };
        Answer { status, session, body }
    }

    /// Sends one JSON-RPC request in the way of the client: with the
    /// session header, or with the version in `_meta` and the mirrored
    /// headers. Returns the HTTP answer.
    pub async fn rpc(&mut self, method: &str, mut params: Value) -> Answer {
        let id = self.next_id;
        self.next_id += 1;
        let request = match &self.mode {
            Mode::Session(_) => self.session_request(),
            Mode::Stateless => {
                params["_meta"] = json!({
                    "io.modelcontextprotocol/protocolVersion": STATELESS,
                    "io.modelcontextprotocol/clientCapabilities": {},
                });
                let request = self
                    .post()
                    .header("MCP-Protocol-Version", STATELESS)
                    .header("Mcp-Method", method);
                match params["name"].as_str() {
                    Some(tool) if method == "tools/call" => request.header("Mcp-Name", tool),
                    _ => request,
                }
            }
        };
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = self.send(request.json(&message)).await;
        if !answer.body.is_null() {
            assert_eq!(answer.body["jsonrpc"], "2.0", "{}", answer.body);
        }
        // An answer for a session that ended names no request.
        if let Some(answered) = answer.body.get("id") {
            assert_eq!(answered, id, "the answer names the request: {}", answer.body);
        }
        answer
    }

    /// Sends a request that must succeed. Returns its result.
    pub async fn result(&mut self, method: &str, params: Value) -> Value {
        let answer = self.rpc(method, params).await;
        assert_eq!(answer.status, StatusCode::OK, "{method}: {}", answer.body);
        assert!(answer.body.get("error").is_none(), "{method}: {}", answer.body);
        answer.body["result"].clone()
    }

    /// Returns the tool definitions: `tools/list`.
    pub async fn tools(&mut self) -> Vec<Value> {
        let result = self.result("tools/list", json!({})).await;
        result["tools"].as_array().expect("`tools` is a list").clone()
    }

    /// Calls a tool. Returns its text and whether it is an error.
    pub async fn call(&mut self, tool: &str, arguments: Value) -> ToolResult {
        let result =
            self.result("tools/call", json!({ "name": tool, "arguments": arguments })).await;
        let content = result["content"].as_array().expect("a tool result has content");
        assert_eq!(content.len(), 1, "one block of text: {result}");
        assert_eq!(content[0]["type"], "text");
        ToolResult {
            text: content[0]["text"].as_str().expect("the block has text").to_string(),
            is_error: result["isError"].as_bool().expect("a tool result has `isError`"),
        }
    }

    /// Calls a tool that must do what was asked. Returns its text.
    pub async fn tool(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.call(tool, arguments).await;
        assert!(!result.is_error, "the tool `{tool}` works: {}", result.text);
        result.text
    }

    /// Ends the session: `DELETE /mcp`. Returns the status.
    pub async fn end(&self) -> StatusCode {
        let Mode::Session(session) = &self.mode else {
            panic!("the client has no session");
        };
        let request =
            self.http.delete(&self.url).bearer_auth(&self.key).header("Mcp-Session-Id", session);
        self.send(request).await.status
    }
}

/// Finds `name: <whole number>` in the text of a tool result. Returns the
/// first one. A name inside braces counts too: `{cash: 5, ...}`.
pub fn number(text: &str, name: &str) -> Option<i64> {
    let label = format!("{name}: ");
    let mut rest = text;
    while let Some(at) = rest.find(&label) {
        // The label is a whole name, not the end of a longer one.
        let whole = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'));
        let after = &rest[at + label.len()..];
        if whole {
            let end =
                after.find(|c: char| !(c.is_ascii_digit() || c == '-')).unwrap_or(after.len());
            if let Ok(number) = after[..end].parse() {
                return Some(number);
            }
        }
        rest = after;
    }
    None
}

/// Returns the line of a tool result that has `needle`, without the
/// spaces at its start.
///
/// # Panics
///
/// Panics when no line has it.
pub fn line<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("a line with `{needle}` in:\n{text}"))
        .trim_start()
}

/// Returns the part of a tool result under one top-level name, for
/// example the section `self` of an observation: the line of the name
/// and the indented lines below it.
///
/// # Panics
///
/// Panics when the text has no such part.
pub fn block<'a>(text: &'a str, name: &str) -> &'a str {
    let label = format!("{name}:");
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with(&label) {
            let rest = &text[at + line.len()..];
            let below: usize = rest
                .split_inclusive('\n')
                .take_while(|line| line.starts_with(' '))
                .map(str::len)
                .sum();
            return &text[at..at + line.len() + below];
        }
        at += line.len();
    }
    panic!("a part `{name}` in:\n{text}");
}

/// Returns the report of one action in the text of a tool result: the
/// entry of the part `reports` that names the action. `None` when the
/// text has no such report.
pub fn report_of(text: &str, action: i64) -> Option<String> {
    if !text.lines().any(|line| line.starts_with("reports:")) {
        return None;
    }
    let needle = format!("action_id: {action}");
    let part = block(text, "reports");
    let mut lines = part.lines();
    // A short list is on the line of its name: `reports: [{...}]`.
    let mut entries: Vec<String> = lines
        .next()
        .filter(|first| first.len() > "reports:".len())
        .map(String::from)
        .into_iter()
        .collect();
    // Each entry of a long list starts with a dash at the first depth.
    for line in lines {
        match (line.starts_with("  - "), entries.last_mut()) {
            (false, Some(entry)) => {
                entry.push('\n');
                entry.push_str(line);
            }
            _ => entries.push(line.to_string()),
        }
    }
    entries.into_iter().find(|entry| {
        entry
            .split(&needle)
            .nth(1)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()))
    })
}
