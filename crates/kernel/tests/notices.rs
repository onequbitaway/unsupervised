//! Events, audiences, notices and messages between realms.

mod common;

use common::{GlobalLog, ZoneLog, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::Tick;
use unsup_kernel::{Pos, RealmId, ResidentId, ZoneId};

#[test]
fn a_nearby_audience_reaches_only_residents_within_the_radius() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(20, 20));
    let near = w.spawn_at("Near", Pos::new(25, 15));
    let edge = w.spawn_at("Edge", Pos::new(15, 25));
    let far = w.spawn_at("Far", Pos::new(26, 20));
    let frames_before = w.step(1);
    assert!(frames_before[0].woke.is_empty());

    w.act_ok(ada, "shout", json!({ "text": "fresh bread!" }));
    let frames = w.step(1);

    for hearer in [near, edge] {
        let notices = w.notices(hearer);
        assert_eq!(notices.len(), 1, "{hearer} is within 5 tiles");
        let notice = &notices[0];
        assert_eq!(
            (notice.seq, notice.kind.as_str(), notice.from, notice.tick),
            (1, "demo.shout", Some(ada), Tick(1))
        );
        // Text of a resident is always marked as untrusted.
        assert_eq!(notice.data, json!({ "untrusted": true, "from": ada, "text": "fresh bread!" }));
    }
    assert!(w.notices(far).is_empty(), "6 tiles is outside the radius");
    assert!(w.notices(ada).is_empty(), "the actor does not get its own event");

    // Viewers see the event, and the runtime learns who has news.
    let shouts: Vec<_> = frames[0].events.iter().filter(|e| e.kind == "demo.shout").collect();
    assert_eq!(shouts.len(), 1);
    assert_eq!(
        (shouts[0].public, shouts[0].actor, shouts[0].realm, shouts[0].drama),
        (true, Some(ada), RealmId::Zone(ZoneId(0)), 1)
    );
    assert_eq!(frames[0].woke, [near, edge]);
}

#[test]
fn a_private_audience_reaches_only_the_listed_residents_in_any_zone() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(20, 20));
    let beside = w.spawn_at("Beside", Pos::new(21, 20));
    let friend = w.spawn_at("Friend", Pos::new(22, 20));
    let remote = w.spawn_at("Remote", Pos::new(100, 100));
    assert_eq!(w.zone_of(remote), ZoneId(3));

    w.act_ok(ada, "whisper", json!({ "to": [friend, remote], "text": "the vault is open" }));
    // In the same zone the notice is there at once.
    assert_eq!(w.notices(friend).len(), 1);
    assert!(w.notices(remote).is_empty(), "another zone gets it in its next tick");
    let frames = w.step(1);
    let notices = w.notices(remote);
    assert_eq!(notices.len(), 1);
    assert_eq!((notices[0].kind.as_str(), notices[0].from), ("demo.whisper", Some(ada)));
    assert_eq!(notices[0].data["text"], json!("the vault is open"));
    assert_eq!(notices[0].data["untrusted"], json!(true));

    assert!(w.notices(beside).is_empty(), "a resident one tile away is not in the list");
    assert!(w.notices(ada).is_empty());
    // Viewers do not see a private event.
    assert!(frames[0].events.iter().all(|e| e.kind != "demo.whisper"));
    assert_eq!(frames[0].woke, [friend, remote]);

    // A notice for a resident that no longer exists is dropped, not kept.
    w.despawn(remote);
    w.act_ok(ada, "whisper", json!({ "to": [remote, ResidentId(999)], "text": "anyone?" }));
    w.step(2);
    assert_eq!(w.world.dropped_messages(), 2);
}

#[test]
fn zone_handlers_run_in_the_same_tick_and_global_handlers_in_the_same_frame() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let cyd = w.spawn_at("Cyd", Pos::new(100, 100));
    w.act_ok(ada, "work", json!({ "ticks": 3 }));
    w.act_ok(cyd, "work", json!({ "ticks": 3 }));
    w.step(2);
    assert_eq!(w.zone(ZoneId(0)).res::<ZoneLog>().worked, 0);
    assert!(w.global().res::<GlobalLog>().events.is_empty());

    let frames = w.step(1);
    // The zone handler ran in the tick in which the event was emitted.
    assert_eq!(w.zone(ZoneId(0)).res::<ZoneLog>().worked, 1);
    assert_eq!(w.zone(ZoneId(3)).res::<ZoneLog>().worked, 1);
    assert_eq!(w.zone(ZoneId(1)).res::<ZoneLog>().worked, 0);
    // The global handler got the events of both zones in the same frame,
    // ordered by zone.
    let events = w.global().res::<GlobalLog>().events.clone();
    assert_eq!(events, [("demo.worked".to_string(), 3), ("demo.worked".to_string(), 3)]);
    // The same events went to viewers, ordered by realm and sequence number.
    let worked: Vec<_> = frames[0]
        .events
        .iter()
        .filter(|e| e.kind == "demo.worked")
        .map(|e| (e.realm, e.actor, e.frame))
        .collect();
    assert_eq!(
        worked,
        [(RealmId::Zone(ZoneId(0)), Some(ada), 3), (RealmId::Zone(ZoneId(3)), Some(cyd), 3)]
    );

    // The global handler listens to `demo.`: a shout between frames reaches
    // it in the next frame, a kernel event never does.
    w.act_ok(ada, "shout", json!({ "text": "x" }));
    w.step(1);
    let events = w.global().res::<GlobalLog>().events.clone();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[2],
        ("demo.shout".to_string(), 3),
        "emitted between frames 3 and 4, stamped with frame 3"
    );
    assert!(events.iter().all(|(kind, _)| kind.starts_with("demo.")));
}

#[test]
fn the_inbox_is_bounded_and_sequence_numbers_go_on() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    assert_eq!(w.param("kernel.inbox_max"), 32);
    w.give_money(ada, unsup_kernel::Money(5_000));
    for i in 0..70 {
        w.act_ok(ada, "pay", json!({ "to": bob, "amount": 1 + i }));
    }
    let observation = w.observe(bob);
    assert_eq!(observation.notices.len(), 32);
    assert_eq!(observation.next_notice, 71);
    assert_eq!(observation.notices.first().map(|n| n.seq), Some(39));
    assert_eq!(
        observation.notices.last().map(|n| (n.seq, n.data.clone())),
        Some((70, json!({ "amount": 70 })))
    );
    // A smaller limit applies from the next notice on.
    w.set_param("kernel.inbox_max", 5);
    w.step(1);
    w.act_ok(ada, "pay", json!({ "to": bob, "amount": 1 }));
    let seqs: Vec<u64> = w.notices(bob).iter().map(|n| n.seq).collect();
    assert_eq!(seqs, [67, 68, 69, 70, 71]);
}

#[test]
fn typed_messages_reach_their_realm_in_order() {
    let mut w = world();
    // World generation: every zone said hello to the global realm, and the
    // global realm answers each in its first tick.
    assert!(w.global().res::<GlobalLog>().hellos.is_empty(), "messages wait for a tick");
    w.step(1);
    let hellos = w.global().res::<GlobalLog>().hellos.clone();
    assert_eq!(
        hellos,
        [(ZoneId(0), 0), (ZoneId(1), 1), (ZoneId(2), 2), (ZoneId(3), 3)],
        "ordered by source zone"
    );
    assert!(
        w.zone(ZoneId(2)).res::<ZoneLog>().hello_back.is_empty(),
        "the answer arrives in the next frame"
    );
    w.step(1);
    for zone in 0..4 {
        assert_eq!(w.zone(ZoneId(zone)).res::<ZoneLog>().hello_back, [zone + 100]);
    }
    // The global realm ticks once per frame.
    assert_eq!(w.global().res::<GlobalLog>().ticks, 2);

    // A message to a resident is handled in the zone where the resident is.
    let ada = w.spawn("Ada");
    let cyd = w.spawn_at("Cyd", Pos::new(100, 20));
    w.act_ok(ada, "poke", json!({ "to": cyd, "amount": 9 }));
    w.act_ok(cyd, "poke", json!({ "to": ada, "amount": 4 }));
    w.step(1);
    assert_eq!(w.zone(ZoneId(1)).res::<ZoneLog>().resident_messages, [(cyd, 9)]);
    assert_eq!(w.zone(ZoneId(0)).res::<ZoneLog>().resident_messages, [(ada, 4)]);
}

#[test]
fn reports_and_notices_share_one_sequence() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 1 }));
    w.act_ok(ada, "work", json!({ "ticks": 1 }));
    w.step(1);
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 2 }));
    let observation = w.observe(ada);
    assert_eq!(observation.notices.iter().map(|n| n.seq).collect::<Vec<_>>(), [1, 3]);
    assert_eq!(observation.next_notice, 4);
    let reports = w.reports(ada);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].verb, "work");
    assert_eq!(
        observation.sections["reports"],
        json!([{
            "seq": 2, "action_id": reports[0].action_id, "verb": "work", "tick": 1, "ok": true,
            "result": { "pay": 500 }
        }])
    );
    assert_eq!(Value::Null, observation.sections["self"]["activity"]);
}
