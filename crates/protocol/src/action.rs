//! Actions: requests, acknowledgements, reports and errors.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::Tick;

/// One action that a resident asks for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionRequest {
    /// Idempotency key chosen by the client, at most 64 bytes. The server
    /// remembers the last 32 accepted keys of a resident and returns the first
    /// answer when a key is sent again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Name of the verb, for example `wait`.
    pub verb: String,
    /// Arguments of the verb. The catalogue has the schema of each verb.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub args: Value,
}

impl ActionRequest {
    /// Makes a request without an idempotency key.
    #[must_use]
    pub fn new(verb: impl Into<String>, args: Value) -> ActionRequest {
        ActionRequest { id: None, verb: verb.into(), args }
    }

    /// Sets the idempotency key.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> ActionRequest {
        self.id = Some(id.into());
        self
    }
}

/// The immediate answer to an [`ActionRequest`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionAck {
    /// The idempotency key of the request, if it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Id that the server gave to this action. The [`ActionReport`] carries
    /// the same id.
    pub action_id: u64,
    /// Name of the verb.
    pub verb: String,
    /// What happened to the request.
    pub status: ActionStatus,
}

/// What happened to an action request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ActionStatus {
    /// The action finished at once. No report follows.
    Done {
        /// Result of the action.
        #[serde(default, skip_serializing_if = "Value::is_null")]
        result: Value,
    },
    /// The action started and the resident is busy. A report follows.
    Started {
        /// Tick of the resident's zone at which the action ends.
        ends_at: Tick,
        /// Duration in ticks.
        ticks: u32,
    },
    /// The action waits for an answer from the global realm or from another
    /// party. A report follows.
    Pending,
    /// The resident is busy. The request is in the queue. A report follows
    /// when the action ends.
    Queued {
        /// Position in the queue, from 1.
        position: u32,
    },
    /// The request was refused. Nothing changed and no report follows.
    Rejected {
        /// Why the request was refused.
        error: ActionError,
    },
}

impl ActionStatus {
    /// Returns true unless the request was rejected.
    #[must_use]
    pub const fn is_accepted(&self) -> bool {
        !matches!(self, ActionStatus::Rejected { .. })
    }
}

/// The result of an action that ended after its acknowledgement. The server
/// writes it to the resident's inbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionReport {
    /// Place of the report in the sequence that the notices and the reports
    /// of one resident share. A client that keeps the highest number that it
    /// saw can drop a repeat and can order a report against a notice.
    #[serde(default)]
    pub seq: u64,
    /// Id of the action, as in the [`ActionAck`].
    pub action_id: u64,
    /// The idempotency key of the request, if it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Name of the verb.
    pub verb: String,
    /// Tick of the resident's zone at which the action ended.
    pub tick: Tick,
    /// True when the action succeeded.
    pub ok: bool,
    /// Result of a successful action.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    /// Error of a failed action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ActionError>,
}

/// Why an action was refused or failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionError {
    /// Machine-readable class of the error.
    pub code: ErrorCode,
    /// One sentence for the agent.
    pub message: String,
    /// Optional structured details.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl ActionError {
    /// Makes an error without details.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> ActionError {
        ActionError { code, message: message.into(), details: Value::Null }
    }

    /// Adds structured details.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> ActionError {
        self.details = details;
        self
    }

    /// The arguments do not match the schema of the verb.
    #[must_use]
    pub fn invalid_args(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::InvalidArgs, message)
    }

    /// The resident is not at the place that the action needs.
    #[must_use]
    pub fn not_here(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::NotHere, message)
    }

    /// The thing that the action names does not exist.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::NotFound, message)
    }

    /// The resident does not have the right to do this.
    #[must_use]
    pub fn not_allowed(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::NotAllowed, message)
    }

    /// The resident does not have enough money, goods or another quantity.
    #[must_use]
    pub fn insufficient(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::Insufficient, message)
    }

    /// The action conflicts with the current state.
    #[must_use]
    pub fn conflict(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::Conflict, message)
    }

    /// The server failed. This is a bug.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> ActionError {
        ActionError::new(ErrorCode::Internal, message)
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ActionError {}

/// Machine-readable class of an [`ActionError`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No verb has this name.
    UnknownVerb,
    /// The arguments do not match the schema of the verb.
    InvalidArgs,
    /// A condition of the resident (for example `jailed`) forbids the verb.
    Blocked,
    /// The queue of planned actions is full.
    QueueFull,
    /// Too many requests in a short time.
    RateLimited,
    /// The resident is not at the place that the action needs.
    NotHere,
    /// The thing that the action names does not exist.
    NotFound,
    /// The resident does not have the right to do this.
    NotAllowed,
    /// Not enough money, goods or another quantity.
    Insufficient,
    /// The action conflicts with the current state.
    Conflict,
    /// The action was stopped before it ended.
    Interrupted,
    /// The action waited too long for an answer.
    Timeout,
    /// The server failed. This is a bug.
    Internal,
}

impl ErrorCode {
    /// Every error code, in the order of the protocol document.
    pub const ALL: [ErrorCode; 13] = [
        ErrorCode::UnknownVerb,
        ErrorCode::InvalidArgs,
        ErrorCode::Blocked,
        ErrorCode::QueueFull,
        ErrorCode::RateLimited,
        ErrorCode::NotHere,
        ErrorCode::NotFound,
        ErrorCode::NotAllowed,
        ErrorCode::Insufficient,
        ErrorCode::Conflict,
        ErrorCode::Interrupted,
        ErrorCode::Timeout,
        ErrorCode::Internal,
    ];

    /// Returns the wire name of the code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::UnknownVerb => "unknown_verb",
            ErrorCode::InvalidArgs => "invalid_args",
            ErrorCode::Blocked => "blocked",
            ErrorCode::QueueFull => "queue_full",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::NotHere => "not_here",
            ErrorCode::NotFound => "not_found",
            ErrorCode::NotAllowed => "not_allowed",
            ErrorCode::Insufficient => "insufficient",
            ErrorCode::Conflict => "conflict",
            ErrorCode::Interrupted => "interrupted",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
