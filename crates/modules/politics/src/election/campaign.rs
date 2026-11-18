//! The verb `campaign`: a rally.
//!
//! 1. The zone checks the amount and asks the global realm.
//! 2. The global realm checks the candidacy and pays the cost from the
//!    campaign fund to the treasury of the city of the rally.
//! 3. The candidate is busy for `politics.campaign_minutes` game minutes.
//!    The global realm counts them in frames.
//! 4. Then the zone of the candidate holds the rally: a public event, with
//!    a notice for each resident nearby. More money reaches more tiles.
//!
//! A candidate that stops the action gets no rally. The money is spent.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Audience, Begin, Builder, Dest, EventMeta, Holder, Message, OrgId, Realm, Scope,
    Value,
};

use super::{find, fund, require_place, wrong_phase};
use crate::model::{Gov, Office};
use crate::params::{self, Params};
use crate::remote::{Origin, origin_of, spec};
use crate::state::election::Phase;
use crate::state::{Politics, Rally};
use crate::{pay, text};

/// Ledger label of the cost of a rally.
const COST: &str = "politics.campaign";

/// Arguments of `campaign`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct CampaignArgs {
    /// Id of an election in its campaign phase in which you are a
    /// candidate. For a city office you must be in that city.
    pub(crate) election: u64,
    /// Cents of your campaign fund to spend on this rally. More money
    /// reaches residents that are farther away.
    pub(crate) spend: i64,
}

/// The request of a candidate in a zone to the global realm.
#[derive(Serialize, Deserialize)]
struct Ask {
    origin: Origin,
    election: u64,
    spend: i64,
}

impl Message for Ask {
    const KIND: &'static str = "politics.campaign";
}

/// The word of the global realm to the zone of a candidate: the time of
/// the rally is over.
#[derive(Serialize, Deserialize)]
struct Held {
    action_id: u64,
    election: u64,
    gov: Gov,
    office: Office,
    platform: String,
    spend: i64,
    radius: u32,
}

impl Message for Held {
    const KIND: &'static str = "politics.rally_held";
}

/// Returns the reach of a rally in tiles for the cents that it costs.
fn reach(realm: &Realm, p: &Params, spend: i64) -> u32 {
    let base = realm.param(p.campaign_radius_base).max(0);
    let per_tile = realm.param(p.campaign_cents_per_tile).max(1);
    let max = realm.param(p.campaign_radius_max).max(0);
    u32::try_from(base.saturating_add(spend / per_tile).min(max)).unwrap_or(0)
}

/// The global part: checks the candidacy, pays the cost and starts the
/// time of the rally.
fn start(realm: &Realm, p: &Params, state: &mut Politics, ask: &Ask) -> Result<(), ActionError> {
    let origin = &ask.origin;
    let election = find(state, ask.election)?;
    if election.phase != Phase::Campaign {
        return Err(wrong_phase(election, "Candidates campaign in the campaign phase."));
    }
    require_place(election, origin)?;
    let (gov, office) = (election.gov, election.office);
    let Some(candidate) =
        state.elections.get_mut(&ask.election).and_then(|e| e.candidates.get_mut(&origin.resident))
    else {
        return Err(ActionError::not_allowed("You are not a candidate in this election."));
    };
    let available = fund::balance(realm, candidate);
    if available < ask.spend {
        return Err(ActionError::insufficient("Your campaign fund does not have that much.")
            .with_details(json!({ "needed": ask.spend, "available": available })));
    }
    // The cost goes to the city in which the rally is.
    let venue = Holder::Org(OrgId::city(origin.city));
    let paid = pay::transfer(realm, fund::holder(candidate), venue, ask.spend, COST);
    candidate.spent += paid;
    let rally = Rally {
        resident: origin.resident,
        election: ask.election,
        gov,
        office,
        platform: candidate.platform.clone(),
        due: realm.frame() + u64::from(params::span(realm, p.campaign_minutes).max(1)),
        spend: paid,
        radius: reach(realm, p, paid),
    };
    state.rallies.insert(origin.action_id, rally);
    state.touch_election(ask.election);
    Ok(())
}

/// Ends the rallies whose time is over: the zone of each candidate gets
/// the word. A global system calls it every frame.
pub(crate) fn end_due(realm: &Realm) {
    let frame = realm.frame();
    let mut state = realm.res_mut::<Politics>();
    let due: Vec<u64> =
        state.rallies.iter().filter(|(_, rally)| rally.due <= frame).map(|(&id, _)| id).collect();
    for action_id in due {
        let Some(rally) = state.rallies.remove(&action_id) else {
            continue;
        };
        realm.send(
            Dest::Resident(rally.resident),
            &Held {
                action_id,
                election: rally.election,
                gov: rally.gov,
                office: rally.office,
                platform: rally.platform,
                spend: rally.spend,
                radius: rally.radius,
            },
        );
    }
}

/// The zone part of the end: the public event, the notices and the result
/// of the action.
fn hold(realm: &Realm, p: &Params, idx: unsup_kernel::Idx, held: Held) {
    // A candidate that stopped the action holds no rally.
    if !realm.activity(idx).is_some_and(|activity| activity.action_id == held.action_id) {
        return;
    }
    let candidate = realm.id_of(idx);
    let pos = realm.pos(idx);
    let heard = realm.nearby(pos, held.radius).into_iter().filter(|&other| other != idx).count();
    realm.emit(
        EventMeta::public("politics.rally_held", params::drama(realm, p.drama_rally))
            .actor(candidate)
            .at(pos)
            .audience(Audience::Nearby(held.radius)),
        &held.gov.describe_with(json!({
            "election": held.election,
            "office": held.office.name(),
            "candidate": candidate,
            "platform": text::untrusted(candidate, &held.platform),
            "spend": held.spend,
            "radius": held.radius,
            "heard": heard,
        })),
    );
    let result = json!({
        "election": held.election,
        "spend": held.spend,
        "radius": held.radius,
        "heard": heard,
    });
    realm.complete_action(candidate, held.action_id, Ok(result));
}

/// Registers the verb, the messages and the handlers.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.verb::<CampaignArgs>(
        spec(
            "campaign",
            "Hold a rally where you stand. It takes time and money of your campaign fund. Residents nearby get your platform.",
        ),
        move |realm, ctx, args| {
            let least = realm.param(p.campaign_cost_min);
            if args.spend < least {
                return Err(ActionError::invalid_args(format!(
                    "A rally costs {least} cents or more."
                ))
                .with_details(json!({ "min": least })));
            }
            let origin = origin_of(realm, ctx, 0)?;
            realm.send(Dest::Global, &Ask { origin, election: args.election, spend: args.spend });
            // The global realm answers a refusal in the next frame and the
            // end of the rally after its time.
            let timeout = params::span(realm, p.campaign_minutes)
                .saturating_add(params::span(realm, p.global_timeout));
            Ok(Begin::Wait { timeout: timeout.max(1) })
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    b.on_resident_message::<Held>(move |realm, idx, _from, held| hold(realm, &p, idx, held));
    if b.scope() == Scope::Global {
        b.on_message::<Ask>(move |realm, _from, ask| {
            let mut state = realm.res_mut::<Politics>();
            if let Err(error) = start(realm, &p, &mut state, &ask) {
                realm.complete_action(ask.origin.resident, ask.origin.action_id, Err(error));
            }
        });
    }
}
