//! The rules of the action engine (architecture.md, section 5.2).

mod common;

use common::{ZoneLog, walker, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::{
    ActionAck, ActionRequest, ActionStatus, Condition, ErrorCode, Routine, Tick,
};
use unsup_kernel::{Pos, ZoneId};

fn routine(name: &str, when: Condition, verb: &str, args: Value) -> Value {
    json!(Routine { name: name.into(), when, then: ActionRequest::new(verb, args) })
}

#[test]
fn an_idle_resident_begins_at_once_and_finish_runs_after_the_ticks() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let start = w.pos(ada);
    let ack = w.act(ada, "work", json!({ "ticks": 3 }));
    assert_eq!(ack.status, ActionStatus::Started { ends_at: Tick(3), ticks: 3 });
    assert_eq!(ack.verb, "work");
    assert_eq!(w.activity(ada), Some("work"));
    assert_eq!(w.cash(ada).cents(), 1_000);

    w.step(2);
    assert_eq!(w.activity(ada), Some("work"), "the action is not over after two of three ticks");
    assert_eq!(w.cash(ada).cents(), 1_000);
    assert!(w.reports(ada).is_empty());

    w.step(1);
    assert_eq!(w.activity(ada), None);
    assert_eq!(w.cash(ada).cents(), 1_500);
    let report = w.last_report(ada).expect("an action that ended writes a report");
    assert_eq!(
        (report.action_id, report.verb.as_str(), report.ok, report.tick),
        (ack.action_id, "work", true, Tick(3))
    );
    assert_eq!(report.result, json!({ "pay": 500 }));
    assert_eq!(w.pos(ada), start);
}

#[test]
fn an_instant_action_answers_in_the_ack_and_writes_no_report() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let ack = w.act(ada, "pay", json!({ "to": bob, "amount": 250 }));
    assert_eq!(ack.status, ActionStatus::Done { result: json!({ "paid": 250 }) });
    assert_eq!((w.cash(ada).cents(), w.cash(bob).cents()), (750, 1_250));
    w.step(2);
    assert!(w.reports(ada).is_empty(), "the result was in the acknowledgement");
    assert_eq!(w.activity(ada), None);
}

#[test]
fn a_busy_resident_queues_requests_and_runs_them_in_order() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let first = w.act(ada, "work", json!({ "ticks": 2 }));
    let second = w.act(ada, "step", json!({ "dx": 1, "dy": 0 }));
    let third = w.act(ada, "work", json!({ "ticks": 1 }));
    assert_eq!(second.status, ActionStatus::Queued { position: 1 });
    assert_eq!(third.status, ActionStatus::Queued { position: 2 });
    let start = w.pos(ada);

    w.step(2);
    // `work` ended at tick 2 and the queued `step` began in the same tick.
    assert_eq!(w.activity(ada), Some("step"));
    w.step(1);
    assert_eq!(w.pos(ada), start.offset(1, 0));
    assert_eq!(w.activity(ada), Some("work"));
    w.step(1);
    assert_eq!(w.activity(ada), None);
    let ids: Vec<u64> = w.reports(ada).iter().map(|r| r.action_id).collect();
    assert_eq!(ids, [first.action_id, second.action_id, third.action_id]);
    assert_eq!(w.cash(ada).cents(), 2_000);
}

#[test]
fn the_queue_is_bounded_by_the_parameter() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(w.param("kernel.queue_max"), 8);
    w.act_ok(ada, "work", json!({ "ticks": 50 }));
    for position in 1..=8 {
        let ack = w.act(ada, "wait", json!({ "ticks": 1 }));
        assert_eq!(ack.status, ActionStatus::Queued { position });
    }
    let error = w.act_err(ada, "wait", json!({ "ticks": 1 }));
    assert_eq!(error.code, ErrorCode::QueueFull);
    // A queued request with bad arguments is refused when it is sent.
    w.act_ok(ada, "cancel", Value::Null);
    w.act_ok(ada, "work", json!({ "ticks": 50 }));
    let error = w.act_err(ada, "step", json!({ "dx": "east" }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
}

#[test]
fn a_free_verb_runs_while_busy_and_is_limited_per_tick() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(w.param("kernel.free_actions_per_tick"), 4);
    w.act_ok(ada, "work", json!({ "ticks": 10 }));
    for _ in 0..4 {
        let status = w.act_ok(ada, "shout", json!({ "text": "hello" }));
        assert_eq!(status, ActionStatus::Done { result: Value::Null });
    }
    assert_eq!(w.activity(ada), Some("work"), "a free verb does not replace the running action");
    let error = w.act_err(ada, "shout", json!({ "text": "hello" }));
    assert_eq!(error.code, ErrorCode::RateLimited);
    // The limit is per tick.
    w.step(1);
    w.act_ok(ada, "shout", json!({ "text": "again" }));
    // A free verb must finish at once.
    let error = w.act_err(ada, "bad_free", Value::Null);
    assert_eq!(error.code, ErrorCode::Internal);
    assert_eq!(w.activity(ada), Some("work"));
}

#[test]
fn unknown_verbs_and_bad_arguments_are_rejected() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(w.act_err(ada, "fly", Value::Null).code, ErrorCode::UnknownVerb);
    assert_eq!(w.act_err(ada, "step", json!({ "dx": 1 })).code, ErrorCode::InvalidArgs);
    assert_eq!(w.act_err(ada, "step", json!({ "dx": 5, "dy": 0 })).code, ErrorCode::InvalidArgs);
    assert_eq!(w.act_err(ada, "wait", json!({ "ticks": 0 })).code, ErrorCode::InvalidArgs);
    assert_eq!(w.act_err(ada, "wait", json!({ "ticks": 1_441 })).code, ErrorCode::InvalidArgs);
    let long_id = "x".repeat(65);
    let ack =
        w.act_request(ada, ActionRequest::new("wait", json!({ "ticks": 1 })).with_id(long_id));
    assert!(
        matches!(&ack.status, ActionStatus::Rejected { error } if error.code == ErrorCode::InvalidArgs)
    );
    assert_eq!(w.activity(ada), None, "a rejected request changes nothing");
    // A verb that needs no arguments accepts none.
    assert!(w.act(ada, "cancel", Value::Null).status.is_accepted());
}

#[test]
fn a_blocking_condition_rejects_every_verb_that_does_not_allow_it() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.act_ok(ada, "jail_me", Value::Null);
    assert!(w.has_condition(ada, "jailed"));

    let error = w.act_err(ada, "work", json!({ "ticks": 1 }));
    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(error.details, json!({ "condition": "jailed" }));
    assert_eq!(w.act_err(ada, "shout", json!({ "text": "help" })).code, ErrorCode::Blocked);
    // `nap` lists `jailed` in `allowed_in`; kernel verbs are never blocked
    // by a module condition.
    assert!(w.act(ada, "nap", json!({ "ticks": 1 })).status.is_accepted());
    assert!(w.act(ada, "wait", json!({ "ticks": 1 })).status.is_accepted());
    assert!(w.act(ada, "set_routines", json!({ "routines": [] })).status.is_accepted());
    // A condition that does not block changes nothing.
    w.set_condition(ada, "tired", None);

    // The condition ends by itself after 5 ticks.
    w.step(5);
    assert!(!w.has_condition(ada, "jailed"));
    assert!(w.has_condition(ada, "tired"));
    w.run_until_idle(ada, 5);
    assert!(w.act(ada, "work", json!({ "ticks": 1 })).status.is_accepted());
    let ended = w.zone(ZoneId(0)).res::<ZoneLog>().conditions_ended.clone();
    assert_eq!(ended, [(ada, "jailed".to_string(), true)]);
}

#[test]
fn a_queued_request_is_checked_again_when_it_starts() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.act_ok(ada, "wait", json!({ "ticks": 2 }));
    let queued = w.act(ada, "work", json!({ "ticks": 1 }));
    assert_eq!(queued.status, ActionStatus::Queued { position: 1 });
    w.set_condition(ada, "jailed", Some(100));
    w.step(2);
    let report = w.last_report(ada).expect("the queued action got a report");
    assert_eq!((report.action_id, report.ok), (queued.action_id, false));
    assert_eq!(report.error.map(|e| e.code), Some(ErrorCode::Blocked));
    assert_eq!(w.cash(ada).cents(), 1_000);
}

#[test]
fn request_ids_are_idempotency_keys() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let pay = || ActionRequest::new("pay", json!({ "to": 2, "amount": 100 })).with_id("pay-1");
    let first = w.act_request(ada, pay());
    assert!(matches!(first.status, ActionStatus::Done { .. }));
    assert_eq!(first.request_id.as_deref(), Some("pay-1"));
    let again = w.act_request(ada, pay());
    assert_eq!(again, first, "a repeat returns the first answer");
    assert_eq!((w.cash(ada).cents(), w.cash(bob).cents()), (900, 1_100), "the payment ran once");

    // The same key with a started action also returns the first answer.
    let work = || ActionRequest::new("work", json!({ "ticks": 5 })).with_id("work-1");
    let started = w.act_request(ada, work());
    assert!(matches!(started.status, ActionStatus::Started { .. }));
    assert_eq!(w.act_request(ada, work()), started);
    assert_eq!(
        w.with_resident(ada, |realm, idx| realm.queue_len(idx)),
        0,
        "the repeat was not queued"
    );

    // A rejected request changed nothing, so its key can be used again.
    let poor =
        || ActionRequest::new("pay", json!({ "to": 2, "amount": 1_000_000 })).with_id("pay-2");
    assert!(!w.act_request(bob, poor()).status.is_accepted());
    w.give_money(bob, unsup_kernel::Money(2_000_000));
    assert!(matches!(w.act_request(bob, poor()).status, ActionStatus::Done { .. }));

    // Keys are per resident: Ada's key means nothing for Bob.
    let other: ActionAck =
        w.act_request(bob, ActionRequest::new("shout", json!({ "text": "x" })).with_id("pay-1"));
    assert_ne!(other.action_id, first.action_id);
    assert_eq!(other.verb, "shout");

    // Only the last 32 keys are remembered. After 32 other keys the first
    // one is forgotten, and a request with it runs again.
    for i in 0..31 {
        let request = ActionRequest::new("shout", json!({ "text": "x" })).with_id(format!("k{i}"));
        assert!(w.act_request(ada, request).status.is_accepted());
        if i % 4 == 3 {
            w.step(1);
        }
    }
    // 33 keys so far: pay-1, work-1 and 31 shouts. pay-1 is gone, work-1
    // is the oldest that is left.
    assert_eq!(w.act_request(ada, work()), started, "work-1 is still remembered");
    let before = w.cash(ada).cents();
    let rerun = w.act_request(ada, pay());
    assert_ne!(rerun.action_id, first.action_id, "pay-1 was forgotten, so the request ran again");
    assert_eq!(w.cash(ada).cents(), before - 100);
}

#[test]
fn cancel_stops_the_action_and_clears_the_queue() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let running = w.act(ada, "work", json!({ "ticks": 10 }));
    let queued = w.act(ada, "work", json!({ "ticks": 10 }));
    w.step(1);
    let status = w.act_ok(ada, "cancel", Value::Null);
    assert_eq!(
        status,
        ActionStatus::Done { result: json!({ "cancelled": running.action_id, "cleared": 1 }) }
    );
    assert_eq!(w.activity(ada), None);
    assert_eq!(w.with_resident(ada, |realm, idx| realm.queue_len(idx)), 0);
    let reports = w.reports(ada);
    let codes: Vec<(u64, Option<ErrorCode>)> =
        reports.iter().map(|r| (r.action_id, r.error.as_ref().map(|e| e.code))).collect();
    assert_eq!(
        codes,
        [
            (queued.action_id, Some(ErrorCode::Interrupted)),
            (running.action_id, Some(ErrorCode::Interrupted))
        ]
    );
    w.step(12);
    assert_eq!(w.cash(ada).cents(), 1_000, "`finish` never ran");
    let log = w.zone(ZoneId(0)).res::<ZoneLog>().interrupted.clone();
    assert_eq!(log, ["work:cancelled"]);
    // Cancel with nothing to cancel is fine.
    let status = w.act_ok(ada, "cancel", Value::Null);
    assert_eq!(status, ActionStatus::Done { result: json!({ "cancelled": null, "cleared": 0 }) });
}

#[test]
fn interrupt_fails_the_running_action_and_the_queue_goes_on() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let running = w.act(ada, "work", json!({ "ticks": 10 }));
    let queued = w.act(ada, "wait", json!({ "ticks": 3 }));
    w.step(1);
    w.with_resident(ada, |realm, idx| realm.interrupt(idx, "A guard stopped you."));
    w.world.settle();
    let report = w.last_report(ada).expect("the interrupted action got a report");
    assert_eq!((report.action_id, report.ok), (running.action_id, false));
    let error = report.error.expect("an error");
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::Interrupted, "A guard stopped you.")
    );
    // The next planned action started.
    assert_eq!(w.activity(ada), Some("wait"));
    w.step(3);
    assert_eq!(w.last_report(ada).map(|r| (r.action_id, r.ok)), Some((queued.action_id, true)));
    assert_eq!(w.cash(ada).cents(), 1_000);
    // Interrupting an idle resident does nothing.
    let before = w.reports(ada).len();
    w.with_resident(ada, |realm, idx| realm.interrupt(idx, "again"));
    assert_eq!(w.reports(ada).len(), before);
    // `clear_queue` before `interrupt` stops the whole plan.
    w.act_ok(ada, "work", json!({ "ticks": 10 }));
    w.act_ok(ada, "wait", json!({ "ticks": 3 }));
    w.with_resident(ada, |realm, idx| {
        assert_eq!(realm.clear_queue(idx), 1);
        realm.interrupt(idx, "stop");
    });
    w.world.settle();
    assert_eq!(w.activity(ada), None);
}

#[test]
fn wait_ends_when_completed_and_times_out_otherwise() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");

    // Completed by another resident in the same zone.
    let offer = w.act(ada, "offer", json!({ "ticks": 5 }));
    assert_eq!(offer.status, ActionStatus::Pending);
    assert_eq!(w.activity(ada), Some("offer"));
    w.step(2);
    w.act_ok(bob, "accept", json!({ "resident": ada, "action_id": offer.action_id }));
    assert_eq!(w.activity(ada), None);
    let report = w.last_report(ada).expect("a completed wait writes a report");
    assert_eq!(
        (report.action_id, report.ok, report.result.clone()),
        (offer.action_id, true, json!({ "accepted_by": bob }))
    );

    // A wrong action id completes nothing.
    let offer = w.act(ada, "offer", json!({ "ticks": 5 }));
    w.act_ok(bob, "accept", json!({ "resident": ada, "action_id": offer.action_id + 1 }));
    assert_eq!(w.activity(ada), Some("offer"));

    // Nobody accepts: timeout after 5 ticks, not before.
    w.step(4);
    assert_eq!(w.activity(ada), Some("offer"));
    w.step(1);
    assert_eq!(w.activity(ada), None);
    let report = w.last_report(ada).expect("a timeout writes a report");
    assert_eq!(
        (report.action_id, report.error.map(|e| e.code)),
        (offer.action_id, Some(ErrorCode::Timeout))
    );
    let log = w.zone(ZoneId(0)).res::<ZoneLog>().interrupted.clone();
    assert_eq!(log, ["offer:timeout"]);

    // Completed from another zone: the call is a message, and applies in
    // the next tick of the zone of the waiting resident.
    w.set_pos(bob, Pos::new(100, 100));
    assert_ne!(w.zone_of(bob), w.zone_of(ada));
    let offer = w.act(ada, "offer", json!({ "ticks": 9 }));
    w.act_ok(bob, "accept", json!({ "resident": ada, "action_id": offer.action_id }));
    assert_eq!(w.activity(ada), Some("offer"), "nothing crosses zones without a tick");
    w.step(1);
    assert_eq!(w.activity(ada), None);
    assert_eq!(w.last_report(ada).map(|r| (r.action_id, r.ok)), Some((offer.action_id, true)));
}

#[test]
fn a_global_verb_is_pending_and_answers_in_the_next_frame() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let ack = w.act(ada, "ping", json!({ "n": 7 }));
    assert_eq!(ack.status, ActionStatus::Pending);
    assert_eq!(w.activity(ada), Some("ping"), "a global verb occupies the resident like a wait");
    // Bad arguments are refused by the zone, before anything is sent.
    let queued = w.act(ada, "ping", json!({ "n": "seven" }));
    assert!(
        matches!(queued.status, ActionStatus::Rejected { ref error } if error.code == ErrorCode::InvalidArgs)
    );

    // Frame 1: the request reaches the global realm and runs there.
    w.step(1);
    assert_eq!(w.global().res::<common::GlobalLog>().pings, 1);
    assert_eq!(w.activity(ada), Some("ping"));
    assert!(w.reports(ada).is_empty());
    // Frame 2: the answer arrives in the zone.
    w.step(1);
    assert_eq!(w.activity(ada), None);
    let report = w.last_report(ada).expect("the answer is a report");
    assert_eq!((report.action_id, report.ok), (ack.action_id, true));
    assert_eq!(report.result, json!({ "pong": 7, "count": 1, "zone": 0 }));

    // An error of the global handler comes back as a failed report.
    let ack = w.act(ada, "ping", json!({ "n": 13 }));
    w.step(2);
    let report = w.last_report(ada).expect("a report");
    assert_eq!(
        (report.action_id, report.error.map(|e| e.code)),
        (ack.action_id, Some(ErrorCode::Conflict))
    );

    // A free global verb does not occupy the resident.
    let ack = w.act(ada, "note", json!({ "n": 5 }));
    assert_eq!(ack.status, ActionStatus::Pending);
    assert_eq!(w.activity(ada), None);
    w.step(2);
    let report = w.last_report(ada).expect("a report");
    assert_eq!((report.action_id, report.result.clone()), (ack.action_id, json!({ "noted": 5 })));
    assert_eq!(w.global().res::<common::GlobalLog>().pings, 6);
}

#[test]
fn a_global_verb_times_out_when_the_answer_never_comes() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let ack = w.act(ada, "ping", json!({ "n": 1 }));
    // Skip nothing, but stop the answer: interrupt the wait and check that
    // the late answer still reaches the resident as a second report.
    w.with_resident(ada, |realm, idx| realm.interrupt(idx, "stopped"));
    w.world.settle();
    w.step(2);
    let reports = w.reports(ada);
    assert_eq!(reports.len(), 2);
    assert_eq!((reports[0].action_id, reports[0].ok), (ack.action_id, false));
    assert_eq!((reports[1].action_id, reports[1].ok), (ack.action_id, true));

    // With a timeout of one tick the wait ends before the answer arrives.
    w.set_param("kernel.global_timeout", 1);
    w.step(1);
    let ack = w.act(ada, "ping", json!({ "n": 2 }));
    w.step(1);
    let report = w.last_report(ada).expect("a report");
    assert_eq!(
        (report.action_id, report.error.map(|e| e.code)),
        (ack.action_id, Some(ErrorCode::Timeout))
    );
    assert_eq!(w.activity(ada), None);
    let log = w.zone(ZoneId(0)).res::<ZoneLog>().interrupted.clone();
    assert_eq!(log, ["ping:interrupted", "ping:timeout"]);
    // The global realm did run the verb; its answer follows the timeout.
    w.step(1);
    let report = w.last_report(ada).expect("a report");
    assert_eq!((report.action_id, report.ok), (ack.action_id, true));
}

#[test]
fn routines_fire_when_the_resident_is_idle() {
    let mut w = world();
    // The default reports only failed routine actions. This test reads the
    // reports of successful ones.
    w.set_param("kernel.routine_reports", 1);
    let ada = w.spawn("Ada");
    w.give_item(ada, "bread", 2);
    let routines = json!({ "routines": [
        routine("eat", Condition::All { of: vec![
            Condition::AttrBelow { attr: "energy".into(), value: 40 },
            Condition::HasItem { item: "bread".into(), qty: 1 },
        ] }, "eat", Value::Null),
        routine("work", Condition::Always, "work", json!({ "ticks": 4 })),
    ] });
    assert_eq!(
        w.act_ok(ada, "set_routines", routines),
        ActionStatus::Done { result: json!({ "routines": 2 }) }
    );
    assert_eq!(w.activity(ada), None, "routines are checked in the tick, not when they are set");

    // Tick 1: energy is 49, so `eat` does not fire and `work` does.
    w.step(1);
    assert_eq!(w.activity(ada), Some("work"));
    w.step(3);
    assert_eq!(w.cash(ada).cents(), 1_000);
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 1_500, "the routine action finished");
    assert_eq!(w.activity(ada), Some("work"), "and the routine fired again in the same tick");

    // Energy falls one point per tick. Below 40 the first routine wins when
    // the resident is idle next.
    w.step(10);
    assert_eq!(w.attr(ada, "energy"), 50 - 15 + 30, "`eat` fired once and gave 30 energy");
    assert_eq!(w.qty(unsup_kernel::Holder::Resident(ada), "bread"), 1);
    // Routine actions report like any other action.
    let eats: Vec<bool> = w.reports(ada).iter().filter(|r| r.verb == "eat").map(|r| r.ok).collect();
    assert_eq!(eats, [true]);
    assert!(w.reports(ada).iter().filter(|r| r.verb == "work" && r.ok).count() >= 2);

    // Full detail shows each routine with how often it ran.
    let full = w.observe_with(
        ada,
        &unsup_kernel::ObserveOpts {
            detail: unsup_kernel::protocol::Detail::Full,
            ..Default::default()
        },
    );
    let shown = &full.sections["self"]["routines"];
    assert_eq!(
        (shown[0]["name"].clone(), shown[0]["runs"].clone(), shown[0]["failures"].clone()),
        (json!("eat"), json!(1), json!(0))
    );
    assert_eq!(shown[1]["name"], json!("work"));
    assert_eq!(shown[1]["then"]["verb"], json!("work"));
    assert!(shown[1]["runs"].as_u64().is_some_and(|n| n >= 2), "{shown}");

    // An explicit request wins over routines: the resident is busy with it.
    w.act_ok(ada, "cancel", Value::Null);
    w.act_ok(ada, "wait", json!({ "ticks": 3 }));
    assert_eq!(w.activity(ada), Some("wait"));
    // Removing the routines stops them.
    w.act_ok(ada, "set_routines", json!({ "routines": [] }));
    w.step(6);
    assert_eq!(w.activity(ada), None);
}

#[test]
fn routine_reports_can_be_limited_to_failures() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(w.param("kernel.routine_reports"), 0, "the default reports only failures");
    w.give_item(ada, "bread", 1);
    let routines = json!({ "routines": [
        routine("eat", Condition::Always, "eat", Value::Null),
        routine("work", Condition::Always, "work", json!({ "ticks": 2 })),
    ] });
    w.act_ok(ada, "set_routines", routines);
    // Tick 1: `eat` succeeds (instant) and then waits 30 ticks. Tick 2:
    // `work` starts. It ends at ticks 4, 6, ... 32. At tick 32 the resident
    // is idle again with `eat` due: it fails, there is no bread left.
    w.step(32);
    let reports: Vec<(String, bool)> = w.reports(ada).into_iter().map(|r| (r.verb, r.ok)).collect();
    assert_eq!(reports, [("eat".to_string(), false)], "only the failure is in the inbox");
    // The successes are counted in the routines.
    let full = w.observe_with(
        ada,
        &unsup_kernel::ObserveOpts {
            detail: unsup_kernel::protocol::Detail::Full,
            ..Default::default()
        },
    );
    let shown = &full.sections["self"]["routines"];
    assert_eq!((shown[0]["runs"].clone(), shown[0]["failures"].clone()), (json!(1), json!(1)));
    assert_eq!(shown[1]["runs"], json!(15), "`work` ended at ticks 4, 6, ... 32");
    assert_eq!(w.cash(ada).cents(), 1_000 + 500 * 15);
}

#[test]
fn a_failing_routine_backs_off_instead_of_flooding_the_inbox() {
    let mut w = world();
    let ada = w.spawn("Ada");
    // No bread: `eat` fails every time.
    let routines = json!({ "routines": [routine("eat", Condition::Always, "eat", Value::Null)] });
    w.act_ok(ada, "set_routines", routines);
    w.step(200);
    let failures = w.reports(ada).iter().filter(|r| r.verb == "eat" && !r.ok).count();
    // Delays of 30, 60, 120 ticks: tries at ticks 1, 31, 91 and none again
    // before tick 211.
    assert_eq!(failures, 3);
    let full = w.observe_with(
        ada,
        &unsup_kernel::ObserveOpts {
            detail: unsup_kernel::protocol::Detail::Full,
            ..Default::default()
        },
    );
    let shown = &full.sections["self"]["routines"][0];
    assert_eq!((shown["runs"].clone(), shown["failures"].clone()), (json!(0), json!(3)));
}

#[test]
fn set_routines_checks_its_input() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let set = |w: &mut unsup_kernel::testing::TestWorld<'_>, list: Vec<Value>| {
        // `set_routines` is a free verb; a new tick keeps the test clear of
        // the limit of free actions per tick.
        w.step(1);
        w.act(ada, "set_routines", json!({ "routines": list })).status
    };
    let code = |status: ActionStatus| match status {
        ActionStatus::Rejected { error } => error.code,
        other => panic!("expected a rejection, got {other:?}"),
    };
    assert_eq!(w.param("kernel.routines_max"), 8);
    let nine = (0..9)
        .map(|i| routine(&format!("r{i}"), Condition::Always, "wait", json!({ "ticks": 1 })))
        .collect();
    assert_eq!(code(set(&mut w, nine)), ErrorCode::InvalidArgs);
    let unknown_attr = routine(
        "r",
        Condition::AttrBelow { attr: "mana".into(), value: 1 },
        "wait",
        json!({ "ticks": 1 }),
    );
    assert_eq!(code(set(&mut w, vec![unknown_attr])), ErrorCode::InvalidArgs);
    // A hidden attribute cannot be probed through a routine.
    let hidden = routine(
        "r",
        Condition::AttrBelow { attr: "secret".into(), value: 1 },
        "wait",
        json!({ "ticks": 1 }),
    );
    assert_eq!(code(set(&mut w, vec![hidden])), ErrorCode::InvalidArgs);
    let unknown_item = routine(
        "r",
        Condition::HasItem { item: "gold".into(), qty: 1 },
        "wait",
        json!({ "ticks": 1 }),
    );
    assert_eq!(code(set(&mut w, vec![unknown_item])), ErrorCode::InvalidArgs);
    let bad_hours = routine(
        "r",
        Condition::ClockBetween { from_hour: 24, to_hour: 3 },
        "wait",
        json!({ "ticks": 1 }),
    );
    assert_eq!(code(set(&mut w, vec![bad_hours])), ErrorCode::InvalidArgs);
    let unknown_verb = routine("r", Condition::Always, "fly", Value::Null);
    assert_eq!(code(set(&mut w, vec![unknown_verb])), ErrorCode::UnknownVerb);
    let bad_args = routine("r", Condition::Always, "wait", json!({ "ticks": "long" }));
    assert_eq!(code(set(&mut w, vec![bad_args])), ErrorCode::InvalidArgs);
    let recursive = routine("r", Condition::Always, "set_routines", json!({ "routines": [] }));
    assert_eq!(code(set(&mut w, vec![recursive])), ErrorCode::InvalidArgs);
    let mut deep = Condition::Always;
    for _ in 0..10 {
        deep = Condition::Any { of: vec![deep] };
    }
    assert_eq!(
        code(set(&mut w, vec![routine("r", deep, "wait", json!({ "ticks": 1 }))])),
        ErrorCode::InvalidArgs
    );
    // Nothing was set by the refused requests.
    w.step(3);
    assert_eq!(w.activity(ada), None);
    assert!(w.reports(ada).is_empty());
}

#[test]
fn clock_routines_follow_the_calendar() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let night = routine(
        "sleep",
        Condition::ClockBetween { from_hour: 1, to_hour: 2 },
        "nap",
        json!({ "ticks": 5 }),
    );
    w.act_ok(ada, "set_routines", json!({ "routines": [night] }));
    w.step(59);
    assert_eq!(w.activity(ada), None, "it is 00:59");
    w.step(1);
    assert_eq!(w.activity(ada), Some("nap"), "it is 01:00");
    w.step(60);
    w.run_until_idle(ada, 10);
    w.step(5);
    assert_eq!(w.activity(ada), None, "it is past 02:00");
}

#[test]
fn freeze_stops_everything_until_unfreeze() {
    use unsup_kernel::protocol::{AdminCommand, AdminReply};
    let mut w = world();
    let ada = w.spawn("Ada");
    let routines =
        json!({ "routines": [routine("work", Condition::Always, "work", json!({ "ticks": 2 }))] });
    w.act_ok(ada, "set_routines", routines);
    w.act_ok(ada, "work", json!({ "ticks": 9 }));
    w.act_ok(ada, "wait", json!({ "ticks": 1 }));
    assert_eq!(w.world.admin(AdminCommand::Freeze { resident: ada }), AdminReply::Ok);
    assert!(w.has_condition(ada, "frozen"));
    assert_eq!(w.activity(ada), None);
    assert_eq!(w.with_resident(ada, |realm, idx| realm.queue_len(idx)), 0);
    // Every verb is refused, kernel verbs too, and routines do not fire.
    for (verb, args) in
        [("wait", json!({ "ticks": 1 })), ("cancel", Value::Null), ("nap", json!({ "ticks": 1 }))]
    {
        assert_eq!(w.act_err(ada, verb, args).code, ErrorCode::Blocked);
    }
    let reports = w.reports(ada).len();
    w.step(10);
    assert_eq!(w.activity(ada), None);
    assert_eq!(w.reports(ada).len(), reports);

    assert_eq!(w.world.admin(AdminCommand::Unfreeze { resident: ada }), AdminReply::Ok);
    w.step(1);
    assert_eq!(w.activity(ada), Some("work"), "the routine fires again");
    assert!(matches!(
        w.world.admin(AdminCommand::Freeze { resident: unsup_kernel::ResidentId(99) }),
        AdminReply::Error { .. }
    ));
}

#[test]
fn a_long_action_ends_at_its_tick_also_after_a_restore() {
    // 700 ticks is more than the near-future buckets of the timer wheel
    // hold, so the timer waits in the far map first.
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let long = w.act(ada, "wait", json!({ "ticks": 700 }));
    assert_eq!(long.status, ActionStatus::Started { ends_at: Tick(700), ticks: 700 });
    w.set_condition(bob, "tired", Some(650));
    w.step(300);
    // A restored world builds its timers again from the agendas.
    w.snapshot_restore();
    w.step(349);
    assert!(w.has_condition(bob, "tired"), "tick 649");
    w.step(1);
    assert!(!w.has_condition(bob, "tired"), "tick 650");
    w.step(49);
    assert_eq!(w.activity(ada), Some("wait"), "tick 699");
    assert!(w.reports(ada).is_empty());
    w.step(1);
    assert_eq!(w.activity(ada), None, "tick 700");
    let report = w.last_report(ada).expect("a report");
    assert_eq!(
        (report.action_id, report.ok, report.tick, report.result),
        (long.action_id, true, Tick(700), json!({ "waited": 700 }))
    );
}

#[test]
fn steps_move_the_resident_and_count() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let start = w.pos(ada);
    common::walk(&mut w, ada, 1, 1, 3);
    assert_eq!(w.pos(ada), start.offset(3, 3));
    assert_eq!(walker(&w, ada).steps, 3);
    // A step off the edge of the world fails in `finish`; nothing moves.
    w.set_pos(ada, Pos::new(0, 0));
    w.act_ok(ada, "step", json!({ "dx": -1, "dy": 0 }));
    w.step(1);
    let report = w.last_report(ada).expect("a report");
    assert_eq!(report.error.map(|e| e.code), Some(ErrorCode::NotHere));
    assert_eq!(w.pos(ada), Pos::new(0, 0));
    assert_eq!(walker(&w, ada).steps, 3);
}
