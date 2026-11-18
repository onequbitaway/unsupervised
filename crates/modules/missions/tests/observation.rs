//! The observation section `missions`: the missions that a resident works
//! on, and a few new missions near it.

mod common;

use common::{
    bake, bread_mission, join, ok, one_event, plugins, post, rich, section, sponsor, world,
};
use serde_json::{Value, json};
use unsup_kernel::protocol::{Detail, ObserveOpts, SpawnSpec};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityConfig, CityId, Money, OwnerId, Pos, ResidentId, WorldConfig};

/// Returns the section with full detail.
fn full(w: &TestWorld<'_>, r: ResidentId) -> Option<Value> {
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    w.observe_with(r, &opts).sections.get("missions").cloned()
}

/// Returns the bread mission with another reward and title.
fn mission(title: &str, reward: i64) -> Value {
    let mut definition = bread_mission(reward, 3);
    definition["title"] = json!(title);
    definition
}

#[test]
fn the_section_shows_accepted_missions_with_progress() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    assert_eq!(section(&w, bob), None, "a section with nothing to say is absent");

    // A new mission of the same city. The poster does not see its own
    // mission as new.
    let first = post(&mut w, ann, mission("Bread", 500));
    assert_eq!(
        section(&w, bob),
        Some(json!({ "new": [{ "mission": first, "reward": 500, "deadline": 601 }] }))
    );
    assert_eq!(section(&w, ann), None);

    // Bob takes it: it moves from `new` to `accepted`, with the progress of
    // each objective as [done, needed].
    ok(&mut w, bob, "mission_accept", json!({ "mission": first }));
    assert_eq!(
        section(&w, bob),
        Some(json!({
            "accepted": [{ "mission": first, "reward": 500, "deadline": 601, "progress": [[0, 3]] }],
        }))
    );
    bake(&mut w, bob);
    bake(&mut w, bob);
    w.step(1);
    assert_eq!(section(&w, bob).expect("a section")["accepted"][0]["progress"], json!([[2, 3]]));

    // Full detail adds the title and the objectives. Both are text of the
    // poster.
    assert_eq!(
        full(&w, bob),
        Some(json!({
            "accepted": [{
                "mission": first,
                "reward": 500,
                "deadline": 601,
                "progress": [[2, 3]],
                "title": { "untrusted": true, "from": ann, "text": "Bread" },
                "objectives": [{
                    "event": { "untrusted": true, "from": ann, "text": "economy.goods_produced" },
                    "fields": { "untrusted": true, "from": ann, "text": "{\"item\":\"bread\"}" },
                    "count": 3,
                    "actor": "assignee",
                }],
            }],
        }))
    );

    // The third loaf completes the mission, and the section is absent again.
    bake(&mut w, bob);
    w.step(2);
    assert_eq!(w.cash(bob), Money(500));
    assert_eq!(section(&w, bob), None);
    w.assert_conserved();
}

#[test]
fn the_lists_are_short_and_the_best_new_missions_come_first() {
    let mut w = world();
    w.set_param("missions.posted_max", 10);
    let ann = rich(&mut w, "Ann", 1, 100_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let rewards = [300, 900, 500, 900, 100];
    let ids: Vec<u64> = rewards
        .iter()
        .map(|&reward| post(&mut w, ann, mission(&format!("Bread for {reward}"), reward)))
        .collect();

    // Three entries: the highest rewards first, and the newer mission first
    // among equals.
    let new = section(&w, bob).expect("a section")["new"].clone();
    let shown: Vec<(u64, i64)> = new
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| (entry["mission"].as_u64().unwrap(), entry["reward"].as_i64().unwrap()))
        .collect();
    assert_eq!(shown, [(ids[3], 900), (ids[1], 900), (ids[2], 500)]);
    // Full detail adds the titles and keeps the limit: other residents
    // decide how many new missions there are.
    let detailed = full(&w, bob).expect("a section")["new"].clone();
    assert_eq!(detailed.as_array().map(Vec::len), Some(3));
    assert_eq!(
        detailed[0],
        json!({
            "mission": ids[3],
            "reward": 900,
            "deadline": 607,
            "title": { "untrusted": true, "from": ann, "text": "Bread for 900" },
        })
    );

    // Bob takes four missions. Brief detail shows three and counts the
    // rest; full detail shows all.
    for id in &ids[..4] {
        ok(&mut w, bob, "mission_accept", json!({ "mission": id }));
    }
    let brief = section(&w, bob).expect("a section");
    assert_eq!(brief["more_accepted"], json!(1));
    let accepted: Vec<u64> = brief["accepted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["mission"].as_u64().unwrap())
        .collect();
    assert_eq!(accepted, ids[..3]);
    assert_eq!(brief["new"], json!([{ "mission": ids[4], "reward": 100, "deadline": 609 }]));
    let detailed = full(&w, bob).expect("a section");
    assert_eq!(detailed["accepted"].as_array().map(Vec::len), Some(4));
    assert_eq!(detailed.get("more_accepted"), None);

    // The limit is a parameter.
    w.set_param("missions.observe_max", 1);
    let brief = section(&w, bob).expect("a section");
    assert_eq!(brief["more_accepted"], json!(3));
    assert_eq!(brief["accepted"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_mission_is_new_for_a_time_and_while_it_has_a_place() {
    let mut w = world();
    w.set_param("missions.new_minutes", 30);
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let first = post(&mut w, ann, mission("Bread", 500));
    assert_eq!(first, section(&w, cid).expect("a section")["new"][0]["mission"].as_u64().unwrap());

    // Bob takes the only place: the mission is not new for Cid any more.
    ok(&mut w, bob, "mission_accept", json!({ "mission": first }));
    assert_eq!(section(&w, cid), None);
    ok(&mut w, bob, "mission_abandon", json!({ "mission": first }));
    assert!(section(&w, cid).is_some());

    // It was published in frame 1. In frame 31 it is 30 minutes old and
    // still new; one frame later it is not.
    w.step((31 - w.world.frame()) as u32);
    assert!(section(&w, cid).is_some());
    w.step(1);
    assert_eq!(section(&w, cid), None);
    // It is still open: `mission_list` finds it.
    let listed = ok(&mut w, cid, "mission_list", json!({}));
    assert_eq!(listed["missions"][0]["mission"], json!(first));
}

#[test]
fn near_means_the_same_city_or_a_sponsor() {
    // Two cities. Ann and Bob live in the first one, Cid in the second.
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("North", Pos::new(0, 0), 2),
            CityConfig::new("South", Pos::new(0, 1_000), 1),
        ],
        ..WorldConfig::default()
    };
    let mut w = TestWorld::with_config(&plugins(), config);
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = w
        .world
        .spawn(SpawnSpec {
            owner: OwnerId(3),
            name: "Cid".into(),
            model: None,
            city: Some(CityId(1)),
        })
        .expect("the spawn is valid");

    // A mission of a resident is near the residents of its city.
    let local = post(&mut w, ann, mission("Bread", 500));
    assert_eq!(section(&w, bob).expect("a section")["new"][0]["mission"], json!(local));
    assert_eq!(section(&w, cid), None);

    // A sponsor mission has no place: every resident sees it.
    sponsor(&mut w, 1, "Acme", mission("Bread for all", 300));
    let sponsored = one_event(&w.step(1), "missions.mission_posted")["mission"].clone();
    assert_eq!(
        section(&w, cid),
        Some(json!({ "new": [{ "mission": sponsored, "reward": 300, "deadline": 603 }] }))
    );
    let both = section(&w, bob).expect("a section")["new"].clone();
    assert_eq!(both.as_array().map(Vec::len), Some(2));
    assert_eq!((&both[0]["mission"], &both[1]["mission"]), (&json!(local), &sponsored));

    // Cid can still take the mission of the other city: `mission_list` shows
    // every open mission.
    let listed = ok(&mut w, cid, "mission_list", json!({ "sponsored": false }));
    assert_eq!(listed["missions"][0]["mission"], json!(local));
    ok(&mut w, cid, "mission_accept", json!({ "mission": local }));
    assert_eq!(section(&w, cid).expect("a section")["accepted"][0]["mission"], json!(local));
    w.assert_conserved();
}

#[test]
fn a_team_mission_is_new_for_the_members_of_an_organization() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let dan = rich(&mut w, "Dan", 4, 0);
    join(&mut w, bob, 5_000);
    join(&mut w, cid, 6_000);
    // The global realm learns the organizations of a resident from the
    // report of its zone, each 60 frames.
    w.step(60);
    let mut any_team = mission("Team bread", 500);
    any_team["mode"] = json!("team");
    let any_team = post(&mut w, ann, any_team);
    let mut bakers_only = mission("Bakers only", 400);
    bakers_only["mode"] = json!("team");
    bakers_only["org"] = json!(5_000);
    let bakers_only = post(&mut w, ann, bakers_only);

    // Bob is a baker: he sees both. Cid is in another organization: Cid
    // sees the mission for every team. Dan has no organization.
    assert_eq!(
        section(&w, bob).expect("a section")["new"],
        json!([
            { "mission": any_team, "reward": 500, "deadline": 661, "mode": "team" },
            { "mission": bakers_only, "reward": 400, "deadline": 663, "mode": "team" },
        ])
    );
    assert_eq!(
        section(&w, cid).expect("a section")["new"],
        json!([{ "mission": any_team, "reward": 500, "deadline": 661, "mode": "team" }])
    );
    assert_eq!(section(&w, dan), None);

    // An accepted team mission names the organization.
    ok(&mut w, bob, "mission_accept", json!({ "mission": bakers_only, "org": 5_000 }));
    assert_eq!(
        section(&w, bob).expect("a section")["accepted"],
        json!([{ "mission": bakers_only, "reward": 400, "deadline": 663, "org": 5_000, "progress": [[0, 3]] }])
    );
}
