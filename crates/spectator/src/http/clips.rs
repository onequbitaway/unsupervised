//! Replay clips: `/v1/clips`, `/v1/clips/{event}` and the pin.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use super::error::ApiError;
use crate::clips::{NoClip, Pin, link_of};
use crate::hub::Hub;
use crate::wire::{Clip, ClipList, EventId, FeedEvent, PinAnswer};

fn no_clip(id: EventId, reason: NoClip) -> ApiError {
    let message = match reason {
        NoClip::Off => "This server keeps no clips.".to_string(),
        NoClip::NoPlace => format!("The event {id} has no place, so it has no clip."),
        NoClip::Gone => {
            format!("The server does not have the frames around the event {id} any more.")
        }
    };
    ApiError::not_found("unknown_clip", message)
}

fn unknown_event(id: EventId) -> ApiError {
    ApiError::not_found("unknown_clip", format!("The server has no public event with the id {id}."))
}

fn parse(id: &str) -> Result<EventId, ApiError> {
    id.parse::<EventId>().map_err(|error| {
        ApiError::invalid(format!("The id of a clip is the id of an event: {error}."))
    })
}

/// Answers `/v1/clips`: the pinned clips.
pub(crate) async fn list(State(hub): State<Arc<Hub>>) -> Result<Json<ClipList>, ApiError> {
    hub.ready().await?;
    let clips = hub.studio().recorder.list();
    Ok(Json(ClipList { clips }))
}

/// Finds the event of a clip in the log of the cache, with its line.
fn find(hub: &Hub, id: EventId) -> Option<(Arc<FeedEvent>, String)> {
    let cache = hub.cache();
    let event = cache.event(id)?;
    let line = hub.studio().line(&event, &*cache);
    Some((event, line))
}

/// Answers `/v1/clips/{event}`: the frames around a public event.
pub(crate) async fn clip(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
) -> Result<Json<Clip>, ApiError> {
    hub.ready().await?;
    let id = parse(&id)?;
    // A pinned clip is there also when the event left the log.
    let pinned = hub.studio().recorder.pinned(id);
    if let Some(clip) = pinned {
        return Ok(Json((*clip).clone()));
    }
    let (event, line) = find(&hub, id).ok_or_else(|| unknown_event(id))?;
    let clip = hub.studio().recorder.clip(&event, &line);
    clip.map(Json).map_err(|reason| no_clip(id, reason))
}

/// Answers `POST /v1/clips/{event}/pin`: keeps a clip.
pub(crate) async fn pin(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
) -> Result<Json<PinAnswer>, ApiError> {
    hub.ready().await?;
    let id = parse(&id)?;
    if !hub.config.clips.public_pin {
        return Err(ApiError::forbidden(
            "pin_disabled",
            "This server does not let viewers pin clips.",
        ));
    }
    let known = hub.studio().recorder.pinned(id).is_some();
    let pinned = if known {
        true
    } else {
        let (event, line) = find(&hub, id).ok_or_else(|| unknown_event(id))?;
        let done = hub.studio_mut().recorder.pin(&event, &line);
        match done.map_err(|reason| no_clip(id, reason))? {
            Pin::Done => {
                // The clip goes to the storage now, not at the next write
                // of the chronicle.
                crate::pump::flush(&hub).await;
                true
            }
            Pin::Waiting => false,
        }
    };
    Ok(Json(PinAnswer { id, pinned, link: link_of(id) }))
}
