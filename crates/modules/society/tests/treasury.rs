//! The treasury: deposits from a zone to the global realm, and payments
//! from the global realm to a resident in any zone or to another
//! organization.

mod common;

use common::{CITY, FAR, enlist, events, fail, found, notice, ok, rich, world};
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, OrgId, ResidentId, ZoneId};

#[test]
fn a_deposit_moves_cash_to_the_treasury() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_800);
    let bob = rich(&mut w, "Bob", 90);
    let org = found(&mut w, ada, "Acme");
    let treasury = Holder::Org(org);

    w.act_ok(ada, "org_deposit", json!({ "org": org, "amount": 650 }));
    // The cash is on its way: not with Ada, not yet in the treasury.
    assert_eq!((w.cash(ada), w.money(treasury)), (Money(150), Money(0)));
    assert_eq!(w.assert_conserved().money.in_flight, 650);
    let frames = w.step(2);
    assert_eq!(w.last_report(ada).map(|r| r.result), Some(json!({ "org": org, "amount": 650 })));
    assert_eq!((w.cash(ada), w.money(treasury)), (Money(150), Money(650)));
    assert_eq!(
        w.global().ledger().money(treasury),
        Money(650),
        "a treasury is in the global realm"
    );
    let deposited = events(&frames, "society.org_deposited");
    assert_eq!(deposited.len(), 1);
    assert_eq!(
        (deposited[0].data["from"].clone(), deposited[0].data["amount"].clone()),
        (json!(ada), json!(650))
    );

    // A resident that is not a member can give money too, also to a government.
    ok(&mut w, bob, "org_deposit", json!({ "org": org, "amount": 40 }));
    ok(&mut w, bob, "org_deposit", json!({ "org": OrgId::NATION, "amount": 50 }));
    assert_eq!((w.cash(bob), w.money(treasury)), (Money(0), Money(690)));
    assert_eq!(w.money(Holder::Org(OrgId::NATION)), Money(50));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.in_flight), (1_890, 0));
}

#[test]
fn a_refused_deposit_keeps_no_money() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 300);
    assert_eq!(
        fail(&mut w, ada, "org_deposit", json!({ "org": 1, "amount": 0 })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(&mut w, ada, "org_deposit", json!({ "org": 1, "amount": -5 })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(&mut w, ada, "org_deposit", json!({ "org": 1, "amount": 301 })),
        ErrorCode::Insufficient
    );
    assert_eq!(w.cash(ada), Money(300));

    // No organization has this id. The global realm refuses and sends the
    // cash back.
    w.act_ok(ada, "org_deposit", json!({ "org": 9_000_000, "amount": 300 }));
    assert_eq!(w.cash(ada), Money(0));
    w.step(1);
    assert_eq!(w.cash(ada), Money(0));
    assert_eq!(w.assert_conserved().money.in_flight, 300, "the refund is on its way");
    w.step(1);
    assert_eq!(w.last_report(ada).and_then(|r| r.error).map(|e| e.code), Some(ErrorCode::NotFound));
    assert_eq!(w.cash(ada), Money(300));
    assert_eq!(w.money(Holder::Org(OrgId(9_000_000))), Money(0));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.in_flight), (300, 0));
    // Nothing stays in a holding escrow or anywhere else in the global realm.
    assert_eq!(w.global().ledger().balances().count(), 0);
}

#[test]
fn a_payment_goes_from_the_treasury_to_a_resident_in_another_zone() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 2_000);
    let far = w.spawn_at("Far", FAR);
    assert_eq!(w.zone_of(far), ZoneId(3));
    let org = found(&mut w, ada, "Acme");
    ok(&mut w, ada, "org_deposit", json!({ "org": org, "amount": 1_000 }));

    w.act_ok(ada, "org_pay", json!({ "org": org, "amount": 400, "to_resident": far }));
    // Frame 1: the global realm pays. Frame 2: the money and the answer arrive.
    let mut frames = w.step(1);
    assert_eq!((w.money(Holder::Org(org)), w.cash(far)), (Money(600), Money(0)));
    assert_eq!(w.assert_conserved().money.in_flight, 400);
    frames.extend(w.step(1));
    assert_eq!(w.cash(far), Money(400));
    assert_eq!(
        w.last_report(ada).map(|r| r.result),
        Some(json!({ "org": org, "amount": 400, "treasury": 600 }))
    );
    let told = notice(&w, far, "society.org_payment");
    assert_eq!(
        (told.from, told.data["amount"].clone(), told.data["org"].clone()),
        (Some(ada), json!(400), json!(org))
    );
    let paid = events(&frames, "society.org_paid");
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0].data["to"], json!({ "Resident": far }));
    assert_eq!(
        (paid[0].data["by"].clone(), paid[0].data["amount"].clone()),
        (json!(ada), json!(400))
    );

    // To another organization: the treasury of the city.
    ok(
        &mut w,
        ada,
        "org_pay",
        json!({ "org": org, "amount": 250, "to_org": OrgId::city(unsup_kernel::CityId(0)) }),
    );
    assert_eq!(w.money(Holder::Org(org)), Money(350));
    assert_eq!(w.money(CITY), Money(1_000 + 250));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.minted), (2_000, 2_000));
}

#[test]
fn org_pay_checks_the_permission_in_the_zone_and_the_rest_in_the_global_realm() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_500);
    let bob = w.spawn("Bob");
    let org = found(&mut w, ada, "Acme");
    enlist(&mut w, org, ada, bob);
    ok(&mut w, ada, "org_deposit", json!({ "org": org, "amount": 500 }));
    let pay =
        |amount: i64, to: ResidentId| json!({ "org": org, "amount": amount, "to_resident": to });

    // The zone refuses at once: Bob has no `treasury.spend` in its copy of
    // the memberships. Nothing is sent to the global realm.
    let ack = w.act(bob, "org_pay", pay(100, bob));
    assert!(
        matches!(ack.status, unsup_kernel::protocol::ActionStatus::Rejected { ref error } if error.code == ErrorCode::NotAllowed)
    );
    assert_eq!(
        fail(&mut w, ada, "org_pay", json!({ "org": org, "amount": 100 })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(
            &mut w,
            ada,
            "org_pay",
            json!({ "org": org, "amount": 100, "to_resident": bob, "to_org": 1 })
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(fail(&mut w, ada, "org_pay", pay(0, bob)), ErrorCode::InvalidArgs);
    // The global realm refuses what only it knows.
    assert_eq!(fail(&mut w, ada, "org_pay", pay(501, bob)), ErrorCode::Insufficient);
    assert_eq!(fail(&mut w, ada, "org_pay", pay(100, ResidentId(999))), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, ada, "org_pay", json!({ "org": org, "amount": 100, "to_org": 9_000_000 })),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, ada, "org_pay", json!({ "org": org, "amount": 100, "to_org": org })),
        ErrorCode::InvalidArgs
    );
    assert_eq!((w.money(Holder::Org(org)), w.cash(bob)), (Money(500), Money(0)));

    // The registry decides, not the copy in the zone. Bob gets the right to
    // spend, then loses it in the same frame in which he asks to pay.
    ok(&mut w, ada, "org_set_role", json!({ "org": org, "resident": bob, "role": "owner" }));
    w.step(1);
    w.act_ok(ada, "org_set_role", json!({ "org": org, "resident": bob, "role": "member" }));
    let ack = w.act(bob, "org_pay", pay(500, bob));
    assert_eq!(
        ack.status,
        unsup_kernel::protocol::ActionStatus::Pending,
        "the zone still believes Bob"
    );
    w.step(2);
    let report = w.last_report(bob).expect("the answer");
    assert_eq!(report.error.map(|e| e.code), Some(ErrorCode::NotAllowed));
    assert_eq!((w.money(Holder::Org(org)), w.cash(bob)), (Money(500), Money(0)));
    w.assert_conserved();
}
