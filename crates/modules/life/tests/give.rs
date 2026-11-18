//! Hand-to-hand transfer and goods in a site: `give`, `drop`, `pick_up`.

mod common;

use common::{make_owner, make_tenant, result, site, world};
use unsup_kernel::protocol::{ActionStatus, ErrorCode};
use unsup_kernel::serde_json::json;
use unsup_kernel::{Holder, Money, Pos, UNCLAIMED};
use unsup_life::GIFT_RECEIVED;

#[test]
fn give_moves_cash_and_goods_to_a_resident_next_to_the_giver() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(10, 0));
    let bob = w.spawn_at("Bob", Pos::new(11, 1));
    w.give_money(ada, Money(1_000));
    w.give_item(ada, "bread", 5);

    // Cash and goods in one gift, to a resident on a tile beside.
    let status =
        w.act_ok(ada, "give", json!({ "to": bob, "money": 300, "item": "bread", "qty": 2 }));
    assert!(matches!(status, ActionStatus::Started { ticks: 1, .. }), "{status:?}");
    // The gift is handed over at once; the giver is busy for one tick.
    assert_eq!((w.cash(ada), w.cash(bob)), (Money(700), Money(300)));
    assert_eq!(w.qty(Holder::Resident(bob), "bread"), 2);
    w.step(1);
    assert_eq!(
        result(&w, ada),
        json!({ "from": ada, "to": bob, "money": 300, "item": "bread", "qty": 2 })
    );
    let notice = w.notices(bob).into_iter().find(|n| n.kind == GIFT_RECEIVED).expect("a notice");
    assert_eq!(notice.from, Some(ada));
    assert_eq!(notice.data["money"], 300);
    assert_eq!(notice.data["qty"], 2);

    // Only cash, on the same tile. Only goods, one unit by default.
    w.set_pos(bob, Pos::new(10, 0));
    w.act_ok(ada, "give", json!({ "to": bob, "money": 700 }));
    w.step(1);
    w.act_ok(ada, "give", json!({ "to": bob, "item": "bread" }));
    w.step(1);
    assert_eq!((w.cash(ada), w.cash(bob)), (Money::ZERO, Money(1_000)));
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 2);
    assert_eq!(w.qty(Holder::Resident(bob), "bread"), 3);
    let totals = w.assert_conserved();
    assert_eq!(totals.money.held, 1_000);
}

#[test]
fn give_refusals_change_nothing() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(10, 0));
    let bob = w.spawn_at("Bob", Pos::new(11, 0));
    let far = w.spawn_at("Far", Pos::new(12, 0));
    let away = w.spawn_at("Away", Pos::new(100, 100));
    w.give_money(ada, Money(1_000));
    w.give_item(ada, "bread", 2);

    let cases = [
        // To oneself.
        (json!({ "to": ada, "money": 1 }), ErrorCode::InvalidArgs),
        // Nothing to give.
        (json!({ "to": bob }), ErrorCode::InvalidArgs),
        (json!({ "to": bob, "money": 0 }), ErrorCode::InvalidArgs),
        // A negative amount, a quantity below one, a quantity without item.
        (json!({ "to": bob, "money": -5 }), ErrorCode::InvalidArgs),
        (json!({ "to": bob, "item": "bread", "qty": 0 }), ErrorCode::InvalidArgs),
        (json!({ "to": bob, "qty": 2 }), ErrorCode::InvalidArgs),
        // No receiver.
        (json!({ "money": 5 }), ErrorCode::InvalidArgs),
        // An item that does not exist.
        (json!({ "to": bob, "item": "gold" }), ErrorCode::NotFound),
        // Two tiles away, in another zone, not in the world.
        (json!({ "to": far, "money": 5 }), ErrorCode::NotHere),
        (json!({ "to": away, "money": 5 }), ErrorCode::NotHere),
        (json!({ "to": 9_999, "money": 5 }), ErrorCode::NotHere),
        // More than the giver has.
        (json!({ "to": bob, "money": 1_001 }), ErrorCode::Insufficient),
        (json!({ "to": bob, "item": "bread", "qty": 3 }), ErrorCode::Insufficient),
        (json!({ "to": bob, "item": "stone" }), ErrorCode::Insufficient),
        // A gift is whole or not at all: the cash is there, the goods are not.
        (json!({ "to": bob, "money": 10, "item": "bread", "qty": 3 }), ErrorCode::Insufficient),
        (json!({ "to": bob, "money": 1_001, "item": "bread", "qty": 1 }), ErrorCode::Insufficient),
    ];
    for (args, code) in cases {
        let error = w.act_err(ada, "give", args.clone());
        assert_eq!(error.code, code, "{args}: {}", error.message);
        // A new tick, so that the limit of actions for each tick is not hit.
        w.step(1);
    }
    assert_eq!(w.cash(ada), Money(1_000));
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 2);
    for other in [bob, far, away] {
        assert_eq!(w.cash(other), Money::ZERO);
        assert_eq!(w.qty(Holder::Resident(other), "bread"), 0);
    }
    assert_eq!(w.money(UNCLAIMED), Money::ZERO);
    w.assert_conserved();
}

#[test]
fn a_newcomer_has_nothing_to_give() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn_at("Bob", w.pos(ada));
    assert_eq!(w.cash(ada), Money::ZERO);
    assert!(w.zone(w.zone_of(ada)).ledger().items_of(Holder::Resident(ada)).is_empty());
    assert_eq!(
        w.act_err(ada, "give", json!({ "to": bob, "money": 1 })).code,
        ErrorCode::Insufficient
    );
}

#[test]
fn goods_can_be_left_in_a_site_and_taken_back_by_who_lives_there() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.give_item(ada, "bread", 5);
    w.give_item(bob, "bread", 1);

    // Outside every site.
    w.set_pos(ada, Pos::new(0, 0));
    assert_eq!(w.act_err(ada, "drop", json!({ "item": "bread" })).code, ErrorCode::NotHere);
    assert_eq!(w.act_err(ada, "pick_up", json!({ "item": "bread" })).code, ErrorCode::NotHere);

    let (zone, home) = site(&w, "home");
    let stock = Holder::Site(home.id);
    w.set_pos(ada, home.rect.centre());
    w.set_pos(bob, home.rect.centre());

    // Everybody can leave goods.
    w.act_ok(ada, "drop", json!({ "item": "bread", "qty": 3 }));
    w.step(1);
    assert_eq!(result(&w, ada), json!({ "item": "bread", "qty": 3 }));
    assert_eq!(w.qty(stock, "bread"), 3);
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 2);

    // The city owns the home: nobody can take goods out.
    assert_eq!(w.act_err(ada, "pick_up", json!({ "item": "bread" })).code, ErrorCode::NotAllowed);
    // The owner can, and a tenant can.
    make_owner(&w, zone, home.id, ada);
    make_tenant(&w, zone, home.id, bob);
    w.act_ok(ada, "pick_up", json!({ "item": "bread", "qty": 2 }));
    w.act_ok(bob, "pick_up", json!({ "item": "bread" }));
    w.step(1);
    assert_eq!(w.qty(stock, "bread"), 0);
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 4);
    assert_eq!(w.qty(Holder::Resident(bob), "bread"), 2);

    // More than there is, a wrong quantity, an unknown item.
    assert_eq!(w.act_err(ada, "pick_up", json!({ "item": "bread" })).code, ErrorCode::Insufficient);
    assert_eq!(
        w.act_err(ada, "drop", json!({ "item": "bread", "qty": 9 })).code,
        ErrorCode::Insufficient
    );
    assert_eq!(
        w.act_err(ada, "drop", json!({ "item": "bread", "qty": 0 })).code,
        ErrorCode::InvalidArgs
    );
    w.step(1);
    assert_eq!(w.act_err(ada, "drop", json!({ "item": "gold" })).code, ErrorCode::NotFound);
    w.assert_conserved();
}
