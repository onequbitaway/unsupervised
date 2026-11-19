//! Missions of human sponsors: the typed message, the faucet with its
//! daily limit, and the sink for a reward that nobody earned.

mod common;

use common::{
    bake, bread_mission, burned, escrow, events, minted, notice, ok, one_event, rich, sponsor,
    world,
};
use serde_json::{Value, json};
use unsup_kernel::protocol::FrameOutput;
use unsup_kernel::{GRANT_FAUCET, Holder, Message, Money, SUPPLY_CAP, Scope};
use unsup_missions::{Missions, SPONSOR_FAUCET, SPONSOR_SINK, SponsorMission};

/// Returns the id of the mission that the answer to a request names.
fn posted(frames: &[FrameOutput], request: u64) -> u64 {
    let data = one_event(frames, "missions.mission_posted");
    assert_eq!(data["request"], json!(request));
    data["mission"].as_u64().expect("the id")
}

#[test]
fn the_message_has_a_kind_of_its_own_and_only_the_global_realm_takes_it() {
    let w = world();
    assert_eq!(SponsorMission::KIND, "missions.sponsor_mission");
    let blueprint = w.world.blueprint();
    assert!(blueprint.handles_message(SponsorMission::KIND, Scope::Global));
    assert!(!blueprint.handles_message(SponsorMission::KIND, Scope::Zone));
}

#[test]
fn a_sponsor_mission_is_paid_with_new_money() {
    let mut w = world();
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);

    // Two places of 900 cents. Nobody pays for them: the faucet
    // `sponsor.reward` creates 1,800 cents in the escrow of the mission.
    let mut definition = bread_mission(900, 1);
    definition["takers"] = json!(2);
    definition["deadline_in"] = json!(30);
    sponsor(&mut w, 7, "Acme Mills", definition);
    assert_eq!(minted(&w, SPONSOR_FAUCET), 0, "nothing happens before the next frame");
    let frames = w.step(1);
    let data = one_event(&frames, "missions.mission_posted");
    let mission = data["mission"].as_u64().expect("the id");
    assert_eq!(
        data,
        json!({
            "mission": mission,
            // No resident wrote the text, so `from` is 0.
            "title": { "untrusted": true, "from": 0, "text": "Bake bread" },
            "reward": 900,
            "takers": 2,
            "mode": "solo",
            "deadline": 31,
            "objectives": 1,
            "sponsor": { "untrusted": true, "from": 0, "text": "Acme Mills" },
            "request": 7,
        })
    );
    assert_eq!(events(&frames, "missions.mission_posted")[0].actor, None);
    assert_eq!((minted(&w, SPONSOR_FAUCET), escrow(&w, mission)), (1_800, 1_800));
    assert_eq!(w.global().res::<Missions>().sponsor_minted_on(0), 1_800);
    w.assert_conserved();

    // A resident reads it like every other mission, and earns the reward.
    let listed = ok(&mut w, bob, "mission_list", json!({ "sponsored": true }));
    assert_eq!(listed["missions"][0]["mission"], json!(mission));
    assert_eq!(listed["missions"][0]["sponsored"], json!(true));
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    bake(&mut w, bob);
    w.step(2);
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(900), 900));
    // A mission of a sponsor gives 10 points.
    assert_eq!(
        notice(&w, bob, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 900, "points": 10 })
    );

    // Cid takes the other place and does not finish. At the deadline the
    // 900 cents that are left are destroyed through `sponsor.expired`.
    ok(&mut w, cid, "mission_accept", json!({ "mission": mission }));
    let frames = w.step((31 - w.world.frame()) as u32);
    assert_eq!(
        one_event(&frames, "missions.mission_closed"),
        json!({ "mission": mission, "reason": "expired", "completed": 1, "refunded": 0, "destroyed": 900 })
    );
    assert_eq!((burned(&w, SPONSOR_SINK), escrow(&w, mission)), (900, 0));
    let totals = w.assert_conserved();
    assert_eq!(
        (totals.money.minted, totals.money.burned, totals.money.held),
        (1_800, 900, 900),
        "the only money of the world is the reward of Bob"
    );
    assert_eq!(w.cash(bob), Money(900));
}

#[test]
fn the_sponsor_faucet_has_a_limit_for_each_game_day() {
    let mut w = world();
    w.set_param("missions.sponsor_daily_limit", 3_000);
    let definition = |reward: i64, takers: u32| {
        let mut definition = bread_mission(reward, 1);
        definition["takers"] = json!(takers);
        definition
    };

    // 2,000 of the 3,000 cents of the day.
    sponsor(&mut w, 1, "Acme", definition(1_000, 2));
    let first = posted(&w.step(1), 1);
    assert_eq!(minted(&w, SPONSOR_FAUCET), 2_000);

    // 1,500 more do not fit. The refusal names the request and changes
    // nothing.
    sponsor(&mut w, 2, "Borg", definition(500, 3));
    let frames = w.step(1);
    assert_eq!(
        one_event(&frames, "missions.sponsor_refused"),
        json!({
            "request": 2,
            "code": "rate_limited",
            "message": "The mission needs 1500 cents. The sponsor faucet has 1000 of 3000 cents left today.",
        })
    );
    assert!(events(&frames, "missions.mission_posted").is_empty());
    assert_eq!(minted(&w, SPONSOR_FAUCET), 2_000);
    assert_eq!(w.global().res::<Missions>().open_count(), 1);

    // Exactly the rest of the day fits.
    sponsor(&mut w, 3, "Cato", definition(1_000, 1));
    let third = posted(&w.step(1), 3);
    assert_eq!(minted(&w, SPONSOR_FAUCET), 3_000);
    sponsor(&mut w, 4, "Dora", definition(100, 1));
    assert_eq!(one_event(&w.step(1), "missions.sponsor_refused")["request"], json!(4));

    // The next game day starts at frame 1,440 with a new budget.
    w.step((1_439 - w.world.frame()) as u32);
    sponsor(&mut w, 5, "Dora", definition(100, 1));
    let frames = w.step(1);
    assert_eq!(w.world.frame(), 1_440);
    let fifth = posted(&frames, 5);
    assert_eq!(minted(&w, SPONSOR_FAUCET), 3_100);
    assert_eq!(w.global().res::<Missions>().sponsor_minted_on(1), 100);
    assert_eq!(
        (escrow(&w, first), escrow(&w, third), escrow(&w, fifth)),
        (0, 0, 100),
        "the missions of the first day ended at frame 601 and their rewards were destroyed"
    );
    assert_eq!(burned(&w, SPONSOR_SINK), 3_000);
    w.assert_conserved();
}

#[test]
fn a_bad_sponsor_mission_is_refused_with_an_event() {
    let mut w = world();
    let cases: Vec<(&str, Value, &str, &str)> = vec![
        ("", bread_mission(500, 1), "invalid_args", "The name of the sponsor is empty."),
        (
            "Two\nlines",
            bread_mission(500, 1),
            "invalid_args",
            "The name of the sponsor has a line break.",
        ),
        (
            "Acme",
            bread_mission(50, 1),
            "invalid_args",
            "`reward` must be from 100 to 10000000 cents.",
        ),
        (
            "Acme",
            bread_mission(10_000_000, 1),
            "rate_limited",
            "The mission needs 10000000 cents. The sponsor faucet has 5000000 of 5000000 cents left today.",
        ),
    ];
    for (request, (name, definition, code, message)) in cases.into_iter().enumerate() {
        sponsor(&mut w, request as u64, name, definition);
        let frames = w.step(1);
        assert_eq!(
            one_event(&frames, "missions.sponsor_refused"),
            json!({ "request": request, "code": code, "message": message })
        );
    }
    assert_eq!(minted(&w, SPONSOR_FAUCET), 0);
    assert_eq!(w.global().res::<Missions>().open_count(), 0);
    assert_eq!(w.world.dropped_messages(), 0, "the handler took every message");
    w.assert_conserved();
}

#[test]
fn the_world_limit_of_open_missions_holds_for_sponsors_too() {
    let mut w = world();
    w.set_param("missions.open_max", 1);
    sponsor(&mut w, 1, "Acme", bread_mission(500, 1));
    posted(&w.step(1), 1);
    sponsor(&mut w, 2, "Acme", bread_mission(500, 1));
    assert_eq!(
        one_event(&w.step(1), "missions.sponsor_refused"),
        json!({
            "request": 2,
            "code": "conflict",
            "message": "The world has 1 open missions already. Try again later.",
        })
    );
    assert_eq!(minted(&w, SPONSOR_FAUCET), 500);
    w.assert_conserved();
}

#[test]
fn a_sponsor_mission_is_refused_when_the_money_cannot_be_created() {
    let mut w = world();
    // The global realm has created all of the money that it can create but
    // 1,000 cents. The ledger refuses to create more than that.
    let grant = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet exists");
    w.global()
        .ledger_mut()
        .try_mint(grant, Holder::Account(1), Money(SUPPLY_CAP - 1_000))
        .expect("the amount is below the supply cap");

    // 1,500 cents cannot be created. The mission must not open with an
    // empty escrow, and the amount must not count against the day.
    sponsor(&mut w, 1, "Acme", bread_mission(1_500, 1));
    let frames = w.step(1);
    assert_eq!(
        one_event(&frames, "missions.sponsor_refused"),
        json!({
            "request": 1,
            "code": "conflict",
            "message": "The world cannot create 1500 cents now: the money supply is at its limit. Try again later.",
        })
    );
    assert!(events(&frames, "missions.mission_posted").is_empty());
    assert_eq!(minted(&w, SPONSOR_FAUCET), 0);
    assert_eq!(w.global().res::<Missions>().sponsor_minted_on(0), 0);
    assert_eq!(w.global().res::<Missions>().open_count(), 0);

    // 1,000 cents can be created. Only they count against the day.
    sponsor(&mut w, 2, "Acme", bread_mission(1_000, 1));
    let second = posted(&w.step(1), 2);
    assert_eq!((minted(&w, SPONSOR_FAUCET), escrow(&w, second)), (1_000, 1_000));
    assert_eq!(w.global().res::<Missions>().sponsor_minted_on(0), 1_000);
    w.assert_conserved();
}
