//! The answer of the world to one sponsor mission.
//!
//! The missions module answers a `SponsorMission` with one public event
//! in the frame after the message: `missions.mission_posted` or
//! `missions.sponsor_refused`. Both carry the `request` number that the
//! sender chose. The route reads the frames of the world until it sees
//! its number.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio::time::Instant;
use unsup_protocol::{Event, FrameOutput};

/// Kind of the event for a mission that the world published.
const POSTED: &str = "missions.mission_posted";
/// Kind of the event for a sponsor mission that the module refused.
const REFUSED: &str = "missions.sponsor_refused";

/// What the world said about one sponsor mission.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Verdict {
    /// The world published the mission.
    Posted {
        /// The frame of the publication.
        frame: u64,
        /// The data of `missions.mission_posted`.
        data: Value,
    },
    /// The module refused the mission. Nothing changed in the world.
    Refused {
        /// The code of the `ActionError` of the module.
        code: String,
        /// The message of that error.
        message: String,
    },
    /// No answer came in time, or the reader lost frames. The world has
    /// the mission, and its answer can still come.
    Unknown,
    /// The simulation stopped.
    Stopped,
}

/// Reads the answer to the request from one event, when the event is one.
fn read(event: &Event, request: u64) -> Option<Verdict> {
    if event.data.get("request").and_then(Value::as_u64) != Some(request) {
        return None;
    }
    match event.kind.as_str() {
        POSTED => Some(Verdict::Posted { frame: event.frame, data: event.data.clone() }),
        REFUSED => {
            let text = |field: &str| {
                event.data.get(field).and_then(Value::as_str).unwrap_or_default().to_string()
            };
            Some(Verdict::Refused { code: text("code"), message: text("message") })
        }
        _ => None,
    }
}

/// Waits for the answer of the world to the request, for at most
/// `timeout`. The caller subscribed to `frames` before it sent the
/// mission, so the answer cannot pass before the wait starts.
pub(crate) async fn wait(
    frames: &mut Receiver<Arc<FrameOutput>>,
    request: u64,
    timeout: Duration,
) -> Verdict {
    let deadline = Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, frames.recv()).await {
            Ok(Ok(frame)) => {
                if let Some(verdict) = frame.events.iter().find_map(|event| read(event, request)) {
                    return verdict;
                }
            }
            // The answer can be in a frame that this reader lost.
            Ok(Err(RecvError::Lagged(_))) | Err(_) => return Verdict::Unknown,
            Ok(Err(RecvError::Closed)) => return Verdict::Stopped,
        }
    }
}
