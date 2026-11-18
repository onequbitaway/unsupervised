//! Employment: `post_job`, `close_job`, `apply_job`, `quit_job`, `fire`.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, OrgId};

/// A farm with an owner that has posted one job with two slots.
fn farm_with_job(t: &mut Town) -> (unsup_kernel::ResidentId, u64) {
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.farm);
    let result = ok(&mut t.world, owner, "post_job", json!({ "wage": 3_000, "slots": 2 }));
    (owner, result["job"].as_u64().expect("a job id"))
}

#[test]
fn an_owner_posts_a_job_and_a_resident_takes_it() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.farm);

    let (result, events) =
        ok_with_events(&mut t.world, owner, "post_job", json!({ "wage": 3_000, "slots": 2 }));
    let job = result["job"].as_u64().expect("a job id");
    assert_eq!(
        result,
        json!({ "job": job, "site": t.at.farm, "skill": "farming", "min_level": 0 })
    );
    assert_eq!(
        of_kind(&events, "economy.job_posted"),
        [json!({ "job": job, "site": t.at.farm, "city": 0, "wage": 3_000, "slots": 2 })]
    );

    let worker = resident_at(&mut t.world, "Worker", 0, t.at.farm);
    let (result, events) = ok_with_events(&mut t.world, worker, "apply_job", json!({ "job": job }));
    assert_eq!(result, json!({ "job": job, "site": t.at.farm, "wage": 3_000, "skill": "farming" }));
    assert_eq!(
        of_kind(&events, "economy.worker_hired"),
        [json!({ "resident": worker, "site": t.at.farm, "job": job, "wage": 3_000 })]
    );
    // The worker sees its job, and the owner got a notice.
    let seen = section(&t.world, worker, "job").expect("a job section");
    assert_eq!(seen["job"], json!(job));
    assert_eq!(seen["wage"], 3_000);
    assert_eq!(seen["kind"], "farm");
    assert_eq!(seen["shift_minutes"], 240);
    let notices = t.world.notices(owner);
    assert!(notices.iter().any(|n| n.kind == "economy.worker_hired" && n.from == Some(worker)));
    assert_only_transfers(&t.world);
}

#[test]
fn post_job_refuses_what_it_must() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    let good = json!({ "wage": 3_000, "slots": 1 });

    // Not at a site.
    assert_eq!(refused(&mut t.world, owner, "post_job", good.clone()), ErrorCode::NotHere);
    // At a site of somebody else.
    go(&mut t.world, owner, t.at.farm);
    assert_eq!(refused(&mut t.world, owner, "post_job", good.clone()), ErrorCode::NotAllowed);

    buy_site(&mut t.world, owner, t.at.farm);
    for bad in [json!({ "wage": 0, "slots": 1 }), json!({ "wage": 10_000_001, "slots": 1 })] {
        assert_eq!(refused(&mut t.world, owner, "post_job", bad), ErrorCode::InvalidArgs);
    }
    for bad in [json!({ "wage": 3_000, "slots": 0 }), json!({ "wage": 3_000, "slots": 51 })] {
        assert_eq!(refused(&mut t.world, owner, "post_job", bad), ErrorCode::InvalidArgs);
    }
    // A farm gives grain, not ore, and not a thing that does not exist.
    let ore = json!({ "wage": 3_000, "slots": 1, "task": "ore" });
    assert_eq!(refused(&mut t.world, owner, "post_job", ore), ErrorCode::InvalidArgs);
    let nothing = json!({ "wage": 3_000, "slots": 1, "task": "unobtainium" });
    assert_eq!(refused(&mut t.world, owner, "post_job", nothing), ErrorCode::NotFound);

    // The minimum wage is a policy parameter.
    t.world.set_param("economy.min_wage", 4_000);
    t.world.step(1);
    assert_eq!(refused(&mut t.world, owner, "post_job", good.clone()), ErrorCode::NotAllowed);
    t.world.set_param("economy.min_wage", 0);
    t.world.step(1);

    // At most `economy.site_jobs_max` jobs at one site.
    t.world.set_param("economy.site_jobs_max", 1);
    t.world.step(1);
    ok(&mut t.world, owner, "post_job", good.clone());
    assert_eq!(refused(&mut t.world, owner, "post_job", good), ErrorCode::Conflict);
    assert_only_transfers(&t.world);
}

#[test]
fn post_job_at_a_business_needs_a_permit_and_a_recipe_that_fits() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.workshop);
    let tools = json!({ "wage": 3_000, "slots": 1, "task": "tools" });
    // No permit yet.
    assert_eq!(refused(&mut t.world, owner, "post_job", tools.clone()), ErrorCode::NotAllowed);

    t.world.give_money(owner, unsup_kernel::Money(5_000));
    ok(&mut t.world, owner, "open_business", json!({}));
    let result = ok(&mut t.world, owner, "post_job", tools);
    assert_eq!(result["skill"], "crafting");
    // Tools need level 1 in crafting, so the job needs it too.
    assert_eq!(result["min_level"], 1);
    let novice = resident_at(&mut t.world, "Novice", 0, t.at.workshop);
    let apply = json!({ "job": result["job"] });
    assert_eq!(refused(&mut t.world, novice, "apply_job", apply.clone()), ErrorCode::NotAllowed);
    t.world.set_attr(novice, "skill.crafting", 100);
    ok(&mut t.world, novice, "apply_job", apply);
    // `meal` is a recipe of a restaurant; `nothing` is no recipe.
    let meal = json!({ "wage": 3_000, "slots": 1, "task": "meal" });
    assert_eq!(refused(&mut t.world, owner, "post_job", meal), ErrorCode::InvalidArgs);
    let nothing = json!({ "wage": 3_000, "slots": 1, "task": "nothing" });
    assert_eq!(refused(&mut t.world, owner, "post_job", nothing), ErrorCode::NotFound);
    // One batch of furniture takes 120 minutes. It does not fit a shift of 100.
    t.world.set_param("economy.shift_minutes", 100);
    t.world.step(1);
    let furniture = json!({ "wage": 3_000, "slots": 1, "task": "furniture" });
    assert_eq!(refused(&mut t.world, owner, "post_job", furniture), ErrorCode::InvalidArgs);
    assert_only_transfers(&t.world);
}

#[test]
fn a_member_with_job_hire_posts_for_an_organization() {
    let mut t = Town::new();
    let firm = OrgId(5_000_000);
    let boss = t.world.spawn("Boss");
    t.world.with_resident(boss, |realm, _| realm.grant(boss, firm, "owner", &["site.manage"]));
    t.world.give_money(boss, unsup_kernel::Money(FARM_VALUE));
    go(&mut t.world, boss, t.at.farm);
    ok(&mut t.world, boss, "buy_property", json!({ "org": firm }));
    assert_eq!(site(&t.world, t.at.farm).owner, Some(Holder::Org(firm)));

    let clerk = resident_at(&mut t.world, "Clerk", 0, t.at.farm);
    let post = json!({ "wage": 2_500, "slots": 1 });
    assert_eq!(refused(&mut t.world, clerk, "post_job", post.clone()), ErrorCode::NotAllowed);
    t.world.with_resident(clerk, |realm, _| realm.grant(clerk, firm, "clerk", &["job.hire"]));
    let job = ok(&mut t.world, clerk, "post_job", post)["job"].as_u64().expect("a job id");
    // `job.hire` is enough to close the job too, but not to take the stock.
    assert_eq!(
        refused(&mut t.world, clerk, "unstock", json!({ "item": "grain", "qty": 1 })),
        ErrorCode::NotAllowed
    );
    let closed = ok(&mut t.world, clerk, "close_job", json!({ "job": job }));
    assert_eq!(closed, json!({ "job": job, "released": 0 }));
    assert_only_transfers(&t.world);
}

#[test]
fn apply_job_refuses_what_it_must() {
    let mut t = Town::new();
    let (owner, job) = farm_with_job(&mut t);
    let a = t.world.spawn("A");

    // No such job, and not at the site of the job.
    go(&mut t.world, a, t.at.farm);
    assert_eq!(refused(&mut t.world, a, "apply_job", json!({ "job": 99 })), ErrorCode::NotFound);
    go(&mut t.world, a, t.at.mine);
    assert_eq!(refused(&mut t.world, a, "apply_job", json!({ "job": job })), ErrorCode::NotHere);
    t.world.set_pos(a, unsup_kernel::Pos::new(60, 60));
    assert_eq!(refused(&mut t.world, a, "apply_job", json!({ "job": job })), ErrorCode::NotHere);

    // A job with a least level.
    let hard = ok(
        &mut t.world,
        owner,
        "post_job",
        json!({ "wage": 9_000, "slots": 1, "min_level": 2 }),
    )["job"]
        .as_u64()
        .expect("a job id");
    go(&mut t.world, a, t.at.farm);
    assert_eq!(
        refused(&mut t.world, a, "apply_job", json!({ "job": hard })),
        ErrorCode::NotAllowed
    );
    t.world.set_attr(a, "skill.farming", 400);
    ok(&mut t.world, a, "apply_job", json!({ "job": hard }));

    // One job for each resident.
    assert_eq!(refused(&mut t.world, a, "apply_job", json!({ "job": job })), ErrorCode::Conflict);
    // The hard job is full now.
    let b = resident_at(&mut t.world, "B", 0, t.at.farm);
    t.world.set_attr(b, "skill.farming", 400);
    assert_eq!(refused(&mut t.world, b, "apply_job", json!({ "job": hard })), ErrorCode::Conflict);
    assert_only_transfers(&t.world);
}

#[test]
fn two_residents_cannot_both_take_the_last_slot() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.farm);
    let job = ok(&mut t.world, owner, "post_job", json!({ "wage": 3_000, "slots": 1 }))["job"]
        .as_u64()
        .expect("a job id");
    let [a, b] = ["A", "B"].map(|name| resident_at(&mut t.world, name, 0, t.at.farm));
    // Both start in the same tick. The job has one slot.
    t.world.act_ok(a, "apply_job", json!({ "job": job }));
    t.world.act_ok(b, "apply_job", json!({ "job": job }));
    settle(&mut t.world, a);
    settle(&mut t.world, b);
    assert_eq!(section(&t.world, a, "job").expect("a job section")["job"], json!(job));
    assert_eq!(section(&t.world, b, "job"), None);
    let report = t.world.last_report(b).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::Conflict);
    // The second one can take a job elsewhere: it has none.
    assert_eq!(refused(&mut t.world, b, "quit_job", json!({})), ErrorCode::NotFound);
    assert_only_transfers(&t.world);
}

#[test]
fn close_job_releases_the_workers() {
    let mut t = Town::new();
    let (owner, job) = farm_with_job(&mut t);
    let worker = resident_at(&mut t.world, "Worker", 0, t.at.farm);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    let stranger = resident_at(&mut t.world, "Stranger", 0, t.at.farm);

    assert_eq!(refused(&mut t.world, owner, "close_job", json!({ "job": 7 })), ErrorCode::NotFound);
    assert_eq!(
        refused(&mut t.world, stranger, "close_job", json!({ "job": job })),
        ErrorCode::NotAllowed
    );
    // At another site, and in the street.
    go(&mut t.world, owner, t.at.mine);
    assert_eq!(
        refused(&mut t.world, owner, "close_job", json!({ "job": job })),
        ErrorCode::NotHere
    );
    t.world.set_pos(owner, unsup_kernel::Pos::new(60, 60));
    assert_eq!(
        refused(&mut t.world, owner, "close_job", json!({ "job": job })),
        ErrorCode::NotHere
    );
    go(&mut t.world, owner, t.at.farm);

    let (result, events) = ok_with_events(&mut t.world, owner, "close_job", json!({ "job": job }));
    assert_eq!(result, json!({ "job": job, "released": 1 }));
    assert_eq!(
        of_kind(&events, "economy.worker_left"),
        [json!({ "resident": worker, "site": t.at.farm, "job": job, "reason": "closed" })]
    );
    assert_eq!(section(&t.world, worker, "job"), None);
    let notice = t.world.notices(worker).pop().expect("a notice");
    assert_eq!(notice.kind, "economy.job_ended");
    assert_eq!(notice.data, json!({ "job": job, "site": t.at.farm, "reason": "closed" }));
    // The job is gone.
    assert_eq!(
        refused(&mut t.world, worker, "apply_job", json!({ "job": job })),
        ErrorCode::NotFound
    );
    assert_only_transfers(&t.world);
}

#[test]
fn a_worker_quits_at_the_site_and_from_another_zone() {
    let mut t = Town::new();
    let (_, job) = farm_with_job(&mut t);
    let near = resident_at(&mut t.world, "Near", 0, t.at.farm);
    let far = resident_at(&mut t.world, "Far", 0, t.at.farm);
    for worker in [near, far] {
        ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    }
    // What a resident at the farm sees of its jobs.
    let free_slots =
        |t: &Town| section(&t.world, near, "business").and_then(|b| b.get("jobs").cloned());
    assert_eq!(free_slots(&t), None, "a full job is not offered");

    // From the second zone of the city: the zone of the job hears of it.
    go(&mut t.world, far, t.at.far_home);
    let (result, _) = ok_with_events(&mut t.world, far, "quit_job", json!({}));
    assert_eq!(result, json!({ "job": job, "site": t.at.farm }));
    assert_eq!(section(&t.world, far, "job"), None);
    let events = events_of(t.world.step(2));
    assert_eq!(
        of_kind(&events, "economy.worker_left"),
        [json!({ "resident": far, "site": t.at.farm, "job": job, "reason": "quit" })]
    );
    assert_eq!(free_slots(&t).expect("the job has room")[0]["free_slots"], 1);

    // At the site.
    let (_, events) = ok_with_events(&mut t.world, near, "quit_job", json!({}));
    assert_eq!(
        of_kind(&events, "economy.worker_left"),
        [json!({ "resident": near, "site": t.at.farm, "job": job, "reason": "quit" })]
    );
    assert_eq!(drama_of(&events, "economy.worker_left"), [6]);
    assert_eq!(free_slots(&t).expect("the job has room")[0]["free_slots"], 2);
    // No job, nothing to quit.
    assert_eq!(refused(&mut t.world, near, "quit_job", json!({})), ErrorCode::NotFound);
    assert_only_transfers(&t.world);
}

#[test]
fn an_owner_fires_a_worker_wherever_the_worker_is() {
    let mut t = Town::new();
    let (owner, job) = farm_with_job(&mut t);
    let worker = resident_at(&mut t.world, "Worker", 0, t.at.farm);
    let stranger = resident_at(&mut t.world, "Stranger", 0, t.at.farm);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));

    assert_eq!(
        refused(&mut t.world, stranger, "fire", json!({ "resident": worker })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        refused(&mut t.world, owner, "fire", json!({ "resident": stranger })),
        ErrorCode::NotFound
    );
    t.world.set_pos(owner, unsup_kernel::Pos::new(60, 60));
    assert_eq!(
        refused(&mut t.world, owner, "fire", json!({ "resident": worker })),
        ErrorCode::NotHere
    );
    go(&mut t.world, owner, t.at.farm);

    // The worker walks to the second zone before the owner fires it.
    go(&mut t.world, worker, t.at.far_home);
    let (result, events) =
        ok_with_events(&mut t.world, owner, "fire", json!({ "resident": worker }));
    assert_eq!(result, json!({ "job": job, "resident": worker }));
    assert_eq!(
        of_kind(&events, "economy.worker_left"),
        [json!({ "resident": worker, "site": t.at.farm, "job": job, "reason": "fired" })]
    );
    // A worker that is fired is notable news. A worker that quits is routine.
    assert_eq!(drama_of(&events, "economy.worker_left"), [20]);
    t.world.step(2);
    assert_eq!(section(&t.world, worker, "job"), None);
    let notices = t.world.notices(worker);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, "economy.job_ended");
    assert_eq!(notices[0].data["reason"], "fired");
    // It can take a job again.
    go(&mut t.world, worker, t.at.farm);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    assert_only_transfers(&t.world);
}
