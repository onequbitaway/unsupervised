//! A request that is interrupted on its way to the global realm: the
//! global realm still does the work, no money is lost, and the resident
//! hears about it.

mod common;

use common::{
    AT_EXCHANGE, OPEN_FEE, account_holder, balance, bank_treasury, burned, conserved, customer,
    grant, minted, notice, ok, world,
};
use serde_json::{Value, json};
use unsup_finance::bank::Bank;
use unsup_finance::exchange::Exchange;
use unsup_finance::shares::Shares;
use unsup_kernel::protocol::ActionStatus;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Holder, Money, OrgId, ResidentId, ZoneId};

const ACME: OrgId = OrgId(3_000_000);
const NOTICE: &str = "finance.request_still_handled";

/// Sends an action that waits for the global realm, stops it with `stop`
/// before the next frame, and checks what the resident is told: one report
/// with the error `interrupted`, and the notice of this module.
fn interrupted(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
    reason: &str,
    stop: impl Fn(&mut TestWorld<'_>, ResidentId),
) {
    let ack = w.act(r, verb, args);
    assert_eq!(ack.status, ActionStatus::Pending, "{verb}");
    stop(w, r);
    w.step(2);
    let reports = w.reports(r);
    let own: Vec<_> = reports.iter().filter(|report| report.action_id == ack.action_id).collect();
    assert_eq!(own.len(), 1, "{verb}: the late answer of the global realm is not a report");
    assert_eq!(own[0].error.as_ref().map(|error| error.code), Some(ErrorCode::Interrupted));
    assert_eq!(
        notice(w, r, NOTICE),
        Some(json!({ "verb": verb, "action_id": ack.action_id, "reason": reason })),
        "{verb}"
    );
    assert_eq!(w.assert_conserved().money.in_flight, 0, "{verb}: nothing is left between realms");
}

fn cancel(w: &mut TestWorld<'_>, r: ResidentId) {
    w.act_ok(r, "cancel", Value::Null);
}

#[test]
fn an_interrupted_request_still_takes_effect_and_the_resident_is_told() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", OPEN_FEE + 1_000);
    grant(&mut w, ann, ACME, &["treasury.spend"]);
    grant(&mut w, ann, OrgId::CENTRAL_BANK, &["bank.central"]);
    let bob = account_holder(&mut w, "Bob", 5_000);
    let total = conserved(&w);

    interrupted(&mut w, ann, "open_account", Value::Null, "cancelled", cancel);
    assert!(w.global().res::<Bank>().is_open(ann));
    assert_eq!((w.cash(ann), bank_treasury(&w)), (Money(1_000), 2 * OPEN_FEE));

    interrupted(&mut w, ann, "deposit", json!({ "amount": 800 }), "cancelled", cancel);
    assert_eq!((w.cash(ann), balance(&w, ann)), (Money(200), 800));
    assert_eq!(w.global().res::<Bank>().deposits(), 5_800);

    // Another module stops the action, for example an arrest.
    interrupted(&mut w, ann, "withdraw", json!({ "amount": 300 }), "interrupted", |w, r| {
        w.with_resident(r, |realm, idx| realm.interrupt(idx, "You are under arrest."));
    });
    assert_eq!((w.cash(ann), balance(&w, ann)), (Money(500), 500));
    assert_eq!(w.observe(ann).sections["bank"], json!({ "balance": 500 }));
    assert_eq!(conserved(&w), total);

    let issue = json!({ "org": ACME, "shares": 100, "price": 10 });
    interrupted(&mut w, ann, "issue_shares", issue, "cancelled", cancel);
    assert_eq!(w.global().res::<Shares>().company(ACME).map(|c| c.on_offer()), Some(100));

    ok(&mut w, bob, "buy_shares", json!({ "org": ACME, "shares": 10, "price": 10 }));
    assert_eq!(w.money(Holder::Org(ACME)), Money(100));
    let dividend = json!({ "org": ACME, "amount": 60 });
    interrupted(&mut w, ann, "pay_dividend", dividend, "cancelled", cancel);
    assert_eq!((w.money(Holder::Org(ACME)), balance(&w, bob)), (Money(40), 4_900 + 60));

    let rate = json!({ "rate_bp": 700 });
    interrupted(&mut w, ann, "central_bank_set_rate", rate, "cancelled", cancel);
    assert_eq!(w.param("finance.base_rate_bp"), 700);

    interrupted(&mut w, ann, "central_bank_issue", json!({ "amount": 5_000 }), "cancelled", cancel);
    assert_eq!(minted(&w, "central_bank.issue"), 5_000);

    let retire = json!({ "amount": 2_000 });
    interrupted(&mut w, ann, "central_bank_retire", retire, "cancelled", cancel);
    assert_eq!(burned(&w, "central_bank.retire"), 2_000);
    assert_eq!(w.money(Holder::Org(OrgId::NATION)), Money(3_000));
    assert_eq!(conserved(&w), total + 3_000);
}

#[test]
fn an_action_that_has_sent_nothing_gives_no_notice() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 2_000);
    w.give_money(ann, Money(500));

    // An exchange verb does its work at the end: a cancelled one did
    // nothing, and the report says all there is to say.
    w.set_pos(ann, AT_EXCHANGE);
    let args = json!({ "item": "wheat", "side": "buy", "qty": 2, "price": 100 });
    let ack = w.act(ann, "order_place", args);
    assert!(matches!(ack.status, ActionStatus::Started { .. }));
    cancel(&mut w, ann);
    w.step(2);
    assert_eq!(w.cash(ann), Money(500));
    assert_eq!(w.zone(ZoneId(0)).res::<Exchange>().orders_of(ann).count(), 0);
    assert_eq!(notice(&w, ann, NOTICE), None);

    // The kernel reports the late answer of a global verb by itself.
    let ack = w.act(ann, "loan_repay", json!({ "amount": 10 }));
    assert_eq!(ack.status, ActionStatus::Pending);
    cancel(&mut w, ann);
    w.step(2);
    let codes: Vec<Option<ErrorCode>> = w
        .reports(ann)
        .iter()
        .filter(|report| report.action_id == ack.action_id)
        .map(|report| report.error.as_ref().map(|error| error.code))
        .collect();
    assert_eq!(codes, [Some(ErrorCode::Interrupted), Some(ErrorCode::NotFound)]);
    assert_eq!(notice(&w, ann, NOTICE), None);
    conserved(&w);
}
