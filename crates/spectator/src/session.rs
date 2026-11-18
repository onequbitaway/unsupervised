//! One viewer on `/v1/spectate`.
//!
//! A viewer has a bounded queue of messages. The task of the viewer never
//! waits for the socket: it puts messages in the queue, and a second task
//! writes them. When the queue is full the viewer loses the frame. It gets
//! a new `world` message and new snapshots as soon as the queue has room:
//! the writing task tells the task of the viewer after every message that
//! left.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, mpsc};
use unsup_protocol::ZoneId;

use crate::digest::{Digest, to_text};
use crate::hub::{Hub, Pulse};
use crate::wire::{ClientMessage, ErrorBody, ServerMessage};

/// The state of one subscribed zone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ZoneState {
    /// The viewer needs a snapshot of the zone.
    Pending,
    /// The viewer has a snapshot. It gets the deltas from this frame on.
    Live { from: u64 },
}

/// What the server knows about one viewer.
struct Viewer {
    hub: Arc<Hub>,
    queue: mpsc::Sender<Message>,
    zones: BTreeMap<ZoneId, ZoneState>,
    /// The first frame whose `frame` message the viewer gets. `None`: the
    /// viewer needs a `world` message first.
    world_from: Option<u64>,
    /// Frames that the viewer lost since its last `world` message.
    dropped: u64,
}

impl Viewer {
    fn new(hub: Arc<Hub>, queue: mpsc::Sender<Message>) -> Viewer {
        Viewer { hub, queue, zones: BTreeMap::new(), world_from: None, dropped: 0 }
    }

    /// Puts a message in the queue when there is room. Answers to requests
    /// go this way: a viewer that does not read loses them.
    fn reply(&self, message: &ServerMessage) {
        let _ = self.queue.try_send(Message::Text(to_text(message)));
    }

    fn refuse(&self, code: &str, message: String) {
        self.reply(&ServerMessage::Error { error: ErrorBody { code: code.to_string(), message } });
    }

    /// Returns true when the viewer waits for a `world` message or for a
    /// snapshot.
    fn waits(&self) -> bool {
        self.world_from.is_none() || self.zones.values().any(|state| *state == ZoneState::Pending)
    }

    /// Sends what the viewer waits for, as far as the queue has room: the
    /// `world` message, then one snapshot for each pending zone. All of
    /// them come from one state of the cache.
    fn sync(&mut self) {
        if !self.waits() {
            return;
        }
        let cache = self.hub.cache();
        let next = cache.frame() + 1;
        if self.world_from.is_none() {
            let Ok(permit) = self.queue.try_reserve() else {
                return;
            };
            permit.send(Message::Text(cache.world_message(self.dropped, self.hub.viewers())));
            self.world_from = Some(next);
            self.dropped = 0;
        }
        for (zone, state) in &mut self.zones {
            if *state != ZoneState::Pending {
                continue;
            }
            let Some(text) = cache.snapshot_message(*zone) else {
                continue;
            };
            let Ok(permit) = self.queue.try_reserve() else {
                return;
            };
            permit.send(Message::Text(text));
            *state = ZoneState::Live { from: next };
        }
    }

    /// Sends one frame: a `delta` for each zone that the viewer follows and
    /// that changed, then the `frame` message. The viewer gets all of them
    /// or none.
    fn on_frame(&mut self, digest: &Digest) {
        self.sync();
        let Some(world_from) = self.world_from else {
            // Still no room for the `world` message.
            self.dropped += 1;
            self.hub.count_dropped(1);
            return;
        };
        if digest.frame < world_from {
            // The `world` message already holds this frame.
            return;
        }
        let deltas: Vec<Utf8Bytes> = self
            .zones
            .iter()
            .filter(|(_, state)| matches!(state, ZoneState::Live { from } if *from <= digest.frame))
            .filter_map(|(zone, _)| digest.zone_text(*zone))
            .collect();
        let sent = match self.queue.try_reserve_many(deltas.len() + 1) {
            Ok(permits) => {
                let messages = deltas.into_iter().chain(std::iter::once(digest.frame_text()));
                for (permit, text) in permits.zip(messages) {
                    permit.send(Message::Text(text));
                }
                true
            }
            Err(_) => false,
        };
        if !sent {
            self.lose(1);
        }
    }

    /// The viewer lost frames: it needs the world and every zone again.
    fn lose(&mut self, frames: u64) {
        self.dropped += frames;
        self.hub.count_dropped(frames);
        if self.world_from.take().is_some() {
            self.hub.count_resync();
        }
        for state in self.zones.values_mut() {
            *state = ZoneState::Pending;
        }
        self.sync();
    }

    fn on_text(&mut self, text: &str) {
        match serde_json::from_str::<ClientMessage>(text) {
            Ok(ClientMessage::Subscribe { zones }) => self.subscribe(&zones),
            Ok(ClientMessage::Unsubscribe { zones }) => self.unsubscribe(&zones),
            Ok(ClientMessage::Ping) => self.reply(&ServerMessage::Pong),
            Err(error) => {
                self.refuse("bad_request", format!("The message is not a request: {error}."));
            }
        }
    }

    /// Adds zones. One bad zone refuses the whole request.
    fn subscribe(&mut self, zones: &[ZoneId]) {
        let mut new: Vec<ZoneId> =
            zones.iter().copied().filter(|zone| !self.zones.contains_key(zone)).collect();
        new.sort_unstable();
        new.dedup();
        let unknown = {
            let cache = self.hub.cache();
            new.iter().copied().find(|zone| !cache.serves(*zone))
        };
        if let Some(zone) = unknown {
            self.refuse(
                "unknown_zone",
                format!("Zone {zone} does not exist or is not served here."),
            );
            return;
        }
        let max = self.hub.config.socket.max_zones;
        if self.zones.len() + new.len() > max {
            self.refuse(
                "too_many_zones",
                format!("A viewer can subscribe to at most {max} zones."),
            );
            return;
        }
        for zone in new {
            self.zones.insert(zone, ZoneState::Pending);
        }
        self.sync();
    }

    fn unsubscribe(&mut self, zones: &[ZoneId]) {
        let mut removed = Vec::new();
        for zone in zones {
            if self.zones.remove(zone).is_some() {
                removed.push(*zone);
            }
        }
        self.reply(&ServerMessage::Unsubscribed { zones: removed });
    }
}

/// Writes the queue of a viewer to its socket. It ends when the queue
/// closes, when the socket fails, or when one message does not leave in
/// time. After every message that left it says that the queue has room.
async fn write(
    mut sink: SplitSink<WebSocket, Message>,
    mut queue: mpsc::Receiver<Message>,
    timeout: Duration,
    room: Arc<Notify>,
) {
    while let Some(message) = queue.recv().await {
        match tokio::time::timeout(timeout, sink.send(message)).await {
            Ok(Ok(())) => room.notify_one(),
            Ok(Err(_)) | Err(_) => break,
        }
    }
    // The viewer may be gone already. That is fine.
    let _ = tokio::time::timeout(timeout, sink.close()).await;
}

/// Serves one viewer until it leaves or the simulation stops.
pub(crate) async fn run(socket: WebSocket, hub: Arc<Hub>) {
    if hub.ready().await.is_err() || hub.ended() {
        return;
    }
    let _counted = hub.viewer();
    let timeout = Duration::from_millis(hub.config.socket.send_timeout_ms);
    let (sink, mut stream) = socket.split();
    let (queue, waiting) = mpsc::channel(hub.config.socket.client_queue);
    let room = Arc::new(Notify::new());
    let mut writer = tokio::spawn(write(sink, waiting, timeout, Arc::clone(&room)));
    // Subscribe before the first read of the cache, so that no frame is
    // lost between the two.
    let mut pulses = hub.pulses();
    let mut viewer = Viewer::new(Arc::clone(&hub), queue);
    viewer.sync();

    let writer_ended = loop {
        tokio::select! {
            pulse = pulses.recv() => match pulse {
                Ok(Pulse::Frame(digest)) => viewer.on_frame(&digest),
                Ok(Pulse::Reset) => viewer.lose(0),
                Err(RecvError::Lagged(lost)) => viewer.lose(lost),
                Ok(Pulse::Stopped) | Err(RecvError::Closed) => break false,
            },
            message = stream.next() => match message {
                Some(Ok(Message::Text(text))) => viewer.on_text(&text),
                Some(Ok(Message::Binary(_))) => {
                    viewer.refuse("bad_request", "Send JSON in text frames.".to_string());
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break false,
            },
            // The queue has room again for a viewer that lost frames.
            () = room.notified(), if viewer.waits() => viewer.sync(),
            _ = &mut writer => break true,
        }
    };
    // Closing the queue lets the writer send what waits and close the
    // socket. A viewer that does not read cannot hold the task for long.
    drop(viewer);
    if !writer_ended && tokio::time::timeout(timeout, &mut writer).await.is_err() {
        writer.abort();
    }
}
