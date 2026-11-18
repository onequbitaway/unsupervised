//! What a resident can do while it sleeps and while it is in hospital.
//!
//! The heist (story 2) covers the same for an inmate. A blocking condition
//! refuses every verb that does not list it, so the lists here are exact:
//! a new verb must be put in a list or it stays blocked.

use serde_json::json;
use unsup_kernel::protocol::ActionStatus;
use unsup_kernel::{ErrorCode, ResidentId};
use unsup_scenarios::{Game, Setup, assert_deterministic};

/// The verbs that work in a hospital bed: the body, speech, reads, the
/// law, the bank account and the three kernel verbs.
const PATIENT_VERBS: [&str; 26] = [
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

fn activity(g: &Game, r: ResidentId) -> Option<&'static str> {
    g.world.activity(r)
}

fn story(g: &mut Game) {
    let ada = g.spawn("Ada");
    let bob = g.spawn("Bob");
    let hall = g.site("city_hall");
    let depot = g.site("depot");
    g.walk_all(&[ada, bob], &hall);
    g.each(&[ada, bob], "work", json!({}));
    g.act(ada, "work", json!({}));
    g.walk(ada, &depot);
    g.act(ada, "buy", json!({ "item": "vegetables", "qty": 2 }));
    assert_eq!(g.cash(ada), 2 * 2_520 - 1_500);

    // ---- Asleep ----------------------------------------------------------
    // Ada sleeps in the shelter. Sleep is not a blocking condition: no
    // verb is refused because of it.
    let shelter = g.site("shelter");
    g.walk(ada, &shelter);
    let sleep = g.submit(ada, "sleep", json!({ "ticks": 300 }));
    assert!(matches!(sleep.status, ActionStatus::Started { ticks: 300, .. }));
    g.step(5);
    assert!(g.has_condition(ada, "asleep"));
    assert_eq!(activity(g, ada), Some("sleep"));

    // A free verb runs at once, in her sleep: she can look, talk, read the
    // job board and change her will and her routines.
    assert_eq!(g.act(ada, "inspect", json!({}))["site"]["kind"], "shelter");
    assert_eq!(
        g.act(ada, "say", json!({ "text": "Zzz." }))["heard_by"].as_u64().map(|_| ()),
        Some(())
    );
    assert_eq!(g.act(ada, "write_will", json!({ "heir": bob })), json!({ "heir": bob }));
    assert_eq!(g.act(ada, "list_jobs", json!({}))["public_works"]["wage"], 2_800);
    g.step(1);
    assert_eq!(g.act(ada, "set_routines", json!({ "routines": [] })), json!({ "routines": 0 }));
    // She sees one tile only: Bob, three tiles away in the same room, is
    // out of her sight.
    g.walk_to(bob, g.pos(ada).offset(3, 0));
    assert_eq!(g.refused(ada, "inspect", json!({ "resident": bob })), ErrorCode::NotHere);
    assert_eq!(activity(g, ada), Some("sleep"), "the free verbs did not wake her");

    // A verb that takes time waits in the queue until she wakes up.
    let queued = g.submit(ada, "eat", json!({}));
    assert!(matches!(queued.status, ActionStatus::Queued { position: 1 }), "{:?}", queued.status);
    g.step(3);
    assert_eq!(g.carried(ada, "vegetables"), 2, "nobody eats in her sleep");
    // `cancel` wakes her. She gets the energy of the time that she slept,
    // and the queue is empty.
    let slept = g.frame() - g.events_of("economy.goods_sold")[0].frame;
    assert!(slept > 0);
    let energy = g.attr(ada, "need.energy");
    let woke = g.act(ada, "cancel", json!({}));
    assert_eq!(
        (woke["cancelled"].as_u64(), woke["cleared"].as_u64()),
        (Some(sleep.action_id), Some(1))
    );
    assert!(!g.has_condition(ada, "asleep"));
    assert_eq!(activity(g, ada), None);
    assert!(g.attr(ada, "need.energy") > energy, "the sleep that was cut short gave energy");
    // Awake, the same request runs at once.
    assert_eq!(g.act(ada, "eat", json!({}))["item"], "vegetables");
    assert_eq!(g.carried(ada, "vegetables"), 1);

    // ---- In hospital -------------------------------------------------------
    // A fight costs Bob health. He pays the hospital for 100 points.
    g.walk_to(ada, g.pos(bob).offset(1, 0));
    while g.attr(bob, "need.health") == 1_000 {
        g.act(ada, "assault", json!({ "target": bob }));
    }
    assert_eq!(g.attr(bob, "need.health"), 850);
    let hospital = g.site("hospital");
    g.walk(bob, &hospital);
    let cash = g.cash(bob);
    let stay = g.submit(bob, "heal", json!({ "points": 100 }));
    // 150 points for each hour: 100 points take 40 minutes.
    assert!(matches!(stay.status, ActionStatus::Started { ticks: 40, .. }), "{:?}", stay.status);
    assert_eq!(g.cash(bob), cash - 100 * 20, "the fee is paid at the start");
    g.step(2);
    assert!(g.has_condition(bob, "hospitalized"));

    // The patient can use exactly these verbs. Every other verb answers
    // `blocked` and names the condition.
    let refused = g.submit(bob, "move_to", json!({ "site": shelter.id }));
    match refused.status {
        ActionStatus::Rejected { error } => {
            assert_eq!(error.code, ErrorCode::Blocked);
            assert_eq!(error.details, json!({ "condition": "hospitalized" }));
        }
        other => panic!("a patient walked away: {other:?}"),
    }
    assert_eq!(g.allowed_verbs(bob), PATIENT_VERBS);
    // The probe ended with `cancel`: the patient can leave its bed early.
    // It keeps the health of the minutes that passed and gets no money
    // back.
    assert!(!g.has_condition(bob, "hospitalized"));
    assert_eq!(activity(g, bob), None);
    let health = g.attr(bob, "need.health");
    assert!((850..950).contains(&health), "health {health}");
    assert_eq!(g.cash(bob), cash - 2_000);

    // A stay that runs to its end heals, and the condition ends with it.
    let healed = g.act(bob, "heal", json!({ "points": 20 }));
    assert_eq!(healed["health"], health + 20);
    assert_eq!(g.cash(bob), cash - 2_000 - 400);
    assert!(!g.has_condition(bob, "hospitalized"));
    // The patient that is healed can walk out.
    g.walk(bob, &shelter);
    g.audit();
}

#[test]
fn a_sleeper_and_a_patient_can_do_what_they_need() {
    assert_deterministic(&Setup::town(1), story);
}
