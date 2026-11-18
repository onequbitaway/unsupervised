//! `mission_list`: the filters and the paging.

mod common;

use common::{bake, bread_mission, join, ok, one_event, post, rich, sponsor, world};
use serde_json::{Value, json};
use unsup_kernel::ResidentId;
use unsup_kernel::testing::TestWorld;
use unsup_missions::Missions;

/// Returns the ids of the missions in an answer of `mission_list`.
fn ids(w: &mut TestWorld<'_>, r: ResidentId, args: Value) -> Vec<u64> {
    let answer = ok(w, r, "mission_list", args);
    answer["missions"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| entry["mission"].as_u64().expect("an id"))
        .collect()
}

#[test]
fn the_filters_choose_missions() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 100_000);
    let bob = rich(&mut w, "Bob", 2, 100_000);
    let cid = rich(&mut w, "Cid", 3, 0);
    join(&mut w, cid, 5_000);

    // Four missions: bread for 300 (Ann), a shift for 800 (Ann), a team
    // mission for 500 (Bob) and bread for 900 (a sponsor).
    let bread = post(&mut w, ann, bread_mission(300, 1));
    let shift = post(
        &mut w,
        ann,
        json!({ "title": "A shift", "objectives": [{ "event": "economy.shift_completed" }], "reward": 800, "deadline_in": 600 }),
    );
    let mut team = bread_mission(500, 1);
    team["mode"] = json!("team");
    let team = post(&mut w, bob, team);
    sponsor(&mut w, 1, "Acme", bread_mission(900, 1));
    let sponsored =
        one_event(&w.step(1), "missions.mission_posted")["mission"].as_u64().expect("the id");
    let state = w.global().res::<Missions>();
    assert_eq!(state.ids().collect::<Vec<_>>(), [bread, shift, team, sponsored]);
    assert_eq!(state.open_count(), 4);
    drop(state);

    // Without a filter: every mission that the caller can take, oldest
    // first. The caller's own missions are not in the list.
    assert_eq!(ids(&mut w, cid, json!({})), [bread, shift, team, sponsored]);
    assert_eq!(ids(&mut w, ann, json!({})), [team, sponsored]);
    assert_eq!(ids(&mut w, ann, json!({ "scope": "posted" })), [bread, shift]);
    assert_eq!(ids(&mut w, cid, json!({ "scope": "posted" })), [] as [u64; 0]);

    // One entry of a list: no text and no objectives, only their number.
    let answer = ok(&mut w, cid, "mission_list", json!({ "min_reward": 900 }));
    assert_eq!(
        answer,
        json!({
            "missions": [{
                "mission": sponsored,
                "reward": 900,
                "mode": "solo",
                "deadline": 607,
                "taken": 0,
                "takers": 1,
                "title": { "untrusted": true, "from": 0, "text": "Bake bread" },
                "sponsored": true,
                "objectives": 1,
            }],
        })
    );

    // The filters.
    assert_eq!(ids(&mut w, cid, json!({ "mode": "team" })), [team]);
    assert_eq!(ids(&mut w, cid, json!({ "mode": "solo" })), [bread, shift, sponsored]);
    assert_eq!(ids(&mut w, cid, json!({ "min_reward": 500 })), [shift, team, sponsored]);
    assert_eq!(ids(&mut w, cid, json!({ "event": "economy.shift_completed" })), [shift]);
    assert_eq!(ids(&mut w, cid, json!({ "event": "life.slept" })), [] as [u64; 0]);
    assert_eq!(ids(&mut w, cid, json!({ "sponsored": true })), [sponsored]);
    assert_eq!(ids(&mut w, cid, json!({ "sponsored": false })), [bread, shift, team]);
    assert_eq!(
        ids(&mut w, cid, json!({ "sponsored": false, "mode": "solo", "min_reward": 301 })),
        [shift]
    );

    // A mission that the caller works on is `accepted`, not `open`. A
    // mission without a free place is not `open` for the others.
    ok(&mut w, cid, "mission_accept", json!({ "mission": bread }));
    ok(&mut w, cid, "mission_accept", json!({ "mission": team, "org": 5_000 }));
    assert_eq!(ids(&mut w, cid, json!({ "scope": "accepted" })), [bread, team]);
    assert_eq!(ids(&mut w, cid, json!({})), [shift, sponsored]);
    assert_eq!(ids(&mut w, bob, json!({})), [shift, sponsored]);
    // The poster sees how many takers its mission has.
    let own = ok(
        &mut w,
        ann,
        "mission_list",
        json!({ "scope": "posted", "event": "economy.goods_produced" }),
    );
    assert_eq!(
        (&own["missions"][0]["taken"], &own["missions"][0]["takers"]),
        (&json!(1), &json!(1))
    );

    // One mission by id: every resident can read it, with its own part.
    bake(&mut w, cid);
    w.step(2);
    let read = ok(&mut w, bob, "mission_list", json!({ "mission": sponsored }));
    assert_eq!(read["mission"]["sponsor"], json!({ "untrusted": true, "from": 0, "text": "Acme" }));
    assert_eq!(read["mission"]["text"]["text"], json!("The town needs bread."));
    assert_eq!(read["mission"].get("you"), None);
    assert_eq!(read["mission"].get("poster"), None);
}

#[test]
fn a_long_list_comes_in_pages() {
    let mut w = world();
    w.set_param("missions.posted_max", 10);
    w.set_param("missions.list_max", 3);
    let ann = rich(&mut w, "Ann", 1, 100_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let all: Vec<u64> = (0..7).map(|i| post(&mut w, ann, bread_mission(200 + i, 1))).collect();

    // Three missions for each page: the limit of the parameter. `next` is
    // the cursor for the next page. The last page has none.
    let first = ok(&mut w, bob, "mission_list", json!({}));
    assert_eq!(first["next"], json!(all[2]));
    assert_eq!(first["missions"].as_array().map(Vec::len), Some(3));
    assert_eq!(ids(&mut w, bob, json!({ "after": all[2] })), all[3..6]);
    let last = ok(&mut w, bob, "mission_list", json!({ "after": all[5] }));
    assert_eq!(last["missions"].as_array().map(Vec::len), Some(1));
    assert_eq!(last.get("next"), None);
    assert_eq!(ids(&mut w, bob, json!({ "after": all[6] })), [] as [u64; 0]);

    // A smaller limit works. A larger one is cut to the parameter.
    let two = ok(&mut w, bob, "mission_list", json!({ "limit": 2, "min_reward": 203 }));
    assert_eq!(two["next"], json!(all[4]));
    assert_eq!(ids(&mut w, bob, json!({ "limit": 2, "min_reward": 203 })), all[3..5]);
    assert_eq!(ids(&mut w, bob, json!({ "limit": 50 })), all[..3]);
    // Exactly one page: no cursor.
    let exact = ok(&mut w, bob, "mission_list", json!({ "min_reward": 204 }));
    assert_eq!(exact["missions"].as_array().map(Vec::len), Some(3));
    assert_eq!(exact.get("next"), None);
}
