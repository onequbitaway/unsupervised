//! `Realm::complete_action` after the action already ended: with the
//! parameter `kernel.late_reports` the late answer is one more report
//! (architecture.md, section 5.2).

mod common;

use common::world;
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionAck, ActionReport, ActionRequest, ActionStatus, ErrorCode};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ActionError, Pos, ResidentId, ZoneId};

/// A world in which late answers are reported, with Ada and Bob in zone 0.
fn late_world() -> (TestWorld<'static>, ResidentId, ResidentId) {
    let mut w = world();
    w.set_param("kernel.late_reports", 1);
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    // The zones see a new parameter value in the next frame.
    w.step(1);
    (w, ada, bob)
}

/// Ada offers and nobody accepts in time.
fn timed_out_offer(w: &mut TestWorld<'_>, ada: ResidentId, key: &str) -> ActionAck {
    let request = ActionRequest::new("offer", json!({ "ticks": 2 })).with_id(key);
    let offer = w.act_request(ada, request);
    assert_eq!(offer.status, ActionStatus::Pending);
    w.step(2);
    assert_eq!(w.activity(ada), None);
    offer
}

fn accept(w: &mut TestWorld<'_>, bob: ResidentId, ada: ResidentId, action_id: u64) {
    w.act_ok(bob, "accept", json!({ "resident": ada, "action_id": action_id }));
}

/// The reports of one action: `(ok, error code)`, oldest first.
fn outcomes(w: &TestWorld<'_>, r: ResidentId, action_id: u64) -> Vec<(bool, Option<ErrorCode>)> {
    w.reports(r)
        .iter()
        .filter(|report| report.action_id == action_id)
        .map(|report| (report.ok, report.error.as_ref().map(|error| error.code)))
        .collect()
}

#[test]
fn an_answer_after_a_timeout_is_one_more_report() {
    let (mut w, ada, bob) = late_world();
    let offer = timed_out_offer(&mut w, ada, "offer-1");
    assert_eq!(outcomes(&w, ada, offer.action_id), [(false, Some(ErrorCode::Timeout))]);

    accept(&mut w, bob, ada, offer.action_id);
    let reports = w.reports(ada);
    assert_eq!(reports.len(), 2);
    // The late report has the id, the request id and the verb of the
    // action, and the answer.
    let tick = w.zone(ZoneId(0)).tick();
    assert_eq!(
        reports[1],
        ActionReport {
            action_id: offer.action_id,
            request_id: Some("offer-1".into()),
            verb: "offer".into(),
            tick,
            ok: true,
            result: json!({ "accepted_by": bob }),
            error: None,
        }
    );
    // Ada stays idle: the report is news, not a new action.
    assert_eq!(w.activity(ada), None);

    // The answer is reported once. A second answer does nothing.
    accept(&mut w, bob, ada, offer.action_id);
    w.step(2);
    assert_eq!(w.reports(ada).len(), 2);
}

#[test]
fn an_answer_after_an_interrupt_or_a_cancel_is_one_more_report() {
    let (mut w, ada, bob) = late_world();

    let offer = w.act(ada, "offer", json!({ "ticks": 50 }));
    w.with_resident(ada, |realm, idx| realm.interrupt(idx, "A guard stopped you."));
    w.world.settle();
    accept(&mut w, bob, ada, offer.action_id);
    assert_eq!(
        outcomes(&w, ada, offer.action_id),
        [(false, Some(ErrorCode::Interrupted)), (true, None)]
    );

    let offer = w.act(ada, "offer", json!({ "ticks": 50 }));
    w.act_ok(ada, "cancel", Value::Null);
    // The late answer can be a refusal too.
    w.with_resident(bob, |realm, _| {
        realm.complete_action(ada, offer.action_id, Err(ActionError::conflict("Sold out.")));
    });
    assert_eq!(
        outcomes(&w, ada, offer.action_id),
        [(false, Some(ErrorCode::Interrupted)), (false, Some(ErrorCode::Conflict))]
    );
    assert_eq!(
        w.last_report(ada).and_then(|r| r.error).map(|e| e.message),
        Some("Sold out.".into())
    );
}

#[test]
fn a_late_answer_is_dropped_when_the_parameter_is_off() {
    let mut w = world();
    assert_eq!(w.param("kernel.late_reports"), 0, "the default drops late answers");
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let offer = timed_out_offer(&mut w, ada, "offer-1");
    accept(&mut w, bob, ada, offer.action_id);
    w.step(2);
    assert_eq!(outcomes(&w, ada, offer.action_id), [(false, Some(ErrorCode::Timeout))]);

    // The parameter counts when the action ends: an action that ended
    // before it was set to 1 stays without a late report.
    w.set_param("kernel.late_reports", 1);
    w.step(1);
    accept(&mut w, bob, ada, offer.action_id);
    assert_eq!(w.reports(ada).len(), 1);
    // An action that ends after that has one.
    let offer = timed_out_offer(&mut w, ada, "offer-2");
    accept(&mut w, bob, ada, offer.action_id);
    assert_eq!(
        outcomes(&w, ada, offer.action_id),
        [(false, Some(ErrorCode::Timeout)), (true, None)]
    );
}

#[test]
fn only_an_answer_for_a_waiting_action_that_ended_is_reported() {
    let (mut w, ada, bob) = late_world();
    let offer = timed_out_offer(&mut w, ada, "offer-1");
    let before = w.reports(ada).len();

    // A wrong action id.
    accept(&mut w, bob, ada, offer.action_id + 1);
    assert_eq!(w.reports(ada).len(), before);

    // An action that was completed in time: the second answer is nothing.
    let done = w.act(ada, "offer", json!({ "ticks": 50 }));
    accept(&mut w, bob, ada, done.action_id);
    accept(&mut w, bob, ada, done.action_id);
    assert_eq!(outcomes(&w, ada, done.action_id), [(true, None)]);

    // An action that does not wait for an answer: `work` takes time.
    let work = w.act(ada, "work", json!({ "ticks": 50 }));
    w.act_ok(ada, "cancel", Value::Null);
    accept(&mut w, bob, ada, work.action_id);
    assert_eq!(outcomes(&w, ada, work.action_id), [(false, Some(ErrorCode::Interrupted))]);

    // A global verb gets its late answer from the global realm, once.
    let ping = w.act(ada, "ping", json!({ "n": 1 }));
    w.act_ok(ada, "cancel", Value::Null);
    accept(&mut w, bob, ada, ping.action_id);
    w.step(2);
    assert_eq!(
        outcomes(&w, ada, ping.action_id),
        [(false, Some(ErrorCode::Interrupted)), (true, None)]
    );
}

#[test]
fn a_late_answer_follows_the_resident_and_survives_a_snapshot() {
    let (mut w, ada, bob) = late_world();
    let offer = timed_out_offer(&mut w, ada, "offer-1");
    // Ada goes to another zone. Bob answers from zone 0.
    w.act_ok(ada, "jump", json!({ "zone": 2, "x": 5, "y": 70 }));
    assert_eq!(w.zone_of(ada), ZoneId(2));
    w.snapshot_restore();
    accept(&mut w, bob, ada, offer.action_id);
    assert_eq!(w.reports(ada).len(), 1, "nothing crosses zones without a tick");
    w.step(1);
    assert_eq!(
        outcomes(&w, ada, offer.action_id),
        [(false, Some(ErrorCode::Timeout)), (true, None)]
    );

    // The other way round: the answer comes from another zone.
    let offer = timed_out_offer(&mut w, ada, "offer-2");
    w.set_pos(bob, Pos::new(100, 100));
    accept(&mut w, bob, ada, offer.action_id);
    w.step(1);
    assert_eq!(
        outcomes(&w, ada, offer.action_id),
        [(false, Some(ErrorCode::Timeout)), (true, None)]
    );
    assert_eq!(w.last_report(ada).and_then(|r| r.request_id), Some("offer-2".into()));
}

#[test]
fn the_kernel_remembers_the_last_eight_actions_that_ended_without_an_answer() {
    let (mut w, ada, bob) = late_world();
    let offers: Vec<ActionAck> =
        (0..9).map(|n| timed_out_offer(&mut w, ada, &format!("offer-{n}"))).collect();
    // The first of the nine is forgotten.
    accept(&mut w, bob, ada, offers[0].action_id);
    assert_eq!(outcomes(&w, ada, offers[0].action_id), [(false, Some(ErrorCode::Timeout))]);
    // The other eight are known.
    for offer in &offers[1..] {
        accept(&mut w, bob, ada, offer.action_id);
        assert_eq!(
            outcomes(&w, ada, offer.action_id),
            [(false, Some(ErrorCode::Timeout)), (true, None)]
        );
    }
}
