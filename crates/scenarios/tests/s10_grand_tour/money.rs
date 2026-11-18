//! The bank, credit, the exchange, shares and the central bank.

use serde_json::json;
use unsup_kernel::Holder;
use unsup_scenarios::{Game, Republic};

use crate::Tour;

fn balance(g: &Game, r: unsup_kernel::ResidentId) -> i64 {
    g.money(Holder::Account(r.0))
}

/// Day 6: three accounts, deposits, a transfer, a withdrawal, and the work
/// of the central banker.
pub fn bank(g: &mut Game, cast: &Republic) {
    let [_, ben, cal, dee] = cast.citizens;
    let office = g.site("bank");
    g.walk_all(&[ben, cal, dee], &office);
    for (resident, deposit) in [(ben, 5_000), (cal, 8_000), (dee, 11_000)] {
        g.act(resident, "open_account", json!({}));
        assert_eq!(g.act(resident, "deposit", json!({ "amount": deposit }))["balance"], deposit);
    }
    // A transfer costs 10 basis points on top.
    let sent = g.act(cal, "transfer", json!({ "to": dee, "amount": 1_000 }));
    assert_eq!(sent, json!({ "sent": 1_000, "fee": 1, "balance": 6_999 }));
    assert_eq!(g.act(dee, "withdraw", json!({ "amount": 2_000 }))["balance"], 10_000);
    assert_eq!((balance(g, ben), balance(g, cal), balance(g, dee)), (5_000, 6_999, 10_000));

    // The central banker raises the base rate and takes 100,000 of the
    // national treasury out of the world.
    let bram = cast.banker;
    let treasury = g.national_treasury();
    g.act(bram, "central_bank_set_rate", json!({ "rate_bp": 600 }));
    assert_eq!(g.param("finance.base_rate_bp"), 600);
    g.act(bram, "central_bank_retire", json!({ "amount": 100_000 }));
    assert_eq!(g.national_treasury(), treasury - 100_000);
}

/// Day 8: the deposit of Dee stayed for a full day. She borrows against it
/// and repays at once.
pub fn credit(g: &mut Game, tour: &Tour) {
    let dee = tour.cast.citizens[3];
    let loan = g.act(dee, "loan_request", json!({ "amount": 5_000, "term_days": 2 }));
    // The rate is the base rate of 600 and the spread of 300.
    assert_eq!((loan["rate_bp"].as_i64(), loan["held"].as_i64()), (Some(900), Some(5_000)));
    let before = balance(g, dee);
    let repaid = g.act(dee, "loan_repay", json!({ "amount": 5_000 }));
    assert_eq!(
        (repaid["closed"].as_bool(), repaid["balance"].as_i64()),
        (Some(true), Some(before - 5_000))
    );
}

/// Day 8: an order book for vegetables at the exchange, and the shares of
/// the guild.
pub fn markets(g: &mut Game, tour: &Tour) {
    let [ann, _, cal, dee] = tour.cast.citizens;
    let floor = g.site("exchange");
    g.walk_all(&[cal, dee], &floor);
    // Cal sells five vegetables for 200 each, and Dee buys them. The
    // seller pays the fee of 50 basis points to the city.
    let (cal_cash, dee_food) = (g.cash(cal), g.carried(dee, "vegetables"));
    g.act(
        cal,
        "order_place",
        json!({ "item": "vegetables", "side": "sell", "qty": 5, "price": 200 }),
    );
    g.act(
        dee,
        "order_place",
        json!({ "item": "vegetables", "side": "buy", "qty": 5, "price": 200 }),
    );
    g.step(1);
    let book = g.act(dee, "market_book", json!({ "item": "vegetables" }));
    assert_eq!(book["last_price"], 200, "{book}");
    g.act(cal, "market_claim", json!({}));
    g.act(dee, "market_claim", json!({}));
    assert_eq!((g.cash(cal), g.carried(dee, "vegetables")), (cal_cash + 995, dee_food + 5));
    // An order that nobody takes is cancelled.
    let high = g.act(
        cal,
        "order_place",
        json!({ "item": "vegetables", "side": "sell", "qty": 2, "price": 900 }),
    );
    let order = high["order"].as_u64().expect("an order id");
    let food = g.carried(cal, "vegetables");
    g.act(cal, "order_cancel", json!({ "order": order }));
    assert_eq!(g.carried(cal, "vegetables"), food + 2);

    // The guild issues 100 shares for 50 each. Cal buys ten from his bank
    // account, offers five of them for 80, and takes the offer back.
    let guild = tour.guild;
    g.act(ann, "issue_shares", json!({ "org": guild, "shares": 100, "price": 50 }));
    let treasury = g.money(Holder::Org(guild));
    let account = balance(g, cal);
    g.act(cal, "buy_shares", json!({ "org": guild, "shares": 10, "price": 50 }));
    assert_eq!((balance(g, cal), g.money(Holder::Org(guild))), (account - 500, treasury + 500));
    let ask = g.act(cal, "sell_shares", json!({ "org": guild, "shares": 5, "price": 80 }));
    let book = g.act(cal, "shares_book", json!({ "org": guild }));
    assert_eq!((book["last_price"].as_i64(), book["outstanding"].as_i64()), (Some(50), Some(10)));
    g.act(cal, "cancel_shares", json!({ "order": ask["order"] }));
    // A dividend of 300 goes to the only holder.
    g.act(ann, "pay_dividend", json!({ "org": guild, "amount": 300 }));
    assert_eq!((balance(g, cal), g.money(Holder::Org(guild))), (account - 200, treasury + 200));
}
