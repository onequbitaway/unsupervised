//! Verbs that start in the zone of the resident and end in the case
//! registry of the global realm.
//!
//! The zone part checks what only the zone knows: the place, the cash and
//! the powers of the resident (`realm.has_perm`). The global part checks
//! the registry and makes the change. The answer goes back with
//! `Realm::complete_action`, so the resident waits one frame.
//!
//! Cash that the zone part takes goes by wire to a holding escrow in the
//! global realm, `Holder::Escrow(action id)`. The global part moves what
//! it accepts. The helper sends back what is left, so a refused request
//! keeps no money.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use unsup_kernel::{
    ActionCtx, ActionError, Args, Begin, Builder, CityId, Dest, Holder, Message, Money, Realm,
    ResidentId, Scope, Value, VerbSpec,
};

use crate::ids::Handles;

/// Ledger label of cash on its way to the holding escrow.
const HOLD: &str = "crime.hold";
/// Ledger label of cash that comes back from the holding escrow.
const RETURN: &str = "crime.return";

/// The global part of a verb: what the zone sends to the registry.
pub(crate) trait Call: Serialize + DeserializeOwned + 'static {
    /// Unique name of the message: `crime.<verb>`.
    const KIND: &'static str;
}

/// A request of a resident on its way from its zone to the registry.
#[derive(Serialize, Deserialize)]
pub(crate) struct Request<C> {
    origin: Origin,
    call: C,
}

impl<C: Call> Message for Request<C> {
    const KIND: &'static str = C::KIND;
}

/// What the global part knows about the resident that asked.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct Origin {
    /// The resident that acts.
    pub(crate) resident: ResidentId,
    /// Id of the action. The holding escrow has the same number.
    pub(crate) action_id: u64,
    /// The city in which the resident was when it asked.
    pub(crate) city: CityId,
}

impl Origin {
    /// Returns the holding escrow of the request.
    pub(crate) fn escrow(&self) -> Holder {
        Holder::Escrow(self.action_id)
    }
}

/// Sends what is left in the holding escrow of a request back to the
/// resident. It runs in the global realm.
fn return_rest(realm: &Realm, origin: &Origin) {
    let rest = realm.ledger().money(origin.escrow());
    if rest.is_positive() {
        // The escrow holds `rest`, so the wire cannot fail.
        let _ = realm.wire_money(
            origin.escrow(),
            Dest::Resident(origin.resident),
            Holder::Resident(origin.resident),
            rest,
            RETURN,
        );
    }
}

/// Registers a verb with a zone part and a global part.
///
/// `prepare` runs in the zone when the action starts. It must not change
/// anything. It returns the call for the registry and the cents to take
/// from the resident. `run` runs in the global realm in the same frame.
/// Its result is the result of the action.
pub(crate) fn remote_verb<A, C>(
    b: &mut Builder,
    h: Handles,
    spec: VerbSpec,
    prepare: impl Fn(&Realm, &ActionCtx, &A) -> Result<(C, i64), ActionError> + Send + Sync + 'static,
    run: impl Fn(&Realm, &Origin, C) -> Result<Value, ActionError> + Send + Sync + 'static,
) where
    A: Args,
    C: Call,
{
    match b.scope() {
        Scope::Zone => b.verb::<A>(
            spec,
            move |realm, ctx, args| {
                let Some(zone) = realm.zone_info() else {
                    return Err(ActionError::internal("This verb starts in a zone."));
                };
                let (call, held) = prepare(realm, ctx, args)?;
                let origin =
                    Origin { resident: ctx.resident, action_id: ctx.action_id, city: zone.city };
                if held > 0 {
                    realm.wire_money(
                        Holder::Resident(ctx.resident),
                        Dest::Global,
                        origin.escrow(),
                        Money(held),
                        HOLD,
                    )?;
                }
                realm.send(Dest::Global, &Request { origin, call });
                Ok(Begin::Wait { timeout: h.ticks(realm, h.p.global_timeout) })
            },
            |_realm, _ctx, _args| Ok(Value::Null),
        ),
        Scope::Global => b.on_message::<Request<C>>(move |realm, _from, request| {
            let result = run(realm, &request.origin, request.call);
            return_rest(realm, &request.origin);
            realm.complete_action(request.origin.resident, request.origin.action_id, result);
        }),
    }
}
