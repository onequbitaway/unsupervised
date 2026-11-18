//! A request of an agent that did not get a usable answer.

use std::error::Error;
use std::fmt;
use std::io;

use serde_json::Value;

/// Code of a failure that shows a limit of the machine: the system has no
/// free local port for one more outgoing connection.
pub const NO_LOCAL_PORT: &str = "no_local_port";

/// Code of a failure that shows a limit of the machine: the process or the
/// system cannot open one more file.
pub const TOO_MANY_OPEN_FILES: &str = "too_many_open_files";

/// Error number of "too many open files in the process" (`EMFILE`). Linux
/// and macOS use the same number.
const EMFILE: i32 = 24;

/// Error number of "too many open files in the system" (`ENFILE`).
const ENFILE: i32 = 23;

/// Returns the code of the machine limit that an error of the operating
/// system shows, if it shows one.
fn limit_of(error: &io::Error) -> Option<&'static str> {
    if error.kind() == io::ErrorKind::AddrNotAvailable {
        Some(NO_LOCAL_PORT)
    } else if matches!(error.raw_os_error(), Some(EMFILE | ENFILE)) {
        Some(TOO_MANY_OPEN_FILES)
    } else {
        None
    }
}

/// Looks through an error and its causes for an error of the operating
/// system that shows a machine limit. Returns the code of the limit.
#[must_use]
pub fn machine_limit_in(error: &(dyn Error + 'static)) -> Option<&'static str> {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(io) = error.downcast_ref::<io::Error>() {
            if let Some(code) = limit_of(io) {
                return Some(code);
            }
            // An error that wraps another one gives the cause of the inner
            // error as its source, not the inner error itself.
            if let Some(code) = io.get_ref().and_then(|inner| machine_limit_in(inner)) {
                return Some(code);
            }
        }
        next = error.source();
    }
    None
}

/// HTTP status of an answer that a limit of the server refused.
const TOO_MANY_REQUESTS: u16 = 429;

/// Error codes of the server for a request that one of its limits refused
/// (`docs/design/protocol.md`, section 2). An `error` frame on a WebSocket
/// has the code and no status.
const LIMIT_CODES: [&str; 2] = ["rate_limited", "too_many_connections"];

/// Error codes of the server for a key that it does not accept, or whose
/// resident left the world.
const KEY_CODES: [&str; 3] = ["unauthorized", "invalid_token", "resident_gone"];

/// Why a request failed. `code` is the name under which the report counts
/// the failure. It never holds a key or a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestError {
    /// Short name: the `error.code` of the server (`overloaded`,
    /// `rate_limited`, ...), `http_<status>` for an answer without a
    /// code, the kind of transport failure (`connect`, `timeout`,
    /// `transport`, `decode`, `closed`), or a limit of this machine
    /// ([`NO_LOCAL_PORT`], [`TOO_MANY_OPEN_FILES`]).
    pub code: String,
    /// The HTTP status, when the server answered.
    pub status: Option<u16>,
    /// True when the server answered with a JSON error of the API. False
    /// for an answer of the web server itself.
    pub from_api: bool,
    /// One sentence for the operator.
    pub message: String,
    /// Seconds that the server asks the client to wait, from the header
    /// `Retry-After`.
    pub retry_after_secs: Option<u64>,
}

impl RequestError {
    /// Makes an error of the client side: no answer came.
    #[must_use]
    pub fn local(code: &str, message: impl Into<String>) -> RequestError {
        RequestError {
            code: code.to_string(),
            status: None,
            from_api: false,
            message: message.into(),
            retry_after_secs: None,
        }
    }

    /// Makes an error from the error object of the API: its code and its
    /// message. An `error` frame on a WebSocket has no status.
    #[must_use]
    pub fn api(code: &str, message: impl Into<String>, status: Option<u16>) -> RequestError {
        RequestError {
            code: code.to_string(),
            status,
            from_api: true,
            message: message.into(),
            retry_after_secs: None,
        }
    }

    /// Makes an error from an answer with a status that is not a success.
    /// The body is the error object of the API when it parses as one.
    /// `retry_after_secs` is the value of the header `Retry-After`.
    #[must_use]
    pub fn from_status(status: u16, body: &[u8], retry_after_secs: Option<u64>) -> RequestError {
        let api: Option<(String, String)> =
            serde_json::from_slice::<Value>(body).ok().and_then(|value| {
                let error = value.get("error")?;
                let code = error.get("code")?.as_str()?.to_string();
                let message =
                    error.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
                Some((code, message))
            });
        let error = match api {
            Some((code, message)) => RequestError::api(&code, message, Some(status)),
            None => RequestError {
                code: format!("http_{status}"),
                status: Some(status),
                from_api: false,
                message: format!("the server answered with status {status}"),
                retry_after_secs: None,
            },
        };
        RequestError { retry_after_secs, ..error }
    }

    /// Returns true when a limit of the server refused the request: status
    /// 429, or the codes `rate_limited` and `too_many_connections`. The
    /// server works as its configuration says. Such an answer is not a
    /// failure of the server; the report counts it in a class of its own.
    #[must_use]
    pub fn throttled(&self) -> bool {
        self.status == Some(TOO_MANY_REQUESTS)
            || (self.from_api && LIMIT_CODES.contains(&self.code.as_str()))
    }

    /// Returns true when the failure shows a limit of this machine, not a
    /// fault of the server: no free local port, or too many open files.
    /// More agents cannot connect until the limit is raised.
    #[must_use]
    pub fn machine_limit(&self) -> bool {
        self.code == NO_LOCAL_PORT || self.code == TOO_MANY_OPEN_FILES
    }

    /// Returns true when the server does not accept the key of the agent,
    /// or when its resident left the world: status 401 or 410, or the
    /// codes `unauthorized`, `invalid_token` and `resident_gone`. The
    /// agent must sign up again.
    #[must_use]
    pub fn credentials_refused(&self) -> bool {
        matches!(self.status, Some(401 | 410))
            || (self.from_api && KEY_CODES.contains(&self.code.as_str()))
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RequestError {}

/// Turns a failure of the HTTP client into a request error.
#[must_use]
pub fn from_reqwest(error: &reqwest::Error) -> RequestError {
    let code = if let Some(limit) = machine_limit_in(error) {
        limit
    } else if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_decode() {
        "decode"
    } else {
        "transport"
    };
    // The text of the error can hold the URL. A URL of this tool has no
    // secret in it: keys travel in a header.
    RequestError::local(code, chain_text(error))
}

/// Writes an error with its causes in one line. The last cause is often
/// the one that tells the operator what happened.
#[must_use]
pub fn chain_text(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        next = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_api_error_keeps_its_code() {
        let body = br#"{"error":{"code":"overloaded","message":"the simulation is overloaded"}}"#;
        let error = RequestError::from_status(503, body, None);
        assert_eq!(error.code, "overloaded");
        assert_eq!(error.status, Some(503));
        assert!(error.from_api);
        assert!(!error.throttled());
        assert!(!error.credentials_refused());
        assert_eq!(error.to_string(), "overloaded: the simulation is overloaded");

        // An answer of the web server itself has no error object.
        let plain = RequestError::from_status(502, b"Bad Gateway", None);
        assert_eq!(plain.code, "http_502");
        assert!(!plain.from_api);
    }

    #[test]
    fn a_limit_of_the_server_is_a_class_of_its_own() {
        let body = br#"{"error":{"code":"rate_limited","message":"Too many requests."}}"#;
        let limited = RequestError::from_status(429, body, Some(7));
        assert!(limited.throttled());
        assert_eq!(limited.code, "rate_limited");
        assert_eq!(limited.retry_after_secs, Some(7));
        assert!(!limited.credentials_refused());
        assert!(!limited.machine_limit());

        let body = br#"{"error":{"code":"too_many_connections","message":"At most 4."}}"#;
        assert!(RequestError::from_status(429, body, None).throttled());
        // A proxy can answer 429 without the error object of the API.
        assert!(RequestError::from_status(429, b"", None).throttled());
        // An `error` frame on a WebSocket has the code and no status.
        assert!(RequestError::api("rate_limited", "Too many requests.", None).throttled());

        // Other refusals are not limits.
        assert!(!RequestError::from_status(503, b"", None).throttled());
        assert!(!RequestError::api("overloaded", "busy", Some(503)).throttled());
        // A transport failure with the same name is not an answer of the server.
        assert!(!RequestError::local("rate_limited", "no answer").throttled());
    }

    #[test]
    fn a_key_that_the_server_does_not_accept_is_told_from_other_refusals() {
        let body = br#"{"error":{"code":"invalid_token","message":"The server does not know the token."}}"#;
        assert!(RequestError::from_status(401, body, None).credentials_refused());
        let body =
            br#"{"error":{"code":"resident_gone","message":"The resident left the world."}}"#;
        assert!(RequestError::from_status(410, body, None).credentials_refused());
        assert!(RequestError::api("resident_gone", "gone", None).credentials_refused());

        // The route needs the other kind of token, or the owner has the
        // greatest number of residents: a new sign-up does not help.
        let body = br#"{"error":{"code":"resident_limit","message":"At most 3."}}"#;
        assert!(!RequestError::from_status(403, body, None).credentials_refused());
        let body = br#"{"error":{"code":"not_found","message":"No route has this path."}}"#;
        assert!(!RequestError::from_status(404, body, None).credentials_refused());
        assert!(!RequestError::local("timeout", "no answer").credentials_refused());
    }

    /// An error of a library that has an error of the system as its cause.
    #[derive(Debug)]
    struct Wrapper(io::Error);

    impl fmt::Display for Wrapper {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the connection failed")
        }
    }

    impl Error for Wrapper {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn a_machine_limit_is_found_behind_the_error_of_a_library() {
        let no_port = io::Error::from(io::ErrorKind::AddrNotAvailable);
        assert_eq!(machine_limit_in(&no_port), Some(NO_LOCAL_PORT));
        assert_eq!(machine_limit_in(&Wrapper(no_port)), Some(NO_LOCAL_PORT));
        let files = Wrapper(io::Error::from_raw_os_error(EMFILE));
        assert_eq!(machine_limit_in(&files), Some(TOO_MANY_OPEN_FILES));
        assert_eq!(
            machine_limit_in(&io::Error::from_raw_os_error(ENFILE)),
            Some(TOO_MANY_OPEN_FILES)
        );
        // A system error inside a system error that a library made.
        let nested = io::Error::other(Wrapper(io::Error::from_raw_os_error(EMFILE)));
        assert_eq!(machine_limit_in(&nested), Some(TOO_MANY_OPEN_FILES));
        let twice = io::Error::other(io::Error::from(io::ErrorKind::AddrNotAvailable));
        assert_eq!(machine_limit_in(&twice), Some(NO_LOCAL_PORT));

        // A server that refuses a connection is not a limit of this machine.
        let refused = Wrapper(io::Error::from(io::ErrorKind::ConnectionRefused));
        assert_eq!(machine_limit_in(&refused), None);
        assert!(RequestError::local(NO_LOCAL_PORT, "no port").machine_limit());
        assert!(RequestError::local(TOO_MANY_OPEN_FILES, "no file").machine_limit());
        assert!(!RequestError::local("connect", "refused").machine_limit());
        assert!(!RequestError::from_status(503, b"", None).machine_limit());
    }

    #[test]
    fn the_text_of_an_error_names_its_causes_once() {
        let error = Wrapper(io::Error::other("no route to host"));
        assert_eq!(chain_text(&error), "the connection failed: no route to host");
        // A cause that the text already holds is not written again.
        #[derive(Debug)]
        struct Outer(Wrapper);
        impl fmt::Display for Outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the request failed: the connection failed")
            }
        }
        impl Error for Outer {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }
        let outer = Outer(Wrapper(io::Error::other("no route to host")));
        assert_eq!(
            chain_text(&outer),
            "the request failed: the connection failed: no route to host"
        );
    }
}
