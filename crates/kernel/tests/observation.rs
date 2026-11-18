//! Observations: sections, size, the notice cursor, and no change of state.

mod common;

use common::{Demo, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::{Detail, SimError};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    AttrDef, Builder, Holder, ItemDef, ObserveOpts, OrgId, Plugin, Pos, Rect, ResidentId, Scope,
    Site, SiteId,
};

/// A module that gives residents the amount of state that a full game has:
/// a dozen attributes, a handful of item kinds, sites all around.
struct Crowd;

impl Plugin for Crowd {
    fn name(&self) -> &'static str {
        "crowd"
    }

    fn build(&self, b: &mut Builder) {
        for name in [
            "food",
            "water",
            "sleep",
            "health",
            "mood",
            "hygiene",
            "age",
            "reputation",
            "strength",
            "charm",
            "wits",
            "craft",
        ] {
            b.attr(AttrDef::new(name).range(0, 1_000).default(500));
        }
        for name in ["water_bottle", "apple", "work_clothes", "house_key", "phone", "notebook"] {
            b.item(ItemDef::new(name).weight(1));
        }
        if b.scope() == Scope::Zone {
            b.on_worldgen(10, |realm, _| {
                let Some(info) = realm.zone_info() else { return };
                if info.id.0 != 0 {
                    return;
                }
                let kinds = ["shop", "home", "office", "park", "bank"];
                for i in 0..30 {
                    let id = SiteId(realm.alloc_id());
                    let rect = Rect::sized(10 + (i % 6) * 8, 10 + (i / 6) * 8, 5, 5);
                    realm.sites_mut().insert(Site::new(
                        id,
                        kinds[i as usize % 5],
                        format!("Building number {i}"),
                        rect,
                    ));
                }
            });
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, JsonSchema)]
struct Nothing {}

/// A module with an observer that breaks the rule and writes state.
struct Meddler;

impl Plugin for Meddler {
    fn name(&self) -> &'static str {
        "meddler"
    }

    fn build(&self, b: &mut Builder) {
        let energy = b.attr(AttrDef::new("energy").range(0, 100).default(50));
        b.observe("meddler", 0, move |realm, idx, _opts| {
            realm.attr_set(idx, energy, 1);
            None
        });
        b.instant_verb::<Nothing>(
            unsup_kernel::VerbSpec {
                verb: "noop",
                module: "meddler",
                summary: "Nothing.",
                free: true,
                allowed_in: &[],
            },
            |_, _, _| Ok(Value::Null),
        );
    }
}

fn typical_world() -> (TestWorld<'static>, ResidentId) {
    let mut w = TestWorld::new(&[&Demo, &Crowd]);
    let ada = w.spawn_at("Ada Lovelace", Pos::new(32, 32));
    // Twenty neighbours within sight: more than the list shows.
    for i in 0..20 {
        let other = w.spawn_at(&format!("Neighbour {i}"), Pos::new(26 + i % 10, 30 + i / 10));
        if i % 2 == 0 {
            w.act_ok(other, "work", json!({ "ticks": 30 }));
        }
    }
    for item in
        ["water_bottle", "apple", "work_clothes", "house_key", "phone", "notebook", "bread", "coal"]
    {
        w.give_item(ada, item, 2);
    }
    w.set_condition(ada, "tired", Some(300));
    w.set_condition(ada, "jailed", None);
    w.act_ok(ada, "nap", json!({ "ticks": 20 }));
    w.act_ok(ada, "nap", json!({ "ticks": 5 }));
    w.act_ok(ada, "wait", json!({ "ticks": 5 }));
    w.with_resident(ada, |realm, idx| realm.grant(realm.id_of(idx), OrgId(3), "clerk", &["sell"]));
    w.step(3);
    (w, ada)
}

#[test]
fn a_brief_observation_of_a_typical_resident_is_small() {
    let (w, ada) = typical_world();
    // A client that polls sends its cursor, so old notices are not repeated.
    let cursor = w.observe(ada).next_notice;
    let observation =
        w.observe_with(ada, &ObserveOpts { since_notice: cursor, ..Default::default() });
    let text = serde_json::to_string(&observation).expect("an observation is JSON");
    assert!(text.len() < 4_000, "a brief observation is {} bytes: {text}", text.len());
    // It is small because the lists are bounded, not because it is empty.
    let nearby = &observation.sections["nearby"];
    assert_eq!(nearby["residents"].as_array().map(Vec::len), Some(12));
    assert_eq!(nearby["more_residents"], json!(8));
    assert_eq!(nearby["sites"].as_array().map(Vec::len), Some(12));
    assert!(nearby["more_sites"].as_u64().is_some_and(|n| n > 0));
    let me = &observation.sections["self"];
    assert_eq!(
        me["attrs"].as_object().map(|a| a.len()),
        Some(13),
        "twelve needs and `energy`; `skill` is full-only"
    );
    assert_eq!(me["items"].as_object().map(|a| a.len()), Some(8));
    assert_eq!(me["conditions"], json!({ "jailed": null, "tired": 300 }));
    assert_eq!(me["queue"], json!(2));
    assert_eq!(me["activity"]["verb"], json!("nap"));

    // With a full inbox of ordinary notices the answer is larger; a client
    // keeps it small with the cursor.
    let mut w = w;
    let bob = w.spawn_at("Bob", Pos::new(33, 33));
    for i in 0..5 {
        w.act_ok(bob, "pay", json!({ "to": ada, "amount": 1 + i }));
    }
    let with_news =
        w.observe_with(ada, &ObserveOpts { since_notice: cursor, ..Default::default() });
    assert_eq!(with_news.notices.len(), 5);
    let text = serde_json::to_string(&with_news).expect("an observation is JSON");
    assert!(text.len() < 4_500, "with five notices: {} bytes", text.len());
}

#[test]
fn the_self_section_shows_what_the_resident_is_and_has() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.give_item(ada, "bread", 2);
    let work = w.act(ada, "work", json!({ "ticks": 5 }));
    w.act_ok(ada, "wait", json!({ "ticks": 2 }));
    w.step(1);
    let observation = w.observe(ada);
    assert_eq!((observation.frame, observation.tick.0, observation.resident), (1, 1, ada));
    assert_eq!(
        (observation.clock.hour, observation.clock.minute, observation.clock.day),
        (0, 1, 1)
    );
    let me = &observation.sections["self"];
    assert_eq!(me["id"], json!(ada));
    assert_eq!(me["name"], json!("Ada"));
    assert_eq!(me["pos"], json!({ "x": 32, "y": 32 }));
    assert_eq!((me["zone"].clone(), me["city"].clone()), (json!(0), json!(0)));
    assert_eq!(me["site"]["name"], json!("Plaza"));
    assert_eq!(me["site"]["kind"], json!("plaza"));
    assert_eq!(
        me["activity"],
        json!({ "verb": "work", "action_id": work.action_id, "started": 0, "ends": 5, "waiting": false })
    );
    assert_eq!(me["queue"], json!(1));
    assert_eq!(me["conditions"], json!({}));
    assert_eq!(
        me["attrs"],
        json!({ "energy": 49 }),
        "`skill` shows only in full detail, `secret` never"
    );
    assert_eq!(me["cash"], json!(1_000));
    assert_eq!(me["items"], json!({ "bread": 2 }));
    assert!(me.get("routines").is_none() && me.get("owner").is_none());
    // The zone observer and the global observer added their sections.
    assert_eq!(observation.sections["demo"], json!({ "steps": 0 }));
    assert_eq!(observation.sections["bank"], json!({ "deposits": 0, "pings": 0, "name": "Ada" }));
    let names: Vec<&str> = observation.sections.keys().map(String::as_str).collect();
    assert_eq!(names, ["bank", "demo", "nearby", "reports", "self"]);

    // Full detail adds what a brief observation leaves out.
    let full = w.observe_with(ada, &ObserveOpts { detail: Detail::Full, ..Default::default() });
    let me = &full.sections["self"];
    assert_eq!(me["attrs"], json!({ "energy": 49, "skill": 0 }));
    assert_eq!(me["zone_name"], json!("Ashford 0-0"));
    assert_eq!(me["owner"], json!(1));
    assert_eq!(me["born"], json!(0));
    assert_eq!(me["queued"][0]["verb"], json!("wait"));
    assert_eq!(me["routines"], json!([]));
    assert_eq!(me["memberships"], json!([]));
    assert_eq!(full.sections["demo"], json!({ "steps": 0, "arrivals": 0 }));

    // Off a site there is no `site` key.
    w.set_pos(ada, Pos::new(5, 5));
    assert!(w.observe(ada).sections["self"].get("site").is_none());
}

#[test]
fn the_nearby_section_lists_the_nearest_first() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(10, 10));
    let two = w.spawn_at("Two", Pos::new(12, 10));
    let one = w.spawn_at("One", Pos::new(10, 11));
    let also_two = w.spawn_at("AlsoTwo", Pos::new(8, 12));
    let _far = w.spawn_at("Far", Pos::new(23, 10));
    w.act_ok(one, "work", json!({ "ticks": 9 }));
    let nearby = w.observe(ada).sections["nearby"].clone();
    let listed: Vec<(u64, String)> = nearby["residents"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|r| (r["id"].as_u64().expect("id"), r["name"].as_str().expect("name").to_string()))
        .collect();
    // By distance, then by id. `Far` is 13 tiles away: out of sight.
    assert_eq!(
        listed,
        [
            (one.0, "One".to_string()),
            (two.0, "Two".to_string()),
            (also_two.0, "AlsoTwo".to_string())
        ]
    );
    assert_eq!(nearby["residents"][0]["activity"], json!("work"));
    assert!(nearby["residents"][1].get("activity").is_none());
    assert_eq!(nearby["residents"][1]["pos"], json!({ "x": 12, "y": 10 }));
    assert!(nearby.get("more_residents").is_none());
    // The plaza is 18 tiles away: not in sight from here, in sight from (20, 20).
    assert_eq!(nearby["sites"], json!([]));
    w.set_pos(ada, Pos::new(20, 20));
    let sites = w.observe(ada).sections["nearby"]["sites"].clone();
    assert_eq!(sites[0]["name"], json!("Plaza"));
    assert_eq!(sites[0]["pos"], json!({ "x": 28, "y": 28 }), "the nearest tile of the site");

    // The list length follows the parameter.
    w.set_param("kernel.nearby_max", 1);
    w.set_param("kernel.sight_radius", 30);
    w.step(1);
    w.set_pos(ada, Pos::new(10, 10));
    let nearby = w.observe(ada).sections["nearby"].clone();
    assert_eq!(nearby["residents"].as_array().map(Vec::len), Some(1));
    assert_eq!(nearby["more_residents"], json!(3));
}

#[test]
fn since_notice_filters_notices_and_reports() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 1 }));
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 2 }));
    w.act_ok(ada, "work", json!({ "ticks": 1 }));
    w.step(1);
    let first = w.observe(ada);
    assert_eq!(first.notices.iter().map(|n| n.seq).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(first.sections["reports"].as_array().map(Vec::len), Some(1));
    assert_eq!(first.next_notice, 4);

    // Nothing new: the cursor filters everything.
    let again =
        w.observe_with(ada, &ObserveOpts { since_notice: first.next_notice, ..Default::default() });
    assert!(again.notices.is_empty());
    assert_eq!(again.sections["reports"], json!([]));
    assert_eq!(again.next_notice, 4);

    // One new notice and one new report: only they come back.
    w.act_ok(bob, "pay", json!({ "to": ada, "amount": 3 }));
    w.act_ok(ada, "work", json!({ "ticks": 1 }));
    w.step(1);
    let later =
        w.observe_with(ada, &ObserveOpts { since_notice: first.next_notice, ..Default::default() });
    assert_eq!(
        later.notices.iter().map(|n| (n.seq, n.data.clone())).collect::<Vec<_>>(),
        [(4, json!({ "amount": 3 }))]
    );
    assert_eq!(later.sections["reports"].as_array().map(Vec::len), Some(1));
    assert_eq!(later.next_notice, 6);
    // A cursor in the middle.
    let middle = w.observe_with(ada, &ObserveOpts { since_notice: 2, ..Default::default() });
    assert_eq!(middle.notices.iter().map(|n| n.seq).collect::<Vec<_>>(), [2, 4]);
    assert_eq!(w.world.next_notice(ada), Ok(6));
}

#[test]
fn sections_can_be_chosen() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.step(1);
    let opts = ObserveOpts {
        sections: Some(vec!["self".into(), "bank".into(), "no_such".into()]),
        ..Default::default()
    };
    let observation = w.observe_with(ada, &opts);
    let names: Vec<&str> = observation.sections.keys().map(String::as_str).collect();
    assert_eq!(names, ["bank", "self"]);
    let none =
        w.observe_with(ada, &ObserveOpts { sections: Some(Vec::new()), ..Default::default() });
    assert!(none.sections.is_empty());
}

#[test]
fn observing_does_not_change_the_state_hash() {
    let (mut w, ada) = typical_world();
    let before = w.hash();
    for detail in [Detail::Brief, Detail::Full] {
        for since in [0, 3, 100] {
            let opts = ObserveOpts { detail, since_notice: since, sections: None };
            let _ = w.observe_with(ada, &opts);
        }
    }
    let _ = w.world.view(&unsup_kernel::protocol::ViewQuery::Layout);
    let _ =
        w.world.view(&unsup_kernel::protocol::ViewQuery::Zone { zone: unsup_kernel::ZoneId(0) });
    let _ = w.world.catalogue();
    let _ = w.world.audit();
    let _ = w.world.snapshot();
    assert_eq!(w.hash(), before);
    // The hash is not blind: a real change shows.
    w.give_money(ada, unsup_kernel::Money(1));
    assert_ne!(w.hash(), before);
    w.with_resident(ada, |realm, _| {
        let faucet = realm.defs().faucet_id(unsup_kernel::GRANT_FAUCET).expect("kernel faucet");
        assert_eq!(realm.ledger().minted_by(faucet).cents(), 1);
    });
    assert_eq!(w.cash(ada).cents(), 1_001);
    let _ = Holder::Resident(ada);
}

#[test]
fn unknown_residents_cannot_be_observed() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(
        w.world.observe(ResidentId(99), &ObserveOpts::default()).map(|_| ()),
        Err(SimError::UnknownResident)
    );
    assert_eq!(w.world.next_notice(ResidentId(99)), Err(SimError::UnknownResident));
    w.despawn(ada);
    assert_eq!(
        w.world.observe(ada, &ObserveOpts::default()).map(|_| ()),
        Err(SimError::UnknownResident)
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "an observer must not change state")]
fn an_observer_that_writes_state_is_caught() {
    let mut w = TestWorld::new(&[&Meddler]);
    let ada = w.spawn("Ada");
    let _ = w.world.observe(ada, &ObserveOpts::default());
}
