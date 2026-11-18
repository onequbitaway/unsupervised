//! World generation: the map and the sites of every zone.
//!
//! Each zone builds itself when the world is made. The result depends only
//! on the seed, the configuration of the world and the parameters. See
//! `docs/design/modules/life.md` for the rules.

mod city;
mod kinds;
mod params;
mod zone;

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    Builder, EventMeta, Holder, OrgId, Realm, Resource, Scope, Site, SiteId, Stage, WorldGen,
};

pub use self::kinds::District;
use self::kinds::{KINDS, Role};
use self::params::GenParams;
use crate::names::{TICKS_PER_DAY, key, tag};
use crate::{shelters, sites, travel};

/// Drama of the event that tells viewers the type of a district.
const DRAMA_PUBLISHED: u8 = 0;

/// What the life module knows about the zone of a realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneFacts {
    /// Type of the district: `downtown`, `residential`, `commercial`,
    /// `industrial`, `rural` or `mixed`. Empty before world generation.
    pub district: String,
}

impl Resource for ZoneFacts {
    const NAME: &'static str = "life.zone";
}

/// Registers the world generator.
pub(crate) fn register(b: &mut Builder) {
    if b.scope() != Scope::Zone {
        return;
    }
    b.resource::<ZoneFacts>();
    b.on_worldgen(0, generate);
    // Viewers that join later, or after a restart, learn the district type
    // from the newest event of this kind.
    b.every(TICKS_PER_DAY, Stage::Post, 0, "life.publish_district", publish);
}

/// Builds the zone of a realm: tiles, sites and the district type.
fn generate(realm: &Realm, world: &WorldGen<'_>) {
    let Some(info) = realm.zone_info() else {
        return;
    };
    let Some(city) = world.layout.city(info.city) else {
        return;
    };
    let Some(slot) = city.zones.iter().position(|zone| *zone == info.id) else {
        return;
    };
    let params = GenParams::read(realm);
    let plan = city::plan_city(world.seed, city, world.layout.zone_size(), &params);
    let district = plan.districts[slot];
    let zone = zone::plan_zone(world.seed, info, district, &plan.required[slot], &params);

    {
        let mut tiles = realm.tiles_mut();
        for (pos, code) in zone.canvas.iter() {
            tiles.set(pos, code);
        }
    }

    let owner = Holder::Org(OrgId::city(info.city));
    let mut counts = [0u32; KINDS.len()];
    let mut index = Vec::with_capacity(zone.sites.len());
    for plan in &zone.sites {
        let kind = &KINDS[plan.kind];
        counts[plan.kind] += 1;
        let name = match kind.role {
            Role::Unique => kind.title.to_string(),
            _ => format!("{} {}", kind.title, counts[plan.kind]),
        };
        let id = SiteId(realm.alloc_id());
        let mut site = Site::new(id, kind.name, name, plan.rect)
            .with_owner(owner)
            .with_attr(key::VALUE, plan.value);
        if params.beds[plan.kind] > 0 {
            site.set_attr(key::BEDS, params.beds[plan.kind]);
        }
        if kind.public {
            site.add_tag(tag::PUBLIC);
        }
        if kind.spawn {
            site.add_tag(tag::SPAWN);
        }
        realm.sites_mut().insert(site);
        index.push(id);
    }

    realm.res_mut::<ZoneFacts>().district = district.name().to_string();
    sites::announce(realm, &index);
    travel::announce(realm);
    shelters::announce(realm);
    publish(realm);
}

/// Tells viewers the type of the district of this zone.
fn publish(realm: &Realm) {
    let Some(info) = realm.zone_info() else {
        return;
    };
    let district = realm.res::<ZoneFacts>().district.clone();
    realm.emit(
        EventMeta::public("life.district_published", DRAMA_PUBLISHED),
        &json!({
            "key": info.id,
            "zone": info.id,
            "city": info.city,
            "name": info.name,
            "district": district,
        }),
    );
}
