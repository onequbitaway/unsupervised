//! Story 7: a resident crosses a district border in the middle of an
//! action and still finishes it.
//!
//! A district is one zone, and each zone is its own actor. A walk to
//! another district is one action that two zones carry out. The plan of
//! the resident, its cash and its goods move with it. Two residents on the
//! two sides of a border can still see, hear and pay each other.

use serde_json::json;
use unsup_kernel::{Pos, ZoneId};
use unsup_scenarios::{Game, Setup, assert_deterministic};

fn story(g: &mut Game) {
    let ada = g.spawn("Ada");
    let bob = g.spawn("Bob");
    let hall = g.site("city_hall");
    let depot = g.site("depot");
    let (downtown, industrial) = (g.zone_of_site(hall.id), g.zone_of_site(depot.id));
    assert_eq!((downtown, industrial), (ZoneId(2), ZoneId(0)));

    // Ada earns a wage downtown, so that she has something to carry.
    g.walk(ada, &hall);
    g.act(ada, "work", json!({}));
    assert_eq!((g.cash(ada), g.zone_of(ada)), (2_520, downtown));

    // One plan: walk to the depot in the industrial district, buy food
    // there and work a shift there. The walk starts at once and the rest
    // waits in the queue.
    let since = g.cursor(ada);
    let walk = g.submit(ada, "move_to", json!({ "site": depot.id }));
    let buy = g.submit(ada, "buy", json!({ "item": "vegetables", "qty": 3 }));
    let work = g.submit(ada, "work", json!({}));
    let running = |g: &Game| {
        g.world.with_resident(ada, |realm, idx| {
            (realm.activity(idx).map(|a| (a.verb, a.action_id)), realm.queue_len(idx))
        })
    };
    g.step(1);
    assert_eq!(running(g), (Some(("move_to", walk.action_id)), 2));

    // She walks until she is in the other district. The walk is still the
    // same action, and the plan is still behind it.
    let mut frames = 1;
    while g.zone_of(ada) == downtown {
        g.step(1);
        frames += 1;
    }
    assert_eq!(frames, 62, "the walk to the border takes 62 frames");
    assert_eq!(g.zone_of(ada), industrial);
    assert_eq!(g.pos(ada).y, 63, "the first tile behind the border");
    assert_eq!(running(g), (Some(("move_to", walk.action_id)), 2));
    // Her cash crossed the border with her.
    assert_eq!(g.cash(ada), 2_520);
    g.audit();
    // A snapshot in the middle of the walk changes nothing.
    g.world.snapshot_restore();

    // The walk ends at the depot, and the plan goes on there.
    let walked = g.outcome(ada, since, walk).expect("the walk ends well");
    assert_eq!(walked, json!({ "site": depot.id, "x": 56, "y": 43, "steps": 96, "ticks": 100 }));
    assert!(depot.rect.contains(g.pos(ada)));
    let bought = g.outcome(ada, since, buy).expect("the depot sells");
    assert_eq!(bought["total"], 2_250);
    let shift = g.outcome(ada, since, work).expect("the shift is paid");
    assert_eq!((shift["site"].as_u64(), shift["net"].as_i64()), (Some(depot.id.0), Some(2_520)));
    assert_eq!((g.cash(ada), g.carried(ada, "vegetables")), (2_520 - 2_250 + 2_520, 3));

    // Two residents at a border: Ada on the last tile of the industrial
    // district, Bob on the first tile of downtown.
    g.walk_to(ada, Pos::new(30, 63));
    g.walk_to(bob, Pos::new(30, 64));
    assert_eq!((g.zone_of(ada), g.zone_of(bob)), (industrial, downtown));
    g.step(2);
    // She sees him in her observation and with `inspect`.
    let nearby = g.section(ada, "nearby").expect("a nearby section");
    assert_eq!(nearby["residents"][0]["id"], json!(bob));
    let seen = g.act(ada, "inspect", json!({ "resident": bob }));
    assert_eq!(
        seen,
        json!({
            "resident": bob, "name": "Bob", "pos": { "x": 30, "y": 64 }, "distance": 1,
            "activity": null, "zone": downtown, "across_border": true,
        })
    );
    // He hears her, and she is told that somebody heard.
    let said = g.act(ada, "say", json!({ "text": "Bob, catch." }));
    assert_eq!(said, json!({ "heard_by": 1 }));
    g.step(1);
    let heard = g.notice(bob, "society.said");
    assert_eq!(heard.from, Some(ada));
    assert_eq!(
        heard.data["text"],
        json!({ "untrusted": true, "from": ada, "text": "Bob, catch." })
    );
    // She hands him cash and food across the border.
    let gift = json!({ "to": bob, "money": 500, "item": "vegetables", "qty": 2 });
    let given = g.act(ada, "give", gift);
    assert_eq!(
        given,
        json!({ "from": ada, "to": bob, "money": 500, "item": "vegetables", "qty": 2 })
    );
    g.step(1);
    assert_eq!((g.cash(bob), g.carried(bob, "vegetables")), (500, 2));
    assert_eq!((g.cash(ada), g.carried(ada, "vegetables")), (2_290, 1));
    assert_eq!(g.notice(bob, "life.gift_received").data["money"], 500);
    // Bob eats what he got, on his side.
    assert_eq!(g.act(bob, "eat", json!({}))["item"], "vegetables");

    // Nothing was made or lost on the way: two wages came in, the depot
    // took the price of the food.
    let totals = g.audit();
    assert_eq!((totals.money.minted, totals.money.burned), (2 * 2_800, 2_250));
    assert_eq!(
        totals.money.held,
        g.cash(ada) + g.cash(bob) + g.city_treasury(unsup_kernel::CityId(0))
    );
}

#[test]
fn a_walk_a_plan_and_a_gift_cross_a_district_border() {
    assert_deterministic(&Setup::town(1), story);
}
