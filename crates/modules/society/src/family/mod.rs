//! Relations: private labels, marriage and households.

mod bonds;
mod household;
mod marriage;
mod observe;
mod patch;
mod state;

use unsup_kernel::{Builder, Holder, Realm, ResidentId, Scope, SiteId, Stage};

use self::state::Families;
use crate::params::Params;
use crate::social::HOUR;

/// Removes a resident that left the world from the registry.
pub(crate) fn on_death(realm: &Realm, p: &Params, dead: ResidentId) {
    let mut families = realm.res_mut::<Families>();
    marriage::on_death(realm, &mut families, dead);
    household::on_death(realm, p, &mut families, dead);
}

/// Registers the registry, the verbs, the handlers and the observer.
pub(crate) fn build(b: &mut Builder, p: Params) {
    match b.scope() {
        Scope::Zone => patch::build_zone(b, p),
        Scope::Global => {
            b.resource::<Families>();
            // Proposals that nobody accepted do not stay for ever.
            b.every(HOUR as u32, Stage::Post, 0, "society.proposals_expire", |realm| {
                let frame = realm.frame();
                realm.res_mut::<Families>().proposals.retain(|_, p| p.expires >= frame);
            });
            // The economy module took a home away. The data of these events
            // are fixed in `docs/design/game-design.md`, section 9.
            b.on_event("economy.tenant_evicted", move |realm, event| {
                let tenant = event.data["tenant"].as_u64().map(ResidentId);
                let site = event.data["site"].as_u64().map(SiteId);
                if let (Some(tenant), Some(site)) = (tenant, site) {
                    household::on_home_lost(realm, &p, tenant, site);
                }
            });
            // A lease that ends without an eviction: the tenant left by
            // itself, or the owner ended the lease.
            b.on_event("economy.lease_ended", move |realm, event| {
                let tenant = event.data["tenant"].as_u64().map(ResidentId);
                let site = event.data["site"].as_u64().map(SiteId);
                if let (Some(tenant), Some(site)) = (tenant, site) {
                    household::on_home_lost(realm, &p, tenant, site);
                }
            });
            // A sale and a foreclosure take the home from its owner.
            for kind in ["economy.property_sold", "economy.property_foreclosed"] {
                b.on_event(kind, move |realm, event| {
                    let owner = serde_json::from_value::<Holder>(event.data["from"].clone());
                    let site = event.data["site"].as_u64().map(SiteId);
                    if let (Ok(Holder::Resident(owner)), Some(site)) = (owner, site) {
                        household::on_home_lost(realm, &p, owner, site);
                    }
                });
            }
        }
    }
    bonds::build(b, p);
    marriage::build(b, p);
    household::build(b, p);
    b.observe("relations", 32, observe::section);
}
