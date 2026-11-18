//! The MCP server: Streamable HTTP, written directly on JSON-RPC 2.0.
//!
//! The endpoint speaks two generations of the Model Context Protocol on
//! one path:
//!
//! - **2026-07-28** (stateless). Every request names its protocol version
//!   in `_meta` and in the `MCP-Protocol-Version` header. There is no
//!   handshake and no session. Methods: `server/discover`, `tools/list`,
//!   `tools/call`.
//! - **2025-11-25** and **2025-06-18** (handshake). The client sends
//!   `initialize`, gets an `Mcp-Session-Id` and sends it with every later
//!   message. Methods: `initialize`, `notifications/initialized`, `ping`,
//!   `tools/list`, `tools/call`. `DELETE` ends the session.
//!
//! How the client opens decides: `initialize` or a session id selects the
//! handshake rules; every other request must follow the stateless rules.
//!
//! The server answers every request with one JSON object. It never opens
//! an event stream, so `GET` answers 405.

mod rpc;
pub(crate) mod session;
mod text;
mod tools;

use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use unsup_protocol::ResidentId;
use unsup_store::Principal;

use self::rpc::{Incoming, RpcError};
use self::session::CursorKey;
use self::tools::Call;
use crate::error::ApiError;
use crate::extract::{body_bytes, is_json};
use crate::state::Gateway;

/// The stateless protocol version that this server speaks.
pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// The protocol versions with a handshake that this server speaks, newest
/// first.
pub const HANDSHAKE_VERSIONS: [&str; 2] = ["2025-11-25", "2025-06-18"];

/// Header: the session id of the handshake versions.
pub(crate) const SESSION_HEADER: &str = "mcp-session-id";
/// Header: the protocol version of the request.
const VERSION_HEADER: &str = "mcp-protocol-version";
/// Header: the JSON-RPC method of the request.
const METHOD_HEADER: &str = "mcp-method";
/// Header: the tool name of a `tools/call` request.
const NAME_HEADER: &str = "mcp-name";
/// The headers that a browser client sends, for the CORS rules.
pub(crate) const REQUEST_HEADERS: [&str; 4] =
    [SESSION_HEADER, VERSION_HEADER, METHOD_HEADER, NAME_HEADER];

const META: &str = "_meta";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";

/// Marks a header value that is Base64 text: `=?base64?<text>?=`.
const BASE64_OPEN: &str = "=?base64?";
const BASE64_CLOSE: &str = "?=";

fn server_info() -> Value {
    json!({ "name": "unsupervised", "version": env!("CARGO_PKG_VERSION") })
}

fn supported_versions() -> Vec<&'static str> {
    std::iter::once(PROTOCOL_VERSION).chain(HANDSHAKE_VERSIONS).collect()
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// The generation of the protocol that a request uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Era {
    /// A version with the `initialize` handshake and a session.
    Handshake,
    /// The stateless version.
    Stateless,
}

/// Checks what every MCP request needs before its body is read: an
/// allowed origin and a resident key. The refusal is in a box because a
/// response is a large value.
async fn admit(gateway: &Gateway, headers: &HeaderMap) -> Result<ResidentId, Box<Response>> {
    // A browser page of another origin must not reach this endpoint.
    if let Some(origin) = header(headers, header::ORIGIN.as_str())
        && !gateway.config().allows_origin(origin)
    {
        let error = RpcError::new(
            StatusCode::FORBIDDEN,
            rpc::INVALID_REQUEST,
            "This origin is not allowed to call the MCP endpoint.",
        );
        return Err(Box::new(error.response(None)));
    }
    let resident = match gateway.authenticate(headers).await {
        Ok(Principal::Resident { resident, .. }) => resident,
        Ok(Principal::Owner(_)) => {
            let error = ApiError::forbidden(
                "wrong_token_kind",
                "The MCP endpoint needs a resident key. You sent an owner token.",
            );
            return Err(Box::new(error.into_response()));
        }
        Err(error) => return Err(Box::new(error.into_response())),
    };
    Ok(resident)
}

/// `POST /mcp`: one JSON-RPC message.
pub(crate) async fn post(State(gateway): State<Gateway>, request: Request) -> Response {
    let headers = request.headers().clone();
    let resident = match admit(&gateway, &headers).await {
        Ok(resident) => resident,
        Err(response) => return *response,
    };
    if !is_json(&headers) {
        return ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Send the JSON-RPC message with the header `Content-Type: application/json`.",
        )
        .into_response();
    }
    // The server answers with JSON only, so the client must take JSON.
    let accepts_json = header(&headers, header::ACCEPT.as_str()).is_none_or(|accept| {
        accept.split(',').any(|part| {
            let mime = part.split(';').next().unwrap_or_default().trim();
            matches!(mime, "application/json" | "application/*" | "*/*")
        })
    });
    if !accepts_json {
        return ApiError::new(
            StatusCode::NOT_ACCEPTABLE,
            "not_acceptable",
            "The server answers with `application/json`. List it in the `Accept` header.",
        )
        .into_response();
    }
    let bytes = match body_bytes(request, &()).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return RpcError::bad(rpc::PARSE_ERROR, "The body is not JSON.").response(None);
    };
    let incoming = match rpc::classify(value) {
        Ok(incoming) => incoming,
        Err((error, id)) => return error.response(id.as_ref()),
    };

    let session = header(&headers, SESSION_HEADER);
    if let Some(id) = session
        && !gateway.inner.sessions.touch(id, resident, Instant::now())
    {
        // The client must start again with `initialize`.
        return RpcError::new(
            StatusCode::NOT_FOUND,
            rpc::INVALID_REQUEST,
            "The session does not exist or it ended. Send `initialize` again.",
        )
        .response(None);
    }
    match (incoming, session) {
        // Without a session, a notification or an answer has nothing to
        // belong to. The transport says: accept it.
        (Incoming::Notification { .. } | Incoming::Response, _) => {
            StatusCode::ACCEPTED.into_response()
        }
        (Incoming::Request { id, method, params }, Some(session)) => {
            let cursor = CursorKey::Session(session.to_string());
            let call = Call { gateway: &gateway, resident, cursor: &cursor };
            match in_session(&call, &headers, &method, params).await {
                Ok(result) => rpc::result(&id, result),
                Err(error) => error.response(Some(&id)),
            }
        }
        (Incoming::Request { id, method, params }, None) if method == "initialize" => {
            match initialize(&gateway, resident, &params) {
                Ok((session, result)) => {
                    let mut response = rpc::result(&id, result);
                    if let Ok(value) = HeaderValue::from_str(&session) {
                        response.headers_mut().insert(SESSION_HEADER, value);
                    }
                    response
                }
                Err(error) => error.response(Some(&id)),
            }
        }
        (Incoming::Request { id, method, params }, None) => {
            let cursor = CursorKey::Resident(resident);
            let call = Call { gateway: &gateway, resident, cursor: &cursor };
            match stateless(&call, &headers, &method, params).await {
                Ok(result) => rpc::result(&id, result),
                Err(error) => error.response(Some(&id)),
            }
        }
    }
}

/// `DELETE /mcp`: ends a session of a handshake version.
pub(crate) async fn delete(State(gateway): State<Gateway>, request: Request) -> Response {
    let headers = request.headers();
    let resident = match admit(&gateway, headers).await {
        Ok(resident) => resident,
        Err(response) => return *response,
    };
    let Some(id) = header(headers, SESSION_HEADER) else {
        return ApiError::invalid("Name the session in the header `Mcp-Session-Id`.")
            .into_response();
    };
    if gateway.inner.sessions.close(id, resident) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::not_found("The session does not exist or it ended.").into_response()
    }
}

/// Handles `initialize`: agrees on a handshake version and opens a
/// session.
fn initialize(
    gateway: &Gateway,
    resident: ResidentId,
    params: &Map<String, Value>,
) -> Result<(String, Value), RpcError> {
    let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
        // A client that knows only the stateless version reads this.
        return Err(RpcError::bad(
            rpc::INVALID_PARAMS,
            format!(
                "`initialize` needs `protocolVersion`. This server speaks {}.",
                supported_versions().join(", ")
            ),
        ));
    };
    // The same version when the server speaks it, else the newest
    // handshake version. The client decides whether it can go on.
    let version =
        HANDSHAKE_VERSIONS.into_iter().find(|v| *v == requested).unwrap_or(HANDSHAKE_VERSIONS[0]);
    let session = gateway.inner.sessions.open(resident, Instant::now()).map_err(|error| {
        tracing::error!(%error, "cannot make an MCP session id");
        RpcError::new(StatusCode::INTERNAL_SERVER_ERROR, rpc::INVALID_REQUEST, "The server failed.")
    })?;
    let result = json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": server_info(),
        "instructions": tools::INSTRUCTIONS,
    });
    Ok((session, result))
}

/// Handles a request inside a session of a handshake version.
async fn in_session(
    call: &Call<'_>,
    headers: &HeaderMap,
    method: &str,
    params: Map<String, Value>,
) -> Result<Value, RpcError> {
    if let Some(version) = header(headers, VERSION_HEADER)
        && !HANDSHAKE_VERSIONS.contains(&version)
    {
        return Err(RpcError::bad(
            rpc::INVALID_REQUEST,
            format!(
                "A session speaks {}. `MCP-Protocol-Version` names another version.",
                HANDSHAKE_VERSIONS.join(" or ")
            ),
        ));
    }
    match method {
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
        "tools/call" => call_tool(call, params, Era::Handshake).await,
        "initialize" => {
            Err(RpcError::bad(rpc::INVALID_REQUEST, "This session is initialized already."))
        }
        // The handshake versions carry a JSON-RPC error in a 200 answer.
        _ => Err(RpcError::new(StatusCode::OK, rpc::METHOD_NOT_FOUND, unknown_method(method))),
    }
}

fn unknown_method(method: &str) -> String {
    // The method is text of the client. It goes back in quotes.
    format!("The method {} does not exist.", Value::from(method))
}

/// Handles a request of the stateless version: checks the version and the
/// mirrored headers, then runs the method.
async fn stateless(
    call: &Call<'_>,
    headers: &HeaderMap,
    method: &str,
    mut params: Map<String, Value>,
) -> Result<Value, RpcError> {
    let mismatch = |message: String| RpcError::bad(rpc::HEADER_MISMATCH, message);
    let Some(header_version) = header(headers, VERSION_HEADER) else {
        return Err(mismatch(format!(
            "The header `MCP-Protocol-Version` is missing. Send {PROTOCOL_VERSION} with `_meta` \
             in every request, or start a session with `initialize`."
        )));
    };
    if HANDSHAKE_VERSIONS.contains(&header_version) {
        return Err(RpcError::bad(
            rpc::INVALID_REQUEST,
            format!(
                "Version {header_version} needs a session. Send `initialize` first, then send \
                 the `Mcp-Session-Id` header with every request."
            ),
        ));
    }
    let meta = match params.remove(META) {
        Some(Value::Object(meta)) => meta,
        _ => Map::new(),
    };
    let Some(body_version) = meta.get(META_VERSION).and_then(Value::as_str) else {
        return Err(RpcError::bad(
            rpc::INVALID_PARAMS,
            format!("`params._meta` needs `{META_VERSION}`."),
        ));
    };
    if !meta.get(META_CAPABILITIES).is_some_and(Value::is_object) {
        return Err(RpcError::bad(
            rpc::INVALID_PARAMS,
            format!("`params._meta` needs `{META_CAPABILITIES}`."),
        ));
    }
    if header_version != body_version {
        return Err(mismatch(
            "The header `MCP-Protocol-Version` does not match the version in `_meta`.".into(),
        ));
    }
    if body_version != PROTOCOL_VERSION {
        return Err(RpcError::bad(rpc::UNSUPPORTED_VERSION, "Unsupported protocol version")
            .with_data(json!({ "supported": supported_versions(), "requested": body_version })));
    }
    match header(headers, METHOD_HEADER) {
        Some(mirrored) if mirrored == method => {}
        Some(_) => {
            return Err(mismatch("The header `Mcp-Method` does not match `method`.".into()));
        }
        None => return Err(mismatch("The header `Mcp-Method` is missing.".into())),
    }

    let mut result = match method {
        "server/discover" => json!({
            "supportedVersions": supported_versions(),
            "capabilities": { "tools": {} },
            "instructions": tools::INSTRUCTIONS,
            "ttlMs": call.gateway.config().mcp.list_ttl_ms,
            "cacheScope": "public",
        }),
        "tools/list" => json!({
            "tools": tools::definitions(),
            "ttlMs": call.gateway.config().mcp.list_ttl_ms,
            "cacheScope": "public",
        }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            match header(headers, NAME_HEADER).map(decode_header) {
                Some(Some(mirrored)) if mirrored == name => {}
                Some(_) => {
                    return Err(mismatch(
                        "The header `Mcp-Name` does not match `params.name`.".into(),
                    ));
                }
                None => return Err(mismatch("The header `Mcp-Name` is missing.".into())),
            }
            call_tool(call, params, Era::Stateless).await?
        }
        _ => {
            return Err(RpcError::new(
                StatusCode::NOT_FOUND,
                rpc::METHOD_NOT_FOUND,
                unknown_method(method),
            ));
        }
    };
    result["resultType"] = json!("complete");
    result[META] = json!({ META_SERVER: server_info() });
    Ok(result)
}

/// Runs `tools/call`.
async fn call_tool(
    call: &Call<'_>,
    mut params: Map<String, Value>,
    era: Era,
) -> Result<Value, RpcError> {
    let invalid = |message: String| {
        // The stateless version puts a broken request in a 400 answer.
        let status = match era {
            Era::Stateless => StatusCode::BAD_REQUEST,
            Era::Handshake => StatusCode::OK,
        };
        RpcError::new(status, rpc::INVALID_PARAMS, message)
    };
    let Some(Value::String(name)) = params.remove("name") else {
        return Err(invalid("`params.name` must be the name of a tool.".into()));
    };
    let arguments = match params.remove("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(arguments)) => arguments,
        Some(_) => return Err(invalid("`params.arguments` must be an object.".into())),
    };
    let Some(result) = call.run(&name, arguments).await else {
        return Err(invalid(format!("Unknown tool: {}", Value::from(name))));
    };
    Ok(json!({
        "content": [{ "type": "text", "text": result.text }],
        "isError": result.is_error,
    }))
}

/// Reads a mirrored header value. A value of the form `=?base64?...?=`
/// is Base64 text. Returns `None` when that text is broken.
fn decode_header(value: &str) -> Option<String> {
    match value.strip_prefix(BASE64_OPEN).and_then(|rest| rest.strip_suffix(BASE64_CLOSE)) {
        Some(encoded) => String::from_utf8(decode_base64(encoded)?).ok(),
        None => Some(value.to_string()),
    }
}

/// Decodes standard Base64 with padding.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let digits = text.trim_end_matches('=');
    if !text.len().is_multiple_of(4) || text.len() - digits.len() > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in digits.bytes() {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            // The mask keeps one byte.
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}
