//! Many residents do many things for three game days, in an order that a
//! fixed seed decides. Most requests are refused; many are not. The world
//! stays balanced in every tick (each realm audits itself and panics when
//! it is not), every cent and every unit has a named origin, and the same
//! seed gives the same world, also across a snapshot.

mod common;

use std::collections::BTreeMap;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::protocol::ActionStatus;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Money, Pos, ResidentId, SiteId};

/// A small generator of numbers for the test. The simulation does not use
/// it.
struct Dice(u64);

impl Dice {
    fn below(&mut self, n: u64) -> u64 {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }

    fn pick<'a, T>(&mut self, list: &'a [T]) -> &'a T {
        let n = u64::try_from(list.len()).expect("a short list");
        &list[usize::try_from(self.below(n)).expect("an index")]
    }

    fn range(&mut self, from: i64, to: i64) -> i64 {
        from + i64::try_from(self.below(u64::try_from(to - from).expect("from < to")))
            .expect("small")
    }
}

const GOODS: [&str; 10] = [
    "grain",
    "vegetables",
    "cotton",
    "wood",
    "ore",
    "bread",
    "cloth",
    "metal",
    "tools",
    "moonshine",
];
const RECIPES: [&str; 6] = ["bread", "cloth", "metal", "tools", "moonshine", "meal"];
const KINDS: [&str; 4] = ["shop", "restaurant", "workshop", "factory"];

/// Every verb of the module that changes something.
const VERBS: [&str; 22] = [
    "work",
    "gather",
    "produce",
    "buy",
    "sell",
    "stock",
    "unstock",
    "set_price",
    "post_job",
    "apply_job",
    "quit_job",
    "fire",
    "close_job",
    "buy_property",
    "sell_property",
    "rent_out",
    "rent",
    "end_lease",
    "open_business",
    "close_business",
    "till_fund",
    "till_collect",
];

/// The verbs that make sense at a site of a kind.
fn menu(kind: &str) -> &'static [&'static str] {
    match kind {
        "depot" => &["buy", "sell", "work"],
        "city_hall" => &["work"],
        "farm" | "mine" | "forest" => &["gather", "apply_job", "work", "buy_property"],
        "shop" => &["buy", "sell", "apply_job", "work", "stock", "set_price", "till_collect"],
        "restaurant" => &["buy", "sell", "apply_job", "work", "produce", "stock"],
        "workshop" | "factory" => &["apply_job", "work", "produce", "stock", "open_business"],
        "home" | "apartment" => &["rent", "end_lease", "buy_property", "rent_out"],
        _ => &VERBS,
    }
}

/// Makes one request of a resident. Two times out of three the verb fits
/// the site where the resident is. The arguments are often wrong: the
/// module must refuse those and change nothing.
fn request(
    dice: &mut Dice,
    w: &TestWorld<'_>,
    r: ResidentId,
    others: &[ResidentId],
) -> (&'static str, Value) {
    let kind = w.with_resident(r, |realm, idx| {
        realm.sites().at(realm.pos(idx)).map_or(String::new(), |site| site.kind.clone())
    });
    let verb = if dice.below(3) == 0 { *dice.pick(&VERBS) } else { *dice.pick(menu(&kind)) };
    let item = *dice.pick(&GOODS);
    let qty = dice.range(1, 6);
    // A job that the site where the resident is offers, if any.
    let job = section(w, r, "business")
        .and_then(|seen| seen["jobs"].as_array().and_then(|jobs| jobs.first().cloned()))
        .map_or(json!(1), |job| job["job"].clone());
    let args = match verb {
        "gather" => json!({ "minutes": dice.range(10, 90) }),
        "produce" => json!({ "recipe": dice.pick(&RECIPES), "batches": dice.range(1, 4) }),
        "buy" | "sell" | "stock" | "unstock" => json!({ "item": item, "qty": qty }),
        "set_price" => {
            let price = dice.range(200, 4_000);
            json!({ "item": item, "price": price, "buy_price": price / 3 })
        }
        "post_job" => {
            let task = (dice.below(2) == 0).then(|| *dice.pick(&RECIPES));
            json!({ "wage": dice.range(100, 4_000), "slots": qty, "task": task })
        }
        "apply_job" | "close_job" => json!({ "job": job }),
        "fire" => json!({ "resident": dice.pick(others) }),
        "sell_property" => json!({ "price": dice.range(1_000, 300_000) }),
        "rent_out" => json!({ "rent": dice.range(100, 2_000) }),
        "open_business" => json!({ "kind": dice.pick(&KINDS) }),
        "till_fund" => json!({ "amount": dice.range(100, 6_000) }),
        _ => json!({}),
    };
    (verb, args)
}

/// Gives the town a start: a restaurant and a workshop with inputs and
/// jobs, a shop with bread, a farm with a job and a home to let.
fn open_for_business(w: &mut TestWorld<'static>, at: &Sites, cast: &[ResidentId]) {
    let fund = |w: &mut TestWorld<'static>, r: ResidentId, amount: i64| {
        w.give_money(r, Money(amount));
        ok(w, r, "till_fund", json!({ "amount": amount }));
    };
    let fill = |w: &mut TestWorld<'static>, r: ResidentId, item: &str, qty: i64| {
        w.give_item(r, item, qty);
        ok(w, r, "stock", json!({ "item": item, "qty": qty }));
    };
    let (cook, maker, grocer, farmer, landlord) = (cast[0], cast[1], cast[2], cast[3], cast[4]);

    open_site(w, cook, at.restaurant);
    fund(w, cook, 20_000);
    fill(w, cook, "grain", 40);
    ok(w, cook, "post_job", json!({ "wage": 1_500, "slots": 2, "task": "bread" }));
    ok(w, cook, "set_price", json!({ "item": "bread", "price": 900 }));
    ok(w, cook, "set_price", json!({ "item": "grain", "buy_price": 150 }));

    open_site(w, maker, at.workshop);
    fund(w, maker, 20_000);
    fill(w, maker, "cotton", 40);
    ok(w, maker, "post_job", json!({ "wage": 1_500, "slots": 2, "task": "cloth" }));

    open_site(w, grocer, at.shop);
    fund(w, grocer, 10_000);
    fill(w, grocer, "bread", 30);
    ok(w, grocer, "set_price", json!({ "item": "bread", "price": 800, "buy_price": 300 }));
    ok(w, grocer, "post_job", json!({ "wage": 1_000, "slots": 2 }));

    buy_site(w, farmer, at.farm);
    fund(w, farmer, 20_000);
    ok(w, farmer, "post_job", json!({ "wage": 1_200, "slots": 3 }));

    buy_site(w, landlord, at.home);
    ok(w, landlord, "rent_out", json!({ "rent": 300 }));
}

/// The life module reports a death and removes the resident.
fn die(w: &mut TestWorld<'static>, cast: &mut Vec<ResidentId>, dead: ResidentId, heir: Value) {
    cast.retain(|r| *r != dead);
    let death = json!({ "resident": dead, "cause": "age", "age": 80, "heir": heir, "estate": 0 });
    emit_global(w, "life.resident_died", death);
    w.despawn(dead);
}

/// The finance module reports a loan default.
fn default_of(w: &mut TestWorld<'static>, debtor: ResidentId) {
    emit_global(w, "finance.loan_defaulted", json!({ "resident": debtor, "outstanding": 1 }));
}

/// What a run of the soak gives.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    hash: String,
    flows: Flows,
    /// Requests that the module did not refuse at once.
    accepted: u32,
    /// Requests that the module refused at once.
    refused: u32,
    /// How many public events of each kind the module emitted.
    events: BTreeMap<String, u32>,
}

/// Runs three game days. With `restore_at`, the world goes through a
/// snapshot before that frame.
fn soak(restore_at: Option<u32>) -> Outcome {
    let mut dice = Dice(20_260_101);
    let mut w = TestWorld::with_seed(&[&ECONOMY], 9);
    let at = build(&mut w);
    let places: Vec<SiteId> = vec![
        at.farm,
        at.mine,
        at.forest,
        at.workshop,
        at.factory,
        at.restaurant,
        at.shop,
        at.home,
        at.apartment,
        at.city_hall,
        at.depot,
        at.office,
        at.park,
        at.far_home,
        at.far_shop,
        at.far_hall,
    ];
    // Six residents with money for a site, eighteen with pocket money.
    let mut cast: Vec<ResidentId> = Vec::new();
    for n in 0..24 {
        let cash = if n < 6 { 300_000 } else { 4_000 };
        let r = resident_at(&mut w, &format!("R{n}"), cash, *dice.pick(&places));
        w.give_item(r, "grain", 6);
        cast.push(r);
    }
    open_for_business(&mut w, &at, &cast);
    let (cook, maker, grocer, farmer) = (cast[0], cast[1], cast[2], cast[3]);
    let (landlord, drifter, heir) = (cast[4], cast[5], cast[10]);
    let (mut accepted, mut refused) = (0, 0);
    let mut events: BTreeMap<String, u32> = BTreeMap::new();

    for frame in 0..3 * DAY {
        if restore_at == Some(frame) {
            w.snapshot_restore();
        }
        // The finance module reports two defaults and the life module two
        // deaths: the landlord has an heir, the sixth resident has none.
        match frame {
            700 => die(&mut w, &mut cast, landlord, json!({ "Resident": heir })),
            1_500 => default_of(&mut w, farmer),
            2_500 => die(&mut w, &mut cast, drifter, Value::Null),
            3_300 => default_of(&mut w, grocer),
            _ => {}
        }
        for i in 0..cast.len() {
            let r = cast[i];
            if dice.below(12) != 0 {
                continue;
            }
            // Now and then a resident stops what it does.
            if !is_idle(&w, r) {
                if dice.below(40) == 0 {
                    w.act(r, "cancel", json!({}));
                }
                continue;
            }
            // The cook and the maker stay at their business. The others
            // walk around.
            if r != cook && r != maker {
                match dice.below(4) {
                    0 => go(&mut w, r, *dice.pick(&places)),
                    1 => w.set_pos(r, Pos::new(60, 60)),
                    _ => {}
                }
            }
            let (verb, args) = request(&mut dice, &w, r, &cast);
            match w.act(r, verb, args).status {
                ActionStatus::Rejected { .. } => refused += 1,
                _ => accepted += 1,
            }
        }
        for event in events_of(w.step(1)) {
            if event.kind.starts_with("economy.") {
                *events.entry(event.kind).or_default() += 1;
            }
        }
        if frame % DAY == DAY - 1 {
            // The named flows explain every cent at the end of each day.
            flows(&w);
        }
    }
    Outcome { hash: w.hash(), flows: flows(&w), accepted, refused, events }
}

#[test]
fn three_busy_days_stay_balanced_and_deterministic() {
    let plain = soak(None);
    // The soak is not idle: every faucet and every sink of the module ran,
    // and both the accepted and the refused requests are many.
    let flows = plain.flows.clone().of_module();
    for faucet in ["gov.public_works", "export.sale"] {
        assert!(flows.minted.get(faucet).is_some_and(|sum| *sum > 0), "{faucet}: {flows:?}");
    }
    assert!(flows.burned.get("import.purchase").is_some_and(|sum| *sum > 0), "{flows:?}");
    for faucet in ["nature.harvest:", "production.output:", "import.purchase:"] {
        assert!(flows.produced.keys().any(|name| name.starts_with(faucet)), "{faucet}: {flows:?}");
    }
    for sink in ["production.input:", "export.sale:"] {
        assert!(flows.consumed.keys().any(|name| name.starts_with(sink)), "{sink}: {flows:?}");
    }
    assert!(plain.accepted > 300, "{} requests were accepted", plain.accepted);
    assert!(plain.refused > 300, "{} requests were refused", plain.refused);
    for kind in [
        "economy.shift_completed",
        "economy.goods_produced",
        "economy.goods_sold",
        "economy.worker_hired",
        "economy.worker_left",
        "economy.lease_started",
        "economy.lease_ended",
        "economy.property_sold",
        "economy.property_foreclosed",
        "economy.property_inherited",
    ] {
        assert!(plain.events.get(kind).is_some_and(|n| *n > 0), "no {kind}: {:?}", plain.events);
    }

    // The same seed gives the same world, also through a snapshot in the
    // middle of a day and one just before midnight.
    assert_eq!(soak(None), plain);
    assert_eq!(soak(Some(1_000)), plain);
    assert_eq!(soak(Some(2 * DAY - 1)), plain);
}
