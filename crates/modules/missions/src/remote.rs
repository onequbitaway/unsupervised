//! Verbs that start in the zone of the resident and end in the global
//! realm.
//!
//! The zone part checks what only the zone knows (cash, the copy of the
//! memberships) and can take cash from the resident. The global part checks
//! the missions and makes the change. The answer goes back with
//! `Realm::complete_action`.
//!
//! Cash that the zone part takes goes by wire to a holding escrow in the
//! global realm, `Holder::Escrow(action_id)`. The global part moves what it
//! accepts. The helper sends back whatever is left, so a refused request
//! never keeps money.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use unsup_kernel::{
    ActionCtx, ActionError, Args, Begin, Builder, CityId, Dest, Holder, Message, Money, OwnerId,
    Realm, ResidentId, Scope, Value, VerbSpec,
};

use crate::params::{self, Params};
use crate::pay::{HOLD, RETURN};

/// The global part of a verb: what the zone sends to the global realm.
pub(crate) trait Call: Serialize + DeserializeOwned + 'static {
    /// Unique name of the message: `missions.<verb>`.
    const KIND: &'static str;
}

/// A request of a resident on its way from its zone to the global realm.
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
    /// The owner of the resident.
    pub(crate) owner: OwnerId,
    /// Id of the action. The holding escrow has the same number.
    pub(crate) action_id: u64,
    /// The city in which the resident was when it asked.
    pub(crate) city: CityId,
    /// Cents that the zone part put in the holding escrow.
    pub(crate) held: i64,
}

impl Origin {
    /// Returns the holding escrow of the request.
    pub(crate) fn escrow(&self) -> Holder {
        Holder::Escrow(self.action_id)
    }
}

/// Sends what is left in the holding escrow of a request back to the
/// resident. Call it in the global realm.
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
/// anything. It returns the call for the global realm and the cents to take
/// from the resident. `run` runs in the global realm in the same frame. Its
/// result is the result of the action.
pub(crate) fn remote_verb<A, C>(
    b: &mut Builder,
    spec: VerbSpec,
    p: Params,
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
                let (call, hold) = prepare(realm, ctx, args)?;
                let Some(zone) = realm.zone_info() else {
                    return Err(ActionError::internal("This verb starts in a zone."));
                };
                let origin = Origin {
                    resident: ctx.resident,
                    owner: realm.identity(ctx.idx).owner,
                    action_id: ctx.action_id,
                    city: zone.city,
                    held: hold,
                };
                if hold > 0 {
                    realm.wire_money(
                        Holder::Resident(ctx.resident),
                        Dest::Global,
                        origin.escrow(),
                        Money(hold),
                        HOLD,
                    )?;
                }
                realm.send(Dest::Global, &Request { origin, call });
                Ok(Begin::Wait { timeout: params::span(realm, p.global_timeout) })
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
