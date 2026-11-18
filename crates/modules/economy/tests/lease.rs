//! Leases: `rent_out`, `rent`, `end_lease`, the daily rent, the grace time
//! and the eviction.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, ResidentId};

const RENT: i64 = 1_500;
/// Property tax of the home for one day.
const TAX: i64 = 100;

/// A home with an owner that lets it at 1,500 cents a day.
fn home_to_let(t: &mut Town) -> ResidentId {
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.home);
    let result = ok(&mut t.world, owner, "rent_out", json!({ "rent": RENT }));
    assert_eq!(result, json!({ "site": t.at.home, "rent": RENT }));
    owner
}

#[test]
fn a_tenant_moves_in_and_pays_the_rent_each_day() {
    let mut t = Town::new();
    let owner = home_to_let(&mut t);
    let bob = resident_at(&mut t.world, "Bob", 5_000, t.at.home);
    assert_eq!(section(&t.world, bob, "business").expect("a section")["for_rent"], RENT);

    let (result, events) = ok_with_events(&mut t.world, bob, "rent", json!({}));
    assert_eq!(result, json!({ "site": t.at.home, "rent": RENT }));
    // The rent of the first day, in cash, into the till of the home.
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (3_500, RENT));
    assert_eq!(site(&t.world, t.at.home).tenants, [bob]);
    assert_eq!(
        of_kind(&events, "economy.lease_started"),
        [json!({ "site": t.at.home, "tenant": bob, "rent": RENT })]
    );
    assert_eq!(
        section(&t.world, bob, "property"),
        Some(json!({ "leases": [{ "site": t.at.home, "zone": 0, "rent": RENT }] }))
    );
    let notice = t.world.notices(owner).pop().expect("a notice");
    assert_eq!((notice.kind.as_str(), notice.from), ("economy.tenant_moved_in", Some(bob)));
    // A home has one unit: it is not offered any more.
    assert_eq!(section(&t.world, owner, "business").expect("a section").get("for_rent"), None);

    // Each midnight: one more day of rent, and the property tax of the home.
    next_day(&mut t.world);
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (2_000, 2 * RENT - TAX));
    next_day(&mut t.world);
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (500, 3 * RENT - 2 * TAX));
    // The owner takes the rent out of the till.
    ok(&mut t.world, owner, "till_collect", json!({}));
    assert_eq!(cash(&t.world, owner), 3 * RENT - 2 * TAX);
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", HOME_VALUE + 5_000)]));
    assert_eq!(flows.of_module(), Flows::default());
}

#[test]
fn a_zone_that_skips_the_frame_of_midnight_does_the_daily_work_in_its_next_tick() {
    let mut t = Town::new();
    home_to_let(&mut t);
    let bob = resident_at(&mut t.world, "Bob", 5_000, t.at.home);
    ok(&mut t.world, bob, "rent", json!({}));
    let zone = zone_of_site(&t.world, t.at.home);
    // The last frame of the day.
    let frame = t.world.world.frame();
    t.world.step(DAY - 1 - u32::try_from(frame).expect("a short test"));
    assert_eq!(t.world.world.frame(), u64::from(DAY) - 1);
    // The zone of the home is too slow: it skips midnight and four frames
    // more (time dilation).
    for _ in 0..5 {
        t.world.world.apply(unsup_kernel::Input::Step { skip: vec![zone] });
    }
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (3_500, RENT));
    // Its next tick does the work of the day, once.
    t.world.step(3);
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (2_000, 2 * RENT - TAX));
    t.world.step(100);
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (2_000, 2 * RENT - TAX));
    // The next midnight is a normal one.
    next_day(&mut t.world);
    assert_eq!((cash(&t.world, bob), till(&t.world, t.at.home)), (500, 3 * RENT - 2 * TAX));
    assert_only_transfers(&t.world);
}

#[test]
fn a_tenant_that_misses_the_rent_has_a_grace_time_and_is_then_evicted() {
    let mut t = Town::new();
    home_to_let(&mut t);
    // Cash for the first day and one more.
    let bob = resident_at(&mut t.world, "Bob", 2 * RENT, t.at.home);
    ok(&mut t.world, bob, "rent", json!({}));
    next_day(&mut t.world);
    assert_eq!(cash(&t.world, bob), 0);

    // Two days of grace: the tenant stays, the debt grows.
    for day in 1..=2 {
        let events = next_day(&mut t.world);
        assert_eq!(site(&t.world, t.at.home).tenants, [bob], "day {day}");
        assert_eq!(of_kind(&events, "economy.tenant_evicted"), Vec::<serde_json::Value>::new());
        let notice = t.world.notices(bob).pop().expect("a notice");
        assert_eq!(notice.kind, "economy.rent_missed");
        assert_eq!(notice.data, json!({ "site": t.at.home, "owed": RENT * day, "cash": 0 }));
        let lease = &section(&t.world, bob, "property").expect("a section")["leases"][0];
        assert_eq!(
            (lease["owed"].as_i64(), lease["days_missed"].as_i64()),
            (Some(RENT * day), Some(day))
        );
    }
    // The third missed day ends the lease.
    let events = next_day(&mut t.world);
    assert_eq!(
        of_kind(&events, "economy.tenant_evicted"),
        [json!({ "site": t.at.home, "tenant": bob })]
    );
    assert_eq!(drama_of(&events, "economy.tenant_evicted"), [45]);
    assert!(site(&t.world, t.at.home).tenants.is_empty());
    assert_eq!(section(&t.world, bob, "property"), None);
    let notice = t.world.notices(bob).pop().expect("a notice");
    assert_eq!(
        (notice.kind.as_str(), notice.data.clone()),
        ("economy.evicted", json!({ "site": t.at.home }))
    );
    // Nobody asks an evicted tenant for rent.
    t.world.give_money(bob, Money(9_000));
    next_day(&mut t.world);
    assert_eq!(cash(&t.world, bob), 9_000);
    assert_only_transfers(&t.world);
}

#[test]
fn a_tenant_that_pays_its_debt_in_the_grace_time_stays() {
    let mut t = Town::new();
    home_to_let(&mut t);
    let bob = resident_at(&mut t.world, "Bob", RENT, t.at.home);
    ok(&mut t.world, bob, "rent", json!({}));
    next_day(&mut t.world);
    next_day(&mut t.world);
    // Two days are owed. On the third day the tenant has money for three.
    t.world.give_money(bob, Money(3 * RENT + 7));
    next_day(&mut t.world);
    assert_eq!(cash(&t.world, bob), 7);
    assert_eq!(site(&t.world, t.at.home).tenants, [bob]);
    let lease = &section(&t.world, bob, "property").expect("a section")["leases"][0];
    assert_eq!(lease.get("owed"), None);
    // Property tax left the till on three days.
    assert_eq!(till(&t.world, t.at.home), 4 * RENT - 3 * TAX);
    assert_only_transfers(&t.world);
}

#[test]
fn a_tenant_in_another_zone_pays_by_wire() {
    let mut t = Town::new();
    home_to_let(&mut t);
    let bob = resident_at(&mut t.world, "Bob", 2 * RENT, t.at.home);
    ok(&mut t.world, bob, "rent", json!({}));
    go(&mut t.world, bob, t.at.far_shop);
    next_day(&mut t.world);
    assert_eq!(cash(&t.world, bob), 0);
    assert_eq!(till(&t.world, t.at.home), 2 * RENT - TAX);
    // The next day it cannot pay, and the zone of the home hears of it.
    next_day(&mut t.world);
    assert_eq!(t.world.notices(bob).pop().expect("a notice").kind, "economy.rent_missed");
    next_day(&mut t.world);
    let events = next_day(&mut t.world);
    assert_eq!(
        of_kind(&events, "economy.tenant_evicted"),
        [json!({ "site": t.at.home, "tenant": bob })]
    );
    // The news of the eviction follows the tenant.
    assert_eq!(section(&t.world, bob, "property"), None);
    assert_only_transfers(&t.world);
}

#[test]
fn the_city_lets_its_homes_and_gets_the_rent() {
    let mut t = Town::new();
    let bob = resident_at(&mut t.world, "Bob", 1_000, t.at.far_home);
    // Twenty basis points of 200,000 cents: 400 cents a day.
    assert_eq!(section(&t.world, bob, "business").expect("a section")["for_rent"], 400);
    assert_eq!(ok(&mut t.world, bob, "rent", json!({}))["rent"], 400);
    assert_eq!(till(&t.world, t.at.far_home), 400);
    // At midnight the city takes what its sites hold.
    next_day(&mut t.world);
    assert_eq!(cash(&t.world, bob), 200);
    assert_eq!(till(&t.world, t.at.far_home) + treasury(&t.world), 800);
    next_day(&mut t.world);
    assert_eq!(treasury(&t.world), 800);
    assert_eq!(till(&t.world, t.at.far_home), 0);
    // A law can end the offer of the city.
    t.world.set_param("economy.city_rent_bp", 0);
    t.world.step(1);
    let carol = resident_at(&mut t.world, "Carol", 1_000, t.at.home);
    assert_eq!(refused(&mut t.world, carol, "rent", json!({})), ErrorCode::NotAllowed);
    assert_only_transfers(&t.world);
}

#[test]
fn the_nation_gets_the_rent_of_its_own_homes() {
    let mut t = Town::new();
    let nation = Holder::Org(unsup_kernel::OrgId::NATION);
    let zone = zone_of_site(&t.world, t.at.home);
    t.world.zone(zone).sites_mut().get_mut(t.at.home).expect("the site").owner = Some(nation);
    let bob = resident_at(&mut t.world, "Bob", 1_000, t.at.home);
    assert_eq!(ok(&mut t.world, bob, "rent", json!({}))["rent"], 400);
    // At midnight the treasury of the owner takes what the till holds.
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home) + t.world.money(nation).0, 800);
    next_day(&mut t.world);
    assert_eq!((till(&t.world, t.at.home), t.world.money(nation).0), (0, 800));
    assert_eq!(treasury(&t.world), 0);
    assert_only_transfers(&t.world);
}

#[test]
fn rent_refuses_what_it_must() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.home);
    let bob = t.world.spawn("Bob");
    t.world.give_money(bob, Money(RENT - 1));

    assert_eq!(refused(&mut t.world, bob, "rent", json!({})), ErrorCode::NotHere);
    // A shop takes no tenants. A home whose owner did not offer it neither.
    go(&mut t.world, bob, t.at.shop);
    assert_eq!(refused(&mut t.world, bob, "rent", json!({})), ErrorCode::NotAllowed);
    go(&mut t.world, bob, t.at.home);
    assert_eq!(refused(&mut t.world, bob, "rent", json!({})), ErrorCode::NotAllowed);

    ok(&mut t.world, owner, "rent_out", json!({ "rent": RENT }));
    assert_eq!(refused(&mut t.world, owner, "rent", json!({})), ErrorCode::Conflict);
    let error = t.world.act_err(bob, "rent", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": RENT, "available": RENT - 1 }));
    t.world.give_money(bob, Money(1));
    let limit = json!({ "max_rent": RENT - 1 });
    assert_eq!(refused(&mut t.world, bob, "rent", limit), ErrorCode::Conflict);
    ok(&mut t.world, bob, "rent", json!({ "max_rent": RENT }));
    // Already a tenant; and the only unit is taken.
    assert_eq!(refused(&mut t.world, bob, "rent", json!({})), ErrorCode::Conflict);
    let carol = resident_at(&mut t.world, "Carol", 9_000, t.at.home);
    assert_eq!(refused(&mut t.world, carol, "rent", json!({})), ErrorCode::Conflict);
    assert_only_transfers(&t.world);
}

#[test]
fn two_residents_cannot_both_rent_the_last_unit() {
    let mut t = Town::new();
    home_to_let(&mut t);
    let [a, b] = ["A", "B"].map(|name| resident_at(&mut t.world, name, RENT, t.at.home));
    // Both start in the same tick. The home has one unit.
    t.world.act_ok(a, "rent", json!({}));
    t.world.act_ok(b, "rent", json!({}));
    settle(&mut t.world, a);
    settle(&mut t.world, b);
    assert_eq!(site(&t.world, t.at.home).tenants, [a]);
    let report = t.world.last_report(b).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::Conflict);
    // The second one paid nothing and has no lease.
    assert_eq!((cash(&t.world, a), cash(&t.world, b)), (0, RENT));
    assert_eq!(till(&t.world, t.at.home), RENT);
    assert_eq!(section(&t.world, b, "property"), None);
    assert_only_transfers(&t.world);
}

#[test]
fn an_apartment_house_takes_as_many_tenants_as_it_has_units() {
    let mut t = Town::new();
    // The life module can give a site a `capacity`. Without one, the kind
    // decides: eight units for an apartment house.
    let zone = zone_of_site(&t.world, t.at.apartment);
    t.world
        .zone(zone)
        .sites_mut()
        .get_mut(t.at.apartment)
        .expect("the site")
        .set_attr("capacity", 2);
    let rent = 1_600;
    for name in ["A", "B"] {
        let tenant = resident_at(&mut t.world, name, rent, t.at.apartment);
        assert_eq!(ok(&mut t.world, tenant, "rent", json!({}))["rent"], rent);
    }
    let late = resident_at(&mut t.world, "Late", rent, t.at.apartment);
    assert_eq!(refused(&mut t.world, late, "rent", json!({})), ErrorCode::Conflict);
    assert_eq!(site(&t.world, t.at.apartment).tenants.len(), 2);
    assert_only_transfers(&t.world);
}

#[test]
fn rent_out_refuses_what_it_must_and_an_offer_can_end() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    let offer = json!({ "rent": RENT });
    assert_eq!(refused(&mut t.world, owner, "rent_out", offer.clone()), ErrorCode::NotHere);
    go(&mut t.world, owner, t.at.home);
    assert_eq!(refused(&mut t.world, owner, "rent_out", offer.clone()), ErrorCode::NotAllowed);
    buy_site(&mut t.world, owner, t.at.home);
    for bad in [json!({ "rent": 0 }), json!({ "rent": 100_000_001 })] {
        assert_eq!(refused(&mut t.world, owner, "rent_out", bad), ErrorCode::InvalidArgs);
    }
    // A farm takes no tenants.
    buy_site(&mut t.world, owner, t.at.farm);
    assert_eq!(refused(&mut t.world, owner, "rent_out", offer.clone()), ErrorCode::NotAllowed);

    go(&mut t.world, owner, t.at.home);
    ok(&mut t.world, owner, "rent_out", offer);
    assert_eq!(site(&t.world, t.at.home).attr("rent"), RENT);
    ok(&mut t.world, owner, "rent_out", json!({}));
    let bob = resident_at(&mut t.world, "Bob", 9_000, t.at.home);
    assert_eq!(refused(&mut t.world, bob, "rent", json!({})), ErrorCode::NotAllowed);
    assert_only_transfers(&t.world);
}

#[test]
fn a_tenant_or_the_owner_ends_a_lease() {
    let mut t = Town::new();
    let owner = home_to_let(&mut t);
    let bob = resident_at(&mut t.world, "Bob", 9_000, t.at.home);
    let carol = resident_at(&mut t.world, "Carol", 9_000, t.at.home);

    // Nobody rents yet.
    assert_eq!(refused(&mut t.world, bob, "end_lease", json!({})), ErrorCode::NotFound);
    assert_eq!(
        refused(&mut t.world, owner, "end_lease", json!({ "tenant": bob })),
        ErrorCode::NotFound
    );
    ok(&mut t.world, bob, "rent", json!({}));
    // Only the owner ends the lease of another resident.
    assert_eq!(
        refused(&mut t.world, carol, "end_lease", json!({ "tenant": bob })),
        ErrorCode::NotAllowed
    );
    t.world.set_pos(bob, unsup_kernel::Pos::new(60, 60));
    assert_eq!(refused(&mut t.world, bob, "end_lease", json!({})), ErrorCode::NotHere);
    go(&mut t.world, bob, t.at.home);

    let (result, events) = ok_with_events(&mut t.world, bob, "end_lease", json!({}));
    assert_eq!(result, json!({ "site": t.at.home, "tenant": bob }));
    assert_eq!(
        of_kind(&events, "economy.lease_ended"),
        [json!({ "site": t.at.home, "tenant": bob, "by": "tenant" })]
    );
    assert!(site(&t.world, t.at.home).tenants.is_empty());
    assert_eq!(section(&t.world, bob, "property"), None);

    // The owner ends the lease of the next tenant, who is away.
    ok(&mut t.world, carol, "rent", json!({}));
    go(&mut t.world, carol, t.at.far_shop);
    let (_, events) = ok_with_events(&mut t.world, owner, "end_lease", json!({ "tenant": carol }));
    assert_eq!(
        of_kind(&events, "economy.lease_ended"),
        [json!({ "site": t.at.home, "tenant": carol, "by": "owner" })]
    );
    t.world.step(2);
    assert_eq!(section(&t.world, carol, "property"), None);
    let notice = t.world.notices(carol).pop().expect("a notice");
    assert_eq!((notice.kind.as_str(), notice.from), ("economy.lease_ended", Some(owner)));
    // No rent after the end.
    next_day(&mut t.world);
    assert_eq!((cash(&t.world, bob), cash(&t.world, carol)), (9_000 - RENT, 9_000 - RENT));
    assert_eq!(site(&t.world, t.at.home).owner, Some(Holder::Resident(owner)));
    assert_only_transfers(&t.world);
}

#[test]
fn a_tenant_that_buys_its_home_pays_no_more_rent() {
    let mut t = Town::new();
    let owner = home_to_let(&mut t);
    ok(&mut t.world, owner, "sell_property", json!({ "price": 10_000 }));
    let bob = resident_at(&mut t.world, "Bob", 10_000 + 2 * RENT, t.at.home);
    ok(&mut t.world, bob, "rent", json!({}));
    let (_, events) = ok_with_events(&mut t.world, bob, "buy_property", json!({}));
    assert_eq!(
        of_kind(&events, "economy.lease_ended"),
        [json!({ "site": t.at.home, "tenant": bob, "by": "tenant" })]
    );
    assert!(site(&t.world, t.at.home).tenants.is_empty());
    let mine = section(&t.world, bob, "property").expect("a property section");
    assert_eq!(mine.get("leases"), None);
    assert_eq!(mine["owned"][0]["site"], json!(t.at.home));
    next_day(&mut t.world);
    assert_eq!(cash(&t.world, bob), RENT);
    assert_only_transfers(&t.world);
}

#[test]
fn a_site_of_the_city_with_a_job_keeps_its_till() {
    let mut t = Town::new();
    // An officer of the city runs the city hall: a job, and a till for the
    // wage.
    let gov = unsup_kernel::OrgId::city(CITY);
    let mayor = resident_at(&mut t.world, "Mayor", 3_000, t.at.city_hall);
    t.world.with_resident(mayor, |realm, _| realm.grant(mayor, gov, "mayor", &["site.manage"]));
    ok(&mut t.world, mayor, "till_fund", json!({ "amount": 3_000 }));
    let job =
        ok(&mut t.world, mayor, "post_job", json!({ "wage": 2_500, "slots": 1 }))["job"].clone();
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.city_hall), 3_000);
    // A worker with the job is paid from the till, not from the faucet.
    let clerk = resident_at(&mut t.world, "Clerk", 0, t.at.city_hall);
    ok(&mut t.world, clerk, "apply_job", json!({ "job": job }));
    let shift = ok(&mut t.world, clerk, "work", json!({}));
    assert_eq!(shift.get("public_works"), None);
    assert_eq!(till(&t.world, t.at.city_hall), 500);
    // Without the job the city takes the till at midnight.
    ok(&mut t.world, mayor, "close_job", json!({ "job": job }));
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.city_hall), 0);
    assert_eq!(treasury(&t.world), 500 + 250);
    // The wage came out of the till, not out of the faucet.
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", 3_000)]));
    assert_eq!(flows.of_module(), Flows::default());
}
