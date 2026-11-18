//! JSON-RPC 2.0 messages for the MCP endpoint.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

/// The text is not JSON.
pub(crate) const PARSE_ERROR: i64 = -32700;
/// The JSON is not a JSON-RPC message that this endpoint takes.
pub(crate) const INVALID_REQUEST: i64 = -32600;
/// The method does not exist.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
/// The parameters of the method are not valid.
pub(crate) const INVALID_PARAMS: i64 = -32602;
/// MCP: a request header does not match the body, or is missing.
pub(crate) const HEADER_MISMATCH: i64 = -32020;
/// MCP: the server does not speak the protocol version of the request.
pub(crate) const UNSUPPORTED_VERSION: i64 = -32022;

/// A JSON-RPC error with the HTTP status that carries it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RpcError {
    pub(crate) status: StatusCode,
    pub(crate) code: i64,
    pub(crate) message: String,
    pub(crate) data: Option<Value>,
}

impl RpcError {
    pub(crate) fn new(status: StatusCode, code: i64, message: impl Into<String>) -> RpcError {
        RpcError { status, code, message: message.into(), data: None }
    }

    /// 400 with a JSON-RPC error.
    pub(crate) fn bad(code: i64, message: impl Into<String>) -> RpcError {
        RpcError::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub(crate) fn with_data(mut self, data: Value) -> RpcError {
        self.data = Some(data);
        self
    }

    /// Builds the HTTP answer. `id` is absent when the request had none
    /// that could be read.
    pub(crate) fn response(self, id: Option<&Value>) -> Response {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(data) = self.data {
            error["data"] = data;
        }
        let mut body = json!({ "jsonrpc": "2.0", "error": error });
        if let Some(id) = id {
            body["id"] = id.clone();
        }
        (self.status, Json(body)).into_response()
    }
}

/// Builds the answer to a request that succeeded.
pub(crate) fn result(id: &Value, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

/// One message from a client.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Incoming {
    /// A request: it has an id and gets an answer.
    Request { id: Value, method: String, params: Map<String, Value> },
    /// A notification: no id, no answer.
    Notification { method: String },
    /// An answer of the client to a request of the server.
    Response,
}

/// Reads one JSON-RPC message. On failure returns the error and the id of
/// the message, when it could be read.
pub(crate) fn classify(value: Value) -> Result<Incoming, (RpcError, Option<Value>)> {
    let invalid = |message: &str, id: Option<Value>| (RpcError::bad(INVALID_REQUEST, message), id);
    let mut fields = match value {
        Value::Object(fields) => fields,
        Value::Array(_) => {
            return Err(invalid("Batches are not supported. Send one message in each POST.", None));
        }
        _ => return Err(invalid("A JSON-RPC message is an object.", None)),
    };
    // A request id is a string or a whole number, never null.
    let id = fields.remove("id");
    let good_id = id.as_ref().is_none_or(|id| id.is_string() || id.is_i64() || id.is_u64());
    let id = id.filter(|_| good_id);
    if fields.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("`jsonrpc` must be \"2.0\".", id));
    }
    if !good_id {
        return Err(invalid("`id` must be a string or a whole number.", None));
    }
    let method = match fields.remove("method") {
        Some(Value::String(method)) => method,
        Some(_) => return Err(invalid("`method` must be a string.", id)),
        None if fields.contains_key("result") || fields.contains_key("error") => {
            return Ok(Incoming::Response);
        }
        None => return Err(invalid("The message has no `method`.", id)),
    };
    let Some(id) = id else {
        return Ok(Incoming::Notification { method });
    };
    let params = match fields.remove("params") {
        None => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => {
            return Err((RpcError::bad(INVALID_PARAMS, "`params` must be an object."), Some(id)));
        }
    };
    Ok(Incoming::Request { id, method, params })
}
