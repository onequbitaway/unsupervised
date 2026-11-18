//! Looking at a resident, a site or the place where one stands: the free
//! verb `inspect`. It gives the details that the brief observation leaves
//! out.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{ActionError, Builder, Holder, Idx, Realm, ResidentId, Site, SiteId, VerbSpec};

use crate::care::{BODY_CARE, sleep_quality_in};
use crate::handles::Handles;
use crate::names::{SKILLS, tile};
use crate::sites::{doors, standing_tile};
use crate::skills::skill_level;
use crate::worldgen::ZoneFacts;

/// Arguments of `inspect`. Give `resident` or `site`, or nothing to look
/// at the place where you stand.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct InspectArgs {
    /// A resident that you can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resident: Option<ResidentId>,
    /// A site that you can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteId>,
}

/// Returns how far a resident sees, in tiles. A resident that sleeps sees
/// less.
fn sight(realm: &Realm, h: &Handles, idx: Idx) -> u32 {
    let radius = u32::try_from(realm.param(h.p.sight_radius)).unwrap_or(0);
    if realm.has_condition(idx, h.asleep) {
        radius.min(u32::try_from(realm.param(h.p.asleep_sight)).unwrap_or(0))
    } else {
        radius
    }
}

/// Describes how healthy a resident looks: `good`, `hurt` or `critical`.
fn looks(realm: &Realm, h: &Handles, idx: Idx) -> &'static str {
    match realm.attr(idx, h.health) {
        health if health < realm.param(h.p.need_low) => "critical",
        health if health < realm.param(h.p.hurt_below) => "hurt",
        _ => "good",
    }
}

/// What one resident sees of another.
fn resident_details(realm: &Realm, h: &Handles, me: Idx, other: Idx) -> Value {
    let identity = realm.identity(other);
    let conditions: Vec<String> = realm
        .conditions(other)
        .into_iter()
        .map(|(cond, _)| realm.defs().condition(cond).name.clone())
        .collect();
    // The best skills show in how a resident carries itself.
    let mut skills = Map::new();
    for (name, attr) in SKILLS.iter().zip(h.skills) {
        let level = skill_level(realm.attr(other, attr));
        if level > 0 {
            skills.insert((*name).to_string(), json!(level));
        }
    }
    let pos = realm.pos(other);
    json!({
        "resident": realm.id_of(other),
        "name": identity.name,
        "model": identity.model,
        "age": realm.attr(other, h.age),
        "pos": pos,
        "distance": pos.dist(realm.pos(me)),
        "activity": realm.activity(other).map(|activity| activity.verb),
        "conditions": conditions,
        "health": looks(realm, h, other),
        "skills": skills,
        "site": realm.sites().at(pos).map(|place| place.id),
    })
}

/// What a resident sees of a site. The money in the till shows only to a
/// resident that owns or rents the site.
pub(crate) fn site_details(realm: &Realm, me: Idx, place: &Site) -> Value {
    let resident = realm.id_of(me);
    let tiles = realm.tiles();
    let yours = place.owner == Some(Holder::Resident(resident)) || place.is_tenant(resident);
    let mut out = json!({
        "site": place.id,
        "kind": place.kind,
        "name": place.name,
        "rect": place.rect,
        "owner": place.owner,
        "tenants": place.tenants.len(),
        "yours": yours,
        "tags": place.tags,
        "attrs": place.attrs,
        "doors": doors(&tiles, place.rect),
        "stand": standing_tile(&tiles, place.rect),
        "sleep": sleep_quality_in(place, resident).name(),
    });
    if yours {
        out["till"] = json!(realm.ledger().money(Holder::Site(place.id)));
    }
    out
}

/// What a resident sees of the place where it stands.
fn place_details(realm: &Realm, me: Idx) -> Value {
    let pos = realm.pos(me);
    let zone = realm.zone_info();
    json!({
        "pos": pos,
        "tile": realm.tiles().get(pos).map(tile::name),
        "zone": zone.map(|zone| zone.id),
        "zone_name": zone.map(|zone| zone.name.clone()),
        "bounds": zone.map(|zone| zone.bounds),
        "city": zone.map(|zone| zone.city),
        "district": realm.res::<ZoneFacts>().district,
        "site": realm.sites().at(pos).map(|place| site_details(realm, me, place)),
    })
}

/// Registers the verb `inspect`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.instant_verb::<InspectArgs>(
        VerbSpec {
            verb: "inspect",
            module: "life",
            summary: "Look at a resident or a site that you can see, or at the place where you stand.",
            free: true,
            allowed_in: BODY_CARE,
        },
        move |realm, ctx, args| {
            let here = realm.pos(ctx.idx);
            let radius = sight(realm, &h, ctx.idx);
            match (args.resident, args.site) {
                (Some(_), Some(_)) => {
                    Err(ActionError::invalid_args("Give `resident` or `site`, not both."))
                }
                (Some(resident), None) => {
                    let seen = realm
                        .idx_of(resident)
                        .filter(|&other| realm.pos(other).dist(here) <= radius);
                    match seen {
                        Some(other) => Ok(resident_details(realm, &h, ctx.idx, other)),
                        None => Err(ActionError::not_here(format!(
                            "You cannot see resident {resident} from here."
                        ))),
                    }
                }
                (None, Some(site)) => {
                    let sites = realm.sites();
                    match sites.get(site) {
                        Some(place) if place.rect.dist_to(here) <= radius => {
                            Ok(site_details(realm, ctx.idx, place))
                        }
                        Some(_) => Err(ActionError::not_here(format!(
                            "You cannot see site {site} from here."
                        ))),
                        None => Err(ActionError::not_found(format!(
                            "No site in this district has the id {site}."
                        ))),
                    }
                }
                (None, None) => Ok(place_details(realm, ctx.idx)),
            }
        },
    );
}
