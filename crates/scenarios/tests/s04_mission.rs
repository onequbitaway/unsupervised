//! Story 4: a mission.
//!
//! A resident posts a mission and locks the reward in escrow. Two
//! residents accept it. One of them does the work, and the engine pays it
//! in the frame of the last event: nobody has to ask. The other one does
//! nothing, and at the deadline its share of the reward goes back to the
//! poster.
//!
//! All the money of the story is wages of public works.

use serde_json::json;
use unsup_kernel::{CityId, ErrorCode, Holder};
use unsup_scenarios::{Game, Setup, assert_deterministic};

/// The net wage of one shift of public works.
const NET: i64 = 2_520;
const REWARD: i64 = 1_000;

fn story(g: &mut Game) {
    let ada = g.spawn("Ada");
    let bob = g.spawn("Bob");
    let cyd = g.spawn("Cyd");
    let hall = g.site("city_hall");

    // Ada earns the reward with one shift. She cannot lock more than she
    // has.
    g.walk(ada, &hall);
    g.act(ada, "work", json!({}));
    assert_eq!(g.cash(ada), NET);
    let mission = |reward: i64| {
        json!({
            "title": "Two shifts for the city",
            "text": "Work two shifts of public works.",
            "objectives": [
                { "event": "economy.shift_completed", "fields": { "skill": "labor" }, "count": 2 },
            ],
            "reward": reward,
            "deadline_in": 1_440,
            "takers": 2,
        })
    };
    assert_eq!(g.refused(ada, "mission_post", mission(NET)), ErrorCode::Insufficient);
    assert_eq!(g.cash(ada), NET, "a refused mission keeps no money");

    // The reward is for each taker: two takers lock 2,000.
    let posted = g.act(ada, "mission_post", mission(REWARD));
    let id = posted["mission"].as_u64().expect("a mission id");
    let deadline = posted["deadline"].as_u64().expect("a deadline");
    assert_eq!(posted, json!({ "mission": id, "locked": 2 * REWARD, "deadline": deadline }));
    let escrow = Holder::Escrow(id);
    assert_eq!((g.cash(ada), g.money(escrow)), (NET - 2 * REWARD, 2 * REWARD));
    assert_eq!(deadline, g.last_event("missions.mission_posted").frame + 1_440);

    // Bob sees the new mission in his observation and reads it.
    let new = g.section(bob, "missions").expect("a missions section");
    assert_eq!(new, json!({ "new": [{ "mission": id, "reward": REWARD, "deadline": deadline }] }));
    let read = g.act(bob, "mission_list", json!({ "mission": id }));
    assert_eq!(read["mission"]["poster"], json!(ada));
    assert_eq!(
        read["mission"]["title"],
        json!({ "untrusted": true, "from": ada, "text": "Two shifts for the city" })
    );
    // The poster cannot take its own mission.
    assert_eq!(g.refused(ada, "mission_accept", json!({ "mission": id })), ErrorCode::NotAllowed);
    // Bob and Cyd accept. A third taker finds no place.
    assert_eq!(g.act(bob, "mission_accept", json!({ "mission": id }))["status"], "accepted");
    assert_eq!(g.act(cyd, "mission_accept", json!({ "mission": id }))["status"], "accepted");
    let dan = g.spawn("Dan");
    assert_eq!(g.refused(dan, "mission_accept", json!({ "mission": id })), ErrorCode::Conflict);
    // While takers work on it, the poster cannot take the reward back.
    assert_eq!(g.refused(ada, "mission_claim", json!({ "mission": id })), ErrorCode::Conflict);
    assert_eq!(g.money(escrow), 2 * REWARD);

    // Bob works. The first shift counts one of two.
    g.walk(bob, &hall);
    g.act(bob, "work", json!({}));
    g.step(1);
    let progress = g.section(bob, "missions").expect("a missions section");
    assert_eq!(
        progress["accepted"],
        json!([{ "mission": id, "reward": REWARD, "deadline": deadline, "progress": [[1, 2]] }])
    );
    // A claim before the work is done moves nothing.
    assert_eq!(g.refused(bob, "mission_claim", json!({ "mission": id })), ErrorCode::Conflict);
    assert_eq!((g.cash(bob), g.money(escrow)), (NET, 2 * REWARD));

    // The second shift completes the mission. The engine pays in the frame
    // of the shift, and the money is with Bob one frame later.
    g.act(bob, "work", json!({}));
    let shift = g.last_event("economy.shift_completed").frame;
    let completed = g.last_event("missions.mission_completed");
    assert_eq!(completed.frame, shift);
    assert_eq!(
        completed.data,
        json!({ "mission": id, "resident": bob, "paid": REWARD, "shares": [{ "resident": bob, "amount": REWARD }] })
    );
    g.step(1);
    assert_eq!((g.cash(bob), g.money(escrow)), (2 * NET + REWARD, REWARD));
    assert_eq!(
        g.notice(bob, "missions.mission_completed").data,
        json!({ "mission": id, "paid": REWARD, "points": 3 })
    );
    assert_eq!(g.notice(ada, "missions.mission_paid").data["resident"], json!(bob));
    // The receipt, and the board of the season.
    let receipt = g.act(bob, "mission_claim", json!({ "mission": id }));
    assert_eq!(receipt, json!({ "mission": id, "status": "paid", "paid": REWARD, "frame": shift }));
    let board = g.act(bob, "leaderboard", json!({ "board": "missions" }));
    assert_eq!(board["you"], json!({ "rank": 1, "score": 3 }));
    assert_eq!(board["top"][0]["resident"], json!(bob));

    // Cyd does nothing. At the deadline the mission closes, and the part
    // of the reward that nobody earned goes back to the poster.
    g.run_to_frame(deadline - 1);
    assert_eq!(g.money(escrow), REWARD);
    assert!(g.events_of("missions.mission_closed").is_empty());
    g.step(1);
    let closed = g.last_event("missions.mission_closed");
    assert_eq!(closed.frame, deadline);
    assert_eq!(
        closed.data,
        json!({ "mission": id, "reason": "expired", "completed": 1, "refunded": REWARD, "destroyed": 0 })
    );
    g.step(1);
    assert_eq!((g.cash(ada), g.cash(cyd), g.money(escrow)), (NET - REWARD, 0, 0));
    assert_eq!(
        g.notice(cyd, "missions.mission_ended").data,
        json!({ "mission": id, "reason": "expired" })
    );
    assert_eq!(g.refused(cyd, "mission_claim", json!({ "mission": id })), ErrorCode::NotFound);

    // Only public works made money: three shifts.
    let totals = g.audit();
    assert_eq!((totals.money.minted, totals.money.burned), (3 * 2_800, 0));
    assert_eq!(totals.money.held, g.cash(ada) + g.cash(bob) + g.city_treasury(CityId(0)));
}

#[test]
fn a_mission_is_posted_accepted_completed_by_events_and_paid_by_the_engine() {
    assert_deterministic(&Setup::town(1), story);
}
