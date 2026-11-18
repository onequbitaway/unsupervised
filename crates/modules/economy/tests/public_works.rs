//! Starter shifts at a city hall or a depot, and the daily budget of the
//! city.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, ResidentId};

const WAGE: i64 = 2_800;
const TAX: i64 = 280;

fn minted(t: &Town) -> i64 {
    flows(&t.world).minted.get("gov.public_works").copied().unwrap_or(0)
}

fn error_of(t: &Town, r: ResidentId) -> ErrorCode {
    t.world.last_report(r).expect("a report").error.expect("an error").code
}

#[test]
fn anybody_works_a_starter_shift_and_the_faucet_pays() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.city_hall);
    let seen = section(&t.world, ada, "business").expect("a business section");
    assert_eq!(seen["public_works"], json!({ "wage": WAGE, "minutes": 240 }));

    let (result, events) = ok_with_events(&mut t.world, ada, "work", json!({}));
    assert_eq!(
        result,
        json!({
            "site": t.at.city_hall,
            "wage": WAGE,
            "tax": TAX,
            "net": WAGE - TAX,
            "skill": "labor",
            "xp": 40,
            "public_works": true,
        })
    );
    assert_eq!(cash(&t.world, ada), WAGE - TAX);
    t.world.step(1);
    assert_eq!(treasury(&t.world), TAX);
    assert_eq!(t.world.attr(ada, "skill.labor"), 40);
    assert_eq!(t.world.attr(ada, "need.energy"), 920);
    assert_eq!(
        of_kind(&events, "economy.shift_completed"),
        [json!({
            "resident": ada, "site": t.at.city_hall, "wage": WAGE, "tax": TAX, "skill": "labor",
        })]
    );
    // The depot offers the same shift.
    go(&mut t.world, ada, t.at.depot);
    ok(&mut t.world, ada, "work", json!({}));

    // The two wages are all the money that exists.
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", 2 * WAGE)], [], [], []));
    assert_eq!(cash(&t.world, ada) + treasury(&t.world), 2 * WAGE);
}

#[test]
fn the_daily_budget_of_the_city_runs_out_and_comes_back_the_next_day() {
    let mut t = Town::new();
    // Room for two shifts a day.
    t.world.set_param("economy.public_works_budget", 2 * WAGE);
    t.world.step(1);
    let [a, b, c, d] =
        ["A", "B", "C", "D"].map(|n| resident_at(&mut t.world, n, 0, t.at.city_hall));

    // Three residents start in the same frame. The city has money for two.
    for worker in [a, b, c] {
        t.world.act_ok(worker, "work", json!({}));
    }
    t.world.step(2);
    assert_eq!(t.world.activity(a), Some("work"));
    assert_eq!(t.world.activity(b), Some("work"));
    assert_eq!(t.world.activity(c), None, "the third shift stops");
    let report = t.world.last_report(c).expect("a report");
    let error = report.error.expect("an error");
    assert_eq!(error.code, ErrorCode::Interrupted);
    assert!(error.message.contains("budget"), "{}", error.message);
    // From now on the zone knows, and refuses at once.
    assert_eq!(refused(&mut t.world, d, "work", json!({})), ErrorCode::Insufficient);
    assert_eq!(refused(&mut t.world, c, "work", json!({})), ErrorCode::Insufficient);

    settle(&mut t.world, a);
    settle(&mut t.world, b);
    assert_eq!(cash(&t.world, a), WAGE - TAX);
    assert_eq!(cash(&t.world, b), WAGE - TAX);
    assert_eq!(cash(&t.world, c), 0);
    assert_eq!(minted(&t), 2 * WAGE);
    // Still the same day: no more shifts.
    assert_eq!(refused(&mut t.world, a, "work", json!({})), ErrorCode::Insufficient);

    // The next day has a new budget.
    t.world.step(DAY);
    ok(&mut t.world, d, "work", json!({}));
    assert_eq!(cash(&t.world, d), WAGE - TAX);
    assert_eq!(minted(&t), 3 * WAGE);
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", 3 * WAGE)], [], [], []));
}

#[test]
fn a_shift_that_stops_gives_its_wage_back_to_the_budget() {
    let mut t = Town::new();
    t.world.set_param("economy.public_works_budget", WAGE);
    t.world.step(1);
    let a = resident_at(&mut t.world, "A", 0, t.at.city_hall);
    let b = resident_at(&mut t.world, "B", 0, t.at.city_hall);

    t.world.act_ok(a, "work", json!({}));
    t.world.step(2);
    assert_eq!(refused(&mut t.world, b, "work", json!({})), ErrorCode::Insufficient);

    // A cancels. The budget has room again two ticks later.
    t.world.act_ok(a, "cancel", json!({}));
    assert_eq!(error_of(&t, a), ErrorCode::Interrupted);
    t.world.step(2);
    ok(&mut t.world, b, "work", json!({}));
    assert_eq!(cash(&t.world, a), 0);
    assert_eq!(cash(&t.world, b), WAGE - TAX);
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", WAGE)], [], [], []));
}

#[test]
fn the_zones_of_a_city_share_one_budget() {
    let mut t = Town::new();
    t.world.set_param("economy.public_works_budget", WAGE);
    t.world.step(1);
    let a = resident_at(&mut t.world, "A", 0, t.at.city_hall);
    let b = resident_at(&mut t.world, "B", 0, t.at.far_hall);
    assert_ne!(t.world.zone_of(a), t.world.zone_of(b));

    t.world.act_ok(a, "work", json!({}));
    t.world.step(2);
    // The other zone heard that the budget is spent.
    assert_eq!(refused(&mut t.world, b, "work", json!({})), ErrorCode::Insufficient);
    settle(&mut t.world, a);
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", WAGE)], [], [], []));
}

#[test]
fn two_zones_that_ask_in_the_same_frame_cannot_spend_the_budget_twice() {
    let mut t = Town::new();
    t.world.set_param("economy.public_works_budget", WAGE);
    t.world.step(1);
    let a = resident_at(&mut t.world, "A", 0, t.at.city_hall);
    let b = resident_at(&mut t.world, "B", 0, t.at.far_hall);
    t.world.act_ok(a, "work", json!({}));
    t.world.act_ok(b, "work", json!({}));
    settle(&mut t.world, a);
    settle(&mut t.world, b);
    // The lower zone asked first.
    assert_eq!(cash(&t.world, a), WAGE - TAX);
    assert_eq!(error_of(&t, b), ErrorCode::Interrupted);
    assert_eq!(cash(&t.world, b), 0);
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", WAGE)], [], [], []));
}

#[test]
fn a_resident_that_leaves_the_zone_gets_no_wage_and_frees_the_budget() {
    let mut t = Town::new();
    t.world.set_param("economy.public_works_budget", WAGE);
    t.world.step(1);
    let a = resident_at(&mut t.world, "A", 0, t.at.city_hall);
    let b = resident_at(&mut t.world, "B", 0, t.at.city_hall);
    t.world.act_ok(a, "work", json!({}));
    t.world.step(5);
    go(&mut t.world, a, t.at.far_home);
    settle(&mut t.world, a);
    assert_eq!(error_of(&t, a), ErrorCode::Conflict);
    assert_eq!(cash(&t.world, a), 0);
    // The wage went back to the budget: B can work today.
    ok(&mut t.world, b, "work", json!({}));
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", WAGE)], [], [], []));
}

#[test]
fn a_resident_that_steps_off_the_site_gets_no_wage() {
    let mut t = Town::new();
    let a = resident_at(&mut t.world, "A", 0, t.at.city_hall);
    t.world.act_ok(a, "work", json!({}));
    t.world.step(5);
    go(&mut t.world, a, t.at.park);
    settle(&mut t.world, a);
    assert_eq!(error_of(&t, a), ErrorCode::NotHere);
    assert_eq!(flows(&t.world), Flows::default());
}

#[test]
fn public_works_need_a_public_works_site_and_a_wage() {
    let mut t = Town::new();
    let a = resident_at(&mut t.world, "A", 0, t.at.park);
    // A park is public, but it offers no starter shift.
    assert_eq!(refused(&mut t.world, a, "work", json!({})), ErrorCode::NotAllowed);
    assert_eq!(section(&t.world, a, "business"), None);
    // A law can end public works.
    t.world.set_param("economy.public_works_wage", 0);
    t.world.step(1);
    go(&mut t.world, a, t.at.city_hall);
    assert_eq!(refused(&mut t.world, a, "work", json!({})), ErrorCode::NotAllowed);
    assert_only_transfers(&t.world);
}
