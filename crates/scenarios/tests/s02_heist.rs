//! Story 2: a heist.
//!
//! A burglar empties half the till of a shop at night. The owner and a
//! witness report the crime. A police officer investigates, searches the
//! suspect and arrests it. The officer has its powers from the mayor, and
//! the mayor has its office from an election. A judge convicts the
//! burglar. The engine takes the restitution and the fine, and holds the
//! burglar in jail until the tick at which the sentence ends.
//!
//! The story starts in the republic (`unsup_scenarios::republic`): the
//! election and the appointments are part of its founding. The president
//! pays the shop of the victim from the national treasury.

use serde_json::{Value, json};
use unsup_kernel::{CityId, ErrorCode, Holder, Pos};
use unsup_scenarios::republic::grant;
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic_in_republic};

const CITY: CityId = CityId(0);

/// The verbs that work under arrest, in jail and in hospital: the body,
/// speech, reads, the law, the bank account and the three kernel verbs.
const INMATE_VERBS: [&str; 26] = [
    "cancel",
    "cancel_shares",
    "case_list",
    "case_view",
    "contract_view",
    "eat",
    "inspect",
    "leaderboard",
    "list_jobs",
    "list_property",
    "list_recipes",
    "loan_repay",
    "message",
    "mission_list",
    "org_say",
    "pay_fine",
    "report_crime",
    "rest",
    "say",
    "set_routines",
    "shares_book",
    "sleep",
    "transfer",
    "use_item",
    "wait",
    "write_will",
];

fn law(g: &Game, r: unsup_kernel::ResidentId) -> Value {
    g.section(r, "law").unwrap_or_default()
}

fn story(g: &mut Game, republic: &Republic) {
    let [ann, ben, cal, _] = republic.citizens;
    let (otto, judy) = (republic.officer, republic.judge);
    let treasury_at_start = g.city_treasury(CITY);

    // The officer has its powers from the appointment by the elected mayor.
    assert_eq!(
        g.section(republic.mayor, "politics").unwrap_or_default()["offices"],
        json!([{ "office": "mayor", "city": 0 }])
    );
    assert_eq!(
        g.section(otto, "politics").unwrap_or_default()["offices"],
        json!([{ "office": "police_officer", "city": 0 }])
    );
    assert_eq!(law(g, otto)["powers_here"], json!(["law.arrest", "law.search"]));
    assert_eq!(law(g, cal), json!(null), "a resident without an office has no powers");

    // Ann buys the cheapest shop of the city with a grant of the nation,
    // opens it and puts 20,000 in the till.
    let shop = g
        .sites("shop")
        .into_iter()
        .min_by_key(|shop| (shop.attr("value"), shop.id))
        .expect("a shop");
    let price = shop.attr("value");
    assert_eq!(price, 4_500_000);
    let own = g.cash(ann);
    grant(g, republic, ann, price + 5_000 + 20_000);
    g.walk(ann, &shop);
    assert_eq!(g.act(ann, "buy_property", json!({}))["price"], price);
    assert_eq!(g.act(ann, "open_business", json!({}))["fee"], 5_000);
    assert_eq!(g.act(ann, "till_fund", json!({ "amount": 20_000 }))["till"], 20_000);
    assert_eq!(g.cash(ann), own);
    let till = Holder::Site(shop.id);

    // The night of day 6. Ann sleeps in the shelter. Cal stands in the
    // street in front of the shop. Ben walks in.
    g.run_to(6, 22, 30);
    g.walk(ann, &g.site("shelter"));
    g.walk_to(cal, Pos::new(shop.rect.x1, shop.rect.y0 + 2));
    g.walk(ben, &shop);
    // At the start of day 7 the shop pays its property tax from the till:
    // 5 basis points of its value.
    g.run_to(7, 1, 0);
    assert_eq!(g.money(till), 20_000 - price * 5 / 10_000);
    let in_till = g.money(till);
    let ben_before = g.cash(ben);

    // A burglary can fail. A failed one is a crime too. Ben tries until he
    // has the cash: half of the till.
    let mut attempts = Vec::new();
    let burglary = loop {
        let result = g.act(ben, "burgle", json!({}));
        assert_eq!(result["site"], json!(shop.id));
        if result["success"] == true {
            break result;
        }
        assert_eq!((result["cash"].as_i64(), result["value"].as_i64()), (Some(0), Some(0)));
        attempts.push(result["case"].as_u64().expect("a case"));
        assert!(attempts.len() < 8, "eight burglaries failed in a row");
    };
    let case = burglary["case"].as_u64().expect("a case");
    let loot = in_till / 2;
    assert_eq!(loot, 8_875);
    assert_eq!((burglary["cash"].as_i64(), burglary["value"].as_i64()), (Some(loot), Some(loot)));
    assert_eq!((g.money(till), g.cash(ben)), (in_till - loot, ben_before + loot));
    // Viewers see a theft without a name. The loot is hot.
    let seen = g.last_event("crime.theft_committed");
    assert_eq!(seen.actor, None);
    assert_eq!(
        seen.data,
        json!({ "case": case, "kind": "burglary", "city": 0, "site": shop.id, "value": loot, "attempt": false })
    );
    assert_eq!(law(g, ben)["hot"], json!({ "money": loot, "items": {} }));

    // The owner is told, wherever she is. She did not see the burglar.
    g.step(1);
    let told = g.notice(ann, "crime.witnessed");
    assert_eq!(
        told.data,
        json!({ "case": case, "kind": "burglary", "role": "victim", "site": shop.id, "value": loot })
    );
    let reported = g.act(ann, "report_crime", json!({ "case": case }));
    assert_eq!(reported, json!({ "case": case, "reported": true, "named_suspect": false }));
    // The witness in the street recognized him. Its report names him.
    let known = law(g, cal)["known_cases"]
        .as_array()
        .expect("known cases")
        .iter()
        .find(|entry| entry["case"] == case)
        .cloned()
        .expect("the witness knows the case");
    assert_eq!((known["role"].as_str(), known["suspect"].as_u64()), (Some("witness"), Some(ben.0)));
    let reported = g.act(cal, "report_crime", json!({ "case": case }));
    assert_eq!(reported, json!({ "case": case, "reported": true, "named_suspect": true }));
    assert_eq!(g.refused(cal, "report_crime", json!({ "case": case })), ErrorCode::Conflict);
    // A resident that saw nothing cannot report.
    assert_eq!(g.refused(judy, "report_crime", json!({ "case": case })), ErrorCode::NotFound);

    // The officer reads the docket: one witness, 25 points, no warrant.
    g.step(1);
    let docket = g.act(otto, "case_view", json!({ "case": case }));
    assert_eq!(
        (docket["status"].as_str(), docket["evidence"].as_i64(), docket.get("suspect")),
        (Some("open"), Some(25), None)
    );
    assert_eq!(docket["thresholds"], json!({ "charge": 20, "warrant": 40, "convict": 60 }));
    // Without a warrant nobody can be arrested, and only an officer can
    // arrest at all.
    g.walk_to(otto, g.pos(ben));
    assert_eq!(g.refused(otto, "arrest", json!({ "target": ben })), ErrorCode::NotAllowed);
    assert_eq!(g.refused(cal, "investigate", json!({ "case": case })), ErrorCode::NotAllowed);

    // The officer reads the trace in the shop until it finds something.
    let mut found = json!(null);
    for _ in 0..8 {
        found = g.act(otto, "investigate", json!({ "case": case }));
        if found["found"] == true {
            break;
        }
    }
    assert_eq!(found["found"], true, "eight investigations found nothing");
    let points = found["evidence"].as_i64().expect("points");
    assert!((1..=30).contains(&points), "{points} points");
    // Testimony and the trace together give a warrant. The docket names
    // the suspect now, and the suspect knows.
    g.step(1);
    let docket = g.act(otto, "case_view", json!({ "case": case }));
    assert_eq!(
        (docket["status"].as_str(), docket["evidence"].as_i64(), docket["suspect"].as_u64()),
        (Some("wanted"), Some(25 + points), Some(ben.0))
    );
    assert!(g.has_condition(ben, "wanted"));
    assert_eq!(law(g, ben)["warrants"], json!([{ "case": case, "kind": "burglary" }]));
    assert_eq!(
        g.last_event("crime.warrant_issued").data,
        json!({ "case": case, "kind": "burglary", "city": 0 })
    );

    // Ben walks away to the park. The officer follows, searches him and
    // finds the loot: it goes to the police station.
    g.walk(ben, &g.site("park"));
    g.walk_to(otto, g.pos(ben).offset(1, 0));
    let search = g.act(otto, "search", json!({ "target": ben }));
    assert_eq!(
        search,
        json!({ "target": ben, "found": true, "money": loot, "goods": {}, "contraband": {}, "cases": [case] })
    );
    g.step(1);
    let station = Holder::Site(g.site("police_station").id);
    assert_eq!((g.cash(ben), g.money(station)), (ben_before, loot));

    // The arrest. Ben is in the jail of the city at once, in another
    // district, and he is detained.
    let arrest = g.act(otto, "arrest", json!({ "target": ben }));
    let jail = g.site("jail");
    assert_eq!(arrest, json!({ "target": ben, "cases": [case], "jail": jail.id }));
    assert_eq!(g.site_of(ben).map(|site| site.id), Some(jail.id));
    assert_ne!(g.zone_of(ben), g.zone_of(otto));
    assert!(g.has_condition(ben, "detained") && !g.has_condition(ben, "wanted"));
    assert_eq!(
        law(g, ben)["custody"],
        json!({ "state": "detained", "site": jail.id, "arrested_by": otto })
    );
    let made = g.last_event("crime.arrest_made");
    assert_eq!(made.actor, Some(otto));
    assert_eq!(
        made.data,
        json!({ "officer": otto, "target": ben, "case": case, "cases": [case], "city": 0, "watch": false })
    );
    // Under arrest only the verbs of an inmate work.
    assert_eq!(g.allowed_verbs(ben), INMATE_VERBS);

    // The charge starts a trial of at most 720 minutes.
    let charge = g.act(otto, "charge", json!({ "target": ben, "case": case }));
    assert_eq!(charge, json!({ "case": case, "target": ben, "decided_in": 720 }));
    // Only a judge can rule, and only inside the limits of a burglary.
    assert_eq!(
        g.refused(otto, "rule_case", json!({ "case": case, "guilty": true })),
        ErrorCode::NotAllowed
    );
    let too_long = json!({ "case": case, "guilty": true, "jail_ticks": 4_321 });
    assert_eq!(g.refused(judy, "rule_case", too_long), ErrorCode::InvalidArgs);
    // The judge convicts: 300 minutes of jail and a fine of 1,000. The
    // restitution is the loss of the victim.
    let ann_before = g.cash(ann);
    let ruling = g.act(
        judy,
        "rule_case",
        json!({ "case": case, "guilty": true, "jail_ticks": 300, "fine": 1_000 }),
    );
    assert_eq!(
        ruling,
        json!({ "case": case, "guilty": true, "fine": 1_000, "jail_ticks": 300, "restitution": loot })
    );
    g.step(2);
    let verdict = g.notice(ben, "crime.verdict");
    let until = verdict.data["jailed_until"].as_u64().expect("the end of the sentence");
    assert_eq!(
        verdict.data,
        json!({
            "case": case, "kind": "burglary", "guilty": true, "fine": 1_000, "restitution": loot,
            "jail_ticks": 300, "paid": loot + 1_000, "owed": 0, "jailed_until": until,
        })
    );
    // The engine took the restitution and the fine from his own cash.
    assert_eq!(g.cash(ben), ben_before - loot - 1_000);
    assert_eq!(g.cash(ann), ann_before + loot);
    assert_eq!(g.notice(ann, "crime.restitution").data, json!({ "case": case, "amount": loot }));
    let given = g.last_event("crime.verdict_given");
    assert_eq!(
        (given.data["by"].as_str(), given.data["judge"].as_u64(), given.data["guilty"].as_bool()),
        (Some("judge"), Some(judy.0), Some(true))
    );
    // The conviction is on his name.
    assert_eq!((g.attr(ben, "rep.notoriety"), g.attr(ben, "rep.standing")), (40, -60));

    // Jail. The sentence ends 300 ticks after the verdict reached the zone
    // of the jail.
    assert!(g.has_condition(ben, "jailed") && !g.has_condition(ben, "detained"));
    let tick = |g: &Game| g.world.with_resident(ben, |realm, _| realm.tick().0);
    let custody = law(g, ben)["custody"].clone();
    assert_eq!(
        custody,
        json!({ "state": "jailed", "until": until, "ticks_left": until - tick(g) })
    );
    assert_eq!(until - tick(g), 300 - 2);
    assert_eq!(g.allowed_verbs(ben), INMATE_VERBS);
    assert_eq!(g.refused(ben, "move_to", json!({ "site": shop.id })), ErrorCode::Blocked);
    // An inmate can talk, and the jail feeds it.
    assert_eq!(
        g.act(ben, "message", json!({ "to": cal, "text": "You saw nothing." }))["delivered"],
        false
    );

    // One tick before the end he is an inmate. At the tick he is free.
    let left = until - tick(g);
    g.step(left - 1);
    assert_eq!(tick(g), until - 1);
    assert!(g.has_condition(ben, "jailed"));
    g.step(1);
    assert_eq!(tick(g), until);
    assert!(!g.has_condition(ben, "jailed"));
    let served = g.last_event("crime.sentence_served");
    assert_eq!(
        (served.frame, served.data.clone()),
        (g.frame(), json!({ "resident": ben, "free": true }))
    );
    assert_eq!(
        g.notice(ben, "crime.released").data,
        json!({ "reason": "sentence_served", "free": true })
    );
    assert_eq!(law(g, ben), json!(null));
    // He walks out of the jail by himself.
    g.walk(ben, &g.site("shelter"));

    // The docket closes the case. The failed attempts stay open.
    let closed = g.act(otto, "case_view", json!({ "case": case }));
    assert_eq!(
        (closed["status"].as_str(), closed["sentence"]["jail_ticks"].as_i64()),
        (Some("convicted"), Some(300))
    );
    for attempt in &attempts {
        assert_eq!(g.act(otto, "case_view", json!({ "case": attempt }))["attempt"], true);
    }

    // The money. The city got the price of the shop, the permit, the
    // property tax and the fine. With the price of the shop it could pay
    // its five officials again at the start of day 7: 25,000. The seized
    // loot is in the till of the police station until the city takes it at
    // the next day.
    g.step(1);
    let tax = price * 5 / 10_000;
    let salaries = 8_000 + 2 * 3_000 + 5_000 + 6_000;
    let paid = g
        .events_of("politics.salaries_paid")
        .into_iter()
        .rfind(|event| event.data["gov"] == "city")
        .map(|event| (event.data["paid"].as_i64(), event.data["total"].as_i64()));
    assert_eq!(paid, Some((Some(5), Some(salaries))));
    assert_eq!(g.city_treasury(CITY), treasury_at_start + price + 5_000 + tax + 1_000 - salaries);
    assert_eq!(g.money(station), loot);
    assert_eq!(g.money(till), in_till - loot);
    g.audit();
}

#[test]
fn a_burglar_is_reported_investigated_arrested_tried_and_jailed() {
    assert_deterministic_in_republic(&Setup::town(1), story);
}
