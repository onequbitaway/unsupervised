//! A world that hosts only a part of the realms. The other realms are in a
//! process that never answers. Nobody is paid twice and nobody is paid
//! without a grant.

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Pos, RealmId, WorldConfig, ZoneId};

const RENT: i64 = 1_500;

fn hosting(realms: Vec<RealmId>) -> TestWorld<'static> {
    let config = WorldConfig { hosted: Some(realms), ..WorldConfig::default() };
    TestWorld::with_config(&[&ECONOMY], config)
}

#[test]
fn a_tenant_that_cannot_be_reached_misses_the_rent_and_is_evicted() {
    // This process hosts the global realm and the first zone.
    let mut world = hosting(vec![RealmId::Global, RealmId::Zone(ZoneId(0))]);
    let home = add_site(&mut world, "home", (44, 2), HOME_VALUE, false);
    let owner = world.spawn("Owner");
    buy_site(&mut world, owner, home);
    ok(&mut world, owner, "rent_out", json!({ "rent": RENT }));
    let bob = resident_at(&mut world, "Bob", 9_000, home);
    ok(&mut world, bob, "rent", json!({}));

    // Bob walks into the second zone, which another process hosts.
    world.set_pos(bob, Pos::new(70, 30));
    assert_eq!(world.world.locate(bob), None);

    // Day 1: the zone of the home asks for the rent. Days 2 and 3: no
    // answer came, so each day is a missed day of the grace time.
    for day in 1..=3 {
        let events = next_day(&mut world);
        assert_eq!(site(&world, home).tenants, [bob], "day {day}");
        assert_eq!(of_kind(&events, "economy.tenant_evicted"), Vec::<Value>::new(), "day {day}");
    }
    // Day 4: three missed days are more than `economy.rent_grace_days`.
    let events = next_day(&mut world);
    assert_eq!(
        of_kind(&events, "economy.tenant_evicted"),
        [json!({ "site": home, "tenant": bob })]
    );
    assert!(site(&world, home).tenants.is_empty());
    // The till got the rent of the first day and paid four days of
    // property tax. Bob left with the rest of his cash.
    assert_eq!(till(&world, home), RENT - 4 * 100);
    let totals = world.assert_conserved();
    assert_eq!(totals.money.external, 9_000 - RENT);
    assert_eq!(totals.money.burned, 0);
}

#[test]
fn a_starter_shift_without_an_answer_of_the_city_is_not_paid() {
    // This process hosts only the first zone. The global realm, which
    // counts the public works budget, is in another process.
    let mut world = hosting(vec![RealmId::Zone(ZoneId(0))]);
    let hall = add_site(&mut world, "city_hall", (2, 10), 1_000_000, true);
    let ada = resident_at(&mut world, "Ada", 0, hall);

    let error = fails(&mut world, ada, "work", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert!(error.message.contains("did not confirm"), "{}", error.message);
    // No wage, no tax, no experience.
    assert_eq!(cash(&world, ada), 0);
    assert_eq!(world.attr(ada, "skill.labor"), 0);
    let totals = world.assert_conserved();
    assert_eq!((totals.money.minted, totals.money.held, totals.money.external), (0, 0, 0));
}
