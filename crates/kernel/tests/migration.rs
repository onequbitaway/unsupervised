//! Migration between zones, teleport and despawn.

mod common;

use common::{Walker, ZoneLog, walk, walker, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionRequest, ActionStatus, Condition, Routine, Tick};
use unsup_kernel::{
    Holder, Input, Money, OrgId, Output, Pos, PosError, RealmId, ResidentId, UNCLAIMED, ZoneId,
};

#[test]
fn a_resident_that_walks_across_a_border_keeps_everything() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(62, 32));
    let bob = w.spawn("Bob");
    assert_eq!(w.zone_of(ada), ZoneId(0));

    // Give Ada state of every kind.
    w.set_attr(ada, "energy", 90);
    w.set_attr(ada, "skill", 7);
    w.set_condition(ada, "tired", Some(40));
    w.give_item(ada, "bread", 3);
    w.give_item(ada, "coal", 2);
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 200 }));
    w.with_resident(ada, |realm, idx| {
        realm.grant(realm.id_of(idx), OrgId(5), "clerk", &["sell", "stock"]);
    });
    let routine = Routine {
        name: "nap".into(),
        when: Condition::AttrBelow { attr: "energy".into(), value: 5 },
        then: ActionRequest::new("nap", json!({ "ticks": 2 })),
    };
    w.act_ok(ada, "set_routines", json!({ "routines": [routine] }));
    let notices_before = w.notices(ada);
    assert_eq!(notices_before.len(), 1, "the payment notice");
    let walker_before = walker(&w, ada);
    assert_eq!(walker_before, Walker { steps: 0, arrivals: 0, departures: 0, spawned: true });

    // Two steps east reach x = 64, the first column of zone 1. The second
    // step is followed by a long action, so Ada crosses with an action
    // that runs.
    w.act_ok(ada, "step", json!({ "dx": 1, "dy": 0 }));
    w.act_ok(ada, "step", json!({ "dx": 1, "dy": 0 }));
    let work = w.act(ada, "work", json!({ "ticks": 4 }));
    assert_eq!(work.status, ActionStatus::Queued { position: 2 });
    let frames = w.step(1);
    assert_eq!(w.zone_of(ada), ZoneId(0));
    assert!(frames[0].zones.iter().all(|d| d.left.is_empty()));
    let frames = w.step(1);

    // Ada is in zone 1 now, and the frame said so to viewers.
    assert_eq!(w.pos(ada), Pos::new(64, 32));
    assert_eq!(w.zone_of(ada), ZoneId(1));
    assert_eq!(w.zone(ZoneId(0)).idx_of(ada), None);
    let left: Vec<_> = frames[0]
        .zones
        .iter()
        .filter(|d| d.zone == ZoneId(0))
        .flat_map(|d| d.left.clone())
        .collect();
    assert_eq!(left, [ada]);
    let entered: Vec<_> = frames[0]
        .zones
        .iter()
        .filter(|d| d.zone == ZoneId(1))
        .flat_map(|d| d.entered.clone())
        .collect();
    assert_eq!(entered.len(), 1);
    assert_eq!(
        (entered[0].id, entered[0].pos, entered[0].activity.as_deref()),
        (ada, Pos::new(64, 32), Some("work"))
    );

    // Components, attributes, conditions, holdings, inbox, memberships and
    // routines came along.
    assert_eq!(walker(&w, ada), Walker { steps: 2, arrivals: 1, departures: 1, spawned: true });
    assert_eq!(w.attr(ada, "energy"), 88, "90 minus two ticks of decay");
    assert_eq!(w.attr(ada, "skill"), 7);
    assert!(w.has_condition(ada, "tired"));
    assert_eq!(w.cash(ada).cents(), 1_200);
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 3);
    assert_eq!(w.qty(Holder::Resident(ada), "coal"), 2);
    assert_eq!(
        w.zone(ZoneId(0)).ledger().money(Holder::Resident(ada)),
        Money(0),
        "nothing stays behind"
    );
    assert_eq!(w.zone(ZoneId(1)).ledger().money(Holder::Resident(ada)), Money(1_200));
    assert_eq!(w.notices(ada), notices_before);
    w.with_resident(ada, |realm, idx| {
        assert!(realm.can(idx, OrgId(5), "sell"));
        assert!(!realm.can(idx, OrgId(5), "fire"));
        assert_eq!(realm.has_perm(idx, "stock"), Some(OrgId(5)));
        assert_eq!(realm.memberships(idx).len(), 1);
    });
    let full = w.observe_with(
        ada,
        &unsup_kernel::ObserveOpts {
            detail: unsup_kernel::protocol::Detail::Full,
            ..Default::default()
        },
    );
    assert_eq!(full.sections["self"]["routines"][0]["name"], json!("nap"));
    assert_eq!(full.sections["self"]["zone"], json!(1));

    // The running action finishes in the new zone, at the right tick.
    assert_eq!(w.activity(ada), Some("work"));
    w.step(3);
    assert_eq!(w.activity(ada), Some("work"), "three of four ticks");
    assert_eq!(w.cash(ada).cents(), 1_200);
    w.step(1);
    assert_eq!(w.activity(ada), None);
    assert_eq!(w.cash(ada).cents(), 1_700);
    let report = w.last_report(ada).expect("the action reported in the new zone");
    assert_eq!((report.action_id, report.ok, report.tick), (work.action_id, true, Tick(6)));
    // The event of the finished action was handled in the new zone.
    assert_eq!(w.zone(ZoneId(1)).res::<ZoneLog>().worked, 1);
    assert_eq!(w.zone(ZoneId(0)).res::<ZoneLog>().worked, 0);

    // The condition still ends at its tick, in the new zone.
    w.step(33);
    assert!(w.has_condition(ada, "tired"), "tick 39");
    w.step(1);
    assert!(!w.has_condition(ada, "tired"), "tick 40");
    // The directory of the global realm followed.
    assert_eq!(w.global().directory().zone_of(ada), Some(ZoneId(1)));
    assert_eq!(w.global().directory().zone_of(bob), Some(ZoneId(0)));
    w.assert_conserved();
}

#[test]
fn a_resident_can_walk_there_and_back_through_all_four_zones() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 63));
    let mut zones = vec![w.zone_of(ada)];
    for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
        walk(&mut w, ada, dx, dy, 1);
        zones.push(w.zone_of(ada));
    }
    assert_eq!(zones, [ZoneId(0), ZoneId(1), ZoneId(3), ZoneId(2), ZoneId(0)]);
    assert_eq!(w.pos(ada), Pos::new(63, 63));
    assert_eq!(walker(&w, ada), Walker { steps: 4, arrivals: 4, departures: 4, spawned: true });
    assert_eq!(w.cash(ada).cents(), 1_000);
    assert_eq!(w.world.residents(), 1);
    for zone in 1..4 {
        assert!(w.zone(ZoneId(zone)).is_empty());
    }
    w.assert_conserved();
}

#[test]
fn positions_outside_the_world_are_refused() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let start = w.pos(ada);
    w.with_resident(ada, |realm, idx| {
        assert_eq!(realm.set_pos(idx, Pos::new(-1, 10)), Err(PosError::OutOfWorld));
        assert_eq!(realm.set_pos(idx, Pos::new(128, 10)), Err(PosError::OutOfWorld));
        assert_eq!(realm.teleport(idx, ZoneId(9), Pos::new(1, 1)), Err(PosError::UnknownZone));
        assert_eq!(realm.teleport(idx, ZoneId(1), Pos::new(1, 1)), Err(PosError::OutsideZone));
        assert_eq!(realm.pos(idx), start);
    });
    w.world.settle();
    assert_eq!((w.zone_of(ada), w.pos(ada)), (ZoneId(0), start));
}

#[test]
fn a_resident_that_steps_back_before_the_tick_ends_does_not_migrate() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.with_resident(ada, |realm, idx| {
        realm.set_pos(idx, Pos::new(100, 100)).expect("zone 3");
        // The position changes at once, the table at the end of the tick.
        assert_eq!(realm.pos(idx), Pos::new(100, 100));
        assert_eq!(realm.idx_of(realm.id_of(idx)), Some(idx));
        realm.set_pos(idx, Pos::new(10, 10)).expect("back in zone 0");
    });
    w.world.settle();
    assert_eq!((w.zone_of(ada), w.pos(ada)), (ZoneId(0), Pos::new(10, 10)));
    assert_eq!(walker(&w, ada).departures, 0);
}

#[test]
fn teleport_through_a_verb_moves_at_once_between_frames() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.act_ok(ada, "jump", json!({ "zone": 3, "x": 100, "y": 90 }));
    assert_eq!((w.zone_of(ada), w.pos(ada)), (ZoneId(3), Pos::new(100, 90)));
    // The resident can act in the new zone without a frame in between.
    assert!(w.act(ada, "work", json!({ "ticks": 2 })).status.is_accepted());
    w.step(2);
    assert_eq!(w.cash(ada).cents(), 1_500);
    assert_eq!(
        w.act_err(ada, "jump", json!({ "zone": 3, "x": 0, "y": 0 })).code,
        unsup_kernel::ErrorCode::NotFound
    );
}

#[test]
fn indices_are_stable_during_a_tick_and_removal_happens_at_its_end() {
    let mut w = world();
    let ids: Vec<ResidentId> = (0..6).map(|i| w.spawn(&format!("r{i}"))).collect();
    let zone = w.zone(ZoneId(0));
    let order: Vec<ResidentId> = zone.ids().to_vec();
    assert_eq!(order, ids);
    // Mark three residents to leave. Nothing moves in the table yet.
    for &id in &[ids[0], ids[2], ids[5]] {
        let idx = zone.idx_of(id).expect("in the zone");
        zone.set_pos(idx, Pos::new(70, 70)).expect("zone 3");
    }
    assert_eq!(zone.ids().to_vec(), order, "the table changes only at the end of the tick");
    for (i, &id) in ids.iter().enumerate() {
        assert_eq!(zone.idx_of(id).map(|idx| idx.index()), Some(i));
    }
    w.world.settle();
    // The rest is still there, findable by id, with a dense index.
    let zone = w.zone(ZoneId(0));
    let mut left: Vec<ResidentId> = zone.ids().to_vec();
    left.sort_unstable();
    assert_eq!(left, [ids[1], ids[3], ids[4]]);
    for &id in &left {
        let idx = zone.idx_of(id).expect("in the zone");
        assert!(idx.index() < 3);
        assert_eq!(zone.id_of(idx), id);
    }
    let mut arrived: Vec<ResidentId> = w.zone(ZoneId(3)).ids().to_vec();
    arrived.sort_unstable();
    assert_eq!(arrived, [ids[0], ids[2], ids[5]]);
    // `nearby` still finds the right residents after the table was packed.
    let near = w.zone(ZoneId(3)).nearby(Pos::new(70, 70), 0);
    assert_eq!(near.len(), 3);
    // The three that stayed stand in the row of the centre of the plaza,
    // where they appeared: 1, 3 and 4 tiles to the east of it.
    let zone = w.zone(ZoneId(0));
    assert_eq!(zone.nearby(Pos::new(32, 32), 4).len(), 3);
    assert_eq!(zone.nearby(Pos::new(32, 32), 2).len(), 1);
}

#[test]
fn a_zone_that_skips_frames_keeps_the_remaining_time_of_an_action() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.act_ok(ada, "work", json!({ "ticks": 5 }));
    w.step(2);
    // Zone 3 skips three frames (time dilation). Then Ada moves there with
    // three ticks of work left.
    for _ in 0..3 {
        let out = w.world.apply(Input::Step { skip: vec![ZoneId(3)] });
        assert!(matches!(out, Output::Frame(_)));
    }
    assert_eq!(w.zone(ZoneId(0)).tick(), Tick(5));
    assert_eq!(w.zone(ZoneId(3)).tick(), Tick(2));
    assert_eq!(w.cash(ada).cents(), 1_500, "the work ended in zone 0 at tick 5");

    w.act_ok(ada, "work", json!({ "ticks": 5 }));
    w.step(2);
    w.set_pos(ada, Pos::new(100, 100));
    assert_eq!(w.zone_of(ada), ZoneId(3));
    // Three ticks are left, in the time base of zone 3 (tick 4 now).
    let ends = w.with_resident(ada, |realm, idx| realm.activity(idx).expect("still working").ends);
    assert_eq!((w.zone(ZoneId(3)).tick(), ends), (Tick(4), Tick(7)));
    w.step(2);
    assert_eq!(w.cash(ada).cents(), 1_500);
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 2_000);
    // The calendar follows the frame, not the zone tick.
    assert_eq!(w.zone(ZoneId(3)).frame(), 10);
    assert_eq!(w.zone(ZoneId(3)).clock(), w.zone(ZoneId(0)).clock());
}

#[test]
fn despawn_gives_the_holdings_to_the_heir() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let cyd = w.spawn_at("Cyd", Pos::new(100, 20));
    let dan = w.spawn("Dan");
    w.give_item(ada, "bread", 2);
    w.give_item(cyd, "coal", 4);

    // Heir in the same zone: a local transfer.
    w.act_ok(ada, "work", json!({ "ticks": 10 }));
    w.act_ok(ada, "wait", json!({ "ticks": 1 }));
    w.with_resident(ada, |realm, idx| realm.despawn(idx, Holder::Resident(bob), "old age"));
    assert!(w.is_alive(ada), "the resident leaves at the end of the tick");
    let frames = w.step(1);
    assert!(!w.is_alive(ada));
    assert_eq!(w.cash(bob).cents(), 2_000);
    assert_eq!(w.qty(Holder::Resident(bob), "bread"), 2);
    assert_eq!(w.cash(ada).cents(), 0);
    let despawned: Vec<_> =
        frames[0].events.iter().filter(|e| e.kind == "kernel.despawned").collect();
    assert_eq!(despawned.len(), 1);
    assert_eq!(despawned[0].data, json!({ "resident": ada, "name": "Ada", "cause": "old age" }));
    assert!(frames[0].zones.iter().any(|d| d.left.contains(&ada)));
    let log = w.zone(ZoneId(0)).res::<ZoneLog>();
    assert_eq!(log.despawned, [ada]);
    assert_eq!(log.interrupted, ["work:despawned"]);
    drop(log);
    assert!(!w.global().directory().is_alive(ada));
    assert_eq!(w.global().directory().get(ada).map(|e| e.name.as_str()), Some("Ada"));
    assert_eq!(
        w.world.act(ada, ActionRequest::new("wait", json!({ "ticks": 1 }))),
        Err(unsup_kernel::protocol::SimError::UnknownResident)
    );

    // Heir in another zone: the holdings are wired there.
    w.act_ok(cyd, "leave", json!({ "heir": bob }));
    assert!(!w.is_alive(cyd));
    assert_eq!(w.cash(bob).cents(), 2_000, "the wire is on its way");
    let totals = w.assert_conserved();
    assert_eq!(totals.money.in_flight, 1_000);
    w.step(1);
    assert_eq!(w.cash(bob).cents(), 3_000);
    assert_eq!(w.qty(Holder::Resident(bob), "coal"), 4);

    // No heir: everything goes to the unclaimed account of the global realm.
    w.act_ok(dan, "leave", Value::Null);
    w.step(1);
    assert_eq!(w.global().ledger().money(UNCLAIMED), Money(1_000));
    // An administrator despawn does the same.
    assert_eq!(
        w.world.admin(unsup_kernel::protocol::AdminCommand::Despawn { resident: bob }),
        unsup_kernel::protocol::AdminReply::Ok
    );
    assert!(!w.is_alive(bob));
    w.step(1);
    assert_eq!(w.global().ledger().money(UNCLAIMED), Money(4_000));
    assert_eq!(w.world.residents(), 0);
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.in_flight), (4_000, 0));
    assert_eq!(w.global().directory().living(), 0);
    assert_eq!(w.global().directory().len(), 4);
    assert_eq!(w.world.realm(RealmId::Zone(ZoneId(1))).map(|r| r.len()), Some(0));
}

#[test]
fn the_global_realm_grants_and_revokes_memberships() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(100, 100));
    assert_eq!(w.zone_of(ada), ZoneId(3));
    // The module that owns organizations lives in the global realm. The
    // kernel mirrors the membership to the zone of the resident.
    w.global().grant(ada, OrgId(9), "member", &["vote"]);
    w.global().grant(ada, OrgId(4), "owner", &["vote", "hire"]);
    w.world.settle();
    w.with_resident(ada, |realm, idx| {
        assert!(!realm.can(idx, OrgId(9), "vote"), "the change is a message")
    });
    w.step(1);
    w.with_resident(ada, |realm, idx| {
        assert!(realm.can(idx, OrgId(9), "vote"));
        assert!(!realm.can(idx, OrgId(9), "hire"));
        assert!(realm.can(idx, OrgId(4), "hire"));
        assert_eq!(realm.has_perm(idx, "vote"), Some(OrgId(4)), "the lowest organization id");
        assert_eq!(realm.has_perm(idx, "fire"), None);
        let orgs: Vec<(OrgId, String)> =
            realm.memberships(idx).iter().map(|m| (m.org, m.role.clone())).collect();
        assert_eq!(orgs, [(OrgId(4), "owner".to_string()), (OrgId(9), "member".to_string())]);
    });
    // A new grant replaces the role; a revoke ends the membership.
    w.global().grant(ada, OrgId(4), "clerk", &["sell"]);
    w.global().revoke(ada, OrgId(9));
    w.world.settle();
    w.step(1);
    w.with_resident(ada, |realm, idx| {
        assert!(!realm.can(idx, OrgId(4), "hire"));
        assert!(realm.can(idx, OrgId(4), "sell"));
        assert!(!realm.can(idx, OrgId(9), "vote"));
        assert_eq!(realm.memberships(idx).len(), 1);
    });
    // The global realm can also end a wait of a resident in a zone.
    let offer = w.act(ada, "offer", json!({ "ticks": 50 }));
    w.global().complete_action(ada, offer.action_id, Ok(json!("done by the global realm")));
    w.world.settle();
    w.step(1);
    assert_eq!(
        w.last_report(ada).map(|r| (r.action_id, r.result)),
        Some((offer.action_id, json!("done by the global realm")))
    );
}

#[test]
fn messages_follow_a_resident_that_moved() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    // Bob pokes Ada. The message is in the inbox of zone 0 when Ada jumps
    // to zone 2, so zone 0 sends it on.
    w.act_ok(bob, "poke", json!({ "to": ada, "amount": 41 }));
    w.act_ok(ada, "jump", json!({ "zone": 2, "x": 5, "y": 70 }));
    w.step(1);
    assert!(w.zone(ZoneId(0)).res::<ZoneLog>().resident_messages.is_empty());
    assert!(w.zone(ZoneId(2)).res::<ZoneLog>().resident_messages.is_empty());
    w.step(1);
    assert_eq!(w.zone(ZoneId(2)).res::<ZoneLog>().resident_messages, [(ada, 41)]);
    // The same for money: a wire follows the resident.
    w.act_ok(ada, "jump", json!({ "zone": 3, "x": 100, "y": 100 }));
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 300 }));
    w.act_ok(ada, "jump", json!({ "zone": 1, "x": 100, "y": 5 }));
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 1_000, "zone 3 got the wire, but Ada had left");
    w.assert_conserved();
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 1_300);
    assert_eq!(w.zone(ZoneId(3)).ledger().money(Holder::Resident(ada)), Money(0));
    w.assert_conserved();
    assert_eq!(w.world.dropped_messages(), 0);
}
