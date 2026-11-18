//! The first day of a newcomer, with real game verbs. The story is the
//! same for each [`Link`]: find paid work, earn, get food, eat, plan
//! several actions, sleep, set routines.
//!
//! Every price, wage and time comes from the game. The story holds when
//! the numbers of the game change.

use serde_json::{Value, json};

use super::game::{Catalogue, Map, attr, carried, pos};
use super::link::{Link, accepted, done, myself, report, state};
use super::{World, id, int};

/// Greatest number of starter shifts that the story works for one meal.
const SHIFTS_MAX: u32 = 8;

/// What the story found and did. A test of one transport checks more
/// with it.
#[derive(Debug)]
pub struct FirstDay {
    /// The wage of one starter shift before tax, in cents.
    pub wage: i64,
    /// What the resident earned in all shifts, after tax.
    pub earned: i64,
    /// Number of shifts that the resident worked.
    pub shifts: u32,
    /// The food that the resident bought and ate.
    pub food: String,
    /// The price of that food at the depot.
    pub price: i64,
    /// The cash of the resident at the end.
    pub cash: i64,
    /// The id of the site where the resident sleeps.
    pub home: u64,
}

/// Walks to a site, unless the resident is there already. Returns the
/// section `self` at the site.
async fn walk<L: Link>(world: &World, link: &mut L, site: u64) -> Value {
    let me = myself(link).await;
    if me["site"]["id"] == site {
        return me;
    }
    let result = done(link, world, "move_to", json!({ "site": site })).await;
    assert_eq!(result["site"], site, "the walk ended at the site: {result}");
    let me = myself(link).await;
    assert_eq!(me["site"]["id"], site, "the resident is in the site: {me}");
    assert_eq!(pos(&me), (int(&result["x"], "x"), int(&result["y"], "y")));
    me
}

/// Works one starter shift at the site where the resident stands.
/// Returns what the shift paid after tax.
async fn shift<L: Link>(world: &World, link: &mut L, wage: i64, minutes: i64) -> i64 {
    let before = int(&myself(link).await["cash"], "cash");
    let ack = accepted(link, "work", json!({})).await;
    assert_eq!(state(&ack), "started", "a starter shift starts at once: {ack}");
    assert_eq!(int(&ack["status"]["ticks"], "ticks"), minutes, "the shift has its length: {ack}");
    let report = report(link, world, id(&ack["action_id"], "action_id")).await;
    assert_eq!(report["ok"], true, "the shift went well: {report}");
    let result = &report["result"];
    assert_eq!(result["public_works"], true, "{result}");
    assert_eq!(int(&result["wage"], "wage"), wage, "the shift pays the listed wage: {result}");
    let (net, tax) = (int(&result["net"], "net"), int(&result["tax"], "tax"));
    assert_eq!(net + tax, wage, "the wage is the pay and the tax: {result}");
    assert!(net > 0, "a shift pays: {result}");
    let after = int(&myself(link).await["cash"], "cash");
    assert_eq!(after, before + net, "the pay is in the pocket");
    net
}

/// Returns the cheapest food of the depot where the resident stands, and
/// its price. The price list is in the section `business` with full
/// detail.
async fn cheapest_food<L: Link>(link: &mut L, catalogue: &Catalogue) -> (String, i64) {
    let observation = link.observe("full", &["business"]).await;
    let business = &observation["sections"]["business"];
    assert_eq!(business["depot"], true, "the resident stands in the depot: {business}");
    let prices = business["prices"].as_array().expect("the depot shows its prices");
    prices
        .iter()
        .filter_map(|entry| {
            let item = entry["item"].as_str().expect("a price names its item");
            catalogue.nutrition(item)?;
            Some((item.to_string(), entry["price"].as_i64()?))
        })
        .min_by_key(|(item, price)| (*price, item.clone()))
        .expect("the depot sells food")
}

/// Plays the first day of a newcomer through one link.
///
/// # Panics
///
/// Panics when the game does not do what its documents say.
pub async fn first_day<L: Link>(world: &World, link: &mut L) -> FirstDay {
    let catalogue = Catalogue::read(world).await;
    for verb in ["list_jobs", "move_to", "work", "buy", "eat", "sleep", "wait", "set_routines"] {
        assert!(catalogue.verb(verb).is_some(), "the game has the verb `{verb}`");
    }

    // A newcomer has full needs, a bed in a shelter and nothing to give.
    let observation = link.observe("brief", &[]).await;
    let me = &observation["sections"]["self"];
    assert_eq!(me["activity"], Value::Null, "a newcomer is idle: {me}");
    assert_eq!(me["queue"], 0);
    assert_eq!(me["items"], json!({}), "a newcomer has no goods");
    for need in ["need.food", "need.energy", "need.health"] {
        assert!(attr(me, need) > catalogue.attr_default(need) * 3 / 4, "{need} is near full: {me}");
    }
    let home = &observation["sections"]["home"];
    assert_eq!(home["kind"], "shelter", "a newcomer has a bed in a shelter: {home}");
    let bed = home["sleep"].as_str().expect("how well the resident sleeps at home").to_string();
    let home = id(&home["site"], "the home site");
    let start_cash = int(&me["cash"], "cash");
    let city = id(&me["city"], "city");

    // Find paid work: the list of jobs names the starter shifts.
    let ack = accepted(link, "list_jobs", json!({})).await;
    let listed = report(link, world, id(&ack["action_id"], "action_id")).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let works = &listed["result"]["public_works"];
    assert_eq!(works["open"], true, "the city has budget for a starter shift: {works}");
    let (wage, minutes) = (int(&works["wage"], "wage"), int(&works["minutes"], "minutes"));
    assert!(wage > 0 && minutes > 0, "{works}");
    let kinds: Vec<&str> = works["kinds"]
        .as_array()
        .expect("the kinds of sites with starter shifts")
        .iter()
        .map(|kind| kind.as_str().expect("a site kind"))
        .collect();

    // The public map says where those sites and the depot are.
    let map = Map::read(world, city).await;
    let depot = map.nearest(&["depot"], pos(me)).id;
    let workplace = if kinds.contains(&"depot") { depot } else { map.nearest(&kinds, pos(me)).id };

    // Earn until the cheapest food of the depot is affordable.
    let (mut earned, mut shifts) = (0, 0);
    let (food, price) = loop {
        assert!(shifts < SHIFTS_MAX, "{SHIFTS_MAX} starter shifts pay for one meal");
        walk(world, link, workplace).await;
        earned += shift(world, link, wage, minutes).await;
        shifts += 1;
        let me = walk(world, link, depot).await;
        let (food, price) = cheapest_food(link, &catalogue).await;
        if int(&me["cash"], "cash") >= price {
            break (food, price);
        }
    };
    let nutrition = catalogue.nutrition(&food).expect("the food has nutrition");

    // Get food: one unit at the price of the list.
    let before = myself(link).await;
    let args = json!({ "item": food, "qty": 1, "max_price": price });
    let bought = done(link, world, "buy", args).await;
    assert_eq!(int(&bought["total"], "total"), price, "one unit costs the listed price: {bought}");
    assert_eq!(bought["item"], food.as_str());
    let after = myself(link).await;
    assert_eq!(int(&after["cash"], "cash"), int(&before["cash"], "cash") - price);
    assert_eq!(carried(&after, &food), carried(&before, &food) + 1);

    // Eat it.
    let hungry = attr(&after, "need.food");
    let eaten = done(link, world, "eat", json!({ "item": food })).await;
    assert_eq!(int(&eaten["nutrition"], "nutrition"), nutrition, "{eaten}");
    assert!(int(&eaten["food"], "food") > hungry, "the meal fills: {eaten}, before {hungry}");
    let fed = myself(link).await;
    assert_eq!(carried(&fed, &food), carried(&before, &food), "the unit is eaten");

    // Plan several actions: walk home, sleep, then wait. They run in
    // order, and each one reports.
    let nap = catalogue.param("life.sleep_min_ticks").max(60);
    let plan = vec![
        json!({ "id": "day-1-home", "verb": "move_to", "args": { "site": home } }),
        json!({ "id": "day-1-sleep", "verb": "sleep", "args": { "ticks": nap } }),
        json!({ "id": "day-1-wait", "verb": "wait", "args": { "ticks": 3 } }),
    ];
    let acks = link.plan(plan.clone()).await;
    assert_eq!(acks.len(), 3, "each request of the plan has an acknowledgement: {acks:?}");
    assert!(matches!(state(&acks[0]), "started" | "pending"), "the walk starts: {acks:?}");
    assert_eq!(acks[1]["status"], json!({ "state": "queued", "position": 1 }));
    assert_eq!(acks[2]["status"], json!({ "state": "queued", "position": 2 }));
    let ids = ["day-1-home", "day-1-sleep", "day-1-wait"];
    for (ack, request) in acks.iter().zip(ids) {
        assert_eq!(ack["request_id"], request, "{ack}");
    }
    // The same plan again does nothing: the request ids are the same, so
    // the server gives the first answers.
    let again = link.plan(plan).await;
    assert_eq!(again, acks, "a repeated request id gets the first acknowledgement");

    let mut reports = Vec::new();
    for ack in &acks {
        let report = report(link, world, id(&ack["action_id"], "action_id")).await;
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["request_id"], ack["request_id"]);
        reports.push(report);
    }
    let seqs: Vec<u64> = reports.iter().map(|report| id(&report["seq"], "seq")).collect();
    assert!(seqs[0] < seqs[1] && seqs[1] < seqs[2], "the reports come in the order of the plan");
    assert_eq!(reports[1]["result"]["ticks"], nap, "the sleep had its length: {}", reports[1]);
    assert_eq!(reports[1]["result"]["quality"], bed.as_str(), "the bed is that of the shelter");
    let rested = myself(link).await;
    assert_eq!(rested["site"]["id"], home, "the resident slept at home: {rested}");
    assert_eq!(rested["conditions"], json!({}), "the sleep is over");

    // Sleep gives energy: the points of the bed for each hour.
    let tired = attr(&rested, "need.energy");
    let rate = catalogue.param(&format!("life.sleep_energy_{bed}_per_hour"));
    let slept = done(link, world, "sleep", json!({ "ticks": nap })).await;
    let full = catalogue.attr_default("need.energy");
    assert_eq!(int(&slept["energy"], "energy"), (tired + rate * nap / 60).min(full), "{slept}");

    // A plan stops at the first request that the simulation rejects.
    let acks = link
        .plan(vec![
            json!({ "verb": "wait", "args": { "ticks": 1 } }),
            json!({ "verb": "no_such_verb", "args": {} }),
            json!({ "verb": "wait", "args": { "ticks": 1 } }),
        ])
        .await;
    assert_eq!(acks.len(), 2, "the request after a refusal is not sent: {acks:?}");
    assert_eq!(state(&acks[0]), "started");
    assert_eq!(acks[1]["status"]["error"]["code"], "unknown_verb", "{acks:?}");
    let short = report(link, world, id(&acks[0]["action_id"], "action_id")).await;
    assert_eq!(short["ok"], true, "{short}");

    // Routines: standing orders for the time without the agent.
    let cash = int(&myself(link).await["cash"], "cash");
    assert_eq!(cash, start_cash + earned - price, "the money of the day adds up");
    routines(world, link, &food).await;

    FirstDay { wage, earned, shifts, food, price, cash, home }
}

/// Sets three routines with the verb `set_routines`, sees that the
/// server runs the one whose condition holds, and removes them.
async fn routines<L: Link>(world: &World, link: &mut L, food: &str) {
    let list = json!([
        { "name": "eat when hungry",
          "when": { "type": "all", "of": [
              { "type": "attr_below", "attr": "need.food", "value": 1 },
              { "type": "has_item", "item": food, "qty": 1 } ] },
          "then": { "verb": "eat", "args": { "item": food } } },
        { "name": "sleep when exhausted",
          "when": { "type": "attr_below", "attr": "need.energy", "value": 1 },
          "then": { "verb": "sleep", "args": {} } },
        { "name": "rest when idle",
          "when": { "type": "always" },
          "then": { "verb": "rest", "args": { "ticks": 5 } } },
    ]);
    let ack = link.act("set_routines", json!({ "routines": list })).await;
    assert_eq!(ack["status"], json!({ "state": "done", "result": { "routines": 3 } }), "{ack}");

    // The resident is idle. The first two conditions do not hold, so the
    // server starts the third routine by itself.
    world.step(8).await;
    let observation = link.observe("full", &["self"]).await;
    let me = &observation["sections"]["self"];
    let set = me["routines"].as_array().expect("full detail shows the routines");
    let names: Vec<&str> =
        set.iter().map(|routine| routine["name"].as_str().expect("a name")).collect();
    assert_eq!(names, ["eat when hungry", "sleep when exhausted", "rest when idle"]);
    assert_eq!(me["activity"]["verb"], "rest", "the routine runs: {me}");

    // An empty list removes the routines. `cancel` stops the rest.
    let ack = link.act("set_routines", json!({ "routines": [] })).await;
    assert_eq!(ack["status"], json!({ "state": "done", "result": { "routines": 0 } }), "{ack}");
    let ack = link.act("cancel", json!({})).await;
    assert_eq!(state(&ack), "done", "{ack}");
    world.step(8).await;
    let me = myself(link).await;
    assert_eq!(me["activity"], Value::Null, "no routine starts an action any more: {me}");
}
