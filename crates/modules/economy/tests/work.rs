//! The verb `work` with a job: the task, the wage, the tax and the skill.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, ResidentId};

const WAGE: i64 = 3_000;
const TAX: i64 = 300;

/// An owner buys a site, funds its till and posts one job. A worker takes
/// the job. Returns the owner, the worker and the job.
fn staffed(
    place: unsup_kernel::SiteId,
    t: &mut Town,
    open: bool,
    task: Option<&str>,
) -> (ResidentId, ResidentId, u64) {
    let owner = t.world.spawn("Owner");
    if open {
        open_site(&mut t.world, owner, place);
    } else {
        buy_site(&mut t.world, owner, place);
    }
    t.world.give_money(owner, Money(10_000));
    ok(&mut t.world, owner, "till_fund", json!({ "amount": 10_000 }));
    let job = ok(
        &mut t.world,
        owner,
        "post_job",
        json!({ "wage": WAGE, "slots": 2, "task": task }),
    )["job"]
        .as_u64()
        .expect("a job id");
    let worker = resident_at(&mut t.world, "Worker", 0, place);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    (owner, worker, job)
}

#[test]
fn a_shift_at_a_farm_gathers_for_the_owner_and_pays_the_wage() {
    let mut t = Town::new();
    let (_, worker, job) = staffed(t.at.farm, &mut t, false, None);
    let treasury_before = treasury(&t.world);

    let ack = t.world.act_ok(worker, "work", json!({}));
    assert_eq!(
        serde_json::to_value(&ack).expect("json")["ticks"],
        240,
        "a shift lasts `economy.shift_minutes`"
    );
    let events = settle(&mut t.world, worker);
    let report = t.world.last_report(worker).expect("a report");
    assert!(report.ok, "{:?}", report.error);
    assert_eq!(
        report.result,
        json!({
            "site": t.at.farm,
            "job": job,
            "wage": WAGE,
            "tax": TAX,
            "net": WAGE - TAX,
            "skill": "farming",
            "xp": 40,
            "produced": [{ "item": "grain", "qty": 24 }],
        })
    );
    // The wage in cash, the tax by wire to the city, both out of the till.
    assert_eq!(cash(&t.world, worker), WAGE - TAX);
    assert_eq!(till(&t.world, t.at.farm), 10_000 - WAGE);
    t.world.step(1);
    assert_eq!(treasury(&t.world), treasury_before + TAX);
    // Six units of grain an hour for four hours, into the stock of the farm.
    assert_eq!(t.world.qty(Holder::Site(t.at.farm), "grain"), 24);
    assert_eq!(t.world.qty(Holder::Resident(worker), "grain"), 0);
    // Ten experience points an hour; twenty energy points an hour.
    assert_eq!(t.world.attr(worker, "skill.farming"), 40);
    assert_eq!(t.world.attr(worker, "need.energy"), 1_000 - 80);

    assert_eq!(
        of_kind(&events, "economy.shift_completed"),
        [json!({
            "resident": worker, "site": t.at.farm, "wage": WAGE, "tax": TAX, "skill": "farming",
        })]
    );
    assert_eq!(
        of_kind(&events, "economy.goods_produced"),
        [json!({ "resident": worker, "site": t.at.farm, "item": "grain", "qty": 24 })]
    );
    // Routine events have a low drama score, and the place of the worker.
    assert_eq!(drama_of(&events, "economy.shift_completed"), [2]);
    let event = events.iter().find(|e| e.kind == "economy.shift_completed").expect("the event");
    assert_eq!(
        (event.actor, event.pos, event.public),
        (Some(worker), Some(t.world.pos(worker)), true)
    );

    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", FARM_VALUE + 10_000)]));
    assert_eq!(flows.burned, map([]));
    assert_eq!(flows.produced, map([("nature.harvest:grain", 24)]));
    assert_eq!(flows.consumed, map([]));
}

#[test]
fn a_job_can_name_the_good_to_gather() {
    let mut t = Town::new();
    let (_, worker, _) = staffed(t.at.farm, &mut t, false, Some("cotton"));
    let result = ok(&mut t.world, worker, "work", json!({}));
    // Five units of cotton an hour.
    assert_eq!(result["produced"], json!([{ "item": "cotton", "qty": 20 }]));
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("nature.harvest:cotton", 20)], [])
    );
}

#[test]
fn a_shift_at_a_workshop_runs_the_recipe_with_the_stock_of_the_site() {
    let mut t = Town::new();
    let (owner, worker, _) = staffed(t.at.workshop, &mut t, true, Some("cloth"));
    t.world.give_item(owner, "cotton", 30);
    let stock = |n: i64| json!({ "item": "cotton", "qty": n });

    // Without cotton in the stock the shift does not start.
    let error = t.world.act_err(worker, "work", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details["missing"]["cotton"], json!({ "needed": 2, "available": 0 }));

    // Ten units of cotton feed five batches. A shift has time for eight.
    ok(&mut t.world, owner, "stock", stock(10));
    let (result, events) = ok_with_events(&mut t.world, worker, "work", json!({}));
    assert_eq!(result["produced"], json!([{ "item": "cloth", "qty": 5 }]));
    assert_eq!(result["skill"], "crafting");
    assert_eq!(t.world.qty(Holder::Site(t.at.workshop), "cotton"), 0);
    assert_eq!(t.world.qty(Holder::Site(t.at.workshop), "cloth"), 5);
    assert_eq!(
        of_kind(&events, "economy.goods_produced"),
        [json!({ "resident": worker, "site": t.at.workshop, "item": "cloth", "qty": 5 })]
    );

    // With enough cotton: eight batches of 30 minutes in 240 minutes.
    ok(&mut t.world, owner, "stock", stock(20));
    let result = ok(&mut t.world, worker, "work", json!({}));
    assert_eq!(result["produced"], json!([{ "item": "cloth", "qty": 8 }]));
    assert_eq!(t.world.qty(Holder::Site(t.at.workshop), "cotton"), 4);

    let flows = flows(&t.world);
    assert_eq!(flows.produced, map([("kernel.grant:cotton", 30), ("production.output:cloth", 13)]));
    assert_eq!(
        flows.of_module(),
        module_flows([], [], [("production.output:cloth", 13)], [("production.input:cotton", 26)])
    );
}

#[test]
fn a_skilled_worker_makes_more_batches_in_a_shift() {
    let mut t = Town::new();
    let (owner, worker, _) = staffed(t.at.workshop, &mut t, true, Some("cloth"));
    t.world.give_item(owner, "cotton", 30);
    ok(&mut t.world, owner, "stock", json!({ "item": "cotton", "qty": 30 }));
    // Level 4 is 20 percent faster: 240 * 1.2 / 30 = 9.6 batches.
    t.world.set_attr(worker, "skill.crafting", 1_600);
    let result = ok(&mut t.world, worker, "work", json!({}));
    assert_eq!(result["produced"], json!([{ "item": "cloth", "qty": 9 }]));
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("production.output:cloth", 9)], [("production.input:cotton", 18)])
    );
}

#[test]
fn a_shift_at_a_shop_pays_and_trains_but_makes_nothing() {
    let mut t = Town::new();
    let (_, worker, _) = staffed(t.at.shop, &mut t, true, None);
    let result = ok(&mut t.world, worker, "work", json!({}));
    assert_eq!(result["produced"], json!([]));
    assert_eq!(result["skill"], "trading");
    assert_eq!(t.world.attr(worker, "skill.trading"), 40);
    assert_eq!(cash(&t.world, worker), WAGE - TAX);
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", SHOP_VALUE + 5_000 + 10_000)]));
    assert_eq!(flows.of_module(), Flows::default());
}

#[test]
fn mood_changes_the_harvest_and_the_experience() {
    let mut t = Town::new();
    let (_, worker, _) = staffed(t.at.farm, &mut t, false, None);
    // Mood 0 gives one half (`economy.mood_floor_bp`).
    t.world.set_attr(worker, "need.mood", 0);
    let result = ok(&mut t.world, worker, "work", json!({}));
    assert_eq!(result["produced"], json!([{ "item": "grain", "qty": 12 }]));
    assert_eq!(result["xp"], 20);
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("nature.harvest:grain", 12)], [])
    );
}

#[test]
fn a_till_that_is_short_refuses_the_shift_at_the_start() {
    let mut t = Town::new();
    let (owner, worker, _) = staffed(t.at.farm, &mut t, false, None);
    ok(&mut t.world, owner, "till_collect", json!({ "amount": 10_000 - WAGE + 1 }));

    let error = t.world.act_err(worker, "work", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": WAGE, "available": WAGE - 1 }));
    assert_eq!(cash(&t.world, worker), 0);
    assert_only_transfers(&t.world);
}

#[test]
fn a_till_that_runs_short_during_the_shift_fails_it_and_the_worker_sees_it() {
    let mut t = Town::new();
    let (owner, worker, _) = staffed(t.at.farm, &mut t, false, None);
    t.world.act_ok(worker, "work", json!({}));
    // The owner empties the till while the worker works.
    ok(&mut t.world, owner, "till_collect", json!({}));
    assert_eq!(till(&t.world, t.at.farm), 0);

    let events = settle(&mut t.world, worker);
    let report = t.world.last_report(worker).expect("a report");
    assert!(!report.ok);
    let error = report.error.expect("an error");
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": WAGE, "available": 0 }));
    // No pay, no goods for the owner, no experience.
    assert_eq!(cash(&t.world, worker), 0);
    assert_eq!(t.world.qty(Holder::Site(t.at.farm), "grain"), 0);
    assert_eq!(t.world.attr(worker, "skill.farming"), 0);
    assert_eq!(
        of_kind(&events, "economy.shift_unpaid"),
        [json!({ "resident": worker, "site": t.at.farm, "wage": WAGE })]
    );
    assert_eq!(drama_of(&events, "economy.shift_unpaid"), [35]);
    assert_eq!(of_kind(&events, "economy.shift_completed"), Vec::<serde_json::Value>::new());
    assert_only_transfers(&t.world);
}

#[test]
fn work_refuses_what_it_must() {
    let mut t = Town::new();
    let (owner, worker, _) = staffed(t.at.farm, &mut t, false, None);

    // Not at a site, and at a site without a job for this resident.
    t.world.set_pos(worker, unsup_kernel::Pos::new(60, 60));
    assert_eq!(refused(&mut t.world, worker, "work", json!({})), ErrorCode::NotHere);
    go(&mut t.world, worker, t.at.mine);
    assert_eq!(refused(&mut t.world, worker, "work", json!({})), ErrorCode::NotAllowed);
    // The owner has no job at its own farm.
    assert_eq!(refused(&mut t.world, owner, "work", json!({})), ErrorCode::NotAllowed);

    // Too tired.
    go(&mut t.world, worker, t.at.farm);
    t.world.set_attr(worker, "need.energy", 49);
    let error = t.world.act_err(worker, "work", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 50, "available": 49 }));
    t.world.set_attr(worker, "need.energy", 50);
    t.world.act_ok(worker, "work", json!({}));
    assert_only_transfers(&t.world);
}

#[test]
fn a_worker_that_is_fired_during_the_shift_gets_no_wage_and_the_owner_no_goods() {
    let mut t = Town::new();
    let (owner, worker, _) = staffed(t.at.farm, &mut t, false, None);
    t.world.act_ok(worker, "work", json!({}));
    ok(&mut t.world, owner, "fire", json!({ "resident": worker }));
    settle(&mut t.world, worker);
    let report = t.world.last_report(worker).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::Conflict);
    assert_eq!(cash(&t.world, worker), 0);
    assert_eq!(t.world.qty(Holder::Site(t.at.farm), "grain"), 0);
    assert_eq!(till(&t.world, t.at.farm), 10_000);
    assert_only_transfers(&t.world);
}

#[test]
fn a_worker_that_leaves_the_site_during_the_shift_gets_no_wage() {
    let mut t = Town::new();
    let (_, worker, _) = staffed(t.at.farm, &mut t, false, None);
    t.world.act_ok(worker, "work", json!({}));
    t.world.step(10);
    t.world.set_pos(worker, unsup_kernel::Pos::new(60, 60));
    settle(&mut t.world, worker);
    let report = t.world.last_report(worker).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::NotHere);
    assert_eq!(cash(&t.world, worker), 0);
    assert_eq!(till(&t.world, t.at.farm), 10_000);
    assert_only_transfers(&t.world);
}
