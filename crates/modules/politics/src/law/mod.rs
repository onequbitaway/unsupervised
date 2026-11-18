//! Laws. A law is data: a policy parameter and a value for one government.
//!
//! A member proposes a bill. The members vote. The executive signs or
//! vetoes. The members can override a veto. An enacted bill sets the
//! parameter with `Realm::set_param`, and the simulation obeys.

pub(crate) mod ballot;
pub(crate) mod enact;
pub(crate) mod propose;
pub(crate) mod sign;

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ActionError, Builder, Realm, Scope, Stage, Value};

use crate::offices::{self, GOV_SIGN};
use crate::params::Params;
use crate::state::Politics;
use crate::state::bill::{Bill, How, Stage as BillStage};
use crate::text;

/// Order of the system that ends the stages of the bills. It runs after
/// the calendar, so that it sees the offices of this frame.
const ORDER_BILLS: i32 = 10;

/// Returns the fields that name a bill in an event, a notice or a result:
/// the government, `bill`, `param` and `value`.
pub(crate) fn facts(bill: &Bill) -> Map<String, Value> {
    let mut out = bill.gov.describe();
    out.insert("bill".into(), json!(bill.id));
    out.insert("param".into(), json!(bill.param));
    out.insert("value".into(), json!(bill.value));
    out
}

/// Like [`facts`], with more fields.
pub(crate) fn facts_with(bill: &Bill, more: Value) -> Value {
    let mut out = facts(bill);
    if let Value::Object(more) = more {
        out.extend(more);
    }
    Value::Object(out)
}

/// Returns the reason of a bill as untrusted text, or null.
pub(crate) fn reason_of(bill: &Bill) -> Value {
    bill.reason.as_deref().map_or(Value::Null, |reason| text::untrusted(bill.sponsor, reason))
}

/// Finds an open bill.
pub(crate) fn find(state: &Politics, id: u64) -> Result<&Bill, ActionError> {
    state
        .bills
        .get(&id)
        .ok_or_else(|| ActionError::not_found(format!("No open bill has the id {id}.")))
}

/// The error of a verb that came in the wrong stage of a bill.
pub(crate) fn wrong_stage(bill: &Bill, need: &str) -> ActionError {
    ActionError::conflict(format!("This bill is in the {} stage. {need}", bill.stage.name()))
        .with_details(json!({ "stage": bill.stage.name(), "deadline": bill.deadline }))
}

/// Ends the stage of every bill whose time is over. A bill that waits for
/// the executive also becomes law when no resident can sign it. A global
/// system calls it every frame.
fn run(realm: &Realm, p: &Params) {
    let frame = realm.frame();
    let mut state = realm.res_mut::<Politics>();
    let ids: Vec<u64> = state.bills.keys().copied().collect();
    for id in ids {
        let Some((stage, gov, late)) =
            state.bills.get(&id).map(|bill| (bill.stage, bill.gov, frame >= bill.deadline))
        else {
            continue;
        };
        match stage {
            BillStage::Vote | BillStage::Override if late => {
                ballot::conclude(realm, p, &mut state, id);
            }
            BillStage::Sign if late => {
                enact::enact(realm, p, &mut state, id, How::Unsigned);
            }
            BillStage::Sign if offices::members_with(realm, gov.org(), GOV_SIGN).is_empty() => {
                enact::enact(realm, p, &mut state, id, How::Vacant);
            }
            _ => {}
        }
    }
}

/// Registers the verbs and the system of the laws.
pub(crate) fn build(b: &mut Builder, p: Params) {
    propose::build(b, p);
    ballot::build(b, p);
    sign::build(b, p);
    if b.scope() == Scope::Global {
        b.system(Stage::Update, ORDER_BILLS, "politics.bills", move |realm| run(realm, &p));
    }
}
