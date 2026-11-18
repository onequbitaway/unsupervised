//! `household_invite`, `household_join` and `household_leave`.
//!
//! A household has a host and a home. The host owns or rents the home; the
//! economy module writes that. A resident that joins the household becomes
//! a tenant of the home and gets the household id in its `Kin` component.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Builder, EventMeta, GlobalCtx, Holder, NoArgs, Realm, ResidentId, Site, SiteId,
    Value,
};

use super::patch::{KinPatch, send_suit, set_kin, set_tenant};
use super::state::{Families, Household};
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb};
use crate::social::{Ask, Suit};
use crate::{public, spec};

/// The kinds of site in which a household can live.
const HOME_KINDS: [&str; 2] = ["home", "apartment"];

/// Arguments of `household_invite`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct InviteArgs {
    /// The resident that you invite to live in your home.
    resident: ResidentId,
}

/// Arguments of `household_join`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct JoinArgs {
    /// The host that invited you.
    host: ResidentId,
}

/// The global part of `household_invite`.
#[derive(Serialize, Deserialize)]
struct Invite {
    guest: ResidentId,
    site: SiteId,
}

impl Call for Invite {
    const KIND: &'static str = "society.household_invite";
}

/// Returns true when a resident lives in a site: the site is a home and the
/// resident owns it or is a tenant.
fn lives_in(site: &Site, resident: ResidentId) -> bool {
    HOME_KINDS.contains(&site.kind.as_str())
        && (site.owner == Some(Holder::Resident(resident)) || site.is_tenant(resident))
}

fn invite(realm: &Realm, p: &Params, origin: &Origin, call: Invite) -> Result<Value, ActionError> {
    let mut families = realm.res_mut::<Families>();
    let host = origin.resident;
    let frame = realm.frame();
    if call.guest == host {
        return Err(ActionError::invalid_args("You cannot invite yourself."));
    }
    if !realm.directory().is_alive(call.guest) {
        return Err(ActionError::not_found(format!("No resident has the id {}.", call.guest)));
    }
    let home = (origin.zone, call.site);
    let known = families.household_of.get(&host).copied();
    if let Some(household) = known.and_then(|id| families.households.get(&id)) {
        if household.head != host {
            return Err(ActionError::conflict("Only the host of your household can invite."));
        }
        if household.members.contains(&call.guest) {
            return Err(ActionError::conflict("That resident lives in your household already."));
        }
        if (household.zone, household.site) != home && household.members.len() > 1 {
            return Err(ActionError::conflict(format!(
                "Your household lives in site {}. Invite from there.",
                household.site
            )));
        }
        let open = household.invited.values().filter(|&&expires| expires >= frame).count();
        let max = params::count(realm, p.household_members_max);
        if household.members.len() + open >= max {
            return Err(ActionError::conflict(format!(
                "A household has at most {max} members, with the open invitations."
            )));
        }
    }
    let id = match known {
        Some(id) => id,
        None => {
            let id = realm.alloc_id();
            families.household_of.insert(host, id);
            set_kin(realm, host, KinPatch::Household(Some(id)));
            id
        }
    };
    let household = families.households.entry(id).or_insert_with(|| Household {
        head: host,
        zone: home.0,
        site: home.1,
        members: BTreeSet::from([host]),
        invited: BTreeMap::new(),
    });
    (household.zone, household.site) = home;
    household.invited.retain(|_, expires| *expires >= frame);
    let expires = frame + realm.param(p.proposal_ttl).max(1).unsigned_abs();
    household.invited.insert(call.guest, expires);
    send_suit(
        realm,
        call.guest,
        Suit { from: host, ask: Ask::Household, expires },
        Some(call.site),
    );
    Ok(json!({ "household": id, "invited": call.guest, "site": call.site, "expires": expires }))
}

fn join(realm: &Realm, p: &Params, ctx: &GlobalCtx, args: &JoinArgs) -> Result<Value, ActionError> {
    let mut families = realm.res_mut::<Families>();
    let me = ctx.resident;
    let frame = realm.frame();
    let found = families.household_of.get(&args.host).and_then(|id| {
        let household = families.households.get(id)?;
        let invited = household.invited.get(&me).is_some_and(|&expires| expires >= frame);
        (household.head == args.host && invited).then_some((*id, household))
    });
    let Some((id, household)) = found else {
        return Err(ActionError::not_allowed(
            "You need an invitation of the host. The host uses `household_invite` in its home.",
        ));
    };
    if families.household_of.contains_key(&me) {
        return Err(ActionError::conflict("You are in a household. Use `household_leave` first."));
    }
    let max = params::count(realm, p.household_members_max);
    if household.members.len() >= max {
        return Err(ActionError::conflict(format!("A household has at most {max} members.")));
    }
    let (zone, site) = (household.zone, household.site);
    if let Some(household) = families.households.get_mut(&id) {
        household.invited.remove(&me);
        household.members.insert(me);
    }
    families.household_of.insert(me, id);
    set_tenant(realm, zone, site, me, true);
    set_kin(realm, me, KinPatch::Household(Some(id)));
    realm.notify(args.host, "society.household_joined", Some(me), json!({ "household": id }));
    realm.emit(
        EventMeta::public("society.household_joined", params::drama(realm, p.drama_household))
            .actor(me)
            .at(ctx.pos),
        &json!({ "household": id, "resident": me, "host": args.host, "site": site }),
    );
    Ok(json!({ "household": id, "host": args.host, "site": site, "zone": zone }))
}

/// Ends a household. Every member but the host stops being a tenant of the
/// home. The host keeps the home: its right comes from the economy module.
fn dissolve(realm: &Realm, p: &Params, families: &mut Families, id: u64, cause: &str) {
    let Some(household) = families.households.remove(&id) else {
        return;
    };
    for &member in &household.members {
        families.household_of.remove(&member);
        set_kin(realm, member, KinPatch::Household(None));
        if member != household.head {
            set_tenant(realm, household.zone, household.site, member, false);
            realm.notify(
                member,
                "society.household_dissolved",
                Some(household.head),
                json!({ "household": id, "cause": cause }),
            );
        }
    }
    realm.emit(
        public("society.household_dissolved", params::drama(realm, p.drama_household)),
        &json!({ "household": id, "host": household.head, "cause": cause }),
    );
}

/// Takes one member that is not the host out of a household.
fn remove_member(realm: &Realm, p: &Params, families: &mut Families, id: u64, member: ResidentId) {
    families.household_of.remove(&member);
    let Some(household) = families.households.get_mut(&id) else {
        return;
    };
    household.members.remove(&member);
    set_kin(realm, member, KinPatch::Household(None));
    set_tenant(realm, household.zone, household.site, member, false);
    realm.notify(
        household.head,
        "society.household_left",
        Some(member),
        json!({ "household": id }),
    );
    realm.emit(
        public("society.household_left", params::drama(realm, p.drama_household)),
        &json!({ "household": id, "resident": member, "host": household.head }),
    );
}

/// Takes a resident out of its household. The household ends when the
/// resident is the host. Returns the id of the household.
fn depart(
    realm: &Realm,
    p: &Params,
    families: &mut Families,
    who: ResidentId,
    cause: &str,
) -> Option<u64> {
    let id = families.household_of.get(&who).copied()?;
    if families.households.get(&id).is_some_and(|household| household.head == who) {
        dissolve(realm, p, families, id, cause);
    } else {
        remove_member(realm, p, families, id, who);
    }
    Some(id)
}

/// Takes a resident that left the world out of its household.
pub(crate) fn on_death(realm: &Realm, p: &Params, families: &mut Families, dead: ResidentId) {
    depart(realm, p, families, dead, "death");
}

/// The economy module took a home from a resident (an eviction or a sale).
/// A household that lives there with that resident as host ends. A member
/// that is not the host leaves the household.
pub(crate) fn on_home_lost(realm: &Realm, p: &Params, resident: ResidentId, site: SiteId) {
    let mut families = realm.res_mut::<Families>();
    let lives_there = families
        .household_of
        .get(&resident)
        .and_then(|id| families.households.get(id))
        .is_some_and(|household| household.site == site);
    if lives_there {
        depart(realm, p, &mut families, resident, "home_lost");
    }
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<InviteArgs, Invite>(
        b,
        spec(
            "household_invite",
            "Invite a resident to live in your home. Stand in a home or an apartment that you own or rent.",
        ),
        p.global_timeout,
        |realm, ctx, args| {
            let sites = realm.sites();
            let home = sites.at(realm.pos(ctx.idx)).filter(|site| lives_in(site, ctx.resident));
            let Some(home) = home else {
                return Err(ActionError::not_here(
                    "Stand inside a home or an apartment that you own or rent.",
                ));
            };
            Ok((Invite { guest: args.resident, site: home.id }, 0))
        },
        move |realm, origin, call| invite(realm, &p, origin, call),
    );
    b.global_verb::<JoinArgs>(
        spec(
            "household_join",
            "Join the household of a host that invited you. You become a tenant of its home.",
        ),
        move |realm, ctx, args| join(realm, &p, ctx, args),
    );
    b.global_verb::<NoArgs>(
        spec("household_leave", "Leave your household. When the host leaves, the household ends."),
        move |realm, ctx, _args| {
            let mut families = realm.res_mut::<Families>();
            match depart(realm, &p, &mut families, ctx.resident, "left") {
                Some(id) => Ok(json!({ "household": id })),
                None => Err(ActionError::conflict("You are not in a household.")),
            }
        },
    );
}
