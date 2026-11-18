//! The end of a bill: it becomes law, or it fails.

use unsup_kernel::serde_json::json;
use unsup_kernel::{EventMeta, Realm, ResidentId};

use super::{ballot, facts_with};
use crate::offices::{self, GOV_VOTE};
use crate::params::{self, Params};
use crate::state::Politics;
use crate::state::bill::{Bill, How, Law, Stage, Why};

/// Closes a bill that failed: the public event, the notice for the sponsor
/// and the entry in the register.
fn close_rejected(
    realm: &Realm,
    p: &Params,
    state: &mut Politics,
    mut bill: Bill,
    why: Why,
) -> Stage {
    let members = offices::members_with(realm, bill.gov.org(), GOV_VOTE);
    let (yes, no) = ballot::tally(&bill, &members);
    bill.stage = Stage::Rejected(why);
    bill.deadline = realm.frame();
    let facts = facts_with(
        &bill,
        json!({
            "why": why.name(),
            "yes": yes,
            "no": no,
            "members": members.len(),
            "sponsor": bill.sponsor,
        }),
    );
    realm.notify(bill.sponsor, "politics.bill_rejected", None, facts.clone());
    realm.emit(
        EventMeta::public("politics.bill_rejected", params::drama(realm, p.drama_bill_rejected)),
        &facts,
    );
    let stage = bill.stage;
    state.file(bill, params::count(realm, p.register_max));
    stage
}

/// Ends an open bill without effect. Returns the stage in which the
/// register has it, or `None` when no open bill has the id.
pub(crate) fn reject(
    realm: &Realm,
    p: &Params,
    state: &mut Politics,
    id: u64,
    why: Why,
) -> Option<Stage> {
    let bill = state.bills.remove(&id)?;
    Some(close_rejected(realm, p, state, bill, why))
}

/// Makes an open bill law: the parameter gets the new value for the nation
/// or for the city of the bill. The global realm sees the value at once
/// and the zones see it in the next frame.
///
/// Returns the stage in which the register has the bill, or `None` when no
/// open bill has the id.
pub(crate) fn enact(
    realm: &Realm,
    p: &Params,
    state: &mut Politics,
    id: u64,
    how: How,
) -> Option<Stage> {
    enact_by(realm, p, state, id, how, None)
}

/// Like [`enact`], for a bill that the executive `signer` signed.
pub(crate) fn enact_by(
    realm: &Realm,
    p: &Params,
    state: &mut Politics,
    id: u64,
    how: How,
    signer: Option<ResidentId>,
) -> Option<Stage> {
    let mut bill = state.bills.remove(&id)?;
    let defs = realm.defs();
    // A newer set of modules can have retired the parameter.
    let Some(param) = defs.param_id(&bill.param).filter(|&param| defs.param(param).policy) else {
        return Some(close_rejected(realm, p, state, bill, Why::NoParameter));
    };
    let gov = bill.gov;
    let old = params::value(realm, param, gov);
    let value = realm.set_param(param, bill.value, gov.city());
    let frame = realm.frame();
    bill.value = value;
    bill.stage = Stage::Enacted(how);
    bill.deadline = frame;
    let law = Law { bill: id, value, old, enacted: frame, how };
    state.laws.entry(gov.org()).or_default().insert(bill.param.clone(), law);
    let facts = facts_with(
        &bill,
        json!({ "old": old, "how": how.name(), "sponsor": bill.sponsor, "signed_by": signer }),
    );
    realm.notify(bill.sponsor, "politics.law_enacted", signer, facts.clone());
    let drama = if how == How::Override { p.drama_law_override } else { p.drama_law_enacted };
    let mut event = EventMeta::public("politics.law_enacted", params::drama(realm, drama));
    if let Some(signer) = signer {
        event = event.actor(signer);
    }
    realm.emit(event, &facts);
    let stage = bill.stage;
    state.file(bill, params::count(realm, p.register_max));
    Some(stage)
}
