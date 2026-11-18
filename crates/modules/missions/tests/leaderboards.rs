//! The leaderboards of a small known world: exact scores, top lists and
//! sums by owner, by organization and by declared model.

mod common;

use common::{
    FAR, bake, bread_mission, events, give_deposit, give_site, join, ok, post, site, spawn_as,
    sponsor, world,
};
use serde_json::{Value, json};
use unsup_kernel::protocol::FrameOutput;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Money, ResidentId};
use unsup_missions::{Board, Boards};

/// The two organizations of the town.
const GUILD: u64 = 500;
const UNION: u64 = 600;

/// The residents of the town.
struct Town {
    ann: ResidentId,
    bob: ResidentId,
    cid: ResidentId,
    dan: ResidentId,
}

/// Makes the town and everything that the boards count.
///
/// | Resident | Owner | Model | Organizations | Lives in |
/// | --- | --- | --- | --- | --- |
/// | Ann | 1 | alpha | guild | zone 0 |
/// | Bob | 1 | beta | guild, union | zone 0 |
/// | Cid | 2 | alpha | union | zone 0 |
/// | Dan | 3 | none | none | zone 3 |
fn town(w: &mut TestWorld<'_>) -> Town {
    let ann = spawn_as(w, "Ann", 1, Some("alpha"));
    let bob = spawn_as(w, "Bob", 1, Some("beta"));
    let cid = spawn_as(w, "Cid", 2, Some("alpha"));
    let dan = spawn_as(w, "Dan", 3, None);
    w.set_pos(dan, FAR);
    join(w, ann, GUILD);
    join(w, bob, GUILD);
    join(w, bob, UNION);
    join(w, cid, UNION);

    // Wealth. Ann: cash, a bank deposit and the home. Cid: cash, and the
    // farm, which is in another zone than Cid.
    w.give_money(ann, Money(5_000));
    give_deposit(w, ann, 2_000);
    give_site(w, 0, site(w, 0, "home"), ann);
    w.give_money(bob, Money(1_000));
    w.give_money(cid, Money(300));
    give_site(w, 3, site(w, 3, "farm"), cid);

    // Reputation.
    w.set_attr(ann, "rep.standing", 40);
    w.set_attr(bob, "rep.standing", -10);
    w.set_attr(cid, "rep.standing", 5);
    w.set_attr(bob, "rep.notoriety", 70);

    // Arrests: two by Bob, one by Cid.
    w.act_ok(bob, "arrest_test", json!({ "target": dan }));
    w.act_ok(bob, "arrest_test", json!({ "target": ann }));
    w.act_ok(cid, "arrest_test", json!({ "target": dan }));

    // A sponsor mission with two places of 200 cents: Bob and Cid complete
    // it, for 10 points each.
    let mut definition = bread_mission(200, 1);
    definition["takers"] = json!(2);
    sponsor(w, 1, "Acme", definition);
    let frames = w.step(1);
    let sponsored =
        common::one_event(&frames, "missions.mission_posted")["mission"].as_u64().expect("the id");
    // A mission of Ann with two places of 100 cents: Bob and Cid complete
    // it. Cid gets 3 points. Bob gets none: he has the owner of Ann.
    let shift = json!({
        "title": "Work a shift",
        "objectives": [{ "event": "economy.shift_completed" }],
        "reward": 100,
        "deadline_in": 600,
        "takers": 2,
    });
    let own = post(w, ann, shift);
    for taker in [bob, cid] {
        ok(w, taker, "mission_accept", json!({ "mission": sponsored }));
        ok(w, taker, "mission_accept", json!({ "mission": own }));
        bake(w, taker);
        w.act_ok(taker, "shift_test", Value::Null);
    }
    w.step(2);
    assert_eq!((w.cash(ann), w.cash(bob), w.cash(cid)), (Money(4_800), Money(1_300), Money(600)));
    Town { ann, bob, cid, dan }
}

/// Runs the world to a frame and returns the frames.
fn run_to(w: &mut TestWorld<'_>, frame: u64) -> Vec<FrameOutput> {
    let now = w.world.frame();
    assert!(now <= frame, "the world is at frame {now}, past frame {frame}");
    w.step((frame - now) as u32)
}

/// Returns the boards that one frame published, by key.
fn published(frames: &[FrameOutput], frame: u64) -> Vec<Value> {
    let found: Vec<Value> = events(frames, "missions.leaderboard_published")
        .into_iter()
        .filter(|event| event.frame == frame)
        .map(|event| {
            assert!(event.public && event.drama == 0);
            event.data.clone()
        })
        .collect();
    let keys: Vec<&str> = found.iter().map(|data| data["key"].as_str().expect("a key")).collect();
    assert_eq!(keys, ["wealth", "influence", "notoriety", "law", "missions", "longevity"]);
    found
}

fn entry(
    rank: u32,
    resident: ResidentId,
    name: &str,
    owner: u64,
    model: Value,
    score: i64,
) -> Value {
    json!({ "rank": rank, "resident": resident, "name": name, "owner": owner, "model": model, "score": score })
}

fn group(rank: u32, key: &str, id: Value, score: i64, residents: u64, mean: i64) -> Value {
    json!({ "rank": rank, key: id, "score": score, "residents": residents, "mean": mean })
}

#[test]
fn the_boards_of_a_small_town_are_exact() {
    let mut w = world();
    let Town { ann, bob, cid, dan } = town(&mut w);

    // Nothing is published before frame 60: the zones report each 60
    // frames, and the global realm publishes in the same frame.
    let frames = run_to(&mut w, 59);
    assert!(events(&frames, "missions.leaderboard_published").is_empty());
    let frames = w.step(1);
    let boards = published(&frames, 60);

    // Wealth: cash, deposit and sites.
    // Ann 4,800 + 2,000 + 30,000. Cid 600 + 20,000. Bob 1,300. Dan 0.
    assert_eq!(
        boards[0],
        json!({
            "key": "wealth",
            "season": 1,
            "unit": "cents",
            "residents": 4,
            "model_source": "self_reported",
            "top": [
                entry(1, ann, "Ann", 1, json!("alpha"), 36_800),
                entry(2, cid, "Cid", 2, json!("alpha"), 20_600),
                entry(3, bob, "Bob", 1, json!("beta"), 1_300),
                entry(4, dan, "Dan", 3, Value::Null, 0),
            ],
            "by_owner": [
                group(1, "owner", json!(1), 38_100, 2, 19_050),
                group(2, "owner", json!(2), 20_600, 1, 20_600),
                group(3, "owner", json!(3), 0, 1, 0),
            ],
            "by_org": [
                group(1, "org", json!(GUILD), 38_100, 2, 19_050),
                group(2, "org", json!(UNION), 21_900, 2, 10_950),
            ],
            "by_model": [
                group(1, "model", json!("alpha"), 57_400, 2, 28_700),
                group(2, "model", json!("beta"), 1_300, 1, 1_300),
                group(3, "model", Value::Null, 0, 1, 0),
            ],
        })
    );

    // Influence: `rep.standing`. A negative sum has a mean that is rounded
    // down: -5 / 2 gives -3.
    assert_eq!(
        boards[1],
        json!({
            "key": "influence",
            "season": 1,
            "unit": "standing",
            "residents": 4,
            "model_source": "self_reported",
            "top": [
                entry(1, ann, "Ann", 1, json!("alpha"), 40),
                entry(2, cid, "Cid", 2, json!("alpha"), 5),
                entry(3, dan, "Dan", 3, Value::Null, 0),
                entry(4, bob, "Bob", 1, json!("beta"), -10),
            ],
            "by_owner": [
                group(1, "owner", json!(1), 30, 2, 15),
                group(2, "owner", json!(2), 5, 1, 5),
                group(3, "owner", json!(3), 0, 1, 0),
            ],
            "by_org": [
                group(1, "org", json!(GUILD), 30, 2, 15),
                group(2, "org", json!(UNION), -5, 2, -3),
            ],
            "by_model": [
                group(1, "model", json!("alpha"), 45, 2, 22),
                group(2, "model", Value::Null, 0, 1, 0),
                group(3, "model", json!("beta"), -10, 1, -10),
            ],
        })
    );

    // Notoriety: `rep.notoriety`. Among equals the lower id comes first.
    assert_eq!((&boards[2]["unit"], &boards[2]["residents"]), (&json!("notoriety"), &json!(4)));
    assert_eq!(
        boards[2]["top"],
        json!([
            entry(1, bob, "Bob", 1, json!("beta"), 70),
            entry(2, ann, "Ann", 1, json!("alpha"), 0),
            entry(3, cid, "Cid", 2, json!("alpha"), 0),
            entry(4, dan, "Dan", 3, Value::Null, 0),
        ])
    );
    assert_eq!(
        boards[2]["by_model"],
        json!([
            group(1, "model", json!("beta"), 70, 1, 70),
            group(2, "model", Value::Null, 0, 1, 0),
            group(3, "model", json!("alpha"), 0, 2, 0),
        ])
    );

    // Law: arrests.
    assert_eq!(boards[3]["unit"], json!("arrests"));
    assert_eq!(
        boards[3]["top"],
        json!([
            entry(1, bob, "Bob", 1, json!("beta"), 2),
            entry(2, cid, "Cid", 2, json!("alpha"), 1),
            entry(3, ann, "Ann", 1, json!("alpha"), 0),
            entry(4, dan, "Dan", 3, Value::Null, 0),
        ])
    );
    assert_eq!(
        boards[3]["by_org"],
        json!([group(1, "org", json!(UNION), 3, 2, 1), group(2, "org", json!(GUILD), 2, 2, 1)])
    );

    // Missions: points. Cid 10 + 3. Bob 10 + 0.
    assert_eq!(boards[4]["unit"], json!("points"));
    assert_eq!(
        boards[4]["top"],
        json!([
            entry(1, cid, "Cid", 2, json!("alpha"), 13),
            entry(2, bob, "Bob", 1, json!("beta"), 10),
            entry(3, ann, "Ann", 1, json!("alpha"), 0),
            entry(4, dan, "Dan", 3, Value::Null, 0),
        ])
    );
    assert_eq!(
        boards[4]["by_owner"],
        json!([
            group(1, "owner", json!(2), 13, 1, 13),
            group(2, "owner", json!(1), 10, 2, 5),
            group(3, "owner", json!(3), 0, 1, 0),
        ])
    );
    let state = w.global().res::<Boards>();
    assert_eq!((state.points(cid), state.points(bob), state.arrests(bob)), (13, 10, 2));
    assert_eq!(state.residents(), 4);
    drop(state);

    // Longevity: nobody has lived a game day yet.
    assert_eq!(boards[5]["unit"], json!("days"));
    assert_eq!(boards[5]["top"][0], entry(1, ann, "Ann", 1, json!("alpha"), 0));

    // After two game days, each resident has 2 days.
    let frames = run_to(&mut w, 2_880);
    let boards = published(&frames, 2_880);
    assert_eq!(
        boards[5]["top"],
        json!([
            entry(1, ann, "Ann", 1, json!("alpha"), 2),
            entry(2, bob, "Bob", 1, json!("beta"), 2),
            entry(3, cid, "Cid", 2, json!("alpha"), 2),
            entry(4, dan, "Dan", 3, Value::Null, 2),
        ])
    );
    assert_eq!(boards[5]["by_model"][0], group(1, "model", json!("alpha"), 4, 2, 2));
    w.assert_conserved();
}

#[test]
fn a_board_is_as_old_as_the_last_report_of_each_zone() {
    let mut w = world();
    let Town { ann, bob, .. } = town(&mut w);
    run_to(&mut w, 60);

    // Bob gets rich between two reports. The board still shows the cash of
    // the last report.
    w.give_money(bob, Money(1_000_000));
    let read = ok(&mut w, bob, "leaderboard", json!({ "board": "wealth" }));
    assert_eq!(read["you"], json!({ "rank": 3, "score": 1_300 }));
    // A bank deposit is in the global realm, so the board sees it at once.
    give_deposit(&w, bob, 40_000);
    let read = ok(&mut w, bob, "leaderboard", json!({ "board": "wealth" }));
    assert_eq!(read["you"], json!({ "rank": 1, "score": 41_300 }));

    // The next report is at frame 120.
    let frames = run_to(&mut w, 120);
    let boards = published(&frames, 120);
    assert_eq!(boards[0]["top"][0], entry(1, bob, "Bob", 1, json!("beta"), 1_041_300));
    assert_eq!(boards[0]["top"][1]["resident"], json!(ann));
    w.assert_conserved();
}

#[test]
fn a_resident_reads_a_board_and_its_own_place() {
    let mut w = world();
    let Town { ann, bob, cid, dan } = town(&mut w);
    // Before the first report the boards rank nobody.
    let empty = ok(&mut w, bob, "leaderboard", json!({ "board": "wealth" }));
    assert_eq!(
        empty,
        json!({
            "board": "wealth",
            "season": 1,
            "season_ends": 1_680 * 1_440,
            "unit": "cents",
            "residents": 0,
            "top": [],
            "model_source": "self_reported",
        })
    );
    run_to(&mut w, 60);

    // Short lists: two residents and one group. The place of the caller is
    // there also when the caller is not in the list.
    w.set_param("missions.board_top", 2);
    w.set_param("missions.board_groups", 1);
    let read = ok(&mut w, bob, "leaderboard", json!({ "board": "wealth" }));
    assert_eq!(
        read,
        json!({
            "board": "wealth",
            "season": 1,
            "season_ends": 1_680 * 1_440,
            "unit": "cents",
            "residents": 4,
            "top": [
                entry(1, ann, "Ann", 1, json!("alpha"), 36_800),
                entry(2, cid, "Cid", 2, json!("alpha"), 20_600),
            ],
            "model_source": "self_reported",
            "you": { "rank": 3, "score": 1_300 },
        })
    );
    let read = ok(&mut w, dan, "leaderboard", json!({ "board": "influence", "by": "owner" }));
    assert_eq!(read["by_owner"], json!([group(1, "owner", json!(1), 30, 2, 15)]));
    assert_eq!(read["you"], json!({ "rank": 3, "score": 0 }));
    assert_eq!(read.get("top"), None);
    assert_eq!(read.get("model_source"), None);
    let read = ok(&mut w, cid, "leaderboard", json!({ "board": "missions", "by": "org" }));
    assert_eq!(read["by_org"], json!([group(1, "org", json!(UNION), 23, 2, 11)]));
    assert_eq!(read["you"], json!({ "rank": 1, "score": 13 }));
    let read = ok(&mut w, ann, "leaderboard", json!({ "board": "law", "by": "model" }));
    assert_eq!(read["by_model"], json!([group(1, "model", json!("beta"), 2, 1, 2)]));
    assert_eq!(read["model_source"], json!("self_reported"));
    assert_eq!(read["you"], json!({ "rank": 3, "score": 0 }));

    // The published boards use the same limits.
    let frames = run_to(&mut w, 120);
    let boards = published(&frames, 120);
    assert_eq!(boards[0]["top"].as_array().map(Vec::len), Some(2));
    assert_eq!(boards[0]["by_model"].as_array().map(Vec::len), Some(1));
    assert_eq!(boards[0]["residents"], json!(4));
}

#[test]
fn a_resident_that_left_the_world_keeps_what_it_did() {
    let mut w = world();
    let Town { ann, bob, cid, dan } = town(&mut w);
    run_to(&mut w, 2_880);

    // Bob dies after two days. His wealth and his reputation are gone. His
    // points, his arrests and his age stay until the season ends.
    w.act_ok(bob, "die_test", json!({ "heir": ann }));
    let frames = run_to(&mut w, 4_320);
    let boards = published(&frames, 4_320);
    let residents: Vec<u64> =
        boards.iter().map(|b| b["residents"].as_u64().expect("a count")).collect();
    assert_eq!(residents, [3, 3, 3, 4, 4, 4]);
    // Ann inherited the cash of Bob.
    assert_eq!(boards[0]["top"][0], entry(1, ann, "Ann", 1, json!("alpha"), 38_100));
    assert_eq!(boards[2]["top"][0], entry(1, ann, "Ann", 1, json!("alpha"), 0));
    assert_eq!(boards[3]["top"][0], entry(1, bob, "Bob", 1, json!("beta"), 2));
    assert_eq!(boards[4]["top"][1], entry(2, bob, "Bob", 1, json!("beta"), 10));
    assert_eq!(
        boards[5]["top"],
        json!([
            entry(1, ann, "Ann", 1, json!("alpha"), 3),
            entry(2, cid, "Cid", 2, json!("alpha"), 3),
            entry(3, dan, "Dan", 3, Value::Null, 3),
            entry(4, bob, "Bob", 1, json!("beta"), 2),
        ])
    );
    assert_eq!(
        Board::ALL.map(Board::name),
        ["wealth", "influence", "notoriety", "law", "missions", "longevity"]
    );
    w.assert_conserved();
}
