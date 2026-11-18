//! How the global realm writes family state to the zones: the component
//! `Kin` of a resident, the tenants of a home, and the copy of an open
//! proposal for the observation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, Dest, Kin, Message, Realm, ResidentId, Resource, SiteId, ZoneId};

use crate::params::{self, Params};
use crate::social::{Ask, Social, Suit};

/// A change of the family links of a resident, sent to its zone.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) enum KinPatch {
    /// The new spouse, or none.
    Spouse(Option<ResidentId>),
    /// The new household, or none.
    Household(Option<u64>),
}

impl Message for KinPatch {
    const KIND: &'static str = "society.kin";
}

/// A change of the tenants of a home, sent to its zone.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Tenancy {
    site: SiteId,
    resident: ResidentId,
    add: bool,
}

impl Message for Tenancy {
    const KIND: &'static str = "society.tenancy";
}

/// A proposal or an invitation for a resident, sent to its zone.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct SuitNote {
    suit: Suit,
    /// The home, for an invitation to a household.
    site: Option<SiteId>,
}

impl Message for SuitNote {
    const KIND: &'static str = "society.suit";
}

/// The tenants of the sites of one zone that are there because they joined
/// a household. A tenant with a lease of its own is not in the list, so
/// that the end of a household never ends a lease.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lodgers {
    added: BTreeSet<(SiteId, ResidentId)>,
}

impl Resource for Lodgers {
    const NAME: &'static str = "society.lodgers";
}

/// Tells the zone of a resident to change its family links.
pub(crate) fn set_kin(realm: &Realm, resident: ResidentId, patch: KinPatch) {
    realm.send(Dest::Resident(resident), &patch);
}

/// Tells the zone of a home to add or remove a tenant.
pub(crate) fn set_tenant(
    realm: &Realm,
    zone: ZoneId,
    site: SiteId,
    resident: ResidentId,
    add: bool,
) {
    realm.send(Dest::Zone(zone), &Tenancy { site, resident, add });
}

/// Tells a resident about a proposal or an invitation.
pub(crate) fn send_suit(realm: &Realm, to: ResidentId, suit: Suit, site: Option<SiteId>) {
    realm.send(Dest::Resident(to), &SuitNote { suit, site });
}

/// Registers the zone handlers.
pub(crate) fn build_zone(b: &mut Builder, p: Params) {
    b.resource::<Lodgers>();
    b.on_resident_message::<KinPatch>(|realm, idx, _from, patch| {
        let done = match patch {
            KinPatch::Spouse(spouse) => {
                realm.col_mut::<Kin>()[idx].spouse = spouse;
                spouse.map(|_| Ask::Marriage)
            }
            KinPatch::Household(household) => {
                realm.col_mut::<Kin>()[idx].household = household;
                household.map(|_| Ask::Household)
            }
        };
        // The question is answered: its copies are of no use now.
        if let Some(ask) = done
            && realm.col::<Social>()[idx].get().is_some_and(|state| !state.suits.is_empty())
        {
            realm.col_mut::<Social>()[idx].make().suits.retain(|suit| suit.ask != ask);
        }
    });
    b.on_message::<Tenancy>(|realm, _from, change| {
        let key = (change.site, change.resident);
        let mut lodgers = realm.res_mut::<Lodgers>();
        let mut sites = realm.sites_mut();
        match sites.get_mut(change.site) {
            Some(site) if change.add => {
                if !site.is_tenant(change.resident) {
                    site.add_tenant(change.resident);
                    lodgers.added.insert(key);
                }
            }
            Some(site) => {
                if lodgers.added.remove(&key) {
                    site.remove_tenant(change.resident);
                }
            }
            None => {
                lodgers.added.remove(&key);
            }
        }
    });
    b.on_resident_message::<SuitNote>(move |realm, idx, _from, note| {
        let frame = realm.frame();
        let max = params::count(realm, p.proposals_max);
        {
            let mut column = realm.col_mut::<Social>();
            let suits = &mut column[idx].make().suits;
            suits.retain(|suit| {
                suit.expires >= frame && (suit.from, suit.ask) != (note.suit.from, note.suit.ask)
            });
            suits.push(note.suit);
            if suits.len() > max {
                let extra = suits.len() - max;
                suits.drain(..extra);
            }
        }
        let mut data = json!({ "ask": note.suit.ask.name(), "expires": note.suit.expires });
        if let Some(site) = note.site {
            data["site"] = json!(site);
        }
        realm.notify(realm.id_of(idx), "society.proposal_received", Some(note.suit.from), data);
    });
}
