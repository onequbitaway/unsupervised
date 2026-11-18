//! Salaries: each government pays its officials at the start of each game
//! day, in exact amounts, and only while its treasury can.

mod common;

use common::{
    CITY, DAY, event, events, fund_treasury, notices, ok, plugins, quiet_config, rich, seat,
    step_to, treasury,
};
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{OrgId, ResidentId};
use unsup_politics::roles;

/// The daily salaries of the data file.
const MAYOR: i64 = 8_000;
const COUNCILLOR: i64 = 3_000;

#[test]
fn salaries_move_exact_amounts_and_stop_when_the_treasury_is_empty() {
    let mut w = TestWorld::with_config(&plugins(), quiet_config());
    let mayor = rich(&mut w, "Mayor", 0);
    let ann = rich(&mut w, "Ann", 0);
    let bob = rich(&mut w, "Bob", 0);
    seat(&mut w, CITY, roles::MAYOR, &[mayor]);
    seat(&mut w, CITY, roles::COUNCILLOR, &[ann, bob]);
    // Enough for one full day, then for the mayor and one councillor, with
    // 500 cents left.
    let start = 2 * MAYOR + 3 * COUNCILLOR + 500;
    fund_treasury(&mut w, CITY, start);
    let cash = |w: &TestWorld<'_>| [mayor, ann, bob].map(|r| w.cash(r).cents());

    // Nobody is paid during the first day.
    let frames = step_to(&mut w, DAY - 1);
    assert!(events(&frames, "politics.salaries_paid").is_empty());
    assert_eq!(cash(&w), [0, 0, 0]);

    // The start of day 2: everybody is paid for day 1.
    let frames = w.step(1);
    let paid = event(&frames, "politics.salaries_paid");
    assert_eq!(
        paid.data,
        json!({
            "gov": "city", "city": 0, "paid": 3, "unpaid": 0,
            "total": MAYOR + 2 * COUNCILLOR, "treasury": start - MAYOR - 2 * COUNCILLOR,
        })
    );
    assert_eq!(paid.drama, 2);
    // The money is on its way: it is cash one frame later.
    assert_eq!(treasury(&w, CITY), start - MAYOR - 2 * COUNCILLOR);
    w.step(1);
    assert_eq!(cash(&w), [MAYOR, COUNCILLOR, COUNCILLOR]);
    let told = notices(&w, ann, "politics.salary_paid");
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].data,
        json!({ "gov": "city", "city": 0, "office": "council", "amount": COUNCILLOR })
    );

    // The start of day 3: the treasury pays the mayor and one councillor.
    // The second councillor gets nothing, not a part.
    let frames = step_to(&mut w, 2 * DAY);
    let paid = event(&frames, "politics.salaries_paid");
    assert_eq!(
        paid.data,
        json!({
            "gov": "city", "city": 0, "paid": 2, "unpaid": 1,
            "total": MAYOR + COUNCILLOR, "treasury": 500,
        })
    );
    assert_eq!(paid.drama, 40, "a government that cannot pay is news");
    w.step(1);
    assert_eq!(cash(&w), [2 * MAYOR, 2 * COUNCILLOR, COUNCILLOR]);
    assert_eq!(notices(&w, bob, "politics.salary_unpaid").len(), 1);
    assert_eq!(notices(&w, bob, "politics.salary_paid").len(), 1);

    // The start of day 4: 500 cents pay nobody. No funds, no pay.
    let frames = step_to(&mut w, 3 * DAY);
    let paid = event(&frames, "politics.salaries_paid");
    assert_eq!((&paid.data["paid"], &paid.data["unpaid"]), (&json!(0), &json!(3)));
    assert_eq!((&paid.data["total"], &paid.data["treasury"]), (&json!(0), &json!(500)));
    w.step(1);
    assert_eq!(cash(&w), [2 * MAYOR, 2 * COUNCILLOR, COUNCILLOR]);
    assert_eq!(treasury(&w, CITY), 500);

    // Money in the treasury starts the pay again.
    fund_treasury(&mut w, CITY, COUNCILLOR);
    step_to(&mut w, 4 * DAY + 1);
    assert_eq!(cash(&w), [2 * MAYOR, 3 * COUNCILLOR, COUNCILLOR], "3,500 cents pay one councillor");
    assert_eq!(treasury(&w, CITY), 500);
    w.assert_conserved();
}

#[test]
fn each_government_pays_its_own_officials_and_posts() {
    let mut w = TestWorld::with_config(&plugins(), quiet_config());
    let names = ["Mayor", "Officer", "Judge", "President", "Member", "Banker"];
    let r: Vec<ResidentId> = names.iter().map(|name| rich(&mut w, name, 0)).collect();
    seat(&mut w, CITY, roles::MAYOR, &[r[0]]);
    seat(&mut w, OrgId::NATION, roles::PRESIDENT, &[r[3]]);
    seat(&mut w, OrgId::NATION, roles::LEGISLATOR, &[r[4]]);
    ok(&mut w, r[0], "appoint", json!({ "post": "police_officer", "resident": r[1] }));
    ok(&mut w, r[0], "appoint", json!({ "post": "judge", "resident": r[2] }));
    ok(&mut w, r[3], "appoint", json!({ "post": "central_banker", "resident": r[5] }));
    fund_treasury(&mut w, CITY, 100_000);
    fund_treasury(&mut w, OrgId::NATION, 100_000);

    let frames = step_to(&mut w, DAY);
    let paid = events(&frames, "politics.salaries_paid");
    assert_eq!(paid.len(), 2, "the nation and the city");
    assert_eq!(paid[0].data["gov"], "nation");
    assert_eq!(paid[0].data["total"], 12_000 + 4_000 + 8_000);
    assert_eq!(paid[1].data["city"], 0);
    assert_eq!(paid[1].data["total"], 8_000 + 5_000 + 6_000);
    w.step(1);
    let cash: Vec<i64> = r.iter().map(|&who| w.cash(who).cents()).collect();
    assert_eq!(cash, [8_000, 5_000, 6_000, 12_000, 4_000, 8_000]);
    // The central banker is paid by the nation, not by the central bank.
    assert_eq!(treasury(&w, OrgId::NATION), 100_000 - 24_000);
    assert_eq!(treasury(&w, CITY), 100_000 - 19_000);
    assert_eq!(treasury(&w, OrgId::CENTRAL_BANK), 0);
    w.assert_conserved();
}

#[test]
fn a_law_changes_the_salary_that_the_treasury_pays() {
    let mut w = TestWorld::with_config(&plugins(), quiet_config());
    let mayor = rich(&mut w, "Mayor", 0);
    let member = rich(&mut w, "Member", 0);
    seat(&mut w, CITY, roles::MAYOR, &[mayor]);
    seat(&mut w, CITY, roles::COUNCILLOR, &[member]);
    fund_treasury(&mut w, CITY, 100_000);
    // The council cuts the pay of the mayor. The mayor signs.
    let bill = ok(
        &mut w,
        member,
        "propose_law",
        json!({ "param": "politics.salary_mayor", "value": 125, "city": 0 }),
    );
    let bill = bill["bill"].as_u64().expect("the id");
    ok(&mut w, member, "vote_law", json!({ "bill": bill, "vote": "yes" }));
    ok(&mut w, mayor, "sign_law", json!({ "bill": bill }));
    step_to(&mut w, DAY + 1);
    assert_eq!(w.cash(mayor).cents(), 125);
    assert_eq!(w.cash(member).cents(), COUNCILLOR);
    assert_eq!(treasury(&w, CITY), 100_000 - 125 - COUNCILLOR);
    // The national value did not change.
    assert_eq!(w.param("politics.salary_mayor"), MAYOR);
    w.assert_conserved();
}
