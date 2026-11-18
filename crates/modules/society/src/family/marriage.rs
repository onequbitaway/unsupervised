//! `propose`, `accept_proposal` and `divorce`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, GlobalCtx, NoArgs, Realm, ResidentId, Value};

use super::patch::{KinPatch, send_suit, set_kin};
use super::state::{Families, Proposal};
use crate::params::{self, Params};
use crate::social::{Ask, Suit};
use crate::spec;

/// Arguments of `propose` and `accept_proposal`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct PartnerArgs {
    /// The other resident.
    resident: ResidentId,
}

fn propose(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &PartnerArgs,
) -> Result<Value, ActionError> {
    let mut families = realm.res_mut::<Families>();
    if args.resident == ctx.resident {
        return Err(ActionError::invalid_args("You cannot marry yourself."));
    }
    if !realm.directory().is_alive(args.resident) {
        return Err(ActionError::not_found(format!("No resident has the id {}.", args.resident)));
    }
    if families.spouses.contains_key(&ctx.resident) {
        return Err(ActionError::conflict("You are married."));
    }
    if families.spouses.contains_key(&args.resident) {
        return Err(ActionError::conflict("That resident is married."));
    }
    let expires = realm.frame() + realm.param(p.proposal_ttl).max(1).unsigned_abs();
    // A new proposal replaces the old one of the same resident.
    families.proposals.insert(ctx.resident, Proposal { to: args.resident, expires });
    send_suit(realm, args.resident, Suit { from: ctx.resident, ask: Ask::Marriage, expires }, None);
    Ok(json!({ "to": args.resident, "expires": expires }))
}

fn accept(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &PartnerArgs,
) -> Result<Value, ActionError> {
    let mut families = realm.res_mut::<Families>();
    let open = families
        .proposals
        .get(&args.resident)
        .is_some_and(|proposal| proposal.to == ctx.resident && proposal.expires >= realm.frame());
    if !open {
        return Err(ActionError::not_found("That resident has no open proposal for you."));
    }
    // A resident that marries loses its own open proposal, so only the
    // resident that accepts can be married here. The check covers both.
    if families.spouses.contains_key(&ctx.resident) || families.spouses.contains_key(&args.resident)
    {
        return Err(ActionError::conflict("A married resident cannot marry."));
    }
    families.proposals.remove(&args.resident);
    families.proposals.remove(&ctx.resident);
    families.spouses.insert(ctx.resident, args.resident);
    families.spouses.insert(args.resident, ctx.resident);
    set_kin(realm, ctx.resident, KinPatch::Spouse(Some(args.resident)));
    set_kin(realm, args.resident, KinPatch::Spouse(Some(ctx.resident)));
    realm.notify(args.resident, "society.proposal_accepted", Some(ctx.resident), json!({}));
    realm.emit(
        EventMeta::public("society.married", params::drama(realm, p.drama_married))
            .actor(ctx.resident)
            .at(ctx.pos),
        &json!({ "a": args.resident, "b": ctx.resident }),
    );
    Ok(json!({ "spouse": args.resident }))
}

fn divorce(realm: &Realm, p: &Params, ctx: &GlobalCtx) -> Result<Value, ActionError> {
    let mut families = realm.res_mut::<Families>();
    let Some(spouse) = families.spouses.remove(&ctx.resident) else {
        return Err(ActionError::conflict("You are not married."));
    };
    families.spouses.remove(&spouse);
    set_kin(realm, ctx.resident, KinPatch::Spouse(None));
    set_kin(realm, spouse, KinPatch::Spouse(None));
    realm.notify(spouse, "society.divorce", Some(ctx.resident), json!({}));
    realm.emit(
        EventMeta::public("society.divorced", params::drama(realm, p.drama_divorced))
            .actor(ctx.resident)
            .at(ctx.pos),
        &json!({ "a": ctx.resident, "b": spouse }),
    );
    Ok(json!({ "former_spouse": spouse }))
}

/// Ends the marriage and the proposals of a resident that left the world.
pub(crate) fn on_death(realm: &Realm, families: &mut Families, dead: ResidentId) {
    families.proposals.remove(&dead);
    families.proposals.retain(|_, proposal| proposal.to != dead);
    if let Some(spouse) = families.spouses.remove(&dead) {
        families.spouses.remove(&spouse);
        set_kin(realm, spouse, KinPatch::Spouse(None));
        realm.notify(spouse, "society.widowed", None, json!({ "spouse": dead }));
    }
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<PartnerArgs>(
        spec(
            "propose",
            "Ask a resident to marry you. The resident answers with `accept_proposal`.",
        ),
        move |realm, ctx, args| propose(realm, &p, ctx, args),
    );
    b.global_verb::<PartnerArgs>(
        spec("accept_proposal", "Marry a resident that proposed to you."),
        move |realm, ctx, args| accept(realm, &p, ctx, args),
    );
    b.global_verb::<NoArgs>(
        spec("divorce", "End your marriage. Your spouse cannot stop it."),
        move |realm, ctx, _args| divorce(realm, &p, ctx),
    );
}
