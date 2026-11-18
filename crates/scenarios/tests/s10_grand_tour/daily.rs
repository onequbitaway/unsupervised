//! The kernel verbs, the body, the map and the journey to another city.

use serde_json::json;
use unsup_kernel::{CityId, ErrorCode};
use unsup_scenarios::{Game, Republic};

/// Milo, who lost the election, looks around, learns to cook and leaves
/// for the other city.
pub fn run(g: &mut Game, cast: &Republic) {
    let milo = cast.rival;
    let cal = cast.citizens[2];
    let cash = g.cash(milo);

    // The kernel: wait, and stop a wait.
    assert_eq!(g.act(milo, "wait", json!({ "ticks": 3 })), json!({ "waited": 3 }));
    g.submit(milo, "wait", json!({ "ticks": 600 }));
    let stopped = g.act(milo, "cancel", json!({}));
    assert_eq!(stopped["cleared"], 0);
    assert!(stopped["cancelled"].is_u64(), "the wait was stopped: {stopped}");

    // Where am I, and who gets what I have.
    let here = g.act(milo, "inspect", json!({}));
    assert_eq!((here["city"].as_u64(), here["district"].as_str()), (Some(0), Some("downtown")));
    assert_eq!(g.act(milo, "write_will", json!({ "heir": cal })), json!({ "heir": cal }));

    // Half an hour of rest in the park gives 20 points of mood.
    let park = g.site("park");
    g.walk(milo, &park);
    let mood = g.attr(milo, "need.mood");
    g.act(milo, "rest", json!({ "ticks": 30 }));
    assert_eq!(g.attr(milo, "need.mood"), mood + 20);
    // A gift from hand to hand.
    g.walk_to(cal, g.pos(milo).offset(1, 0));
    let cal_cash = g.cash(cal);
    g.act(milo, "give", json!({ "to": cal, "money": 100 }));
    assert_eq!(g.cash(cal), cal_cash + 100);

    // One hour at the school costs 1,000.
    let school = g.site("school");
    g.walk(milo, &school);
    let lesson = g.act(milo, "train", json!({ "skill": "cooking", "ticks": 60 }));
    assert_eq!(lesson["skill"], "cooking");
    assert!(g.attr(milo, "skill.cooking") > 0);

    // The journey: 2,500 at the station, two hours later in the other
    // city. Without a station there is no journey.
    assert_eq!(g.refused(milo, "travel", json!({ "city": 1 })), ErrorCode::NotHere);
    let station = g.site("station");
    g.walk(milo, &station);
    let journey = g.act(milo, "travel", json!({ "city": 1 }));
    assert_eq!(journey["city"], 1);
    let traveled = g.last_event("life.traveled");
    assert_eq!(
        (traveled.data["from_city"].as_u64(), traveled.data["to_city"].as_u64()),
        (Some(0), Some(1))
    );
    let zone = g.zone_of(milo);
    assert_eq!(g.world.world.layout().zone(zone).map(|zone| zone.city), Some(CityId(1)));
    assert_eq!(g.site_of(milo).map(|site| site.kind), Some("station".to_string()));
    assert_eq!(g.cash(milo), cash - 100 - 1_000 - 2_500);
}
