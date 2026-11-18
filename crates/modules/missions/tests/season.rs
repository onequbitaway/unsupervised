//! The end of a season: final standings, the hall of fame, the soft reset
//! with its amounts and its sink, and the next season.

mod common;

use common::{
    FAR, bake, bread_mission, burned, count_notices, events, fail, notice, ok, one_event, spawn_as,
    sponsor, world,
};
use serde_json::{Value, json};
use unsup_kernel::protocol::FrameOutput;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Input, Money, Pos, ResidentId, ZoneId};
use unsup_missions::{Board, Boards, SEASON_SINK, SPONSOR_FAUCET, Seasons};

/// The residents of the test.
struct Town {
    ann: ResidentId,
    bob: ResidentId,
    cid: ResidentId,
    dan: ResidentId,
}

/// Makes a town whose seasons last one game day.
///
/// | Resident | Zone | Cash | Standing | Notoriety |
/// | --- | --- | --- | --- | --- |
/// | Ann | 0 | 300,000 | 40 | 0 |
/// | Bob | 0 | 50,000 | -45 | 81 |
/// | Cid | 0 | 100,001 | 0 | 0 |
/// | Dan | 3 | 1,000,000 | 0 | 7 |
///
/// Bob made one arrest. Cid has 10 mission points.
fn town(w: &mut TestWorld<'_>) -> Town {
    w.set_param("missions.season_days", 1);
    let ann = spawn_as(w, "Ann", 1, Some("alpha"));
    let bob = spawn_as(w, "Bob", 2, Some("beta"));
    let cid = spawn_as(w, "Cid", 3, None);
    let dan = spawn_as(w, "Dan", 4, Some("alpha"));
    w.set_pos(dan, FAR);
    w.give_money(ann, Money(300_000));
    w.give_money(bob, Money(50_000));
    w.give_money(cid, Money(99_501));
    w.give_money(dan, Money(1_000_000));
    w.set_attr(ann, "rep.standing", 40);
    w.set_attr(bob, "rep.standing", -45);
    w.set_attr(bob, "rep.notoriety", 81);
    w.set_attr(dan, "rep.notoriety", 7);
    w.act_ok(bob, "arrest_test", json!({ "target": dan }));
    // Cid earns the 500 cents of a sponsor.
    sponsor(w, 1, "Acme", bread_mission(500, 1));
    let frames = w.step(1);
    let mission = one_event(&frames, "missions.mission_posted")["mission"].clone();
    ok(w, cid, "mission_accept", json!({ "mission": mission }));
    bake(w, cid);
    w.step(2);
    assert_eq!(w.cash(cid), Money(100_001));
    Town { ann, bob, cid, dan }
}

fn run_to(w: &mut TestWorld<'_>, frame: u64) -> Vec<FrameOutput> {
    let now = w.world.frame();
    assert!(now <= frame, "the world is at frame {now}, past frame {frame}");
    w.step((frame - now) as u32)
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

/// Returns what each zone says about the reset that it applied.
fn applied(frames: &[FrameOutput]) -> Vec<Value> {
    let mut list: Vec<Value> = events(frames, "missions.season_reset_applied")
        .into_iter()
        .map(|event| event.data.clone())
        .collect();
    list.sort_by_key(|data| data["zone"].as_u64());
    list
}

#[test]
fn the_first_season_starts_with_the_world() {
    let mut w = world();
    w.set_param("missions.season_first", 5);
    w.set_param("missions.season_days", 3);
    let frames = w.step(1);
    let started = events(&frames, "missions.season_started");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].data, json!({ "season": 5, "started": 0, "ends": 3 * 1_440 }));
    assert_eq!(started[0].drama, 60);
    let seasons = w.global().res::<Seasons>();
    assert_eq!((seasons.number(), seasons.started(), seasons.hall_of_fame().len()), (5, 0, 0));
    drop(seasons);
    // No second announcement, and no end before the length is over.
    let frames = run_to(&mut w, 3 * 1_440 - 1);
    assert!(events(&frames, "missions.season_started").is_empty());
    assert!(events(&frames, "missions.season_ended").is_empty());
    let frames = w.step(1);
    assert_eq!(one_event(&frames, "missions.season_ended")["season"], json!(5));
    assert_eq!(
        one_event(&frames, "missions.season_started"),
        json!({ "season": 6, "started": 3 * 1_440, "ends": 6 * 1_440 })
    );
}

#[test]
fn the_season_has_its_number_before_the_first_tick() {
    let mut w = world();
    w.set_param("missions.season_first", 5);
    let ann = spawn_as(&mut w, "Ann", 1, None);
    // The global realm reads the board in frame 1, before its systems ran
    // for the first time.
    let read = ok(&mut w, ann, "leaderboard", json!({ "board": "law", "season": 5 }));
    assert_eq!(
        read,
        json!({
            "board": "law",
            "season": 5,
            "season_ends": 1_680 * 1_440,
            "unit": "arrests",
            "residents": 0,
            "top": [],
            "model_source": "self_reported",
        })
    );
}

#[test]
fn a_season_ends_with_standings_a_reset_and_a_new_season() {
    let mut w = world();
    let Town { ann, bob, cid, dan } = town(&mut w);
    let frames = run_to(&mut w, 1_439);
    assert!(events(&frames, "missions.season_ended").is_empty());

    // Frame 1,440: the season ends. The zones reported in this frame, so
    // the final standings are exact.
    let frames = w.step(1);
    let ended = events(&frames, "missions.season_ended");
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].drama, 90, "a headline");
    let data = ended[0].data.clone();
    assert_eq!(
        (&data["season"], &data["started"], &data["ended"], &data["next"]),
        (&json!(1), &json!(0), &json!(1_440), &json!(2))
    );
    assert_eq!(
        data["reset"],
        json!({
            "standing_keep_bp": 5_000,
            "notoriety_keep_bp": 5_000,
            "wealth_floor": 100_000,
            "wealth_cut_bp": 5_000,
        })
    );
    let boards = data["boards"].as_object().expect("the boards by name");
    assert_eq!(
        boards.keys().map(String::as_str).collect::<Vec<_>>(),
        ["influence", "law", "longevity", "missions", "notoriety", "wealth"]
    );
    assert_eq!(
        boards["wealth"],
        json!({
            "unit": "cents",
            "residents": 4,
            "model_source": "self_reported",
            "top": [
                entry(1, dan, "Dan", 4, json!("alpha"), 1_000_000),
                entry(2, ann, "Ann", 1, json!("alpha"), 300_000),
                entry(3, cid, "Cid", 3, Value::Null, 100_001),
                entry(4, bob, "Bob", 2, json!("beta"), 50_000),
            ],
            "by_owner": [
                { "rank": 1, "owner": 4, "score": 1_000_000, "residents": 1, "mean": 1_000_000 },
                { "rank": 2, "owner": 1, "score": 300_000, "residents": 1, "mean": 300_000 },
                { "rank": 3, "owner": 3, "score": 100_001, "residents": 1, "mean": 100_001 },
                { "rank": 4, "owner": 2, "score": 50_000, "residents": 1, "mean": 50_000 },
            ],
            "by_org": [],
            "by_model": [
                { "rank": 1, "model": "alpha", "score": 1_300_000, "residents": 2, "mean": 650_000 },
                { "rank": 2, "model": null, "score": 100_001, "residents": 1, "mean": 100_001 },
                { "rank": 3, "model": "beta", "score": 50_000, "residents": 1, "mean": 50_000 },
            ],
        })
    );
    assert_eq!(boards["influence"]["top"][0], entry(1, ann, "Ann", 1, json!("alpha"), 40));
    assert_eq!(boards["influence"]["top"][3], entry(4, bob, "Bob", 2, json!("beta"), -45));
    assert_eq!(boards["notoriety"]["top"][0], entry(1, bob, "Bob", 2, json!("beta"), 81));
    assert_eq!(boards["law"]["top"][0], entry(1, bob, "Bob", 2, json!("beta"), 1));
    assert_eq!(boards["missions"]["top"][0], entry(1, cid, "Cid", 3, Value::Null, 10));
    assert_eq!(boards["longevity"]["top"][0], entry(1, ann, "Ann", 1, json!("alpha"), 1));

    // The same data is published state with the season as the key.
    let mut hall = one_event(&frames, "missions.hall_of_fame_published");
    assert_eq!(hall["key"], json!(1));
    hall.as_object_mut().expect("an object").remove("key");
    assert_eq!(hall, data);
    // The next season starts in the same frame.
    assert_eq!(
        one_event(&frames, "missions.season_started"),
        json!({ "season": 2, "started": 1_440, "ends": 2_880 })
    );
    // The reset has not reached the zones yet.
    assert_eq!((w.cash(ann), burned(&w, SEASON_SINK)), (Money(300_000), 0));
    assert!(applied(&frames).is_empty());

    // Frame 1,441: each zone applies the reset to its residents.
    // Ann: (300,000 - 100,000) / 2 = 100,000. Bob is below the floor.
    // Cid: (100,001 - 100,000) / 2 = 0. Dan: (1,000,000 - 100,000) / 2.
    let frames = w.step(1);
    assert_eq!(
        applied(&frames),
        [
            json!({ "season": 1, "zone": 0, "residents": 3, "burned": 100_000 }),
            json!({ "season": 1, "zone": 1, "residents": 0, "burned": 0 }),
            json!({ "season": 1, "zone": 2, "residents": 0, "burned": 0 }),
            json!({ "season": 1, "zone": 3, "residents": 1, "burned": 450_000 }),
        ]
    );
    assert_eq!(
        (w.cash(ann), w.cash(bob), w.cash(cid), w.cash(dan)),
        (Money(200_000), Money(50_000), Money(100_001), Money(550_000))
    );
    assert_eq!(burned(&w, SEASON_SINK), 550_000);
    // Reputations decay toward zero: 40 to 20, -45 to -22, 81 to 40, 7 to 3.
    assert_eq!((w.attr(ann, "rep.standing"), w.attr(bob, "rep.standing")), (20, -22));
    assert_eq!((w.attr(bob, "rep.notoriety"), w.attr(dan, "rep.notoriety")), (40, 3));
    assert_eq!(
        notice(&w, ann, "missions.season_reset").data,
        json!({ "season": 1, "cash_cut": 100_000, "standing": 20, "notoriety": 0 })
    );
    assert_eq!(
        notice(&w, bob, "missions.season_reset").data,
        json!({ "season": 1, "cash_cut": 0, "standing": -22, "notoriety": 40 })
    );
    // Nothing changed for Cid, so Cid gets no notice.
    assert_eq!(count_notices(&w, cid, "missions.season_reset"), 0);

    // Money: 1,449,501 given and 500 from the sponsor; 550,000 destroyed.
    let totals = w.assert_conserved();
    assert_eq!(common::minted(&w, SPONSOR_FAUCET), 500);
    assert_eq!(
        (totals.money.minted, totals.money.burned, totals.money.held),
        (1_450_001, 550_000, 900_001)
    );

    // The boards of the new season start from zero points and arrests.
    let boards = w.global().res::<Boards>();
    assert_eq!((boards.points(cid), boards.arrests(bob)), (0, 0));
    drop(boards);
    let now = ok(&mut w, cid, "leaderboard", json!({ "board": "missions" }));
    assert_eq!((&now["season"], &now["season_ends"]), (&json!(2), &json!(2_880)));
    assert_eq!(now["you"], json!({ "rank": 3, "score": 0 }));

    // The hall of fame keeps the final standings of season 1.
    let seasons = w.global().res::<Seasons>();
    assert_eq!(seasons.number(), 2);
    let hall = seasons.hall_of_fame();
    assert_eq!(hall.len(), 1);
    assert_eq!((hall[0].season, hall[0].started, hall[0].ended), (1, 0, 1_440));
    let wealth = hall[0].standing(Board::Wealth).expect("the wealth board");
    assert_eq!(
        (wealth.top[0].resident, wealth.top[0].score, wealth.residents),
        (dan, 1_000_000, 4)
    );
    drop(seasons);
    let past = ok(&mut w, cid, "leaderboard", json!({ "board": "missions", "season": 1 }));
    assert_eq!(
        past,
        json!({
            "board": "missions",
            "season": 1,
            "final": true,
            "started": 0,
            "ended": 1_440,
            "unit": "points",
            "residents": 4,
            "top": [
                entry(1, cid, "Cid", 3, Value::Null, 10),
                entry(2, ann, "Ann", 1, json!("alpha"), 0),
                entry(3, bob, "Bob", 2, json!("beta"), 0),
                entry(4, dan, "Dan", 4, json!("alpha"), 0),
            ],
            "model_source": "self_reported",
            "you": { "rank": 1, "score": 10 },
        })
    );
    let past =
        ok(&mut w, bob, "leaderboard", json!({ "board": "wealth", "season": 1, "by": "model" }));
    assert_eq!(past["by_model"][0]["score"], json!(1_300_000));
    assert_eq!(past["you"], json!({ "rank": 4, "score": 50_000 }));
}

#[test]
fn each_season_resets_again_and_the_hall_of_fame_is_bounded() {
    let mut w = world();
    let Town { ann, bob, dan, .. } = town(&mut w);
    w.set_param("missions.hall_of_fame_max", 1);
    run_to(&mut w, 1_441);
    assert_eq!((w.cash(ann), w.cash(dan)), (Money(200_000), Money(550_000)));

    // Season 2 ends at frame 2,880. The reset takes half of what is above
    // the floor again: 50,000 from Ann and 225,000 from Dan.
    let frames = run_to(&mut w, 2_881);
    assert_eq!(one_event(&frames, "missions.season_ended")["season"], json!(2));
    assert_eq!((w.cash(ann), w.cash(dan)), (Money(150_000), Money(325_000)));
    assert_eq!(burned(&w, SEASON_SINK), 550_000 + 275_000);
    assert_eq!((w.attr(ann, "rep.standing"), w.attr(bob, "rep.standing")), (10, -11));
    assert_eq!((w.attr(bob, "rep.notoriety"), w.attr(dan, "rep.notoriety")), (20, 1));

    // The hall keeps one season: season 1 made room for season 2.
    let seasons = w.global().res::<Seasons>();
    assert_eq!(seasons.hall_of_fame().iter().map(|r| r.season).collect::<Vec<_>>(), [2]);
    drop(seasons);
    assert_eq!(
        fail(&mut w, ann, "leaderboard", json!({ "board": "wealth", "season": 1 })),
        ErrorCode::NotFound
    );
    w.assert_conserved();
}

#[test]
fn the_reset_numbers_come_from_parameters() {
    let mut w = world();
    let Town { ann, bob, dan, .. } = town(&mut w);
    w.set_param("missions.reset_wealth_floor", 250_000);
    w.set_param("missions.reset_wealth_cut_bp", 1_000);
    w.set_param("missions.reset_standing_keep_bp", 0);
    w.set_param("missions.reset_notoriety_keep_bp", 10_000);
    let frames = run_to(&mut w, 1_441);
    assert_eq!(
        one_event(&frames, "missions.season_ended")["reset"],
        json!({
            "standing_keep_bp": 0,
            "notoriety_keep_bp": 10_000,
            "wealth_floor": 250_000,
            "wealth_cut_bp": 1_000,
        })
    );
    // Ann: (300,000 - 250,000) / 10. Dan: (1,000,000 - 250,000) / 10.
    assert_eq!((w.cash(ann), w.cash(dan)), (Money(295_000), Money(925_000)));
    assert_eq!(burned(&w, SEASON_SINK), 80_000);
    assert_eq!((w.attr(ann, "rep.standing"), w.attr(bob, "rep.standing")), (0, 0));
    assert_eq!(w.attr(bob, "rep.notoriety"), 81);
    w.assert_conserved();
}

#[test]
fn a_resident_gets_the_reset_once_when_a_zone_is_late() {
    let mut w = world();
    let Town { ann, bob, cid, dan } = town(&mut w);
    run_to(&mut w, 1_440);

    // Zone 3 skips the next frame (time dilation). The other zones apply
    // the reset: Ann has 200,000 now. Dan, in zone 3, is not cut yet.
    let late = w.world.apply(Input::Step { skip: vec![ZoneId(3)] });
    drop(late);
    assert_eq!((w.cash(ann), w.cash(dan)), (Money(200_000), Money(1_000_000)));

    // Ann walks into the late zone and Dan walks out of it. Dan gets the
    // reset when he arrives in a zone that applied it.
    w.set_pos(ann, FAR);
    w.set_pos(dan, Pos::new(32, 32));
    assert_eq!(w.cash(dan), Money(550_000));
    assert_eq!(
        notice(&w, dan, "missions.season_reset").data,
        json!({ "season": 1, "cash_cut": 450_000, "standing": 0, "notoriety": 3 })
    );

    // The late zone applies the reset now. Ann got it already.
    let frames = w.step(1);
    assert_eq!(applied(&frames), [json!({ "season": 1, "zone": 3, "residents": 0, "burned": 0 })]);
    assert_eq!((w.cash(ann), w.cash(dan)), (Money(200_000), Money(550_000)));
    assert_eq!((w.attr(ann, "rep.standing"), w.attr(dan, "rep.notoriety")), (20, 3));
    assert_eq!(burned(&w, SEASON_SINK), 550_000);
    assert_eq!(count_notices(&w, ann, "missions.season_reset"), 1);
    assert_eq!(count_notices(&w, dan, "missions.season_reset"), 1);

    // Walks between zones after the reset change nothing.
    w.set_pos(bob, FAR);
    w.set_pos(cid, FAR);
    w.set_pos(ann, Pos::new(32, 32));
    w.step(2);
    assert_eq!(burned(&w, SEASON_SINK), 550_000);
    assert_eq!(w.attr(bob, "rep.standing"), -22);

    // A resident that is new after the reset has nothing to reset, also
    // when it is rich and changes zone.
    let eve = spawn_as(&mut w, "Eve", 5, None);
    w.give_money(eve, Money(700_000));
    w.set_attr(eve, "rep.standing", 90);
    w.set_pos(eve, FAR);
    w.step(2);
    assert_eq!((w.cash(eve), w.attr(eve, "rep.standing")), (Money(700_000), 90));
    assert_eq!(count_notices(&w, eve, "missions.season_reset"), 0);
    w.assert_conserved();
}
