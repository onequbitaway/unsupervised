//! An agent over the agent WebSocket, in JSON or in MessagePack.

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::link::{Inbox, Link};
use super::{World, id, soon};

/// The format of the frames of one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// Text frames with JSON.
    Json,
    /// Binary frames with MessagePack, written with field names.
    MsgPack,
}

/// The raw connection of a socket.
pub type Stream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// One connection to `GET /v1/ws`.
pub struct Socket {
    /// The news that the server pushed so far.
    pub inbox: Inbox,
    codec: Codec,
    stream: Stream,
    requests: u64,
}

impl Socket {
    /// Opens the socket of a resident. `query` has the cursors of a
    /// client that connects again, for example `since=4&reports_since=7`.
    /// A MessagePack client also asks for binary pushes from the start.
    pub async fn open(world: &World, key: &str, codec: Codec, query: &str) -> Socket {
        let mut parts: Vec<&str> = Vec::new();
        if codec == Codec::MsgPack {
            parts.push("codec=msgpack");
        }
        if !query.is_empty() {
            parts.push(query);
        }
        let query = if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) };
        let address = format!("ws://{}/v1/ws{query}", world.addr);
        let mut request = address.into_client_request().expect("a request");
        let value = format!("Bearer {key}").parse().expect("a header value");
        request.headers_mut().insert("Authorization", value);
        let (stream, _) =
            tokio_tungstenite::connect_async(request).await.expect("the socket opens");
        Socket { inbox: Inbox::default(), codec, stream, requests: 0 }
    }

    /// Sends one frame in the format of the connection.
    pub async fn send(&mut self, frame: &Value) {
        let message = match self.codec {
            Codec::Json => Message::Text(frame.to_string().into()),
            Codec::MsgPack => {
                Message::Binary(rmp_serde::to_vec_named(frame).expect("MessagePack").into())
            }
        };
        self.stream.send(message).await.expect("the frame is sent");
    }

    /// Reads the next frame of the server. The frame must have the
    /// format of the connection.
    ///
    /// # Panics
    ///
    /// Panics when the connection ends, or when the frame has the other
    /// format.
    pub async fn frame(&mut self) -> Value {
        loop {
            let message = self.stream.next().await.expect("the socket is open").expect("a frame");
            match (message, self.codec) {
                (Message::Text(text), Codec::Json) => {
                    return serde_json::from_str(&text).expect("a text frame is JSON");
                }
                (Message::Binary(bytes), Codec::MsgPack) => {
                    return rmp_serde::from_slice(&bytes).expect("a binary frame is MessagePack");
                }
                // The WebSocket layer answers a ping by itself.
                (Message::Ping(_) | Message::Pong(_), _) => {}
                (other, codec) => panic!("a frame in {codec:?}, not {other:?}"),
            }
        }
    }

    /// Reads frames until one has the tag `op`. A `report` or a `notice`
    /// on the way goes into the inbox.
    ///
    /// # Panics
    ///
    /// Panics when the server sends an `error` frame instead, or any
    /// frame that is not news.
    pub async fn answer(&mut self, op: &str) -> Value {
        loop {
            let frame = soon(self.frame()).await;
            if frame["op"] == op {
                return frame;
            }
            assert!(self.take_push(&frame), "a `{op}` frame or news, not {frame}");
        }
    }

    /// Puts a pushed frame into the inbox. Returns false when the frame
    /// is not a `report` or a `notice`.
    fn take_push(&mut self, frame: &Value) -> bool {
        match frame["op"].as_str() {
            Some("report") => self.inbox.report(frame["report"].clone()),
            Some("notice") => self.inbox.notice(frame["notice"].clone()),
            _ => return false,
        }
        true
    }

    /// Reads the next frame. It must be pushed news; it goes into the
    /// inbox.
    pub async fn push(&mut self) {
        let frame = self.frame().await;
        assert!(self.take_push(&frame), "pushed news, not {frame}");
    }

    /// Sends a `ping` frame of the protocol and reads the `pong`.
    pub async fn ping(&mut self) {
        self.send(&json!({ "op": "ping" })).await;
        let pong = self.answer("pong").await;
        assert_eq!(pong, json!({ "op": "pong" }));
    }

    /// Sends a request that the server must refuse. Returns the `error`
    /// object of the answer.
    pub async fn refused(&mut self, frame: &Value) -> Value {
        self.send(frame).await;
        self.answer("error").await["error"].clone()
    }

    /// Returns the cursors for the next connection: `since` is the `seq`
    /// of the last notice plus 1, and `reports_since` is the `seq` of
    /// the last report plus 1. Without a notice or a report so far, the
    /// cursor is `start`.
    pub fn cursors(&self, start: u64) -> String {
        let next = |news: &[Value]| news.last().map_or(start, |last| id(&last["seq"], "seq") + 1);
        format!("since={}&reports_since={}", next(&self.inbox.notices), next(&self.inbox.reports))
    }

    /// Gives the raw connection, for a test that reads its end.
    pub fn into_stream(self) -> Stream {
        self.stream
    }

    /// Closes the connection.
    pub async fn close(mut self) -> Inbox {
        // The server can be gone already.
        let _ = self.stream.close(None).await;
        self.inbox
    }

    fn request_id(&mut self) -> String {
        self.requests += 1;
        format!("ws-{}", self.requests)
    }
}

impl Link for Socket {
    async fn act(&mut self, verb: &str, args: Value) -> Value {
        let request = json!({ "id": self.request_id(), "verb": verb, "args": args });
        self.send(&json!({ "op": "act", "request": request })).await;
        let ack = self.answer("ack").await["ack"].clone();
        assert_eq!(ack["request_id"], request["id"], "the acknowledgement answers the request");
        ack
    }

    async fn plan(&mut self, requests: Vec<Value>) -> Vec<Value> {
        self.send(&json!({ "op": "plan", "requests": requests })).await;
        // The server sends one `ack` for each request up to the first
        // refusal, then nothing. A `ping` marks the end of the answers.
        self.send(&json!({ "op": "ping" })).await;
        let mut acks = Vec::new();
        loop {
            let frame = soon(self.frame()).await;
            match frame["op"].as_str() {
                Some("ack") => acks.push(frame["ack"].clone()),
                Some("pong") => return acks,
                _ => assert!(self.take_push(&frame), "an `ack`, a `pong` or news, not {frame}"),
            }
        }
    }

    async fn observe(&mut self, detail: &str, sections: &[&str]) -> Value {
        let mut opts = json!({ "detail": detail });
        if !sections.is_empty() {
            opts["sections"] = json!(sections);
        }
        self.send(&json!({ "op": "observe", "opts": opts })).await;
        self.answer("observation").await["observation"].clone()
    }

    async fn news(&mut self, world: &World) {
        world.until(self.push()).await;
    }

    fn inbox(&self) -> &Inbox {
        &self.inbox
    }
}
