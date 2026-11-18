//! The world: spawn, views, the catalogue, snapshots across plugin changes,
//! and realms in other processes.

mod common;

use common::{BANK, Demo, PLAZA, world};
use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::protocol::{ActionStatus, SimError, SpawnSpec, VerbScope, ViewQuery, WorldView};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    AttrDef, Begin, Builder, CityConfig, CityId, Component, CondDef, Envelope, Holder, Money,
    OwnerId, Plugin, Pos, RealmId, Rect, ResidentId, Resource, Scope, SnapshotError, VerbSpec,
    World, WorldConfig, ZoneId,
};

fn spec(name: &str, city: Option<CityId>) -> SpawnSpec {
    SpawnSpec { owner: OwnerId(1), name: name.to_string(), model: Some("model-x".into()), city }
}

#[test]
fn residents_spawn_at_a_spawn_site_or_at_the_centre_of_the_first_zone() {
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("Ashford", Pos::new(0, 0), 2),
            CityConfig::new("Brine", Pos::new(1_000, 1_000), 2),
        ],
        ..WorldConfig::default()
    };
    let mut w = TestWorld::with_config(&[&Demo], config);
    // Ashford has a site with the tag `spawn`: the plaza in zone 0.
    let ada = w.world.spawn(spec("Ada", Some(CityId(0)))).expect("spawn");
    assert_eq!(ada, ResidentId(1));
    assert_eq!((w.zone_of(ada), w.pos(ada)), (ZoneId(0), PLAZA.centre()));
    // Brine has none: the centre of its first zone.
    let bea = w.world.spawn(spec("Bea", Some(CityId(1)))).expect("spawn");
    assert_eq!((w.zone_of(bea), w.pos(bea)), (ZoneId(4), Pos::new(1_032, 1_032)));
    // Without a city, the city with the fewest residents; the lowest id wins
    // a tie.
    let cyd = w.world.spawn(spec("Cyd", None)).expect("spawn");
    assert_eq!(w.zone_of(cyd), ZoneId(0));
    let dan = w.world.spawn(spec("Dan", None)).expect("spawn");
    assert_eq!(w.zone_of(dan), ZoneId(4));

    // The identity is what was asked for, and the spawn hook ran.
    w.with_resident(ada, |realm, idx| {
        let identity = realm.identity(idx);
        assert_eq!(
            (identity.name.as_str(), identity.owner, identity.model.as_deref(), identity.born),
            ("Ada", OwnerId(1), Some("model-x"), 0)
        );
    });
    assert_eq!(w.cash(ada).cents(), 1_000);
    assert!(common::walker(&w, ada).spawned);
    // The global realm learns about spawns in the next frame.
    assert!(w.global().directory().is_empty());
    let frames = w.step(1);
    let dir = w.global().directory();
    assert_eq!((dir.len(), dir.living()), (4, 4));
    assert_eq!(
        dir.get(bea).map(|e| (e.zone, e.owner, e.name.clone(), e.alive)),
        Some((ZoneId(4), OwnerId(1), "Bea".to_string(), true))
    );
    assert_eq!(dir.of_owner(OwnerId(1)).len(), 4);
    drop(dir);
    // Viewers saw the residents enter, and the public spawn events.
    let entered: usize = frames[0].zones.iter().map(|d| d.entered.len()).sum();
    assert_eq!(entered, 4);
    assert_eq!(frames[0].events.iter().filter(|e| e.kind == "kernel.spawned").count(), 4);

    // Bad requests are refused and change nothing.
    for bad in [
        spec("", None),
        spec("   ", None),
        spec(&"n".repeat(65), None),
        spec("Eve", Some(CityId(7))),
    ] {
        assert_eq!(w.world.spawn(bad), Err(SimError::Invalid));
    }
    let long_model = SpawnSpec { model: Some("m".repeat(65)), ..spec("Eve", None) };
    assert_eq!(w.world.spawn(long_model), Err(SimError::Invalid));
    assert_eq!(w.world.residents(), 4);
    assert_eq!(
        w.world.spawn(spec("  Eve  ", None)).map(|id| id.0),
        Ok(5),
        "ids are not burned by refused requests"
    );
    w.with_resident(ResidentId(5), |realm, idx| assert_eq!(realm.identity(idx).name, "Eve"));
}

#[test]
fn views_show_the_layout_and_one_zone() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn_at("Bob", Pos::new(70, 10));
    w.act_ok(ada, "work", json!({ "ticks": 5 }));
    w.step(2);

    let WorldView::Layout { frame, clock, cities, zones } =
        w.world.view(&ViewQuery::Layout).expect("layout")
    else {
        panic!("a layout query gives a layout");
    };
    assert_eq!((frame, clock.minute), (2, 2));
    assert_eq!(cities.len(), 1);
    assert_eq!(
        (cities[0].name.as_str(), cities[0].bounds, cities[0].zones.len()),
        ("Ashford", Rect::new(0, 0, 128, 128), 4)
    );
    assert_eq!(zones.iter().map(|z| z.residents).collect::<Vec<_>>(), [1, 1, 0, 0]);
    assert_eq!(zones[3].bounds, Rect::new(64, 64, 128, 128));

    let view = w.world.view(&ViewQuery::Zone { zone: ZoneId(0) }).expect("zone 0");
    let WorldView::Zone { zone, width, height, tiles, sites, residents } = view else {
        panic!("a zone query gives a zone");
    };
    assert_eq!((zone.id, zone.name.as_str(), width, height), (ZoneId(0), "Ashford 0-0", 64, 64));
    assert_eq!(tiles.len(), 64 * 64);
    // World generation painted the zone with 1 and the plaza with 2.
    assert_eq!(tiles.iter().filter(|&&t| t == 2).count(), 81);
    assert_eq!(tiles.iter().filter(|&&t| t == 1).count(), 64 * 64 - 81);
    assert_eq!(tiles[28 * 64 + 28], 2);
    assert_eq!(sites.len(), 1);
    assert_eq!(
        (sites[0].name.as_str(), sites[0].rect, sites[0].tags.clone()),
        ("Plaza", PLAZA, vec!["spawn".to_string()])
    );
    assert_eq!(residents.len(), 1);
    assert_eq!(
        (residents[0].id, residents[0].activity.as_deref(), residents[0].pos),
        (ada, Some("work"), PLAZA.centre())
    );

    let WorldView::Zone { residents, sites, .. } =
        w.world.view(&ViewQuery::Zone { zone: ZoneId(1) }).expect("zone 1")
    else {
        panic!("a zone query gives a zone");
    };
    assert_eq!(residents.iter().map(|r| r.id).collect::<Vec<_>>(), [bob]);
    assert!(sites.is_empty());
    assert_eq!(w.world.view(&ViewQuery::Zone { zone: ZoneId(4) }), Err(SimError::Invalid));
}

#[test]
fn frames_tell_viewers_what_moved_and_what_changed() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.step(1);
    w.act_ok(ada, "step", json!({ "dx": 1, "dy": 0 }));
    let frames = w.step(1);
    let delta = frames[0].zones.iter().find(|d| d.zone == ZoneId(0)).expect("zone 0 changed");
    assert_eq!(delta.moved, [(ada, PLAZA.centre().offset(1, 0))]);
    // The activity went from `step` to nothing in this frame.
    assert_eq!(delta.activity.last(), Some(&(ada, None)));
    assert_eq!(delta.tick.0, 2);
    assert_eq!(frames[0].frame, 2);
    assert_eq!(frames[0].woke, [ada], "the report woke the resident");
    // A frame in which nothing happens has no deltas.
    let frames = w.step(1);
    assert!(frames[0].zones.is_empty());
    assert!(frames[0].events.is_empty());
    assert!(frames[0].woke.is_empty());
}

#[test]
fn the_catalogue_lists_verbs_with_schemas() {
    let w = world();
    let catalogue = w.world.catalogue();
    let names: Vec<(&str, &str)> =
        catalogue.verbs.iter().map(|v| (v.module.as_str(), v.verb.as_str())).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "verbs are ordered by module and name");
    for verb in ["wait", "cancel", "set_routines"] {
        assert!(names.contains(&("kernel", verb)), "the kernel verb `{verb}` is in the catalogue");
    }
    let step = catalogue.verbs.iter().find(|v| v.verb == "step").expect("step");
    assert_eq!((step.module.as_str(), step.free, step.scope), ("demo", false, VerbScope::Zone));
    assert_eq!(step.summary, "Walk one tile.");
    // Doc comments of the arguments are in the schema that agents read.
    assert_eq!(
        step.args_schema["properties"]["dx"]["description"],
        json!("Columns to move: -1, 0 or 1.")
    );
    assert_eq!(step.args_schema["required"], json!(["dx", "dy"]));
    let ping = catalogue.verbs.iter().find(|v| v.verb == "ping").expect("ping");
    assert_eq!(ping.scope, VerbScope::Global);
    let shout = catalogue.verbs.iter().find(|v| v.verb == "shout").expect("shout");
    assert!(shout.free);
    let wait = catalogue.verbs.iter().find(|v| v.verb == "wait").expect("wait");
    assert!(
        wait.args_schema["properties"]["ticks"]["description"]
            .as_str()
            .is_some_and(|d| d.contains("kernel.wait_max"))
    );

    assert!(catalogue.params.iter().any(|p| p.name == "kernel.queue_max" && p.value == 8));
    assert_eq!(
        catalogue.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["bread", "coal"]
    );
    let attrs: Vec<&str> = catalogue.attrs.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(attrs, ["energy", "skill"], "a hidden attribute is not in the catalogue");
    let conditions: Vec<(&str, bool)> =
        catalogue.conditions.iter().map(|c| (c.name.as_str(), c.blocking)).collect();
    assert_eq!(conditions, [("frozen", true), ("jailed", true), ("tired", false)]);
    // The blueprint knows the systems in running order.
    let systems: Vec<&str> =
        w.world.blueprint().systems(Scope::Zone).map(|(name, _, _)| name).collect();
    assert_eq!(systems, ["demo.decay", "demo.skill"]);
}

#[test]
fn ids_from_alloc_id_are_unique_across_realms_and_fit_in_json() {
    let w = world();
    let mut seen = std::collections::BTreeSet::new();
    for realm in [RealmId::Global, RealmId::Zone(ZoneId(0)), RealmId::Zone(ZoneId(3))] {
        let realm = w.world.realm(realm).expect("hosted");
        for _ in 0..100 {
            let id = realm.alloc_id();
            assert!(id > 0 && id < 1 << 53, "{id}");
            assert!(seen.insert(id), "{id} was given twice");
        }
    }
    assert_eq!(seen.len(), 300);
}

// ---- Snapshots across plugin changes ------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Mood {
    happy: i32,
}

impl Component for Mood {
    const NAME: &'static str = "v.mood";
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Hat {
    colour: String,
}

impl Component for Hat {
    const NAME: &'static str = "v.hat";
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Tally {
    count: u32,
}

impl Resource for Tally {
    const NAME: &'static str = "v.tally";
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct Ticks {
    ticks: u32,
}

/// Version 1 of a module.
struct V1;

/// Version 2: a new attribute and a new component come first, a verb and
/// a condition are gone, a new verb is there.
struct V2;

fn dance(b: &mut Builder) {
    b.verb::<Ticks>(
        VerbSpec { verb: "dance", module: "v", summary: "Dance.", free: false, allowed_in: &[] },
        |_, _, args| Ok(Begin::Busy { ticks: args.ticks }),
        |_, _, _| Ok(json!("danced")),
    );
}

impl Plugin for V1 {
    fn name(&self) -> &'static str {
        "v"
    }

    fn build(&self, b: &mut Builder) {
        b.attr(AttrDef::new("calm").range(0, 100).default(10));
        b.attr(AttrDef::new("luck").range(0, 100).default(20));
        b.condition(CondDef::new("dizzy"));
        b.component::<Mood>();
        if b.scope() == Scope::Zone {
            b.resource::<Tally>();
        }
        dance(b);
        b.verb::<Ticks>(
            VerbSpec {
                verb: "juggle",
                module: "v",
                summary: "Juggle.",
                free: false,
                allowed_in: &[],
            },
            |_, _, args| Ok(Begin::Busy { ticks: args.ticks }),
            |_, _, _| Ok(json!("juggled")),
        );
    }
}

impl Plugin for V2 {
    fn name(&self) -> &'static str {
        "v"
    }

    fn build(&self, b: &mut Builder) {
        b.attr(AttrDef::new("zeal").range(0, 100).default(33));
        b.attr(AttrDef::new("luck").range(0, 100).default(20));
        b.attr(AttrDef::new("calm").range(0, 100).default(10));
        b.component::<Hat>();
        b.component::<Mood>();
        if b.scope() == Scope::Zone {
            b.resource::<Tally>();
        }
        b.verb::<Ticks>(
            VerbSpec { verb: "sing", module: "v", summary: "Sing.", free: false, allowed_in: &[] },
            |_, _, args| Ok(Begin::Busy { ticks: args.ticks }),
            |_, _, _| Ok(json!("sang")),
        );
        dance(b);
    }
}

#[test]
fn a_snapshot_restores_with_a_newer_set_of_plugins() {
    let mut old = TestWorld::new(&[&V1]);
    let ada = old.spawn("Ada");
    let bob = old.spawn("Bob");
    old.set_attr(ada, "calm", 77);
    old.set_attr(ada, "luck", 5);
    old.set_condition(ada, "dizzy", None);
    old.with_resident(ada, |realm, idx| realm.col_mut::<Mood>()[idx].happy = 9);
    old.zone(ZoneId(0)).res_mut::<Tally>().count = 4;
    let dance = old.act(ada, "dance", json!({ "ticks": 6 }));
    let juggle = old.act(bob, "juggle", json!({ "ticks": 6 }));
    old.act_ok(bob, "dance", json!({ "ticks": 2 }));
    old.step(2);
    let luck_id = old.world.blueprint().attr_id("luck").expect("registered");
    let bytes = old.world.snapshot();

    // The same plugins restore to the same state.
    let same = World::restore(&[&V1], &bytes).expect("restores");
    assert_eq!(same.state_hash(), old.world.state_hash());

    // The newer plugins restore too.
    let mut new = TestWorld::restore(&[&V2], &bytes);
    assert_eq!(new.world.frame(), 2);
    // Ids are stable: `luck` keeps its id although `zeal` is registered
    // first now, and the values are those of the snapshot.
    assert_eq!(new.world.blueprint().attr_id("luck"), Some(luck_id));
    assert_eq!((new.attr(ada, "calm"), new.attr(ada, "luck")), (77, 5));
    assert_eq!(new.attr(bob, "calm"), 10);
    // A new attribute and a new component start with their defaults.
    assert_eq!((new.attr(ada, "zeal"), new.attr(bob, "zeal")), (33, 33));
    new.with_resident(ada, |realm, idx| {
        assert_eq!(realm.col::<Hat>()[idx], Hat::default());
        assert_eq!(realm.col::<Mood>()[idx], Mood { happy: 9 });
    });
    assert_eq!(new.zone(ZoneId(0)).res::<Tally>().count, 4);
    // A condition that no plugin registers any more stays, retired.
    assert!(new.has_condition(ada, "dizzy"));

    // A running action of a verb that still exists goes on.
    assert_eq!(new.activity(ada), Some("dance"));
    new.step(4);
    assert_eq!(
        new.last_report(ada).map(|r| (r.action_id, r.ok, r.result)),
        Some((dance.action_id, true, json!("danced")))
    );
    // A running action of a verb that is gone fails cleanly, and the queue
    // goes on.
    new.step(2);
    let reports = new.reports(bob);
    assert_eq!(reports.len(), 2);
    assert_eq!(
        (reports[0].action_id, reports[0].ok, reports[0].verb.as_str()),
        (juggle.action_id, false, "juggle")
    );
    assert_eq!(reports[0].error.as_ref().map(|e| e.code), Some(unsup_kernel::ErrorCode::Internal));
    assert_eq!((reports[1].ok, reports[1].verb.as_str()), (true, "dance"));
    // The retired verb cannot be used, and is not in the catalogue.
    assert_eq!(
        new.act_err(bob, "juggle", json!({ "ticks": 1 })).code,
        unsup_kernel::ErrorCode::UnknownVerb
    );
    let catalogue = new.world.catalogue();
    assert!(catalogue.verbs.iter().all(|v| v.verb != "juggle"));
    assert!(catalogue.verbs.iter().any(|v| v.verb == "sing"));
    assert!(matches!(
        new.act(bob, "sing", json!({ "ticks": 1 })).status,
        ActionStatus::Started { .. }
    ));
    new.assert_conserved();
    // And the new world saves and restores like any other.
    new.snapshot_restore();
}

#[test]
fn a_snapshot_with_its_hash_is_the_snapshot_and_the_hash() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn_at("Bob", Pos::new(100, 100));
    w.step(2);
    // State of every kind, and messages on their way between realms.
    w.act_ok(ada, "work", json!({ "ticks": 9 }));
    w.act_ok(ada, "ping", json!({ "n": 4 }));
    w.act_ok(bob, "deposit", json!({ "amount": 120 }));
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 30 }));
    let (bytes, hash) = w.world.snapshot_with_hash();
    assert_eq!(hash, w.world.state_hash());
    assert_eq!(bytes, w.world.snapshot());
    let restored = World::restore(&[&Demo], &bytes).expect("the snapshot restores");
    assert_eq!(restored.state_hash(), hash);
    assert_eq!(restored.snapshot_with_hash(), (bytes.clone(), hash));

    // The pair is that of the state now: a frame later both are new.
    w.step(1);
    let (later_bytes, later_hash) = w.world.snapshot_with_hash();
    assert_ne!(later_hash, hash);
    assert_ne!(later_bytes, bytes);
    assert_eq!((later_bytes, later_hash), (w.world.snapshot(), w.world.state_hash()));

    // A world that hosts a part of the realms.
    let part = WorldConfig {
        hosted: Some(vec![RealmId::Zone(ZoneId(1)), RealmId::Zone(ZoneId(3))]),
        ..WorldConfig::default()
    };
    let east = World::new(&[&Demo], part);
    assert_eq!(east.snapshot_with_hash(), (east.snapshot(), east.state_hash()));
}

#[test]
fn a_bad_snapshot_is_refused_with_a_reason() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.act_ok(ada, "deposit", json!({ "amount": 5 }));
    w.step(1);
    let bytes = w.world.snapshot();
    assert!(World::restore(&[&Demo], &bytes).is_ok());

    let error = World::restore(&[&Demo], b"not a snapshot").expect_err("garbage");
    assert!(matches!(error, SnapshotError::Decode(_)), "{error}");
    assert!(error.to_string().starts_with("cannot restore the snapshot:"), "{error}");
    let error = World::restore(&[&Demo], &bytes[..bytes.len() / 2]).expect_err("truncated");
    assert!(matches!(error, SnapshotError::Decode(_)), "{error}");
    let error = World::restore(&[&Demo], &[]).expect_err("empty");
    assert!(matches!(error, SnapshotError::Decode(_)), "{error}");
    // Data with the right shape but another marker is refused by the marker.
    let error = World::restore(
        &[&Demo],
        &patch(&bytes, b"unsupervised-snapshot", b"unsupervised-snapshoX"),
    )
    .expect_err("wrong marker");
    assert!(error.to_string().contains("not a snapshot"), "{error}");
    // A snapshot of a format version that this build does not know is
    // refused by the version. `\x01` after the key is the number 1.
    let error = World::restore(&[&Demo], &patch(&bytes, b"\xa7version\x01", b"\xa7version\x63"))
        .expect_err("wrong version");
    assert_eq!(error, SnapshotError::Version { found: 99 });
}

/// Replaces the first occurrence of `from` in `bytes` with `to`. Both have
/// the same length, so the MessagePack structure stays valid.
fn patch(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let at = bytes
        .windows(from.len())
        .position(|window| window == from)
        .expect("the snapshot has the pattern");
    let mut out = bytes.to_vec();
    out[at..at + to.len()].copy_from_slice(to);
    out
}

// ---- Realms in other processes ----------------------------------------------------

/// Moves the external messages of `from` to `to`, through bytes, the way
/// two processes would.
fn carry(from: &mut World, to: &mut World) -> usize {
    let messages: Vec<Envelope> = from
        .take_external()
        .iter()
        .map(|envelope| {
            Envelope::from_bytes(&envelope.to_bytes()).expect("an envelope survives the wire")
        })
        .collect();
    let count = messages.len();
    to.deliver_external(messages);
    count
}

#[test]
fn a_world_can_host_a_subset_of_the_realms() {
    let hosted =
        |realms: Vec<RealmId>| WorldConfig { hosted: Some(realms), ..WorldConfig::default() };
    // West hosts the global realm and zones 0 and 2. East hosts zones 1 and 3.
    let mut west = World::new(
        &[&Demo],
        hosted(vec![RealmId::Global, RealmId::Zone(ZoneId(0)), RealmId::Zone(ZoneId(2))]),
    );
    let mut east =
        World::new(&[&Demo], hosted(vec![RealmId::Zone(ZoneId(1)), RealmId::Zone(ZoneId(3))]));
    assert!(!west.is_complete() && !east.is_complete());
    assert!(west.realm(RealmId::Zone(ZoneId(1))).is_none());
    assert!(east.realm(RealmId::Global).is_none());
    // World generation in the east sent `hello` to the global realm, which
    // is in the west.
    assert_eq!(carry(&mut east, &mut west), 2);

    let ada = west.spawn(spec("Ada", Some(CityId(0)))).expect("spawn in zone 0");
    let zone0 = west.realm(RealmId::Zone(ZoneId(0))).expect("hosted");
    let idx = zone0.idx_of(ada).expect("in zone 0");
    let grant = zone0.defs().faucet_id(unsup_kernel::GRANT_FAUCET).expect("kernel faucet");
    let bread = zone0.defs().item_id("bread").expect("item");
    zone0.ledger_mut().produce(grant, Holder::Resident(ada), bread, 3);
    west.act(ada, unsup_kernel::ActionRequest::new("work", json!({ "ticks": 4 })))
        .expect("ada acts");
    west.step();
    // The global realm answered the two zones in the east.
    assert_eq!(carry(&mut west, &mut east), 2);
    east.step();
    for zone in [1, 3] {
        let log = east
            .realm(RealmId::Zone(ZoneId(zone)))
            .expect("hosted")
            .res::<common::ZoneLog>()
            .hello_back
            .clone();
        assert_eq!(log, [zone + 100]);
    }
    // Ada goes east while she works.
    let zone0 = west.realm(RealmId::Zone(ZoneId(0))).expect("hosted");
    zone0.set_pos(idx, Pos::new(70, 30)).expect("zone 1 exists");
    west.settle();
    assert_eq!(west.locate(ada), None);
    assert_eq!(
        west.act(ada, unsup_kernel::ActionRequest::new("wait", json!({ "ticks": 1 }))),
        Err(SimError::NotHosted)
    );
    assert_eq!(west.observe(ada, &Default::default()).map(|_| ()), Err(SimError::NotHosted));
    // Both worlds stay balanced on their own: the west counts what it sent
    // out, the east what it took in.
    let totals = west.audit().expect("west is balanced");
    assert_eq!((totals.money.held, totals.money.external), (0, 1_000));
    assert_eq!(totals.items[&bread].external, 3);

    assert_eq!(carry(&mut west, &mut east), 1, "the resident is the one message");
    assert_eq!(east.locate(ada), Some(ZoneId(1)));
    let totals = east.audit().expect("east is balanced");
    assert_eq!((totals.money.held, totals.money.external), (1_000, -1_000));
    let zone1 = east.realm(RealmId::Zone(ZoneId(1))).expect("hosted");
    let idx = zone1.idx_of(ada).expect("arrived");
    assert_eq!(zone1.pos(idx), Pos::new(70, 30));
    assert_eq!(zone1.ledger().qty(Holder::Resident(ada), bread), 3);
    assert_eq!(zone1.activity(idx).map(|a| a.verb), Some("work"));

    // The action finishes in the east: one tick was done in the west, three
    // are left. Its event goes to the global realm in the west.
    for _ in 0..3 {
        east.step();
        west.step();
        carry(&mut east, &mut west);
        carry(&mut west, &mut east);
    }
    let zone1 = east.realm(RealmId::Zone(ZoneId(1))).expect("hosted");
    assert_eq!(zone1.ledger().money(Holder::Resident(ada)), Money(1_500));
    west.step();
    let global = west.realm(RealmId::Global).expect("hosted");
    assert!(global.res::<common::GlobalLog>().events.iter().any(|(kind, _)| kind == "demo.worked"));
    assert_eq!(global.directory().zone_of(ada), Some(ZoneId(1)));

    // A deposit from the east reaches the bank in the west.
    east.act(ada, unsup_kernel::ActionRequest::new("deposit", json!({ "amount": 300 })))
        .expect("ada acts");
    assert_eq!(carry(&mut east, &mut west), 1);
    west.step();
    assert_eq!(west.realm(RealmId::Global).expect("hosted").ledger().money(BANK), Money(300));
    let west_totals = west.audit().expect("west is balanced");
    let east_totals = east.audit().expect("east is balanced");
    // Across both processes nothing was created or lost.
    assert_eq!(west_totals.money.external + east_totals.money.external, 0);
    assert_eq!(west_totals.money.held + east_totals.money.held, 1_500);
    assert_eq!(west_totals.money.minted + east_totals.money.minted, 1_500);

    // A world that hosts a part of the realms saves and restores.
    let bytes = east.snapshot();
    let restored = World::restore(&[&Demo], &bytes).expect("restores");
    assert_eq!(restored.state_hash(), east.state_hash());
    assert_eq!(restored.locate(ada), Some(ZoneId(1)));
    assert!(restored.realm(RealmId::Zone(ZoneId(0))).is_none());
}

#[test]
fn the_world_knows_its_layout_and_config() {
    let w = world();
    assert_eq!(w.world.config().zone_size, 64);
    assert_eq!(w.world.layout().zones().len(), 4);
    assert!(w.world.is_complete());
    let zone = w.zone(ZoneId(2));
    assert_eq!(
        zone.zone_info().map(|z| (z.id, z.city, z.bounds)),
        Some((ZoneId(2), CityId(0), Rect::new(0, 64, 64, 128)))
    );
    assert_eq!(zone.layout().zone_at(Pos::new(100, 100)), Some(ZoneId(3)));
    assert_eq!(zone.layout().neighbours(ZoneId(2)), [ZoneId(0), ZoneId(3)]);
    assert_eq!(zone.scope(), Scope::Zone);
    assert_eq!(w.global().zone_info(), None);
    assert_eq!(w.global().scope(), Scope::Global);
    assert_eq!(w.global().len(), 0);
    assert_eq!(w.world.memory_estimate().residents, 0);
}
