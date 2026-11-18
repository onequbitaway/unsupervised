//! Crimes, the police, the court and the jail.

use serde_json::{Value, json};
use unsup_kernel::{ResidentId, Site};
use unsup_scenarios::Game;

use crate::Tour;

/// Repeats a crime until it works. A crime can fail, and a failed one is
/// a crime too. Returns the result of the one that worked.
fn until_success(g: &mut Game, r: ResidentId, verb: &str, args: Value) -> Value {
    for _ in 0..20 {
        let result = g.act(r, verb, args.clone());
        if result["success"] == true {
            return result;
        }
    }
    panic!("`{verb}` failed twenty times in a row");
}

/// Day 8: Cal steals, robs, attacks, burgles, sells the loot, breaks
/// things and tries to buy the officer. The officer takes him in. Returns
/// the case of the robbery, which waits for its jury.
pub fn crimes(g: &mut Game, tour: &Tour, place: &Site) -> u64 {
    let [ann, ben, cal, dee] = tour.cast.citizens;
    let (otto, judy) = (tour.cast.officer, tour.cast.judge);

    // Three crimes against Dee in the park.
    let park = g.site("park");
    g.walk(dee, &park);
    g.walk_to(cal, g.pos(dee).offset(1, 0));
    until_success(g, cal, "pickpocket", json!({ "target": dee }));
    let robbery = until_success(g, cal, "rob", json!({ "target": dee }));
    let robbery = robbery["case"].as_u64().expect("a case");
    let health = g.attr(dee, "need.health");
    let attack = until_success(g, cal, "assault", json!({ "target": dee }));
    assert_eq!(attack["damage"], 150);
    // An hourly pass can give 5 points of health back between the attacks.
    let hurt = health - g.attr(dee, "need.health");
    assert!((145..=150).contains(&hurt), "the attack took {hurt} points of health");
    // The victim knows the cases. She reports the robbery.
    assert_eq!(g.act(dee, "report_crime", json!({ "case": robbery }))["reported"], true);

    // The restaurant at night: the till, the bread, and a broken window.
    g.walk(cal, place);
    let burglary = until_success(g, cal, "burgle", json!({}));
    let burglary_case = burglary["case"].as_u64().expect("a case");
    let bread = burglary["goods"]["bread"].as_i64().expect("bread in the loot");
    // The fence pays 60 percent of the floor price of the depot: 192.
    let sold = g.act(cal, "fence", json!({ "item": "bread" }));
    assert_eq!((sold["qty"].as_i64(), sold["unit_price"].as_i64()), (Some(bread), Some(192)));
    let damage = until_success(g, cal, "vandalize", json!({}));
    assert!(damage["damage"].as_i64().unwrap_or(0) > 0, "{damage}");
    // The owner repairs a part of the damage.
    g.walk(ann, place);
    let fixed = g.act(ann, "repair", json!({ "amount": 1_000 }));
    assert_eq!((fixed["repaired"].as_i64(), fixed["cost"].as_i64()), (Some(1_000), Some(1_000)));

    // Dee takes money out of the bank, puts a price on the head of Cal
    // and has her wounds seen to: the hospital and then medicine.
    let office = g.site("bank");
    g.walk(dee, &office);
    g.act(dee, "withdraw", json!({ "amount": 9_500 }));
    let bounty = g.act(dee, "post_bounty", json!({ "target": cal, "amount": 500 }));
    assert_eq!(bounty["amount"], 500);
    let depot = g.site("depot");
    g.walk(dee, &depot);
    g.act(dee, "buy", json!({ "item": "medicine", "qty": 1 }));
    let hospital = g.site("hospital");
    g.walk(dee, &hospital);
    let health = g.attr(dee, "need.health");
    g.act(dee, "heal", json!({ "points": 50 }));
    g.act(dee, "use_item", json!({ "item": "medicine" }));
    assert_eq!(g.attr(dee, "need.health"), (health + 50 + 400).min(1_000));

    // The officer: a patrol, the docket and the trace in the restaurant.
    g.walk(otto, place);
    assert_eq!(g.act(otto, "patrol", json!({ "ticks": 10 }))["ticks"], 10);
    let docket = g.act(otto, "case_list", json!({}));
    assert!(docket["cases"].as_array().is_some_and(|cases| !cases.is_empty()));
    assert_eq!(g.act(otto, "case_view", json!({ "case": burglary_case }))["kind"], "burglary");
    assert_eq!(g.act(otto, "investigate", json!({ "case": burglary_case }))["case"], burglary_case);
    // Cal offers the officer money. The officer takes it and searches him:
    // the stolen cash is evidence for each theft.
    g.walk_to(cal, g.pos(otto).offset(1, 0));
    g.act(cal, "bribe", json!({ "to": otto, "amount": 300, "note": "Look away." }));
    let search = g.act(otto, "search", json!({ "target": cal }));
    assert_eq!(search["found"], true);
    g.step(1);
    // An arrest, a release and a second arrest. The bounty pays the
    // officer for the first one.
    g.act(otto, "arrest", json!({ "target": cal }));
    g.step(2);
    let paid = g.last_event("crime.bounty_paid");
    assert_eq!(
        (paid.data["officer"].as_u64(), paid.data["target"].as_u64(), paid.data["amount"].as_i64()),
        (Some(otto.0), Some(cal.0), Some(500))
    );
    let jail = g.site("jail");
    g.walk(otto, &jail);
    g.walk_to(otto, g.pos(cal).offset(1, 0));
    g.act(otto, "release", json!({ "target": cal }));
    assert!(!g.has_condition(cal, "detained"));
    g.step(2);
    g.act(otto, "arrest", json!({ "target": cal }));
    assert!(g.has_condition(cal, "detained"));

    // Two charges. The judge rules the burglary at once: no restitution,
    // and a fine of 1,000 more than Cal has on him.
    g.act(otto, "charge", json!({ "target": cal, "case": robbery }));
    g.act(otto, "charge", json!({ "target": cal, "case": burglary_case }));
    let fine = g.cash(cal) + 1_000;
    assert!(fine <= 30_000, "the greatest fine for a burglary");
    let ruling = json!({
        "case": burglary_case, "guilty": true, "fine": fine, "restitution": 0, "jail_ticks": 60,
    });
    g.act(judy, "rule_case", ruling);
    g.step(2);
    assert_eq!(g.cash(cal), 0);
    let owed = g.section(cal, "law").expect("a law section")["debts"]["owed"].clone();
    assert_eq!(owed, 1_000);
    // A gift reaches an inmate. Ben brings the money, and Cal pays.
    g.walk(ben, &jail);
    g.walk_to(ben, g.pos(cal).offset(1, 0));
    g.act(ben, "give", json!({ "to": cal, "money": 1_000 }));
    assert_eq!(g.act(cal, "pay_fine", json!({})), json!({ "paid": 1_000, "owed": 0 }));
    robbery
}

/// Day 9: the jury of the robbery votes.
pub fn court(g: &mut Game, tour: &Tour, robbery: u64) {
    let cal = tour.cast.citizens[2];
    // The city has a judge, so the court waits four hours for a ruling.
    // Then it calls a jury.
    let charged = g
        .events_of("crime.suspect_charged")
        .into_iter()
        .find(|event| event.data["case"] == robbery)
        .map(|event| event.frame)
        .expect("the robbery was charged");
    if g.frame() < charged + 241 {
        g.run_to_frame(charged + 241);
    }
    let summoned = g.last_event("crime.jury_summoned");
    assert_eq!((summoned.frame, summoned.data["case"].as_u64()), (charged + 240, Some(robbery)));
    let jurors: Vec<ResidentId> = tour
        .cast
        .everyone()
        .into_iter()
        .filter(|resident| {
            g.section(*resident, "law").unwrap_or_default()["jury_duty"]
                .as_array()
                .is_some_and(|duties| duties.iter().any(|duty| duty["case"] == robbery))
        })
        .collect();
    assert_eq!(jurors.len(), 5, "a jury of five");
    assert!(!jurors.contains(&cal));
    g.each(&jurors, "jury_vote", json!({ "case": robbery, "guilty": true }));
    g.step(2);
    let verdict = g.last_event("crime.verdict_given");
    assert_eq!(
        (
            verdict.data["case"].as_u64(),
            verdict.data["by"].as_str(),
            verdict.data["votes_guilty"].as_u64()
        ),
        (Some(robbery), Some("jury"), Some(5))
    );
    assert!(g.has_condition(cal, "jailed"));
}
