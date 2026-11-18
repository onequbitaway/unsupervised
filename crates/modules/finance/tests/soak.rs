//! A busy town: random actions of every kind for three game days. The
//! audit of the world and the invariants of the module hold in every
//! frame.

mod common;

use common::{AT_BANK, AT_EXCHANGE, STREET, SUBURB, conserved, grant, world};
use serde_json::{Value, json};
use unsup_finance::bank::Bank;
use unsup_finance::exchange::{Exchange, Side};
use unsup_finance::shares::Shares;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, Money, OrgId, ResidentId, Rng, ZoneId};

const ORGS: [OrgId; 2] = [OrgId(3_000_000), OrgId(3_000_001)];
const ITEMS: [&str; 3] = ["wheat", "fish", "ore"];

/// Checks everything that must be true between two frames.
fn invariants(w: &TestWorld<'_>) {
    conserved(w);
    let global = w.global();

    // The bank: the loan total is the sum of the loans, and no loan is
    // empty or owes more than its principal now.
    let bank = global.res::<Bank>();
    let mut principal = 0;
    for (resident, account) in bank.accounts() {
        assert!(account.low_today <= Bank::balance(global, resident), "low water of {resident}");
        if let Some(loan) = &account.loan {
            principal += loan.principal;
            assert!(loan.principal > 0, "an empty loan of {resident} is still open");
            assert!((0..=loan.principal).contains(&loan.due_principal));
            assert!(loan.due_interest >= 0 && loan.interest_carry >= 0);
        }
    }
    assert_eq!(bank.outstanding_principal(), principal);

    // The exchange: every escrow covers what its order locks, the books
    // list exactly the open orders, sorted, and no bid crosses an ask.
    let zone = w.zone(ZoneId(0));
    let exchange = zone.res::<Exchange>();
    let ledger = zone.ledger();
    for order in exchange.orders() {
        assert!((0..=order.qty).contains(&order.filled));
        assert_eq!(order.open, order.remaining() > 0, "order {}", order.id);
        assert!(ledger.money(order.escrow()).0 >= order.locked_cash(), "order {}", order.id);
        assert!(ledger.qty(order.escrow(), order.item) >= order.locked_goods());
    }
    let mut quoted = 0;
    for (item, book) in exchange.books() {
        for (side, quotes) in [(Side::Buy, book.bids()), (Side::Sell, book.asks())] {
            quoted += quotes.len();
            for quote in quotes {
                let order = exchange.order(quote.order).expect("a quote names an order");
                assert!(order.open && order.item == item && order.side == side);
                assert_eq!((order.price, order.seq), (quote.price, quote.seq));
            }
            // Best last: a better price later, and the older order later
            // among equal prices.
            for pair in quotes.windows(2) {
                let better = match side {
                    Side::Buy => pair[0].price < pair[1].price,
                    Side::Sell => pair[0].price > pair[1].price,
                };
                let older = pair[0].price == pair[1].price && pair[0].seq > pair[1].seq;
                assert!(better || older, "the book of item {item} is not sorted");
            }
        }
        if let (Some(bid), Some(ask)) = (book.bids().last(), book.asks().last()) {
            assert!(bid.price < ask.price, "a bid crosses an ask after matching");
        }
    }
    assert_eq!(quoted, exchange.orders().filter(|order| order.open).count());

    // The shares: the holdings are the shares outstanding, and the locks
    // match the open orders.
    let shares = global.res::<Shares>();
    for (org, company) in shares.companies() {
        let held: i64 = company.holders().map(|(_, holding)| holding.held).sum();
        assert_eq!(held, company.outstanding(), "shares of {org}");
        let offered: i64 =
            shares.orders().filter(|o| o.org == org && o.issue).map(|o| o.remaining()).sum();
        assert_eq!(offered, company.on_offer(), "new shares of {org} on offer");
        for (resident, holding) in company.holders() {
            let locked: i64 = shares
                .orders()
                .filter(|o| {
                    o.org == org && !o.issue && o.side == Side::Sell && o.placed_by == resident
                })
                .map(|o| o.remaining())
                .sum();
            assert!(holding.held > 0);
            assert_eq!(holding.locked, locked, "locked shares of {resident} in {org}");
            assert!(holding.locked <= holding.held);
        }
        let quotes = company.book().bids().len() + company.book().asks().len();
        assert_eq!(quotes, shares.orders().filter(|o| o.org == org).count());
        if let (Some(bid), Some(ask)) = (company.book().bids().last(), company.book().asks().last())
        {
            assert!(bid.price < ask.price, "a share bid crosses an ask");
        }
    }
    for order in shares.orders() {
        assert!(order.remaining() > 0, "a filled share order is still open");
        let locked = global.ledger().money(order.escrow()).0;
        let expected = if order.side == Side::Buy { order.remaining() * order.price } else { 0 };
        assert_eq!(locked, expected, "escrow of share order {}", order.id);
    }
}

/// Sends one random action of a resident. Many of them are refused; that
/// is part of the test.
fn random_action(w: &mut TestWorld<'_>, rng: &mut Rng, who: ResidentId, others: &[ResidentId]) {
    let other = others[rng.below(others.len() as u64) as usize];
    let org = ORGS[rng.below(2) as usize];
    let item = ITEMS[rng.below(3) as usize];
    let side = if rng.chance(1, 2) { "buy" } else { "sell" };
    let amount = rng.range(1, 6_000);
    let (verb, args) = match rng.below(25) {
        0 => ("open_account", Value::Null),
        1 | 2 => ("deposit", json!({ "amount": amount })),
        3 => ("withdraw", json!({ "amount": amount })),
        4 | 5 => ("transfer", json!({ "to": other, "amount": amount })),
        6 => ("loan_request", json!({ "amount": amount * 3, "term_days": rng.range(1, 6) })),
        7 => ("loan_repay", json!({ "amount": amount })),
        8..=11 => (
            "order_place",
            json!({ "item": item, "side": side, "qty": rng.range(1, 9), "price": rng.range(80, 121) }),
        ),
        12 => {
            let own: Vec<u64> =
                w.zone(ZoneId(0)).res::<Exchange>().orders_of(who).map(|order| order.id).collect();
            let order = own.get(rng.below(own.len().max(1) as u64) as usize).copied().unwrap_or(7);
            ("order_cancel", json!({ "order": order }))
        }
        13 => ("market_claim", Value::Null),
        14 => ("market_book", json!({ "item": item })),
        15 => (
            "issue_shares",
            json!({ "org": org, "shares": rng.range(1, 50), "price": rng.range(8, 13) }),
        ),
        16 | 17 => (
            "buy_shares",
            json!({ "org": org, "shares": rng.range(1, 20), "price": rng.range(8, 13) }),
        ),
        18 => (
            "sell_shares",
            json!({ "org": org, "shares": rng.range(1, 20), "price": rng.range(8, 13) }),
        ),
        19 => {
            let own: Vec<u64> =
                w.global().res::<Shares>().orders_of(who).map(|order| order.id).collect();
            let order = own.get(rng.below(own.len().max(1) as u64) as usize).copied().unwrap_or(7);
            ("cancel_shares", json!({ "order": order }))
        }
        20 => ("pay_dividend", json!({ "org": org, "amount": rng.range(1, 400) })),
        21 => ("central_bank_issue", json!({ "amount": amount })),
        22 => ("central_bank_retire", json!({ "amount": amount / 2 })),
        23 => ("central_bank_set_rate", json!({ "rate_bp": rng.range(0, 3_001) })),
        // Stops whatever runs: an order on its way to the book, or a
        // request that has already left for the global realm.
        _ => ("cancel", Value::Null),
    };
    w.act(who, verb, args);
}

#[test]
fn random_actions_keep_every_invariant() {
    let mut w = world();
    w.set_param("finance.loan_base_limit", 15_000);
    w.set_param("finance.account_open_fee", 50);
    let mut rng = Rng::seeded(2024);
    let places = [AT_BANK, AT_EXCHANGE, AT_EXCHANGE, STREET, SUBURB];
    let mut residents: Vec<ResidentId> = Vec::new();
    let spawn = |w: &mut TestWorld<'_>, rng: &mut Rng, residents: &mut Vec<ResidentId>| {
        let r = w.spawn_at(&format!("r{}", residents.len()), AT_BANK);
        w.give_money(r, Money(rng.range(2_000, 30_000)));
        for item in ITEMS {
            w.give_item(r, item, rng.range(0, 30));
        }
        // Everybody is a treasurer of one organization, and one in four
        // is a central banker.
        grant(w, r, ORGS[residents.len() % 2], &["treasury.spend"]);
        if residents.len().is_multiple_of(4) {
            grant(w, r, OrgId::CENTRAL_BANK, &["bank.central"]);
        }
        residents.push(r);
    };
    for _ in 0..14 {
        spawn(&mut w, &mut rng, &mut residents);
    }

    let mut deaths = 0;
    for frame in 0..3 * 1_440 {
        for _ in 0..3 {
            let who = residents[rng.below(residents.len() as u64) as usize];
            if !w.is_alive(who) {
                continue;
            }
            match rng.below(40) {
                // Walk somewhere.
                0..=3 => w.set_pos(who, places[rng.below(places.len() as u64) as usize]),
                // Die, with or without an heir, or be removed.
                4 if frame % 97 == 0 => {
                    let heir = residents[rng.below(residents.len() as u64) as usize];
                    let heir =
                        (rng.chance(2, 3) && w.is_alive(heir) && heir != who).then_some(heir);
                    w.act(who, "test_die", json!({ "heir": heir, "silent": rng.chance(1, 4) }));
                    deaths += 1;
                    spawn(&mut w, &mut rng, &mut residents);
                }
                _ => random_action(&mut w, &mut rng, who, &residents),
            }
        }
        w.step(1);
        invariants(&w);
    }

    // The run did real work of every kind.
    assert!(deaths >= 3, "{deaths} deaths");
    let bank = w.global().res::<Bank>();
    assert!(bank.accounts().count() >= 8);
    assert!(common::minted(&w, "bank.loan_issue") > 0);
    assert!(common::burned(&w, "bank.loan_repay") > 0);
    assert!(common::minted(&w, "central_bank.issue") > 0);
    let shares = w.global().res::<Shares>();
    assert!(shares.companies().all(|(_, company)| company.last_price() > 0));
    assert_eq!(shares.companies().count(), 2);
    let exchange = w.zone(ZoneId(0)).res::<Exchange>();
    assert!(exchange.books().filter(|(_, book)| book.last_price() > 0).count() == 3);
    let city = Holder::Org(OrgId::city(unsup_kernel::CityId(0)));
    assert!(w.money(city).is_positive(), "the city got exchange fees");
    let stats = w.global().res::<unsup_finance::stats::Stats>();
    let latest = stats.latest().expect("three days were published");
    assert_eq!(latest.day, 3);
    // The last frame was the start of day 4: the statistics are from it.
    let money = w.assert_conserved().money;
    assert_eq!(latest.money_supply, money.minted - money.burned);
    assert_eq!(latest.loans.outstanding, bank.outstanding_principal());
}
