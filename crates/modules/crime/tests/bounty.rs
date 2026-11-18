//! `post_bounty`: money in escrow for the officer that makes the arrest.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Holder, Money, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// A world with a victim, a thief and an officer. The city watch sees
/// every crime, so a theft gives a warrant.
struct Cast {
    w: TestWorld<'static>,
    bo: ResidentId,
    cy: ResidentId,
    eve: ResidentId,
}

fn cast(rules: &[(&'static str, i64)]) -> Cast {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND, rules]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let bo = resident_at(&mut w, "Bo", 10_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    Cast { w, bo, cy, eve }
}

#[test]
fn a_bounty_is_paid_to_the_officer_that_arrests_the_target() {
    let Cast { mut w, bo, cy, eve } = cast(&[]);
    let (posted, events) =
        run_with_events(&mut w, bo, "post_bounty", json!({ "target": cy, "amount": 1_500 }));
    let bounty = posted.result["bounty"].as_u64().expect("a bounty");
    assert_eq!(
        posted.result,
        json!({ "bounty": bounty, "target": cy, "amount": 1_500, "open_for": 4_320 })
    );
    let news = of_kind(&events, "crime.bounty_posted");
    assert_eq!(news.len(), 1);
    assert_eq!(
        news[0].data,
        json!({ "bounty": bounty, "by": bo, "target": cy, "amount": 1_500, "open_for": 4_320 })
    );
    // The money is locked in an escrow of the global realm.
    assert_eq!(w.cash(bo), Money(8_500));
    assert_eq!(w.money(Holder::Escrow(bounty)), Money(1_500));

    // A bounty gives no grounds: the target did nothing yet.
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    assert_eq!(w.money(Holder::Escrow(bounty)), Money(1_500));

    ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    w.step(1);
    let (_, mut events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    events.extend(step_events(&mut w, 2));
    let paid = of_kind(&events, "crime.bounty_paid");
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0].actor, Some(eve));
    let data = json!({ "bounty": bounty, "target": cy, "officer": eve, "amount": 1_500 });
    assert_eq!(paid[0].data, data);
    // The arrest was an input between two frames. The registry pays in its
    // next tick.
    assert_eq!(paid[0].frame, frame_of(&events, "crime.arrest_made") + 1);
    assert_eq!(w.cash(eve), Money(1_500));
    assert_eq!(w.money(Holder::Escrow(bounty)), Money(0));
    assert_eq!(notice(&w, eve, "crime.bounty").expect("the officer is told").data, data);
    assert_eq!(notice(&w, bo, "crime.bounty").expect("the poster is told").data, data);
    // The bounty is gone: nothing comes back later.
    let events = step_events(&mut w, 4_400);
    assert_eq!(of_kind(&events, "crime.bounty_expired").len(), 0);
    // Bo lost 2,500 to the thief and got 2,500 back with the verdict.
    assert_eq!(w.cash(bo), Money(8_500));
    w.assert_conserved();
}

#[test]
fn a_bounty_that_nobody_earns_goes_back_to_the_poster() {
    let Cast { mut w, bo, cy, .. } = cast(&[]);
    let (posted, events) = run_with_events(
        &mut w,
        bo,
        "post_bounty",
        json!({ "target": cy, "amount": 500, "ticks": 50 }),
    );
    let bounty = posted.result["bounty"].as_u64().expect("a bounty");
    let from = frame_of(&events, "crime.bounty_posted");
    assert_eq!(w.cash(bo), Money(9_500));

    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(from + 49 - now).expect("soon"));
    assert_eq!(of_kind(&events, "crime.bounty_expired").len(), 0);
    let events = step_events(&mut w, 2);
    let expired = of_kind(&events, "crime.bounty_expired");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].frame, from + 50);
    assert_eq!(
        expired[0].data,
        json!({ "bounty": bounty, "target": cy, "amount": 500, "reason": "expired" })
    );
    assert_eq!(w.cash(bo), Money(10_000));
    assert_eq!(
        notice(&w, bo, "crime.bounty").expect("the poster is told").data,
        json!({ "bounty": bounty, "target": cy, "refunded": 500, "reason": "expired" })
    );
    w.assert_conserved();
}

#[test]
fn the_city_watch_earns_no_bounty() {
    let Cast { mut w, bo, cy, .. } =
        cast(&[("crime.watch_arrest_bp", 10_000), ("crime.bounty_ticks", 300)]);
    let shelter = site(&w, "shelter");
    let posted = ok(&mut w, bo, "post_bounty", json!({ "target": cy, "amount": 800 }));
    let bounty = posted["bounty"].as_u64().expect("a bounty");
    ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    w.set_pos(cy, shelter.rect.centre());
    let events = step_events(&mut w, 62);
    assert_eq!(of_kind(&events, "crime.arrest_made")[0].data["officer"], json!(null));
    assert_eq!(of_kind(&events, "crime.bounty_paid").len(), 0);
    assert_eq!(w.money(Holder::Escrow(bounty)), Money(800));
    // It stays open until its time is over. Then the poster has it back.
    let events = step_events(&mut w, 300);
    assert_eq!(of_kind(&events, "crime.bounty_expired").len(), 1);
    assert_eq!(w.money(Holder::Escrow(bounty)), Money(0));
    w.assert_conserved();
}

#[test]
fn a_bounty_on_a_resident_that_leaves_the_world_goes_back() {
    let Cast { mut w, bo, cy, .. } = cast(&[]);
    let posted = ok(&mut w, bo, "post_bounty", json!({ "target": cy, "amount": 900 }));
    let bounty = posted["bounty"].as_u64().expect("a bounty");
    w.despawn(cy);
    let events = step_events(&mut w, 2);
    let expired = of_kind(&events, "crime.bounty_expired");
    assert_eq!(expired.len(), 1);
    assert_eq!(
        expired[0].data,
        json!({ "bounty": bounty, "target": cy, "amount": 900, "reason": "target_gone" })
    );
    assert_eq!(w.cash(bo), Money(10_000));
    w.assert_conserved();
}

#[test]
fn post_bounty_refuses_bad_requests_and_keeps_no_money() {
    let Cast { mut w, bo, cy, eve } = cast(&[]);
    let post = |target: ResidentId, amount: i64| json!({ "target": target, "amount": amount });
    assert_eq!(fail(&mut w, bo, "post_bounty", post(bo, 1_000)), ErrorCode::InvalidArgs);
    // The least bounty is 500 cents.
    assert_eq!(fail(&mut w, bo, "post_bounty", post(cy, 499)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, bo, "post_bounty", json!({ "target": cy })), ErrorCode::InvalidArgs);
    for ticks in [0, 20_161] {
        let args = json!({ "target": cy, "amount": 500, "ticks": ticks });
        assert_eq!(fail(&mut w, bo, "post_bounty", args), ErrorCode::InvalidArgs);
    }
    assert_eq!(fail(&mut w, bo, "post_bounty", post(cy, 10_001)), ErrorCode::Insufficient);
    // Nobody has the id 999. The registry refuses; the cash comes back.
    assert_eq!(fail(&mut w, bo, "post_bounty", post(ResidentId(999), 1_000)), ErrorCode::NotFound);
    w.step(1);
    assert_eq!(w.cash(bo), Money(10_000));

    // A resident has at most three open bounties.
    for target in [cy, eve, cy] {
        ok(&mut w, bo, "post_bounty", post(target, 500));
    }
    assert_eq!(fail(&mut w, bo, "post_bounty", post(eve, 500)), ErrorCode::Conflict);
    w.step(1);
    assert_eq!(w.cash(bo), Money(8_500));
    assert_eq!(w.global().res::<unsup_crime::Cases>().bounties().count(), 3);
    w.assert_conserved();
}

#[test]
fn a_contract_of_the_society_module_can_wait_for_the_arrest_event() {
    let Cast { mut w, bo, cy, eve } = cast(&[]);
    // Bo promises 2,000 to Eve by contract when Cy is arrested.
    let clause = json!([{
        "kind": "pay_on_event", "by": "offerer", "amount": 2_000,
        "event": "crime.arrest_made", "fields": { "target": cy }, "due_in": 500,
    }]);
    let offer = ok(&mut w, bo, "contract_offer", json!({ "to": eve, "clauses": clause }));
    let contract = offer["contract"].as_u64().expect("a contract");
    ok(&mut w, eve, "contract_sign", json!({ "contract": contract }));
    assert_eq!(w.cash(bo), Money(8_000));

    ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    w.step(1);
    let (_, mut events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    events.extend(step_events(&mut w, 2));
    assert_eq!(of_kind(&events, "society.contract_fulfilled").len(), 1);
    assert_eq!(w.cash(eve), Money(2_000));
    w.assert_conserved();
}
