//! Story 11: a soak of the full game.
//!
//! Thirty-six scripted residents live for four game days in a world with
//! two cities. Each one has a simple plan: work, farm, mine, save, talk or
//! steal. They keep the default routines of a newcomer. The world is
//! audited after every frame, and the whole run gives the same state hash
//! a second time and with a restore from a snapshot every 331 frames.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use unsup_kernel::protocol::ActionStatus;
use unsup_kernel::{CityId, ResidentId, SiteId};
use unsup_scenarios::{Game, Setup, assert_deterministic};

/// The game days of the soak.
const DAYS: u64 = 4;
/// Frames that a resident waits after a request that was refused.
const PAUSE: u64 = 30;

/// What a scripted resident does with its days.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// Public works, and food from the depot.
    Worker,
    /// Vegetables to eat and grain to sell to the depot.
    Farmer,
    /// Ore to sell to the depot, and food from the depot.
    Miner,
    /// Public works, and the wages go to the bank.
    Saver,
    /// Public works, and a word for everybody.
    Talker,
    /// Public works by day, the pockets of the others by night.
    Thief,
}

const PLANS: [Plan; 9] = [
    Plan::Worker,
    Plan::Farmer,
    Plan::Worker,
    Plan::Miner,
    Plan::Saver,
    Plan::Worker,
    Plan::Farmer,
    Plan::Talker,
    Plan::Thief,
];

/// The places of one city that the plans use.
struct Places {
    hall: SiteId,
    depot: SiteId,
    farm: SiteId,
    mine: SiteId,
    bank: SiteId,
}

struct Actor {
    id: ResidentId,
    plan: Plan,
    city: CityId,
    /// The frame before which the actor sends nothing.
    ready: u64,
    /// True when the actor has a bank account.
    account: bool,
}

/// Returns the next request of an actor that does nothing.
fn decide(g: &Game, actor: &Actor, places: &Places) -> (&'static str, Value) {
    let r = actor.id;
    let here = g.site_of(r).map(|site| site.id);
    let go = |site: SiteId| ("move_to", json!({ "site": site }));
    let at = |site: SiteId, verb: &'static str, args: Value| {
        if here == Some(site) { (verb, args) } else { go(site) }
    };
    let (food, energy, cash) = (g.attr(r, "need.food"), g.attr(r, "need.energy"), g.cash(r));
    let vegetables = g.carried(r, "vegetables");
    let hour = g.clock().hour;

    // A resident in custody cannot walk. It waits: the jail feeds it.
    if g.has_condition(r, "jailed") || g.has_condition(r, "detained") {
        return ("wait", json!({ "ticks": 60 }));
    }
    // The body comes first: a bed in the shelter, then a meal.
    if energy < 400 {
        let bed = g.section(r, "home").and_then(|home| home["site"].as_u64()).map(SiteId);
        return match bed {
            Some(bed) => at(bed, "sleep", json!({})),
            None => ("sleep", json!({})),
        };
    }
    if food <= 880 && vegetables > 0 {
        return ("eat", json!({ "item": "vegetables" }));
    }
    let shop_for_food = vegetables == 0 && cash >= 2_250;
    let buy_food = json!({ "item": "vegetables", "qty": 3 });
    let shift = at(places.hall, "work", json!({}));
    match actor.plan {
        Plan::Farmer if vegetables < 4 => {
            at(places.farm, "gather", json!({ "item": "vegetables", "minutes": 60 }))
        }
        Plan::Farmer if g.carried(r, "grain") >= 20 => {
            at(places.depot, "sell", json!({ "item": "grain", "qty": 20 }))
        }
        Plan::Farmer => at(places.farm, "gather", json!({ "item": "grain", "minutes": 120 })),
        _ if shop_for_food => at(places.depot, "buy", buy_food),
        Plan::Miner if g.carried(r, "ore") >= 6 => {
            at(places.depot, "sell", json!({ "item": "ore", "qty": 6 }))
        }
        // A miner with nothing to eat earns its first meal with a shift.
        Plan::Miner if vegetables > 0 => {
            at(places.mine, "gather", json!({ "item": "ore", "minutes": 120 }))
        }
        Plan::Saver if cash >= 6_000 && !actor.account => {
            at(places.bank, "open_account", json!({}))
        }
        Plan::Saver if cash >= 6_000 => {
            at(places.bank, "deposit", json!({ "amount": cash - 3_000 }))
        }
        Plan::Talker if g.frame().is_multiple_of(7) => {
            ("say", json!({ "text": "Good day to you." }))
        }
        Plan::Talker if g.frame().is_multiple_of(11) => {
            ("post", json!({ "text": format!("Day {} in the city.", g.day()) }))
        }
        Plan::Thief if !(6..22).contains(&hour) => {
            // The nearest resident in sight, when it stands next to the
            // thief.
            let nearby = g.section(r, "nearby").unwrap_or_default();
            let nearest = &nearby["residents"][0];
            let place = |axis: &str| nearest["pos"][axis].as_i64().unwrap_or(i64::MAX);
            let me = g.pos(r);
            let close = (place("x") - i64::from(me.x)).abs() <= 1
                && (place("y") - i64::from(me.y)).abs() <= 1;
            match nearest["id"].as_u64() {
                Some(victim) if close => ("pickpocket", json!({ "target": victim })),
                _ => shift,
            }
        }
        _ => shift,
    }
}

fn story(g: &mut Game) {
    let mut actors = Vec::new();
    let mut places = BTreeMap::new();
    for city in [CityId(0), CityId(1)] {
        let first = |kind: &str| g.sites_in(kind, city)[0].id;
        places.insert(
            city,
            Places {
                hall: first("city_hall"),
                depot: first("depot"),
                farm: first("farm"),
                mine: first("mine"),
                bank: first("bank"),
            },
        );
    }
    for index in 0..36 {
        let city = CityId(index % 2);
        let id = g.spawn_in(&format!("Soak {index}"), city);
        let plan = PLANS[usize::from(index / 2) % PLANS.len()];
        actors.push(Actor { id, plan, city, ready: 0, account: false });
    }

    let mut refused: Vec<String> = Vec::new();
    let mut sent = 0_u64;
    while g.frame() < DAYS * 1_440 {
        let frame = g.frame();
        for actor in &mut actors {
            if frame < actor.ready || !g.is_alive(actor.id) {
                continue;
            }
            let idle = g.world.with_resident(actor.id, |realm, idx| {
                realm.activity(idx).is_none() && realm.queue_len(idx) == 0
            });
            if !idle {
                continue;
            }
            let (verb, args) = decide(g, actor, &places[&actor.city]);
            sent += 1;
            let ack = g.submit(actor.id, verb, args);
            if let ActionStatus::Rejected { error } = &ack.status {
                refused.push(format!("{verb} of {}: {}", actor.id, error.message));
                actor.ready = frame + PAUSE;
            }
            if verb == "open_account" {
                actor.account = true;
            }
        }
        // One frame, with the audit of the whole world.
        g.step(1);
    }

    // The plans fit the world: no request was refused.
    assert_eq!(refused, Vec::<String>::new());
    assert_eq!(sent, 2_098);

    // Everybody lived through the four days, in good health.
    for actor in &actors {
        assert!(g.is_alive(actor.id), "{:?} {} died", actor.plan, actor.id);
        assert_eq!(g.attr(actor.id, "need.health"), 1_000, "{:?} {}", actor.plan, actor.id);
        assert!(g.attr(actor.id, "need.food") > 250, "{:?} {} is hungry", actor.plan, actor.id);
    }

    // What happened in the four days, exactly.
    let count = |kind: &str| g.events_of(kind).len();
    assert_eq!(count("economy.shift_completed"), 169);
    assert_eq!(count("economy.goods_produced"), 174, "harvests of the farmers and the miners");
    assert_eq!(count("economy.goods_sold"), 183, "trades with the depot");
    assert_eq!(count("life.meal_eaten"), 341);
    assert_eq!(count("life.slept"), 127);
    assert_eq!((count("society.said"), count("society.posted")), (10, 4));
    assert_eq!(count("finance.account_opened"), 3);
    // The thieves were busy, and the city watch took four of them in.
    assert_eq!(count("crime.theft_committed"), 237);
    assert_eq!(count("crime.arrest_made"), 4);
    assert!(g.events_of("crime.arrest_made").iter().all(|event| event.data["watch"] == true));

    // The money of the world: public works and the sales to the depot made
    // it, the purchases at the depot destroyed it, and the rest is held.
    let totals = g.audit();
    assert_eq!((totals.money.minted, totals.money.burned), (593_200, 227_250));
    assert_eq!(totals.money.held + totals.money.in_flight, 593_200 - 227_250);
}

#[test]
fn thirty_six_scripted_residents_live_four_days_and_the_world_stays_whole() {
    assert_deterministic(&Setup::shaped(7, 2, 2), story);
}
