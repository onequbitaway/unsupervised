//! Shared helpers of the life tests: a plugin with test goods and short
//! functions to find sites and tiles.
//!
//! The life module defines no goods. The tests bring their own, the way the
//! economy module does in the game.

#![allow(dead_code)]

use unsup_kernel::protocol::{ActionReport, CityId};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Builder, CityConfig, Holder, OrgId, Plugin, Pos, ResidentId, Site, SiteId, Value, WorldConfig,
    ZoneId,
};
use unsup_life::LifePlugin;
use unsup_life::names::tile;

/// The goods of the tests.
pub const GOODS: &str = r#"
[[items]]
name = "bread"
weight = 1
tags = ["food"]
attrs = { nutrition = 300 }
summary = "A loaf of bread."

[[items]]
name = "apple"
weight = 1
tags = ["food"]
attrs = { nutrition = 100 }
summary = "An apple."

[[items]]
name = "feast"
weight = 3
tags = ["food"]
attrs = { nutrition = 900 }
summary = "A meal for a whole day."

[[items]]
name = "bandage"
weight = 1
tags = ["medicine"]
attrs = { healing = 200 }
summary = "A bandage."

[[items]]
name = "stone"
weight = 5
summary = "A stone. It is not food and not medicine."
"#;

/// A plugin that defines the goods of the tests.
pub struct Goods;

impl Plugin for Goods {
    fn name(&self) -> &'static str {
        "test_goods"
    }

    fn build(&self, b: &mut Builder) {
        b.load_data(GOODS);
    }
}

/// The life module.
pub static LIFE: LifePlugin = LifePlugin;

/// The goods of the tests.
pub static TEST_GOODS: Goods = Goods;

/// The plugins of every test world.
pub fn plugins() -> [&'static dyn Plugin; 2] {
    [&LIFE, &TEST_GOODS]
}

/// Makes the default test world: one city of 2 by 2 zones, seed 0.
pub fn world() -> TestWorld<'static> {
    TestWorld::new(&plugins())
}

/// Makes a test world from a configuration.
pub fn world_with(config: WorldConfig) -> TestWorld<'static> {
    TestWorld::with_config(&plugins(), config)
}

/// Returns a configuration with cities of the given sizes in a row, with a
/// gap of one zone between them.
pub fn cities(seed: u64, districts: &[u32]) -> WorldConfig {
    let mut config = WorldConfig::with_seed(seed);
    let mut x = 0i32;
    config.cities = districts
        .iter()
        .enumerate()
        .map(|(index, &side)| {
            let city = CityConfig::new(format!("City {index}"), Pos::new(x, 0), side);
            x += ((side + 1) * config.zone_size) as i32;
            city
        })
        .collect();
    config
}

/// Returns a configuration that changes parameters of the default world.
pub fn with_params(params: &[(&str, i64)]) -> WorldConfig {
    let mut config = WorldConfig::default();
    add_params(&mut config, params);
    config
}

/// Adds parameter overrides to a configuration.
pub fn add_params(config: &mut WorldConfig, params: &[(&str, i64)]) {
    for (name, value) in params {
        config.data.push(format!("[params.\"{name}\"]\nvalue = {value}\n"));
    }
}

/// Returns every site of the world with its zone, by zone and site id.
pub fn all_sites(w: &TestWorld<'_>) -> Vec<(ZoneId, Site)> {
    let mut out = Vec::new();
    for zone in w.world.layout().zones() {
        out.extend(w.zone(zone.id).sites().iter().map(|site| (zone.id, site.clone())));
    }
    out
}

/// Returns the first site of a kind in a city.
pub fn site_in(w: &TestWorld<'_>, city: CityId, kind: &str) -> (ZoneId, Site) {
    let layout = w.world.layout();
    all_sites(w)
        .into_iter()
        .find(|(zone, site)| {
            site.kind == kind && layout.zone(*zone).is_some_and(|info| info.city == city)
        })
        .unwrap_or_else(|| panic!("city {city} has no site of kind `{kind}`"))
}

/// Returns the first site of a kind in the first city.
pub fn site(w: &TestWorld<'_>, kind: &str) -> (ZoneId, Site) {
    site_in(w, CityId(0), kind)
}

/// Returns the first site of a kind in a zone.
pub fn site_in_zone(w: &TestWorld<'_>, zone: ZoneId, kind: &str) -> Option<Site> {
    w.zone(zone).sites().of_kind(kind).next().cloned()
}

/// Returns the code of a tile.
pub fn tile_at(w: &TestWorld<'_>, pos: Pos) -> u8 {
    let zone =
        w.world.layout().zone_at(pos).unwrap_or_else(|| panic!("{pos} is outside the world"));
    w.zone(zone).tiles().get(pos).expect("the zone has the tile")
}

/// Returns true when a resident can stand on a tile.
pub fn walkable(w: &TestWorld<'_>, pos: Pos) -> bool {
    tile::is_walkable(tile_at(w, pos))
}

/// Sets the code of every tile of a rectangle.
pub fn paint(w: &TestWorld<'_>, rect: unsup_kernel::Rect, code: u8) {
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let pos = Pos::new(x, y);
            let zone = w.world.layout().zone_at(pos).expect("the tile is in the world");
            assert!(w.zone(zone).tiles_mut().set(pos, code));
        }
    }
}

/// Makes a resident the owner of a site.
pub fn make_owner(w: &TestWorld<'_>, zone: ZoneId, site: SiteId, resident: ResidentId) {
    w.zone(zone).sites_mut().get_mut(site).expect("the site exists").owner =
        Some(Holder::Resident(resident));
}

/// Makes a resident a tenant of a site.
pub fn make_tenant(w: &TestWorld<'_>, zone: ZoneId, site: SiteId, resident: ResidentId) {
    w.zone(zone).sites_mut().get_mut(site).expect("the site exists").add_tenant(resident);
}

/// Returns the treasury of a city.
pub fn treasury(city: u16) -> Holder {
    Holder::Org(OrgId::city(CityId(city)))
}

/// Returns the newest report of a resident, which must exist.
pub fn report(w: &TestWorld<'_>, resident: ResidentId) -> ActionReport {
    w.last_report(resident).expect("the resident has a report")
}

/// Returns the result of the newest report of a resident, which must be a
/// success.
pub fn result(w: &TestWorld<'_>, resident: ResidentId) -> Value {
    let report = report(w, resident);
    assert!(report.ok, "the action failed: {:?}", report.error);
    report.result
}
