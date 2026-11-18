//! Story 1: a newcomer survives three game days with no help.
//!
//! It finds paid work that needs no skill, buys food, eats and sleeps in a
//! shelter. Nobody gives it anything: every cent comes from the faucet
//! `gov.public_works`, and every meal from the depot.

use serde_json::json;
use unsup_kernel::{CityId, ErrorCode, ResidentId};
use unsup_scenarios::{Game, Setup, assert_deterministic};

const CITY: CityId = CityId(0);

/// What the newcomer did in its three days.
#[derive(Debug, Default, PartialEq, Eq)]
struct Diary {
    shifts: u32,
    meals: u32,
    sleeps: u32,
    trips_to_the_depot: u32,
}

/// The needs of a resident: food, energy, health and mood.
fn needs(g: &Game, r: ResidentId) -> [i64; 4] {
    ["need.food", "need.energy", "need.health", "need.mood"].map(|name| g.attr(r, name))
}

/// Lives one step of the plain life of a newcomer: sleep when tired, eat
/// when there is room for a meal, buy one day of food when the pocket is
/// empty, else work one shift.
fn live(g: &mut Game, r: ResidentId, diary: &mut Diary) {
    let [food, energy, _, _] = needs(g, r);
    let vegetables = g.carried(r, "vegetables");
    // The bed of a newcomer is in the `home` section of its observation.
    let home = g.section(r, "home").expect("a newcomer has a bed");
    let hall = g.on_map("city_hall", CITY)[0];
    let depot = g.on_map("depot", CITY)[0];
    if energy < 500 {
        g.act(r, "move_to", json!({ "site": home["site"] }));
        let slept = g.act(r, "sleep", json!({}));
        assert_eq!(slept["quality"], "fair", "a shelter gives a fair sleep");
        diary.sleeps += 1;
    } else if food <= 880 && vegetables > 0 {
        let meal = g.act(r, "eat", json!({}));
        assert_eq!(
            (meal["item"].as_str(), meal["nutrition"].as_i64()),
            (Some("vegetables"), Some(120))
        );
        diary.meals += 1;
    } else if vegetables == 0 && g.cash(r) >= 2_250 {
        g.act(r, "move_to", json!({ "site": depot }));
        let bought = g.act(r, "buy", json!({ "item": "vegetables", "qty": 3 }));
        assert_eq!(
            (bought["unit_price"].as_i64(), bought["total"].as_i64()),
            (Some(750), Some(2_250))
        );
        diary.trips_to_the_depot += 1;
    } else {
        g.act(r, "move_to", json!({ "site": hall }));
        let shift = g.act(r, "work", json!({}));
        assert_eq!(shift["public_works"], true);
        assert_eq!(
            (shift["wage"].as_i64(), shift["tax"].as_i64(), shift["net"].as_i64()),
            (Some(2_800), Some(280), Some(2_520))
        );
        diary.shifts += 1;
    }
}

fn story(g: &mut Game) {
    let ada = g.spawn("Ada");
    // A newcomer has full needs, no cash and no goods. It has the two
    // default routines and a bed in a shelter.
    assert_eq!(needs(g, ada), [1_000; 4]);
    let me = g.section_full(ada, "self").expect("a self section");
    assert_eq!((me["cash"].as_i64(), me["items"].clone()), (Some(0), json!({})));
    let routines: Vec<&str> = me["routines"]
        .as_array()
        .expect("routines")
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert_eq!(routines, ["eat when hungry", "sleep when tired"]);
    let home = g.section(ada, "home").expect("a bed");
    assert_eq!((home["kind"].as_str(), home["sleep"].as_str()), (Some("shelter"), Some("fair")));

    // The job board has no job, but it names the starter shifts.
    let board = g.act(ada, "list_jobs", json!({}));
    assert_eq!(board["jobs"], json!([]));
    assert_eq!(
        board["public_works"],
        json!({ "wage": 2_800, "minutes": 240, "kinds": ["city_hall", "depot"], "open": true })
    );
    // Without money the depot sells nothing.
    assert_eq!(g.on_map("depot", CITY).len(), 1);

    // The first day, step by step. One shift pays for one day of food.
    let mut diary = Diary::default();
    live(g, ada, &mut diary);
    assert_eq!(diary, Diary { shifts: 1, ..Diary::default() });
    assert_eq!(g.cash(ada), 2_520);
    live(g, ada, &mut diary);
    assert_eq!(diary, Diary { shifts: 1, trips_to_the_depot: 1, ..Diary::default() });
    assert_eq!((g.cash(ada), g.carried(ada, "vegetables")), (270, 3));
    // The depot does not give credit.
    assert_eq!(
        g.refused(ada, "buy", json!({ "item": "bread", "qty": 1 })),
        ErrorCode::Insufficient
    );

    // Three days of the same plain life. The last action ends in the
    // morning of day 4.
    let mut lowest = needs(g, ada);
    while g.frame() < Game::frame_at(4, 0, 0) {
        live(g, ada, &mut diary);
        let now = needs(g, ada);
        for (low, value) in lowest.iter_mut().zip(now) {
            *low = (*low).min(value);
        }
    }
    assert_eq!(g.frame(), 4_562);
    assert_eq!(diary, Diary { shifts: 9, meals: 8, sleeps: 4, trips_to_the_depot: 3 });

    // The margin. The newcomer was never hungry, never tired and never
    // hurt: food stayed above 700 of 1,000 and energy above 350.
    assert!(g.is_alive(ada));
    assert_eq!(lowest, [734, 360, 1_000, 976]);
    assert_eq!(needs(g, ada), [896, 720, 1_000, 990]);
    assert!(g.notices(ada, "life.need_low").is_empty());
    assert!(g.notices(ada, "life.need_empty").is_empty());
    // It looked after itself, so its routines had nothing to do.
    let me = g.section_full(ada, "self").expect("a self section");
    let runs: Vec<u64> = me["routines"]
        .as_array()
        .expect("routines")
        .iter()
        .filter_map(|r| r["runs"].as_u64())
        .collect();
    assert_eq!(runs, [0, 0]);

    // The money. Nine shifts gave 9 * 2,520. Three days of food cost
    // 3 * 2,250. More than two thirds of the wages are left: work pays.
    assert_eq!(g.cash(ada), 9 * 2_520 - 3 * 2_250);
    assert_eq!(g.cash(ada), 15_930);
    assert_eq!(g.carried(ada, "vegetables"), 9 - 8);
    // Nine shifts gave experience in `labor`: 39 points for a shift in a
    // good mood. That is level 1.
    assert_eq!(g.attr(ada, "skill.labor"), 351);
    // The city got the income tax of the nine shifts.
    assert_eq!(g.city_treasury(CITY), 9 * 280);

    // Conservation: the only faucet was public works, the only money sink
    // the depot. Nothing else made or destroyed a cent.
    let totals = g.audit();
    assert_eq!(totals.money.minted, 9 * 2_800);
    assert_eq!(totals.money.burned, 3 * 2_250);
    assert_eq!(totals.money.held, g.cash(ada) + g.city_treasury(CITY));
    let shifts = g.events_of("economy.shift_completed");
    assert_eq!(shifts.len(), 9);
    assert!(shifts.iter().all(|event| event.actor == Some(ada) && event.data["wage"] == 2_800));
    assert_eq!(g.events_of("life.meal_eaten").len(), 8);
    assert_eq!(g.events_of("life.slept").len(), 4);
}

#[test]
fn a_newcomer_survives_three_days_on_public_works_and_depot_food() {
    assert_deterministic(&Setup::town(1), story);
}
