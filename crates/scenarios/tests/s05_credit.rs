//! Story 5: credit.
//!
//! A resident owns a home. It opens a bank account, lets a deposit season
//! for a day and borrows against it. The bank holds the deposit as
//! collateral. The resident takes the loan out as cash and then pays
//! nothing. Three instalments are missed, the loan defaults, the bank
//! takes its collateral, and the city forecloses the home.
//!
//! The story starts in the republic (`unsup_scenarios::republic`). The
//! president pays the price of the home from the national treasury. All
//! other money is wages.

use serde_json::{Value, json};
use unsup_kernel::{CityId, ErrorCode, Holder, OrgId, ResidentId, Site};
use unsup_scenarios::republic::{feed, grant};
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic_in_republic};

const CITY: CityId = CityId(0);

fn bank(g: &Game, r: ResidentId) -> Value {
    g.section_full(r, "bank").unwrap_or_default()
}

/// Lets the debtor eat and sleep in a site until a frame.
fn rest_until(g: &mut Game, r: ResidentId, place: &Site, frame: u64) {
    g.walk(r, place);
    feed(g, &[r]);
    if g.attr(r, "need.energy") < 1_000 {
        g.act(r, "sleep", json!({}));
    }
    g.run_to_frame(frame);
}

fn story(g: &mut Game, republic: &Republic) {
    let dee = republic.citizens[3];
    let city = Holder::Org(OrgId::city(CITY));
    assert_eq!(g.cash(dee), 12_600, "five shifts of public works");

    // Day 6. Dee buys the cheapest home of the city with a grant of the
    // nation. A home pays property tax from its till each day: 5 basis
    // points of its value. She puts seven days of tax in the till.
    let home = g
        .sites("home")
        .into_iter()
        .min_by_key(|home| (home.attr("value"), home.id))
        .expect("a home");
    let price = home.attr("value");
    let tax = price * 5 / 10_000;
    assert_eq!((price, tax), (3_440_000, 1_720));
    grant(g, republic, dee, price + 7 * tax);
    g.walk(dee, &home);
    assert_eq!(g.act(dee, "buy_property", json!({}))["owner"], json!({ "Resident": dee }));
    g.act(dee, "till_fund", json!({ "amount": 7 * tax }));
    assert_eq!(g.site_now(home.id).owner, Some(Holder::Resident(dee)));
    assert_eq!(
        g.section(dee, "property").expect("a property section")["owned"],
        json!([{ "site": home.id, "kind": "home", "zone": g.zone_of_site(home.id), "till": 7 * tax }])
    );
    // In her own home she sleeps well.
    assert_eq!(g.act(dee, "inspect", json!({ "site": home.id }))["sleep"], "good");

    // The account costs 100. A deposit of today gives no credit today.
    let office = g.site("bank");
    g.walk(dee, &office);
    assert_eq!(g.act(dee, "open_account", json!({}))["fee"], 100);
    assert_eq!(g.act(dee, "deposit", json!({ "amount": 10_000 }))["balance"], 10_000);
    assert_eq!(g.cash(dee), 2_500);
    assert_eq!(
        bank(g, dee)["credit"],
        json!({ "limit": 0, "seasoned_balance": 0, "loans_repaid": 0, "defaults": 0 })
    );
    let request = json!({ "amount": 9_000, "term_days": 3 });
    assert_eq!(g.refused(dee, "loan_request", request.clone()), ErrorCode::NotAllowed);
    // An account alone gives no credit, however old it is.
    rest_until(g, dee, &home, Game::frame_at(7, 0, 5));
    assert_eq!(bank(g, dee)["credit"]["limit"], 0);

    // Day 8. The deposit stayed for the whole of day 7: it is the limit.
    // The deposit earned two days of interest, 2 percent a year.
    rest_until(g, dee, &home, Game::frame_at(8, 0, 5));
    assert_eq!(bank(g, dee)["balance"], 10_007);
    assert_eq!(bank(g, dee)["credit"]["limit"], 10_000);
    // The bank lends at most 90 percent of all deposits: 9,006.
    assert_eq!(
        g.refused(dee, "loan_request", json!({ "amount": 10_000, "term_days": 3 })),
        ErrorCode::Insufficient
    );
    let loan = g.act(dee, "loan_request", request);
    assert_eq!(
        loan,
        json!({
            "amount": 9_000, "rate_bp": 800, "term_days": 3, "instalment": 3_000,
            "first_due_in_ticks": 2 * 1_440 - (g.frame() - 1 - Game::frame_at(8, 0, 0)),
            "balance": 19_007, "held": 9_000,
        })
    );
    assert_eq!(
        g.last_event("finance.loan_granted").data,
        json!({ "resident": dee, "amount": 9_000, "rate_bp": 800, "term_days": 3 })
    );

    // The bank holds 9,000 as collateral. Dee cannot take the deposit and
    // the loan. She takes what is free, as cash.
    g.walk(dee, &office);
    assert_eq!(g.refused(dee, "withdraw", json!({ "amount": 19_000 })), ErrorCode::Insufficient);
    assert_eq!(g.act(dee, "withdraw", json!({ "amount": 10_000 }))["balance"], 9_007);
    assert_eq!(g.cash(dee), 12_500);

    // Day 9: nothing is due yet. Day 10 and day 11: an instalment of 3,000
    // and the interest are due. The balance is the collateral, and the
    // bank does not take an instalment from it.
    rest_until(g, dee, &home, Game::frame_at(9, 0, 5));
    assert!(g.notices(dee, "finance.instalment_missed").is_empty());
    rest_until(g, dee, &home, Game::frame_at(10, 0, 5));
    assert_eq!(
        g.notice(dee, "finance.instalment_missed").data,
        json!({ "due": 3_025, "balance": 9_013, "held": 9_000, "missed": 1, "default_after": 3 })
    );
    rest_until(g, dee, &home, Game::frame_at(11, 0, 5));
    assert_eq!(
        g.notice(dee, "finance.instalment_missed").data,
        json!({ "due": 6_051, "balance": 9_020, "held": 9_000, "missed": 2, "default_after": 3 })
    );
    assert_eq!(bank(g, dee)["loan"]["missed"], 2);
    // The home is still hers, and its till paid the tax of five days.
    assert_eq!(g.site_now(home.id).owner, Some(Holder::Resident(dee)));
    assert_eq!(g.money(Holder::Site(home.id)), 2 * tax);
    assert!(g.events_of("economy.property_foreclosed").is_empty());

    // Day 12, midnight: the third miss is the default. The bank takes the
    // balance: the whole principal and a part of the interest.
    rest_until(g, dee, &home, Game::frame_at(12, 0, 0) - 1);
    let treasury = g.city_treasury(CITY);
    g.step(1);
    let defaulted = g.last_event("finance.loan_defaulted");
    assert_eq!(defaulted.frame, Game::frame_at(12, 0, 0));
    assert_eq!(defaulted.data, json!({ "resident": dee, "outstanding": 51 }));
    g.step(3);
    assert_eq!(
        g.notice(dee, "finance.loan_defaulted").data,
        json!({ "outstanding": 51, "seized": 9_026, "bankrupt_until": { "year": 2, "day": 12 } })
    );
    assert_eq!(
        bank(g, dee),
        json!({
            "balance": 0,
            "bankrupt_until": { "year": 2, "day": 12 },
            "credit": { "limit": 0, "seasoned_balance": 0, "loans_repaid": 0, "defaults": 1 },
        })
    );
    // The cash that she took out is not seized.
    assert_eq!(g.cash(dee), 12_500);

    // The city forecloses the home in the next frame.
    let foreclosed = g.last_event("economy.property_foreclosed");
    assert_eq!(foreclosed.frame, Game::frame_at(12, 0, 0) + 1);
    assert_eq!(
        foreclosed.data,
        json!({ "site": home.id, "city": 0, "from": { "Resident": dee }, "reason": "loan_default" })
    );
    assert_eq!(g.site_now(home.id).owner, Some(city));
    assert_eq!(
        g.notice(dee, "economy.property_lost").data,
        json!({ "site": home.id, "reason": "loan_default" })
    );
    assert_eq!(g.section(dee, "property"), None);
    // The home is for sale again, and Dee sleeps there like a stranger.
    assert_eq!(g.act(dee, "inspect", json!({ "site": home.id }))["sleep"], "poor");
    let offers =
        g.act(dee, "list_property", json!({ "kind": "home", "offer": "sale", "max_price": price }));
    assert_eq!(offers["listings"][0]["site"], json!(home.id));
    // A bankrupt resident gets no loan.
    assert_eq!(
        g.refused(dee, "loan_request", json!({ "amount": 1_000, "term_days": 2 })),
        ErrorCode::NotAllowed
    );

    // The money. The loan made 9,000 and the default destroyed 9,000: the
    // bank lost no principal, and the loan made no money.
    let flow = |name: &str| {
        let defs = g.world.world.blueprint();
        let global = g.world.global();
        let ledger = global.ledger();
        let made = defs.faucet_id(name).map_or(0, |faucet| ledger.minted_by(faucet).cents());
        let gone = defs.sink_id(name).map_or(0, |sink| ledger.burned_by(sink).cents());
        (made, gone)
    };
    assert_eq!(flow("bank.loan_issue"), (9_000, 0));
    assert_eq!(flow("bank.loan_repay"), (0, 9_000));
    // The bank treasury has the fee of the account. It paid 26 of interest
    // on the deposit in five days, and the default gave it 26 of the
    // interest of the loan.
    assert_eq!(g.money(Holder::Org(OrgId::CENTRAL_BANK)), 100 - 26 + 26);
    // The city has the two days of tax that were left in the till of the
    // home, or the till still has them for the city. At midnight the city
    // paid its five officials.
    let salaries = 8_000 + 2 * 3_000 + 5_000 + 6_000;
    let in_till = g.money(Holder::Site(home.id));
    assert_eq!(g.city_treasury(CITY) + in_till, treasury + 2 * tax - salaries);
    g.audit();
}

#[test]
fn a_loan_defaults_and_the_city_forecloses_the_home_of_the_debtor() {
    assert_deterministic_in_republic(&Setup::town(1), story);
}
