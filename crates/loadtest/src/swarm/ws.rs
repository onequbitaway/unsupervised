//! The WebSocket side of an agent: frames on one connection.
//!
//! The frames are those of `docs/design/protocol.md`, section 2. The agent
//! writes JSON text frames. It reads text frames as JSON and binary frames
//! as MessagePack.

use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use unsup_protocol::{ActionAck, ActionReport, ActionRequest, Notice, Observation, ObserveOpts};

use super::error::{RequestError, machine_limit_in};

/// A frame from the agent to the server.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientFrame<'a> {
    /// One action.
    Act { request: &'a ActionRequest },
    /// Ask for an observation.
    Observe { opts: ObserveOpts },
}

/// A frame from the server to the agent.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ServerFrame {
    /// The acknowledgement of an action.
    Ack { ack: ActionAck },
    /// The answer to `observe`.
    Observation { observation: Box<Observation> },
    /// A notice that the server pushes when it happens.
    Notice {
        #[allow(dead_code)]
        notice: Notice,
    },
    /// The report of an ended action, pushed when it happens.
    Report {
        #[allow(dead_code)]
        report: ActionReport,
    },
    /// The server refused a frame.
    Error { error: Value },
    /// A frame that this tool does not use (`pong` and later additions).
    #[serde(other)]
    Other,
}

/// What an agent waits for on its socket.
enum Awaited<'a> {
    /// The acknowledgement of the request with this idempotency key.
    Ack(Option<&'a str>),
    /// An observation.
    Observation,
    /// The report of an ended action.
    Report,
    /// Nothing: only read what the server pushes.
    Nothing,
}

/// What came while the agent waited.
enum Arrived {
    Ack(ActionAck),
    Observation(Box<Observation>),
    Report,
    /// The time ran out.
    Timeout,
}

/// The WebSocket link of one agent.
pub struct WsLink {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
    /// Notices and reports that the server pushed since the last call of
    /// [`WsLink::take_pushed`].
    pushed: u64,
    /// True when a report came since the last action.
    report_seen: bool,
}

fn ws_error(error: &WsError) -> RequestError {
    match error {
        WsError::Http(response) => RequestError::from_status(
            response.status().as_u16(),
            response.body().as_deref().unwrap_or_default(),
        ),
        WsError::ConnectionClosed | WsError::AlreadyClosed => {
            RequestError::local("closed", "the server closed the WebSocket")
        }
        WsError::Io(io) => {
            RequestError::local(machine_limit_in(io).unwrap_or("connect"), io.to_string())
        }
        other => RequestError::local("transport", other.to_string()),
    }
}

fn timeout_error(what: &str) -> RequestError {
    RequestError::local("timeout", format!("no {what} came in time on the WebSocket"))
}

impl WsLink {
    /// Opens the WebSocket of a resident. The key travels in the
    /// `Authorization` header of the handshake.
    ///
    /// # Errors
    ///
    /// Fails when the handshake fails. [`RequestError::route_absent`] is
    /// true when the server has no WebSocket route.
    pub async fn connect(url: &str, key: &str, timeout: Duration) -> Result<WsLink, RequestError> {
        let mut request = url.into_client_request().map_err(|error| ws_error(&error))?;
        let bearer = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
            RequestError::local("signup_answer", "the key cannot travel in an HTTP header")
        })?;
        request.headers_mut().insert(AUTHORIZATION, bearer);
        // Small frames must leave at once: the latency of each one is
        // measured.
        let connect = tokio_tungstenite::connect_async_with_config(request, None, true);
        match tokio::time::timeout(timeout, connect).await {
            Ok(Ok((socket, _))) => Ok(WsLink { socket, timeout, pushed: 0, report_seen: false }),
            Ok(Err(error)) => Err(ws_error(&error)),
            Err(_) => Err(timeout_error("handshake answer")),
        }
    }

    /// Returns the number of notices and reports that the server pushed
    /// since the last call, and starts again from zero.
    pub fn take_pushed(&mut self) -> u64 {
        std::mem::take(&mut self.pushed)
    }

    async fn send(&mut self, frame: &ClientFrame<'_>) -> Result<(), RequestError> {
        let text = serde_json::to_string(frame)
            .map_err(|error| RequestError::local("encode", error.to_string()))?;
        self.socket.send(Message::text(text)).await.map_err(|error| ws_error(&error))
    }

    /// Reads frames until the awaited one comes or the deadline passes.
    /// Frames that the server pushes meanwhile are counted.
    async fn read_until(
        &mut self,
        awaited: Awaited<'_>,
        deadline: Instant,
    ) -> Result<Arrived, RequestError> {
        loop {
            let next = tokio::time::timeout_at(deadline.into(), self.socket.next()).await;
            let message = match next {
                Err(_) => return Ok(Arrived::Timeout),
                Ok(None) => return Err(RequestError::local("closed", "the WebSocket ended")),
                Ok(Some(Err(error))) => return Err(ws_error(&error)),
                Ok(Some(Ok(message))) => message,
            };
            let frame: ServerFrame = match &message {
                Message::Text(text) => serde_json::from_str(text.as_str())
                    .map_err(|error| RequestError::local("decode", error.to_string()))?,
                Message::Binary(bytes) => unsup_protocol::codec::from_msgpack(bytes)
                    .map_err(|error| RequestError::local("decode", error.to_string()))?,
                Message::Close(_) => {
                    return Err(RequestError::local("closed", "the server closed the WebSocket"));
                }
                // The library answers pings by itself.
                _ => continue,
            };
            match (frame, &awaited) {
                (ServerFrame::Ack { ack }, Awaited::Ack(id))
                    if ack.request_id.as_deref() == *id || id.is_none() =>
                {
                    return Ok(Arrived::Ack(ack));
                }
                (ServerFrame::Observation { observation }, Awaited::Observation) => {
                    return Ok(Arrived::Observation(observation));
                }
                (ServerFrame::Report { .. }, awaited) => {
                    self.pushed += 1;
                    self.report_seen = true;
                    if matches!(awaited, Awaited::Report) {
                        return Ok(Arrived::Report);
                    }
                }
                (ServerFrame::Notice { .. }, _) => self.pushed += 1,
                (ServerFrame::Error { error }, Awaited::Ack(_) | Awaited::Observation) => {
                    let code =
                        error.get("code").and_then(Value::as_str).unwrap_or("error").to_string();
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("the server refused the frame")
                        .to_string();
                    return Err(RequestError { code, status: None, from_api: true, message });
                }
                // An answer that nobody waits for, or a frame that this
                // tool does not use.
                _ => {}
            }
        }
    }

    /// Sends one action and waits for its acknowledgement. The latency is
    /// the time from the send to the acknowledgement frame.
    ///
    /// # Errors
    ///
    /// Fails when the server refuses the frame, closes the socket or gives
    /// no acknowledgement in time.
    pub async fn act(
        &mut self,
        request: &ActionRequest,
    ) -> Result<(ActionAck, Duration), RequestError> {
        self.report_seen = false;
        let start = Instant::now();
        self.send(&ClientFrame::Act { request }).await?;
        match self.read_until(Awaited::Ack(request.id.as_deref()), start + self.timeout).await? {
            Arrived::Ack(ack) => Ok((ack, start.elapsed())),
            _ => Err(timeout_error("acknowledgement")),
        }
    }

    /// Asks for an observation with the notices since `since`.
    ///
    /// # Errors
    ///
    /// Fails like [`WsLink::act`].
    pub async fn observe(&mut self, since: u64) -> Result<(Observation, Duration), RequestError> {
        let start = Instant::now();
        let opts = ObserveOpts { since_notice: since, ..ObserveOpts::default() };
        self.send(&ClientFrame::Observe { opts }).await?;
        match self.read_until(Awaited::Observation, start + self.timeout).await? {
            Arrived::Observation(observation) => Ok((*observation, start.elapsed())),
            _ => Err(timeout_error("observation")),
        }
    }

    /// Reads what the server pushes until `deadline`. An agent that thinks
    /// calls this, so that its socket stays alive.
    ///
    /// # Errors
    ///
    /// Fails when the server closes the socket.
    pub async fn idle_until(&mut self, deadline: Instant) -> Result<(), RequestError> {
        self.read_until(Awaited::Nothing, deadline).await.map(|_| ())
    }

    /// Waits until the server pushes the report of an ended action, or
    /// until `deadline`. Returns true when a report came, also when it
    /// came before this call and after the last action.
    ///
    /// # Errors
    ///
    /// Fails when the server closes the socket.
    pub async fn wait_report(&mut self, deadline: Instant) -> Result<bool, RequestError> {
        if self.report_seen {
            return Ok(true);
        }
        Ok(matches!(self.read_until(Awaited::Report, deadline).await?, Arrived::Report))
    }

    /// Closes the socket. Errors do not matter: the agent is done.
    pub async fn close(mut self) {
        let _ = self.socket.close(None).await;
    }
}
