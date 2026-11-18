//! Parameters: data files, overrides, cities, and changes from the global
//! realm.

mod common;

use common::{Demo, world};
use serde_json::json;
use unsup_kernel::protocol::{AdminCommand, AdminReply};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityConfig, CityId, Pos, World, WorldConfig, WorldError, ZoneId};

fn two_cities() -> WorldConfig {
    WorldConfig {
        cities: vec![
            CityConfig::new("Ashford", Pos::new(0, 0), 1),
            CityConfig::new("Brine", Pos::new(1_000, 0), 2),
        ],
        ..WorldConfig::default()
    }
}

#[test]
fn defaults_come_from_the_data_files() {
    let w = world();
    assert_eq!(w.param("demo.work_pay"), 500);
    assert_eq!(w.param("demo.step_ticks"), 1);
    assert_eq!(w.param("kernel.queue_max"), 8);
    assert_eq!(w.param("kernel.free_actions_per_tick"), 4);
    assert_eq!(w.param("kernel.inbox_max"), 32);
    assert_eq!(w.param("kernel.routines_max"), 8);
    assert_eq!(w.param("kernel.sight_radius"), 12);
    assert_eq!(w.param("kernel.nearby_max"), 12);
    let bp = w.world.blueprint();
    let pay = bp.param(bp.param_id("demo.work_pay").expect("registered"));
    assert_eq!((pay.min, pay.max, pay.policy), (0, 100_000, true));
    assert_eq!(pay.summary, "Cents that one `work` action pays.");
    let bread = bp.item(bp.item_id("bread").expect("registered")).expect("defined");
    assert_eq!((bread.weight, bread.attr_value("nutrition"), bread.has_tag("food")), (1, 30, true));
    assert_eq!(
        bp.items_with_tag("fuel").map(|(_, def)| def.name.as_str()).collect::<Vec<_>>(),
        ["coal"]
    );
}

#[test]
fn a_data_override_replaces_the_default() {
    let data = r#"
[params."demo.work_pay"]
value = 700

[params."kernel.queue_max"]
value = 2

[[items]]
name = "bread"
weight = 3

[[items]]
name = "gold"
weight = 9
tags = ["metal"]
"#;
    let config = WorldConfig { data: vec![data.to_string()], ..WorldConfig::default() };
    let mut w = TestWorld::with_config(&[&Demo], config);
    assert_eq!(w.param("demo.work_pay"), 700);
    let ada = w.spawn("Ada");
    w.act_ok(ada, "work", json!({ "ticks": 1 }));
    // The override is in force in the zone: the queue takes two requests.
    w.act_ok(ada, "wait", json!({ "ticks": 1 }));
    w.act_ok(ada, "wait", json!({ "ticks": 1 }));
    assert_eq!(
        w.act_err(ada, "wait", json!({ "ticks": 1 })).code,
        unsup_kernel::ErrorCode::QueueFull
    );
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 1_700);
    // Only the given fields changed; a new item was added.
    let bp = w.world.blueprint();
    let pay = bp.param(bp.param_id("demo.work_pay").expect("registered"));
    assert_eq!((pay.value, pay.max, pay.policy), (700, 100_000, true));
    let bread = bp.item(bp.item_id("bread").expect("registered")).expect("defined");
    assert_eq!((bread.weight, bread.attr_value("nutrition")), (3, 30));
    assert!(bp.item_id("gold").is_some());
    // The override survives a snapshot.
    w.snapshot_restore();
    assert_eq!(w.param("demo.work_pay"), 700);
}

#[test]
fn a_bad_override_is_refused_when_the_world_is_made() {
    let make = |data: &str| {
        let config = WorldConfig { data: vec![data.to_string()], ..WorldConfig::default() };
        World::try_new(&[&Demo], config).map(|_| ())
    };
    let message = |data: &str| match make(data) {
        Err(WorldError::Data(e)) => e.message,
        other => panic!("expected a data error, got {other:?}"),
    };
    assert!(
        message("[params.\"demo.no_such\"]\nvalue = 1")
            .contains("unknown parameter `demo.no_such`")
    );
    assert!(message("[params.\"demo.work_pay\"]\nvalue = 100001").contains("outside"));
    assert!(message("[params.\"demo.work_pay\"]\nvalue = \"much\"").contains("invalid type"));
    assert!(message("this is not toml").len() > 5);
    assert!(make("[params.\"demo.work_pay\"]\nvalue = 100000").is_ok());
    let invalid = WorldConfig { zone_size: 1, ..WorldConfig::default() };
    assert!(matches!(World::try_new(&[&Demo], invalid), Err(WorldError::Config(_))));
}

#[test]
fn a_change_in_the_global_realm_reaches_the_zones_in_the_next_frame() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let pay = w.world.blueprint().param_id("demo.work_pay").expect("registered");
    w.act_ok(ada, "set_pay", json!({ "amount": 900 }));
    // Frame 1: the global realm runs the verb and changes the parameter.
    w.step(1);
    assert_eq!(w.global().param(pay), 900);
    for zone in 0..4 {
        assert_eq!(w.zone(ZoneId(zone)).param(pay), 500, "zone {zone} has not heard yet");
    }
    // Frame 2: every zone has the new value from the start of its tick.
    w.step(1);
    for zone in 0..4 {
        assert_eq!(w.zone(ZoneId(zone)).param(pay), 900, "zone {zone}");
    }
    w.act_ok(ada, "work", json!({ "ticks": 1 }));
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 1_900);

    // The value is clamped to the range of the parameter.
    w.act_ok(ada, "set_pay", json!({ "amount": 5_000_000 }));
    w.step(2);
    assert_eq!(w.last_report(ada).map(|r| r.result), Some(json!({ "pay": 100_000 })));
    assert_eq!(w.zone(ZoneId(0)).param(pay), 100_000);
    // The catalogue shows the current national value.
    let catalogue = w.world.catalogue();
    let info =
        catalogue.params.iter().find(|p| p.name == "demo.work_pay").expect("in the catalogue");
    assert_eq!(
        (info.value, info.min, info.max, info.policy),
        (100_000, Some(0), Some(100_000), true)
    );
}

#[test]
fn a_city_override_applies_only_in_that_city() {
    let mut w = TestWorld::with_config(&[&Demo], two_cities());
    let pay = w.world.blueprint().param_id("demo.work_pay").expect("registered");
    let brine = CityId(1);
    assert_eq!(
        w.world.admin(AdminCommand::SetParam {
            name: "demo.work_pay".into(),
            value: 650,
            city: Some(brine)
        }),
        AdminReply::Ok
    );
    w.step(1);
    // Zone 0 is Ashford; zones 1 to 4 are Brine.
    assert_eq!(w.zone(ZoneId(0)).param(pay), 500);
    for zone in 1..5 {
        assert_eq!(w.zone(ZoneId(zone)).param(pay), 650, "zone {zone}");
    }
    assert_eq!(w.global().param(pay), 500, "the national value did not change");
    assert_eq!(w.global().param_in(pay, brine), 650);
    assert_eq!(w.global().param_in(pay, CityId(0)), 500);
    assert_eq!(w.zone(ZoneId(0)).param_in(pay, brine), 650, "every zone knows the overrides");

    // A national change does not touch the override.
    w.set_param("demo.work_pay", 510);
    w.step(1);
    assert_eq!(w.zone(ZoneId(0)).param(pay), 510);
    assert_eq!(w.zone(ZoneId(3)).param(pay), 650);

    // A resident in Brine earns the city value.
    let spec = unsup_kernel::protocol::SpawnSpec {
        owner: unsup_kernel::OwnerId(1),
        name: "Bea".into(),
        model: None,
        city: Some(brine),
    };
    let bea = w.world.spawn(spec).expect("spawn in Brine");
    assert_eq!(w.zone_of(bea), ZoneId(1));
    w.act_ok(bea, "work", json!({ "ticks": 1 }));
    w.step(1);
    assert_eq!(w.cash(bea).cents(), 1_650);

    // Removing the override gives the city the national value again.
    w.global().clear_param(pay, brine);
    w.step(2);
    for zone in 0..5 {
        assert_eq!(w.zone(ZoneId(zone)).param(pay), 510, "zone {zone}");
    }
    // The override state survives a snapshot.
    w.world.admin(AdminCommand::SetParam {
        name: "demo.work_pay".into(),
        value: 777,
        city: Some(brine),
    });
    w.step(1);
    w.snapshot_restore();
    assert_eq!(w.zone(ZoneId(2)).param(pay), 777);
    assert_eq!(w.zone(ZoneId(0)).param(pay), 510);
}

#[test]
fn admin_set_param_checks_its_input() {
    let mut w = world();
    let reply =
        w.world.admin(AdminCommand::SetParam { name: "demo.nope".into(), value: 1, city: None });
    assert_eq!(reply, AdminReply::Error { message: "No parameter is named `demo.nope`.".into() });
    let reply = w.world.admin(AdminCommand::SetParam {
        name: "demo.work_pay".into(),
        value: 1,
        city: Some(CityId(9)),
    });
    assert_eq!(reply, AdminReply::Error { message: "No city has the id 9.".into() });
    let reply = w.world.admin(AdminCommand::RollbackZone { zone: ZoneId(0), frames: 10 });
    assert!(matches!(reply, AdminReply::Error { .. }), "the kernel has no snapshot history");
    assert_eq!(w.param("demo.work_pay"), 500);
}

#[test]
#[should_panic(expected = "only the global realm can change a parameter")]
fn a_zone_cannot_change_a_parameter() {
    let w = world();
    let pay = w.world.blueprint().param_id("demo.work_pay").expect("registered");
    w.zone(ZoneId(0)).set_param(pay, 1, None);
}
