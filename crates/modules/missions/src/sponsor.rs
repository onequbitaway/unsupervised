//! Missions of human sponsors.
//!
//! A sponsor is not a resident. The server sends its mission to the global
//! realm as a typed message (`SimHandle::send_global`). The reward is new
//! money from the faucet `sponsor.reward`, up to a limit for each game day.
//! The module answers with a public event. A reward that the ledger cannot
//! create (the supply cap of the kernel) refuses the mission.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, ErrorCode, EventMeta, LedgerError, Message, Money, Realm, TICKS_PER_DAY,
};

use crate::model::{Mission, Missions, Poster};
use crate::params::{self, Params};
use crate::spec::MissionSpec;
use crate::{check, engine, post};

/// A mission that a human sponsor publishes. The server sends it to the
/// global realm with `SimHandle::send_global`.
///
/// The module answers in the next frame with one public event:
/// `missions.mission_posted` with `request` and the id of the mission, or
/// `missions.sponsor_refused` with `request`, `code` and `message`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SponsorMission {
    /// A number that the sender chooses. The answer carries it.
    pub request: u64,
    /// The display name of the sponsor. A human wrote it.
    pub sponsor: String,
    /// The definition of the mission.
    pub mission: MissionSpec,
}

impl Message for SponsorMission {
    const KIND: &'static str = "missions.sponsor_mission";
}

/// The refusal of a mission whose reward the ledger did not create.
fn not_created(total: i64, error: LedgerError) -> ActionError {
    match error {
        // The faucets of the global realm are at the supply cap.
        LedgerError::Overflow => ActionError::conflict(format!(
            "The world cannot create {total} cents now: the money supply is at its limit. Try again later."
        )),
        other => other.into(),
    }
}

/// Checks a sponsor mission, creates its reward and opens it. A mission
/// whose reward the ledger cannot create is refused and changes nothing.
fn publish(realm: &Realm, p: &Params, message: &SponsorMission) -> Result<(), ActionError> {
    check::sponsor_name(realm, p, &message.sponsor)?;
    let def = check::definition(realm, p, &message.mission)?;
    let mut state = realm.res_mut::<Missions>();
    if state.open_count() >= params::count(realm, p.open_max) {
        return Err(post::world_full(realm, p));
    }
    let now = realm.frame();
    let day = now / TICKS_PER_DAY;
    let used = state.sponsor_minted_on(day);
    let limit = realm.param(p.sponsor_daily_limit);
    let total = def.total();
    if total > limit.saturating_sub(used) {
        return Err(ActionError::new(
            ErrorCode::RateLimited,
            format!(
                "The mission needs {total} cents. The sponsor faucet has {} of {limit} cents left today.",
                limit.saturating_sub(used).max(0)
            ),
        ));
    }
    let id = realm.alloc_id();
    // The ledger creates the whole reward or nothing. Without the money
    // there is no mission, and nothing counts against the day.
    realm
        .ledger_mut()
        .try_mint(p.sponsor_faucet, Missions::escrow(id), Money(total))
        .map_err(|error| not_created(total, error))?;
    state.sponsor_day = day;
    state.sponsor_minted = used + total;
    let deadline = now + u64::from(def.deadline_in);
    let mission = Mission {
        id,
        poster: Poster::Sponsor { name: message.sponsor.clone() },
        def,
        posted: now,
        deadline,
        city: None,
        takers: Default::default(),
    };
    engine::open(realm, p, &mut state, mission, Some(message.request));
    Ok(())
}

/// Handles a [`SponsorMission`]. A refusal opens no mission, creates no
/// money and uses nothing of the limit of the day. It is told with the
/// public event `missions.sponsor_refused`.
pub(crate) fn on_message(realm: &Realm, p: &Params, message: &SponsorMission) {
    if let Err(error) = publish(realm, p, message) {
        realm.emit(
            EventMeta::public("missions.sponsor_refused", params::drama(realm, p.drama_routine)),
            &json!({
                "request": message.request,
                "code": error.code.as_str(),
                "message": error.message,
            }),
        );
    }
}
