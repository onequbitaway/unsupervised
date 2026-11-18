//! The WebSocket for agents.
//!
//! A text frame is JSON and a binary frame is MessagePack; both carry the
//! same messages. The server answers a frame in the format of that frame.
//! It pushes notices and reports in the format of the last frame that the
//! client sent, and in JSON before the first one unless the address says
//! `?codec=msgpack`.
//!
//! Everything that the server sends goes through one bounded queue. A
//! client that reads too slowly fills the queue, and the server drops the
//! connection. The client connects again with `?since=`, the sequence
//! number after the last notice that it got, and misses nothing that the
//! inbox still holds. It can get a report a second time.

use std::time::Instant;

use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::watch;
use unsup_protocol::codec::{self, Format};
use unsup_protocol::{ActionAck, ActionReport, Detail, Notice, Observation, ResidentId, SimError};

use crate::actions::{observe_opts, run_plan};
use crate::auth::ResidentAuth;
use crate::error::ApiError;
use crate::extract::Params;
use crate::gauge::Slot;
use crate::push::{Watch, cursor_now, news_since};
use crate::state::{Counters, Gateway, LimitKey};
use crate::validate;

/// The kind that an action report has in a `kinds` filter.
pub(crate) const REPORT_KIND: &str = "report";
/// Characters that a notice kind has in addition to lowercase letters,
/// digits and `_`.
const KIND_EXTRA: &str = ".";

/// Close code: the server stops.
const CLOSE_GOING_AWAY: u16 = 1001;
/// Close code: the client broke a rule of the server. Here: it was idle.
const CLOSE_POLICY: u16 = 1008;

/// The wire format that the client asks for in the address.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Codec {
    Json,
    Msgpack,
}

/// The query of `GET /v1/ws`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WsQuery {
    /// Format of the pushed frames before the first frame of the client.
    #[serde(default)]
    codec: Option<Codec>,
    /// Push the notices and reports with a sequence number at or above
    /// this. Without it, only news that comes after the connection.
    #[serde(default)]
    since: Option<u64>,
}

/// The options of an `observe` frame: the fields of `ObserveOpts`. A
/// field that the server does not know is an error, so that a wrong name
/// does not pass as a default.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameOpts {
    #[serde(default)]
    detail: Option<Detail>,
    #[serde(default)]
    since_notice: Option<u64>,
    #[serde(default)]
    sections: Option<Vec<String>>,
}

/// A frame from the client. An action request stays a JSON value here:
/// the same check as for `POST /v1/act` reads it.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum ClientFrame {
    /// One action.
    Act { request: Value },
    /// Several actions, in order.
    Plan { requests: Vec<Value> },
    /// Ask for an observation.
    Observe {
        #[serde(default)]
        opts: FrameOpts,
    },
    /// Choose which pushes to get. Without `kinds`: all of them.
    Subscribe {
        #[serde(default)]
        kinds: Option<Vec<String>>,
    },
    /// Ask for a `pong`. The braces make serde refuse unknown fields
    /// here too.
    Ping {},
}

/// A frame from the server.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ServerFrame {
    Ack { ack: ActionAck },
    Observation { observation: Box<Observation> },
    Notice { notice: Notice },
    Report { report: ActionReport },
    Error { error: Value },
    Pong,
}

/// Why a connection ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// The client left, or the resident is gone.
    Gone,
    /// The client sent nothing for too long.
    Idle,
    /// The client reads too slowly: the queue is full.
    Slow,
    /// The server stops.
    Shutdown,
}

/// `GET /v1/ws`: upgrades to a WebSocket.
pub(crate) async fn upgrade(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
    Params(query): Params<WsQuery>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            "upgrade_required",
            "This path is a WebSocket. Send a WebSocket upgrade request.",
        )
    })?;
    let resident = auth.resident;
    let config = &gateway.config().ws;
    let slot =
        gateway.inner.sockets.acquire(resident, config.connections_per_resident).ok_or_else(
            || {
                ApiError::too_many(format!(
                    "A resident has at most {} WebSocket connections at a time.",
                    config.connections_per_resident
                ))
            },
        )?;
    // Watch first, then read the cursor: news between the two steps wakes
    // the connection, and the cursor is before it.
    let watch = gateway.inner.hub.watch(resident);
    let cursor = match query.since {
        Some(since) => since,
        None => match cursor_now(gateway.sim(), resident).await {
            Ok(cursor) => cursor,
            Err(error) => return Err(gateway.resident_error(resident, error).await),
        },
    };
    let format = match query.codec {
        Some(Codec::Msgpack) => Format::MsgPack,
        Some(Codec::Json) | None => Format::Json,
    };
    let max = config.max_message_bytes;
    let (out, queue) = mpsc::channel(config.queue.max(1));
    let connection =
        Connection { gateway: gateway.clone(), resident, format, cursor, kinds: None, out };
    Ok(upgrade
        .max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| connection.serve(socket, queue, watch, slot)))
}

/// Waits until the gateway stops.
async fn stopped(stopping: &mut watch::Receiver<bool>) {
    // The sender lives as long as the gateway, so this cannot fail.
    let _ = stopping.wait_for(|stopping| *stopping).await;
}

/// The state of one connection.
struct Connection {
    gateway: Gateway,
    resident: ResidentId,
    /// Format of the pushed frames.
    format: Format,
    /// News at or above this sequence number was not pushed yet.
    cursor: u64,
    /// The kinds that the client wants pushed. `None`: all.
    kinds: Option<Vec<String>>,
    /// The bounded queue to the socket.
    out: mpsc::Sender<Message>,
}

impl Connection {
    async fn serve(
        mut self,
        socket: WebSocket,
        mut queue: mpsc::Receiver<Message>,
        watch: Watch,
        _slot: Slot<ResidentId>,
    ) {
        let (mut sink, mut stream) = socket.split();
        // The writer: the only task that touches the sending half.
        let writer = tokio::spawn(async move {
            while let Some(message) = queue.recv().await {
                let last = matches!(message, Message::Close(_));
                if sink.send(message).await.is_err() || last {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let config = self.gateway.config().ws.clone();
        let mut ping = tokio::time::interval_at(
            tokio::time::Instant::now() + config.ping_interval(),
            config.ping_interval(),
        );
        let mut stopping = self.gateway.inner.stopping.subscribe();
        let mut last_seen = Instant::now();
        // News that came between the cursor and now, for `?since=`.
        let mut step = self.push_news().await;

        let end = loop {
            if let Err(end) = step {
                break end;
            }
            step = tokio::select! {
                incoming = stream.next() => match incoming {
                    Some(Ok(message)) => {
                        last_seen = Instant::now();
                        self.incoming(message).await
                    }
                    Some(Err(_)) | None => Err(End::Gone),
                },
                () = watch.changed() => self.push_news().await,
                _ = ping.tick() => {
                    if last_seen.elapsed() >= config.idle_timeout() {
                        Err(End::Idle)
                    } else {
                        self.queue(Message::Ping(Default::default()))
                    }
                }
                () = stopped(&mut stopping) => Err(End::Shutdown),
            };
        };

        let close = |code, reason: &'static str| {
            Message::Close(Some(CloseFrame { code, reason: reason.into() }))
        };
        let closing = match end {
            End::Slow => {
                // The queue is full of frames that the client does not
                // read. Waiting for it would keep the connection.
                Counters::add(&self.gateway.inner.counters.ws_dropped);
                writer.abort();
                return;
            }
            End::Idle => self.queue(close(CLOSE_POLICY, "idle timeout")).is_ok(),
            End::Shutdown => self.queue(close(CLOSE_GOING_AWAY, "the server stops")).is_ok(),
            End::Gone => false,
        };
        drop(self.out);
        // The end of a connection gets one ping interval for each step:
        // first the writer sends what is in the queue, then the client
        // answers the close frame.
        let grace = config.ping_interval();
        let mut writer = writer;
        if tokio::time::timeout(grace, &mut writer).await.is_err() {
            writer.abort();
            return;
        }
        if closing {
            let answer = async {
                while let Some(Ok(message)) = stream.next().await {
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                }
            };
            let _ = tokio::time::timeout(grace, answer).await;
        }
    }

    /// Puts a message into the queue to the socket.
    fn queue(&self, message: Message) -> Result<(), End> {
        self.out.try_send(message).map_err(|error| match error {
            TrySendError::Full(_) => End::Slow,
            TrySendError::Closed(_) => End::Gone,
        })
    }

    /// Sends a frame in the given format.
    fn send(&self, format: Format, frame: &ServerFrame) -> Result<(), End> {
        let message = match format {
            Format::Json => codec::to_json(frame).map(|text| Message::Text(text.into())),
            Format::MsgPack => codec::to_msgpack(frame).map(|bytes| Message::Binary(bytes.into())),
        };
        match message {
            Ok(message) => self.queue(message),
            Err(error) => {
                // Every frame is plain data, so this is a bug.
                tracing::error!(%error, "cannot encode a WebSocket frame");
                Ok(())
            }
        }
    }

    fn error(&self, format: Format, code: &str, message: impl Into<String>) -> Result<(), End> {
        let error = json!({ "code": code, "message": message.into() });
        self.send(format, &ServerFrame::Error { error })
    }

    /// Handles a message of the client.
    async fn incoming(&mut self, message: Message) -> Result<(), End> {
        let (format, bytes) = match &message {
            Message::Text(text) => (Format::Json, text.as_bytes()),
            Message::Binary(bytes) => (Format::MsgPack, bytes.as_ref()),
            // The WebSocket layer answers a ping by itself.
            Message::Ping(_) | Message::Pong(_) => return Ok(()),
            Message::Close(_) => return Err(End::Gone),
        };
        self.format = format;
        if let Err(limit) = self.gateway.take_request(LimitKey::Resident(self.resident)) {
            return self.error(format, limit.code, limit.message);
        }
        match codec::decode::<ClientFrame>(format, bytes) {
            Ok(frame) => self.frame(format, frame).await,
            Err(error) => {
                self.error(format, "invalid_request", format!("The frame is not valid: {error}."))
            }
        }
    }

    /// Handles one frame of the client and answers in its format.
    async fn frame(&mut self, format: Format, frame: ClientFrame) -> Result<(), End> {
        let limits = &self.gateway.config().limits;
        let invalid = |message: String| self.error(format, "invalid_request", message);
        match frame {
            ClientFrame::Act { request } => {
                let request = match validate::action_from_value("`request`", request) {
                    Ok(request) => request,
                    Err(message) => return invalid(message),
                };
                match self.gateway.sim().act(self.resident, request).await {
                    Ok(ack) => self.send(format, &ServerFrame::Ack { ack }),
                    Err(error) => self.sim_error(format, error).await,
                }
            }
            ClientFrame::Plan { requests } => {
                let checked = validate::plan_from_values("requests", requests, limits.max_plan);
                let requests = match checked {
                    Ok(requests) => requests,
                    Err(message) => return invalid(message),
                };
                match run_plan(self.gateway.sim(), self.resident, requests).await {
                    Ok(acks) => acks
                        .into_iter()
                        .try_for_each(|ack| self.send(format, &ServerFrame::Ack { ack })),
                    Err(error) => self.sim_error(format, error).await,
                }
            }
            ClientFrame::Observe { opts } => {
                if let Some(sections) = &opts.sections
                    && let Err(error) =
                        validate::check_names("sections", sections, "", limits.max_names)
                {
                    return invalid(error.message);
                }
                let opts = observe_opts(opts.detail, opts.since_notice, opts.sections);
                match self.gateway.sim().observe(self.resident, opts).await {
                    Ok(observation) => {
                        let observation = Box::new(observation);
                        self.send(format, &ServerFrame::Observation { observation })
                    }
                    Err(error) => self.sim_error(format, error).await,
                }
            }
            ClientFrame::Subscribe { kinds } => {
                if let Some(kinds) = &kinds
                    && let Err(error) =
                        validate::check_names("kinds", kinds, KIND_EXTRA, limits.max_names)
                {
                    return invalid(error.message);
                }
                self.kinds = kinds;
                Ok(())
            }
            ClientFrame::Ping {} => self.send(format, &ServerFrame::Pong),
        }
    }

    /// Reports an error of the simulation. A resident that is gone ends
    /// the connection.
    async fn sim_error(&self, format: Format, error: SimError) -> Result<(), End> {
        let error = self.gateway.resident_error(self.resident, error).await;
        self.error(format, error.code, error.message)?;
        if error.code == ApiError::resident_gone().code { Err(End::Gone) } else { Ok(()) }
    }

    fn wants(&self, kind: &str) -> bool {
        self.kinds.as_ref().is_none_or(|kinds| kinds.iter().any(|k| k == kind))
    }

    /// Pushes the notices and reports since the cursor.
    async fn push_news(&mut self) -> Result<(), End> {
        let news = match news_since(self.gateway.sim(), self.resident, self.cursor).await {
            Ok(news) => news,
            Err(SimError::UnknownResident) => {
                return self.sim_error(self.format, SimError::UnknownResident).await;
            }
            // The simulation is busy. The next wake reads from the same
            // cursor, so nothing is lost.
            Err(_) => return Ok(()),
        };
        self.cursor = news.next;
        // Reports first, then the notices by sequence number. A report
        // has no sequence number of its own. In this order, a client
        // that got a notice also got every report that the server read
        // with it, so the number after that notice is a safe cursor.
        for report in news.reports {
            if self.wants(REPORT_KIND) {
                self.send(self.format, &ServerFrame::Report { report })?;
            }
        }
        for notice in news.notices {
            if self.wants(&notice.kind) {
                self.send(self.format, &ServerFrame::Notice { notice })?;
            }
        }
        Ok(())
    }
}
