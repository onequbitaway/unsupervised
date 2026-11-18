//! What a resident learns when a request to the global realm is
//! interrupted on its way.
//!
//! Six verbs of this module send a request to the global realm when the
//! action starts, and wait for the answer. When such an action stops in
//! between (the resident cancels it, another module interrupts it, it
//! times out), the request has already left. The global realm does the
//! work anyway, and the kernel can drop its late answer. The action report
//! says `interrupted`, which is true for the action and not for the
//! request. So the resident also gets the notice
//! `politics.request_still_handled`: it must not send the request again,
//! and it finds the outcome in its observation.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, ResidentId, Scope};

/// The verbs that send their request when the action starts and then wait
/// for the global realm.
const WAITING_VERBS: [&str; 6] =
    ["register_candidate", "donate", "campaign", "vote", "start_recall", "sign_recall"];

/// The reason that the kernel gives when the resident left the world.
/// Nobody is there to read a notice then.
const DESPAWNED: &str = "despawned";

/// Registers the handler of interrupted requests.
pub(crate) fn build(b: &mut Builder) {
    if b.scope() != Scope::Zone {
        return;
    }
    b.on_event("kernel.action_interrupted", |realm, event| {
        let data = &event.data;
        let Some(verb) = data["verb"].as_str().filter(|verb| WAITING_VERBS.contains(verb)) else {
            return;
        };
        if data["reason"].as_str() == Some(DESPAWNED) {
            return;
        }
        let Some(resident) = event.actor.or_else(|| data["resident"].as_u64().map(ResidentId))
        else {
            return;
        };
        realm.notify(
            resident,
            "politics.request_still_handled",
            None,
            json!({ "verb": verb, "action_id": data["action_id"], "reason": data["reason"] }),
        );
    });
}
