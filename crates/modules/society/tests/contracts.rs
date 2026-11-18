//! The contract engine: the life of a contract from the offer to the end,
//! with the balance of every party and of the escrow at each step.

mod common;

use common::{DEPOT, FAR, events, fail, notice, ok, rich, site, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionStatus, Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Holder, Money, Pos, ResidentId, SiteId, ZoneId};
use unsup_society::Contracts;

/// Offers a contract and returns its id.
fn offer(w: &mut TestWorld<'_>, from: ResidentId, to: Option<ResidentId>, clauses: Value) -> u64 {
    let result = ok(w, from, "contract_offer", json!({ "to": to, "clauses": clauses }));
    result["contract"].as_u64().expect("the result has the id")
}

/// Returns the `contracts` section of a resident with full detail.
fn section(w: &TestWorld<'_>, r: ResidentId) -> Option<Value> {
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    w.observe_with(r, &opts).sections.get("contracts").cloned()
}

/// Steps the world until the frame of a deadline has run.
fn run_to(w: &mut TestWorld<'_>, frame: u64) -> Vec<unsup_kernel::protocol::FrameOutput> {
    let left = frame.checked_sub(w.world.frame()).expect("the deadline is not in the past");
    w.step(u32::try_from(left).expect("a short wait"))
}

fn escrow(w: &TestWorld<'_>, contract: u64) -> Money {
    w.global().ledger().money(Holder::Escrow(contract))
}

fn open_contracts(w: &TestWorld<'_>) -> usize {
    w.global().res::<Contracts>().len()
}

/// The clauses of a sale: the buyer pays 100 at the signing and 300 at the
/// deadline; the seller delivers 3 loaves to the depot and risks 50.
fn sale(w: &TestWorld<'_>) -> Value {
    sale_at(site(w, 0, "warehouse"))
}

/// The clauses of [`sale`] with a given depot.
fn sale_at(depot: SiteId) -> Value {
    json!([
        { "kind": "pay_at_signing", "by": "offerer", "amount": 100 },
        { "kind": "pay_at_deadline", "by": "offerer", "amount": 300, "due_in": 30 },
        { "kind": "deliver", "by": "counterparty", "item": "bread", "qty": 3, "site": depot, "due_in": 20 },
        { "kind": "penalty", "by": "counterparty", "amount": 50 },
    ])
}

#[test]
fn a_fulfilled_contract_pays_every_clause_and_gives_the_penalty_back() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    w.give_item(bob, "bread", 5);
    let depot = site(&w, 0, "warehouse");

    // The offer locks the part of the offerer at once.
    let ack = w.act(ada, "contract_offer", json!({ "to": bob, "clauses": sale(&w) }));
    assert_eq!(ack.status, ActionStatus::Pending);
    assert_eq!(w.cash(ada), Money(600));
    w.step(2);
    let report = w.last_report(ada).expect("the answer");
    let id = report.result["contract"].as_u64().expect("an id");
    assert_eq!(report.result["locked"], json!(400));
    assert_eq!(escrow(&w, id), Money(400));
    w.assert_conserved();

    // The offer tells the other party what it must lock.
    let offered = notice(&w, bob, "society.contract_offered");
    assert_eq!((offered.from, offered.data["contract"].clone()), (Some(ada), json!(id)));
    assert_eq!(offered.data["your_part"], json!(50));
    assert_eq!(
        offered.data["clauses"][2],
        json!({ "kind": "deliver", "by": "counterparty", "item": "bread", "qty": 3, "site": depot, "due_in": 20, "state": "open" })
    );

    // The signer offers 80; its part is 50; 30 come back. The payment at the
    // signing arrives with them.
    w.act_ok(bob, "contract_sign", json!({ "contract": id, "stake": 80 }));
    assert_eq!(w.cash(bob), Money(20));
    let frames = w.step(2);
    let signed_at = w.world.frame() - 1;
    assert_eq!(
        w.last_report(bob).map(|r| r.result),
        Some(json!({ "contract": id, "locked": 50, "status": "active" }))
    );
    assert_eq!((w.cash(ada), w.cash(bob)), (Money(600), Money(100 - 50 + 100)));
    assert_eq!(escrow(&w, id), Money(400 + 50 - 100));
    let signed = events(&frames, "society.contract_signed");
    assert_eq!(signed.len(), 1);
    assert_eq!(signed[0].data, json!({ "contract": id, "parties": [ada, bob] }));
    assert_eq!(notice(&w, ada, "society.contract_signed").from, Some(bob));
    w.assert_conserved();

    // Both parties see the contract, with the deadlines as frames.
    let seen = section(&w, bob).expect("a section");
    assert_eq!(seen["list"][0]["contract"], json!(id));
    assert_eq!(
        (seen["list"][0]["you_are"].clone(), seen["list"][0]["with"].clone()),
        (json!("counterparty"), json!(ada))
    );
    assert_eq!(seen["list"][0]["status"], json!("active"));
    assert_eq!(seen["list"][0]["next_due"], json!(signed_at + 20));
    assert_eq!(seen["list"][0]["clauses"][0]["state"], json!("done"));
    assert_eq!(seen["list"][0]["clauses"][1]["due"], json!(signed_at + 30));

    // The seller carries the bread to the depot and claims the delivery.
    w.set_pos(bob, Pos::new(DEPOT.x0 + 1, DEPOT.y0 + 1));
    let delivered = ok(&mut w, bob, "contract_claim", json!({ "contract": id }));
    assert_eq!(delivered, json!({ "contract": id, "clause": 2, "item": "bread", "qty": 3 }));
    assert_eq!(
        (w.qty(Holder::Resident(bob), "bread"), w.qty(Holder::Site(depot), "bread")),
        (2, 3)
    );
    w.step(1);
    assert_eq!(
        notice(&w, ada, "society.contract_delivery").data,
        json!({ "contract": id, "clause": 2 })
    );
    assert_eq!(open_contracts(&w), 1, "the payment at the deadline is still open");

    // At the deadline the engine pays the rest and gives the penalty back.
    let before = run_to(&mut w, signed_at + 29);
    assert!(events(&before, "society.contract_fulfilled").is_empty());
    assert_eq!(escrow(&w, id), Money(350));
    let frames = run_to(&mut w, signed_at + 30);
    let fulfilled = events(&frames, "society.contract_fulfilled");
    assert_eq!(fulfilled.len(), 1);
    assert_eq!(fulfilled[0].data, json!({ "contract": id, "parties": [ada, bob] }));
    assert_eq!(escrow(&w, id), Money(0));
    assert_eq!(open_contracts(&w), 0);
    w.step(1);
    assert_eq!((w.cash(ada), w.cash(bob)), (Money(600), Money(150 + 300 + 50)));
    let ended = notice(&w, bob, "society.contract_ended");
    assert_eq!(ended.data, json!({ "contract": id, "outcome": "fulfilled" }));
    assert!(section(&w, bob).is_none() && section(&w, ada).is_none());

    // A kept contract raises the standing of both parties.
    let gain = w.param("society.rep_contract_fulfilled");
    assert_eq!(gain, 10);
    assert_eq!((w.attr(ada, "rep.standing"), w.attr(bob, "rep.standing")), (10, 10));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.in_flight), (1_100, 0));
    assert_eq!(totals.items[&w.world.blueprint().item_id("bread").expect("an item")].held, 5);
}

#[test]
fn a_missed_delivery_is_a_breach_and_the_penalty_goes_to_the_other_party() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    let terms = sale(&w);
    let id = offer(&mut w, ada, Some(bob), terms);
    ok(&mut w, bob, "contract_sign", json!({ "contract": id, "stake": 50 }));
    let signed_at = w.world.frame() - 1;
    assert_eq!((w.cash(ada), w.cash(bob), escrow(&w, id)), (Money(600), Money(150), Money(350)));

    // The seller never delivers. One frame before the deadline nothing has
    // happened.
    let before = run_to(&mut w, signed_at + 19);
    assert!(events(&before, "society.contract_breached").is_empty());
    assert_eq!(open_contracts(&w), 1);

    // At the deadline of the delivery the engine ends the contract: the
    // penalty goes to the buyer, and the buyer gets its unpaid 300 back.
    let frames = run_to(&mut w, signed_at + 20);
    let breached = events(&frames, "society.contract_breached");
    assert_eq!(breached.len(), 1);
    assert_eq!(breached[0].data, json!({ "contract": id, "by": bob, "parties": [ada, bob] }));
    assert_eq!(breached[0].actor, Some(bob));
    assert!(events(&frames, "society.contract_fulfilled").is_empty());
    assert_eq!((escrow(&w, id), open_contracts(&w)), (Money(0), 0));
    w.assert_conserved();
    w.step(1);
    assert_eq!((w.cash(ada), w.cash(bob)), (Money(600 + 300 + 50), Money(150)));
    for party in [ada, bob] {
        let ended = notice(&w, party, "society.contract_ended");
        assert_eq!(ended.data, json!({ "contract": id, "outcome": "breached", "by": [bob] }));
    }

    // The breach costs the seller standing. The buyer keeps its own.
    let loss = w.param("society.rep_contract_breached");
    assert_eq!(loss, 50);
    assert_eq!((w.attr(ada, "rep.standing"), w.attr(bob, "rep.standing")), (0, -50));
    // The later deadline of the payment finds nothing to do.
    let later = run_to(&mut w, signed_at + 31);
    assert!(later.iter().all(|frame| frame.events.is_empty()));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.in_flight), (1_100, 0));
}

#[test]
fn an_offer_that_nobody_signs_expires_and_the_money_comes_back() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 500);
    let bob = w.spawn("Bob");
    let clauses = json!([{ "kind": "pay_at_signing", "by": "offerer", "amount": 500 }]);
    let result = ok(
        &mut w,
        ada,
        "contract_offer",
        json!({ "to": bob, "clauses": clauses, "expires_in": 5 }),
    );
    let id = result["contract"].as_u64().expect("an id");
    let expires = result["expires"].as_u64().expect("a frame");
    assert_eq!(expires, w.world.frame() - 1 + 5);
    assert_eq!((w.cash(ada), escrow(&w, id)), (Money(0), Money(500)));
    let seen = section(&w, bob).expect("the offer names Bob");
    assert_eq!(seen["list"][0]["status"], json!("offered"));
    assert_eq!(
        (seen["list"][0]["your_part"].clone(), seen["list"][0]["expires"].clone()),
        (json!(0), json!(expires))
    );

    run_to(&mut w, expires - 1);
    assert_eq!(escrow(&w, id), Money(500));
    run_to(&mut w, expires);
    assert_eq!((escrow(&w, id), open_contracts(&w)), (Money(0), 0));
    w.step(1);
    assert_eq!(w.cash(ada), Money(500));
    for party in [ada, bob] {
        assert_eq!(
            notice(&w, party, "society.contract_closed").data,
            json!({ "contract": id, "reason": "expired" })
        );
    }
    // An offer that expired cannot be signed.
    assert_eq!(fail(&mut w, bob, "contract_sign", json!({ "contract": id })), ErrorCode::NotFound);
    assert_eq!(w.attr(ada, "rep.standing"), 0);
    w.assert_conserved();
}

/// A bounty: 500 for the arrest of `target` by an officer, not by the
/// city watch, for `due_in` minutes.
fn bounty(target: ResidentId, due_in: u32) -> Value {
    json!([{
        "kind": "pay_on_event",
        "by": "offerer",
        "amount": 500,
        "event": "crime.arrest_made",
        "fields": { "target": target, "watch": false },
        "due_in": due_in,
    }])
}

#[test]
fn a_bounty_pays_when_the_event_happens_in_any_zone() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 500);
    let hunter = w.spawn("Hunter");
    let thief = w.spawn("Thief");
    let other = w.spawn("Other");
    let officer = w.spawn_at("Officer", FAR);
    assert_eq!(w.zone_of(officer), ZoneId(3));
    let id = offer(&mut w, ada, Some(hunter), bounty(thief, 100));
    // The offerer wrote the kind of the event and the data fields. The
    // reader gets them as untrusted text; the rest comes from the engine.
    let offered = notice(&w, hunter, "society.contract_offered");
    assert_eq!(
        offered.data["clauses"],
        json!([{
            "kind": "pay_on_event",
            "by": "offerer",
            "amount": 500,
            "event": { "untrusted": true, "from": ada, "text": "crime.arrest_made" },
            "fields": {
                "untrusted": true,
                "from": ada,
                "text": format!("{{\"target\":{},\"watch\":false}}", thief.0),
            },
            "due_in": 100,
            "state": "open",
        }])
    );
    ok(&mut w, hunter, "contract_sign", json!({ "contract": id }));
    assert_eq!((w.cash(ada), w.cash(hunter), escrow(&w, id)), (Money(0), Money(0), Money(500)));

    // An arrest of another resident does not count.
    ok(&mut w, officer, "arrest_test", json!({ "target": other }));
    w.step(2);
    assert_eq!((w.cash(hunter), escrow(&w, id), open_contracts(&w)), (Money(0), Money(500), 1));

    // The arrest of the thief, in a zone far away: the global realm sees
    // the event in the same frame and pays.
    ok(&mut w, officer, "arrest_test", json!({ "target": thief }));
    let frames = w.step(1);
    assert_eq!(events(&frames, "crime.arrest_made").len(), 1);
    assert_eq!(events(&frames, "society.contract_fulfilled").len(), 1);
    assert_eq!((escrow(&w, id), open_contracts(&w)), (Money(0), 0));
    w.step(1);
    assert_eq!((w.cash(ada), w.cash(hunter)), (Money(0), Money(500)));
    assert_eq!((w.attr(ada, "rep.standing"), w.attr(hunter, "rep.standing")), (10, 10));
    w.assert_conserved();
}

#[test]
fn a_bounty_can_name_who_must_cause_the_event() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 500);
    let hunter = w.spawn("Hunter");
    let thief = w.spawn("Thief");
    let rival = w.spawn("Rival");
    let mut clauses = bounty(thief, 100);
    clauses[0]["actor"] = json!(hunter);
    let id = offer(&mut w, ada, Some(hunter), clauses);
    ok(&mut w, hunter, "contract_sign", json!({ "contract": id }));
    let signed_at = w.world.frame() - 1;
    // The contract names the resident that must cause the event, and the
    // deadline is a frame now.
    let seen = section(&w, hunter).expect("a section");
    let clause = &seen["list"][0]["clauses"][0];
    assert_eq!(
        (clause["actor"].clone(), clause["due"].clone()),
        (json!(hunter), json!(signed_at + 100))
    );
    assert_eq!(clause.get("due_in"), None);

    ok(&mut w, rival, "arrest_test", json!({ "target": thief }));
    w.step(2);
    assert_eq!((w.cash(hunter), open_contracts(&w)), (Money(0), 1), "the rival made the arrest");
    ok(&mut w, hunter, "arrest_test", json!({ "target": thief }));
    w.step(2);
    assert_eq!((w.cash(hunter), open_contracts(&w)), (Money(500), 0));
    w.assert_conserved();
}

#[test]
fn a_bounty_that_nobody_earns_goes_back_at_the_deadline() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 500);
    let hunter = w.spawn("Hunter");
    let thief = w.spawn("Thief");
    let id = offer(&mut w, ada, Some(hunter), bounty(thief, 10));
    ok(&mut w, hunter, "contract_sign", json!({ "contract": id }));
    let signed_at = w.world.frame() - 1;

    run_to(&mut w, signed_at + 9);
    assert_eq!((escrow(&w, id), open_contracts(&w)), (Money(500), 1));
    let frames = run_to(&mut w, signed_at + 10);
    let expired = events(&frames, "society.contract_expired");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].data, json!({ "contract": id, "parties": [ada, hunter] }));
    assert!(events(&frames, "society.contract_fulfilled").is_empty());
    assert!(events(&frames, "society.contract_breached").is_empty());
    w.step(1);
    assert_eq!((w.cash(ada), w.cash(hunter), escrow(&w, id)), (Money(500), Money(0), Money(0)));
    assert_eq!(
        notice(&w, hunter, "society.contract_ended").data,
        json!({ "contract": id, "outcome": "expired" })
    );
    // Nobody broke a promise and nobody earned the bounty: no standing moves.
    assert_eq!((w.attr(ada, "rep.standing"), w.attr(hunter, "rep.standing")), (0, 0));

    // An arrest after the deadline pays nothing.
    ok(&mut w, hunter, "arrest_test", json!({ "target": thief }));
    w.step(2);
    assert_eq!(w.cash(hunter), Money(0));
    w.assert_conserved();
}

#[test]
fn goods_can_be_handed_to_the_other_party_in_person() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(20, 20));
    let bob = w.spawn_at("Bob", Pos::new(30, 20));
    w.give_money(ada, Money(200));
    w.give_item(bob, "coal", 4);
    let clauses = json!([
        { "kind": "deliver", "by": "counterparty", "item": "coal", "qty": 4, "due_in": 50 },
        { "kind": "pay_at_deadline", "by": "offerer", "amount": 200, "due_in": 60 },
    ]);
    let id = offer(&mut w, ada, Some(bob), clauses);
    ok(&mut w, bob, "contract_sign", json!({ "contract": id }));
    let signed_at = w.world.frame() - 1;

    // Ten tiles away is too far to hand anything over.
    assert_eq!(fail(&mut w, bob, "contract_claim", json!({ "contract": id })), ErrorCode::NotHere);
    assert_eq!(w.qty(Holder::Resident(bob), "coal"), 4);
    w.set_pos(bob, Pos::new(22, 20));
    assert_eq!(
        ok(&mut w, bob, "contract_claim", json!({ "contract": id, "clause": 0 }))["qty"],
        json!(4)
    );
    assert_eq!(
        (w.qty(Holder::Resident(bob), "coal"), w.qty(Holder::Resident(ada), "coal")),
        (0, 4)
    );
    // A delivery is made once.
    assert_eq!(fail(&mut w, bob, "contract_claim", json!({ "contract": id })), ErrorCode::Conflict);

    run_to(&mut w, signed_at + 61);
    assert_eq!((w.cash(ada), w.cash(bob), open_contracts(&w)), (Money(0), Money(200), 0));
    w.assert_conserved();
}

#[test]
fn contract_claim_refuses_the_wrong_place_goods_and_party() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    let cyd = w.spawn("Cyd");
    w.give_item(bob, "bread", 2);
    let depot = site(&w, 0, "warehouse");
    let terms = sale(&w);
    let id = offer(&mut w, ada, Some(bob), terms);
    let claim = json!({ "contract": id });

    assert_eq!(fail(&mut w, bob, "contract_claim", json!({ "contract": 77 })), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, bob, "contract_claim", claim.clone()),
        ErrorCode::Conflict,
        "not signed yet"
    );
    ok(&mut w, bob, "contract_sign", json!({ "contract": id, "stake": 50 }));
    let signed_at = w.world.frame() - 1;

    assert_eq!(fail(&mut w, cyd, "contract_claim", claim.clone()), ErrorCode::NotAllowed);
    // The buyer has no delivery to make; clause 1 is not a delivery.
    assert_eq!(fail(&mut w, ada, "contract_claim", claim.clone()), ErrorCode::Conflict);
    assert_eq!(
        fail(&mut w, bob, "contract_claim", json!({ "contract": id, "clause": 1 })),
        ErrorCode::Conflict
    );
    assert_eq!(
        fail(&mut w, bob, "contract_claim", json!({ "contract": id, "clause": 9 })),
        ErrorCode::Conflict
    );
    // Not in the depot.
    assert_eq!(fail(&mut w, bob, "contract_claim", claim.clone()), ErrorCode::NotHere);
    // In the depot, with two loaves of the three.
    w.set_pos(bob, Pos::new(DEPOT.x0, DEPOT.y0));
    assert_eq!(fail(&mut w, bob, "contract_claim", claim.clone()), ErrorCode::Insufficient);
    assert_eq!(
        (w.qty(Holder::Resident(bob), "bread"), w.qty(Holder::Site(depot), "bread")),
        (2, 0)
    );

    // A failed claim does not stop the engine: the deadline still comes.
    let frames = run_to(&mut w, signed_at + 20);
    assert_eq!(events(&frames, "society.contract_breached").len(), 1);
    assert_eq!(fail(&mut w, bob, "contract_claim", claim), ErrorCode::NotFound);
    w.assert_conserved();
}

#[test]
fn a_delivery_that_starts_before_the_deadline_counts() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    w.give_item(bob, "bread", 3);
    let terms = sale(&w);
    let id = offer(&mut w, ada, Some(bob), terms);
    ok(&mut w, bob, "contract_sign", json!({ "contract": id, "stake": 50 }));
    let signed_at = w.world.frame() - 1;
    w.set_pos(bob, Pos::new(DEPOT.x0, DEPOT.y0));

    // The claim reaches the global realm in the frame of the deadline. The
    // zone moves the goods one frame later, and the answer comes after it.
    run_to(&mut w, signed_at + 19);
    w.act_ok(bob, "contract_claim", json!({ "contract": id }));
    let frames = w.step(3);
    assert!(events(&frames, "society.contract_breached").is_empty());
    assert_eq!(w.last_report(bob).map(|r| r.ok), Some(true));
    assert_eq!(open_contracts(&w), 1);
    let frames = run_to(&mut w, signed_at + 30);
    assert_eq!(events(&frames, "society.contract_fulfilled").len(), 1);
    w.assert_conserved();
}

#[test]
fn contract_offer_refuses_clauses_that_break_the_rules() {
    let mut w = world();
    w.set_param("society.contract_clauses_max", 2);
    w.set_param("society.contracts_open_max", 1);
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    w.step(1);
    let pay = json!({ "kind": "pay_at_signing", "by": "offerer", "amount": 10 });
    let try_offer = |w: &mut TestWorld<'_>, args: Value| fail(w, ada, "contract_offer", args);
    let with = |clause: Value| json!({ "to": bob, "clauses": [clause] });

    assert_eq!(try_offer(&mut w, json!({ "to": bob, "clauses": [] })), ErrorCode::InvalidArgs);
    assert_eq!(
        try_offer(&mut w, json!({ "to": bob, "clauses": [pay, pay, pay] })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(try_offer(&mut w, json!({ "to": ada, "clauses": [pay] })), ErrorCode::InvalidArgs);
    assert_eq!(
        try_offer(&mut w, json!({ "to": bob, "clauses": [pay], "expires_in": 0 })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(&mut w, json!({ "to": bob, "clauses": [pay], "expires_in": 10_081 })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(&mut w, with(json!({ "kind": "pay_at_signing", "by": "offerer", "amount": 0 }))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(&mut w, with(json!({ "kind": "pay_at_signing", "by": "nobody", "amount": 5 }))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(&mut w, with(json!({ "kind": "gift", "by": "offerer", "amount": 5 }))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(
            &mut w,
            with(json!({ "kind": "pay_at_deadline", "by": "offerer", "amount": 5, "due_in": 0 }))
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(
            &mut w,
            with(
                json!({ "kind": "pay_at_deadline", "by": "offerer", "amount": 5, "due_in": 40_321 })
            )
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(
            &mut w,
            with(
                json!({ "kind": "deliver", "by": "offerer", "item": "gold", "qty": 1, "due_in": 5 })
            )
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(
            &mut w,
            with(
                json!({ "kind": "deliver", "by": "offerer", "item": "bread", "qty": 0, "due_in": 5 })
            )
        ),
        ErrorCode::InvalidArgs
    );
    let event = |kind: &str, fields: Value| json!({ "kind": "pay_on_event", "by": "offerer", "amount": 5, "event": kind, "fields": fields, "due_in": 5 });
    // The kernel has events too, but a contract cannot wait for them.
    assert_eq!(
        try_offer(&mut w, with(event("kernel.despawned", json!({})))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(try_offer(&mut w, with(event("society.said", json!({})))), ErrorCode::InvalidArgs);
    assert_eq!(try_offer(&mut w, with(event("Crime Arrest", json!({})))), ErrorCode::InvalidArgs);
    assert_eq!(
        try_offer(&mut w, with(event("crime.arrest_made", json!({ "Not A Field": 1 })))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(&mut w, with(event("crime.arrest_made", json!({ "target": 1.5 })))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(&mut w, with(event("crime.arrest_made", json!({ "note": "two words" })))),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        try_offer(
            &mut w,
            with(event("crime.arrest_made", json!({ "a": 1, "b": 2, "c": 3, "d": 4, "e": 5 })))
        ),
        ErrorCode::InvalidArgs
    );
    // Amounts that do not fit in the ledger together.
    let huge = json!({ "kind": "penalty", "by": "counterparty", "amount": i64::MAX });
    assert_eq!(
        try_offer(&mut w, json!({ "to": bob, "clauses": [huge, huge] })),
        ErrorCode::InvalidArgs
    );
    // The same for the part of the offerer: the sum is checked before the
    // cash, so the answer is not `insufficient`.
    let huge = json!({ "kind": "penalty", "by": "offerer", "amount": i64::MAX });
    assert_eq!(
        try_offer(&mut w, json!({ "to": bob, "clauses": [huge, huge] })),
        ErrorCode::InvalidArgs
    );
    // More than the offerer carries: the zone refuses, nothing moves.
    assert_eq!(
        try_offer(&mut w, with(json!({ "kind": "penalty", "by": "offerer", "amount": 1_001 }))),
        ErrorCode::Insufficient
    );
    assert_eq!(w.cash(ada), Money(1_000));

    // The global realm refuses a reader that does not exist and an offerer
    // with too many open contracts. The locked money comes back.
    assert_eq!(try_offer(&mut w, json!({ "to": 999, "clauses": [pay] })), ErrorCode::NotFound);
    offer(&mut w, ada, Some(bob), json!([pay]));
    assert_eq!(
        fail(&mut w, ada, "contract_offer", json!({ "to": bob, "clauses": [pay] })),
        ErrorCode::Conflict
    );
    w.step(1);
    assert_eq!(w.cash(ada), Money(990));
    assert_eq!(open_contracts(&w), 1);
    w.assert_conserved();
}

/// Makes one clause of a kind with an amount (a quantity for a delivery)
/// and a deadline. The other fields are valid.
fn clause_of(kind: &str, amount: i64, due_in: i64) -> Value {
    match kind {
        "pay_at_signing" | "penalty" => json!({ "kind": kind, "by": "offerer", "amount": amount }),
        "pay_at_deadline" => {
            json!({ "kind": kind, "by": "offerer", "amount": amount, "due_in": due_in })
        }
        "pay_on_event" => {
            json!({ "kind": kind, "by": "offerer", "amount": amount, "event": "crime.arrest_made", "due_in": due_in })
        }
        "deliver" => {
            json!({ "kind": kind, "by": "offerer", "item": "bread", "qty": amount, "due_in": due_in })
        }
        other => panic!("no clause is of the kind `{other}`"),
    }
}

#[test]
fn each_kind_of_clause_checks_its_amount_and_its_deadline() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    w.step(1);
    let longest = w.param("society.contract_due_max");
    assert_eq!(longest, 40_320);
    let offer_of = |clause: Value| json!({ "to": bob, "clauses": [clause] });

    // An amount and a quantity are above zero, in every kind of clause.
    for kind in ["pay_at_signing", "pay_at_deadline", "pay_on_event", "deliver", "penalty"] {
        for amount in [0, -1] {
            assert_eq!(
                fail(&mut w, ada, "contract_offer", offer_of(clause_of(kind, amount, 10))),
                ErrorCode::InvalidArgs,
                "{kind} with {amount}"
            );
        }
    }
    // A deadline is from one game minute to the greatest time, in every
    // kind of clause that has one.
    for kind in ["pay_at_deadline", "pay_on_event", "deliver"] {
        for due_in in [0, longest + 1] {
            assert_eq!(
                fail(&mut w, ada, "contract_offer", offer_of(clause_of(kind, 1, due_in))),
                ErrorCode::InvalidArgs,
                "{kind} with a deadline of {due_in}"
            );
        }
        // Both ends of the range are valid.
        for due_in in [1, longest] {
            ok(&mut w, ada, "contract_offer", offer_of(clause_of(kind, 1, due_in)));
        }
    }
    // Six offers are open. The four that pay locked one cent each.
    assert_eq!((open_contracts(&w), w.cash(ada)), (6, Money(996)));
    w.assert_conserved();
}

#[test]
fn the_names_in_an_event_clause_have_length_limits() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let event = |kind: &str, fields: Value| {
        let clause = json!({ "kind": "pay_on_event", "by": "offerer", "amount": 5, "event": kind, "fields": fields, "due_in": 5 });
        json!({ "to": bob, "clauses": [clause] })
    };
    // A zone checks the clauses. It sees a new value of a parameter in the
    // next frame.
    let set = |w: &mut TestWorld<'_>, name: &str, value: i64| {
        w.set_param(name, value);
        w.step(1);
    };

    // `crime.arrest_made` has 17 bytes.
    set(&mut w, "society.contract_event_kind_max", 16);
    let offer = event("crime.arrest_made", json!({}));
    assert_eq!(fail(&mut w, ada, "contract_offer", offer.clone()), ErrorCode::InvalidArgs);
    set(&mut w, "society.contract_event_kind_max", 17);
    ok(&mut w, ada, "contract_offer", offer);

    // The field name `target` has 6 bytes.
    set(&mut w, "society.contract_field_name_max", 5);
    let offer = event("crime.arrest_made", json!({ "target": 7 }));
    assert_eq!(fail(&mut w, ada, "contract_offer", offer.clone()), ErrorCode::InvalidArgs);
    set(&mut w, "society.contract_field_name_max", 6);
    ok(&mut w, ada, "contract_offer", offer);

    // The value `bread` has 5 bytes.
    set(&mut w, "society.contract_field_text_max", 4);
    let offer = event("life.meal_eaten", json!({ "item": "bread" }));
    assert_eq!(fail(&mut w, ada, "contract_offer", offer.clone()), ErrorCode::InvalidArgs);
    set(&mut w, "society.contract_field_text_max", 5);
    ok(&mut w, ada, "contract_offer", offer);

    // Three offers are open; each locked 5 cents.
    assert_eq!((open_contracts(&w), w.cash(ada)), (3, Money(985)));
    w.assert_conserved();
}

#[test]
fn contract_sign_refuses_the_wrong_signer_and_a_stake_that_is_too_small() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    let cyd = rich(&mut w, "Cyd", 100);
    let terms = sale(&w);
    let id = offer(&mut w, ada, Some(bob), terms);
    let sign = |stake: i64| json!({ "contract": id, "stake": stake });

    assert_eq!(fail(&mut w, bob, "contract_sign", json!({ "contract": 77 })), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, bob, "contract_sign", sign(-1)), ErrorCode::InvalidArgs);
    assert_eq!(
        fail(&mut w, bob, "contract_sign", sign(101)),
        ErrorCode::Insufficient,
        "more than Bob carries"
    );
    assert_eq!(fail(&mut w, cyd, "contract_sign", sign(50)), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, ada, "contract_sign", sign(50)), ErrorCode::NotAllowed);
    // One cent less than the part: refused, and the stake comes back.
    assert_eq!(fail(&mut w, bob, "contract_sign", sign(49)), ErrorCode::Insufficient);
    w.step(1);
    assert_eq!((w.cash(bob), w.cash(cyd), escrow(&w, id)), (Money(100), Money(100), Money(400)));

    ok(&mut w, bob, "contract_sign", sign(50));
    assert_eq!(fail(&mut w, bob, "contract_sign", sign(50)), ErrorCode::Conflict);
    w.step(1);
    assert_eq!(w.cash(bob), Money(150));
    // A signed contract cannot be cancelled or rejected: no party can stop it.
    assert_eq!(
        fail(&mut w, ada, "contract_cancel", json!({ "contract": id })),
        ErrorCode::Conflict
    );
    assert_eq!(
        fail(&mut w, bob, "contract_reject", json!({ "contract": id })),
        ErrorCode::Conflict
    );
    assert_eq!(open_contracts(&w), 1);
    w.assert_conserved();
}

#[test]
fn an_offer_can_be_cancelled_by_the_offerer_and_rejected_by_the_other_party() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");

    let terms = sale(&w);
    let first = offer(&mut w, ada, Some(bob), terms);
    assert_eq!(
        fail(&mut w, bob, "contract_cancel", json!({ "contract": first })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, cyd, "contract_reject", json!({ "contract": first })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, ada, "contract_reject", json!({ "contract": first })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, ada, "contract_cancel", json!({ "contract": 77 })),
        ErrorCode::NotFound
    );
    assert_eq!(w.cash(ada), Money(600));

    assert_eq!(
        ok(&mut w, ada, "contract_cancel", json!({ "contract": first })),
        json!({ "contract": first, "status": "cancelled" })
    );
    w.step(1);
    assert_eq!((w.cash(ada), escrow(&w, first)), (Money(1_000), Money(0)));
    let closed = notice(&w, bob, "society.contract_closed");
    assert_eq!(
        (closed.from, closed.data.clone()),
        (Some(ada), json!({ "contract": first, "reason": "cancelled" }))
    );

    let terms = sale(&w);
    let second = offer(&mut w, ada, Some(bob), terms);
    assert_eq!(
        ok(&mut w, bob, "contract_reject", json!({ "contract": second })),
        json!({ "contract": second, "status": "rejected" })
    );
    w.step(1);
    assert_eq!((w.cash(ada), open_contracts(&w)), (Money(1_000), 0));
    let closed = notice(&w, ada, "society.contract_closed");
    assert_eq!(
        (closed.from, closed.data.clone()),
        (Some(bob), json!({ "contract": second, "reason": "rejected" }))
    );
    assert_eq!(
        fail(&mut w, bob, "contract_sign", json!({ "contract": second })),
        ErrorCode::NotFound
    );
    w.assert_conserved();
}

#[test]
fn an_offer_without_a_name_is_for_the_first_resident_that_signs() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 300);
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    let clauses = json!([{ "kind": "pay_at_signing", "by": "offerer", "amount": 300 }]);
    let id = offer(&mut w, ada, None, clauses);
    assert!(section(&w, bob).is_none(), "an offer to everyone is in nobody's list");
    assert_eq!(
        fail(&mut w, ada, "contract_sign", json!({ "contract": id })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, bob, "contract_reject", json!({ "contract": id })),
        ErrorCode::NotAllowed
    );

    // Two residents sign in the same frame. The first request wins.
    w.act_ok(cyd, "contract_sign", json!({ "contract": id }));
    w.act_ok(bob, "contract_sign", json!({ "contract": id }));
    let frames = w.step(2);
    assert_eq!(events(&frames, "society.contract_signed")[0].data["parties"], json!([ada, cyd]));
    assert_eq!(w.last_report(bob).and_then(|r| r.error).map(|e| e.code), Some(ErrorCode::NotFound));
    // The only clause paid at the signing, so the contract is fulfilled at once.
    assert_eq!(
        w.last_report(cyd).map(|r| r.result),
        Some(json!({ "contract": id, "locked": 0, "status": "ended" }))
    );
    assert_eq!(events(&frames, "society.contract_fulfilled").len(), 1);
    assert_eq!((w.cash(cyd), w.cash(bob), open_contracts(&w)), (Money(300), Money(0), 0));
    w.assert_conserved();
}

#[test]
fn contract_view_shows_a_contract_to_those_that_can_sign_it_or_signed_it() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    let cyd = w.spawn_at("Cyd", FAR);
    let depot = site(&w, 0, "warehouse");
    let terms = sale(&w);
    let view = |id: u64| json!({ "contract": id });

    // An offer without a name is for everyone. A resident reads it before
    // it signs: what it must lock, and every clause.
    let made = ok(&mut w, ada, "contract_offer", json!({ "clauses": terms.clone() }));
    let (open, expires) = (made["contract"].as_u64().expect("an id"), made["expires"].clone());
    let seen = ok(&mut w, cyd, "contract_view", view(open));
    assert_eq!(
        seen,
        json!({
            "contract": open,
            "offerer": ada,
            "you_are": "counterparty",
            "your_part": 50,
            "status": "offered",
            "expires": expires,
            "clauses": [
                { "kind": "pay_at_signing", "by": "offerer", "amount": 100, "state": "open" },
                { "kind": "pay_at_deadline", "by": "offerer", "amount": 300, "due_in": 30, "state": "open" },
                { "kind": "deliver", "by": "counterparty", "item": "bread", "qty": 3, "site": depot, "due_in": 20, "state": "open" },
                { "kind": "penalty", "by": "counterparty", "amount": 50, "state": "open" },
            ],
        })
    );
    // The offerer reads its own side.
    let own = ok(&mut w, ada, "contract_view", view(open));
    assert_eq!((own["you_are"].clone(), own["your_part"].clone()), (json!("offerer"), json!(400)));

    // The verb is free: a resident that is busy can read.
    w.act_ok(cyd, "post", json!({ "text": "a long text" }));
    assert_eq!(ok(&mut w, cyd, "contract_view", view(open))["your_part"], json!(50));
    assert_eq!(w.activity(cyd), Some("post"));

    // An offer with a name is for that resident only.
    let named = offer(&mut w, ada, Some(bob), terms);
    assert_eq!(fail(&mut w, cyd, "contract_view", view(named)), ErrorCode::NotAllowed);
    assert_eq!(ok(&mut w, bob, "contract_view", view(named))["status"], json!("offered"));
    assert_eq!(fail(&mut w, bob, "contract_view", view(77)), ErrorCode::NotFound);

    // A signed contract is private to its parties.
    ok(&mut w, bob, "contract_sign", json!({ "contract": open, "stake": 50 }));
    assert_eq!(fail(&mut w, cyd, "contract_view", view(open)), ErrorCode::NotAllowed);
    let signed = ok(&mut w, ada, "contract_view", view(open));
    assert_eq!(
        (signed["status"].clone(), signed["counterparty"].clone(), signed.get("expires")),
        (json!("active"), json!(bob), None)
    );
    assert_eq!(signed["clauses"][0]["state"], json!("done"));
    w.assert_conserved();
}

#[test]
fn both_parties_can_breach_and_each_pays_its_penalty() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 100);
    let bob = rich(&mut w, "Bob", 40);
    let clauses = json!([
        { "kind": "deliver", "by": "offerer", "item": "coal", "qty": 1, "due_in": 5 },
        { "kind": "deliver", "by": "counterparty", "item": "bread", "qty": 1, "due_in": 5 },
        { "kind": "penalty", "by": "offerer", "amount": 100 },
        { "kind": "penalty", "by": "counterparty", "amount": 40 },
    ]);
    let id = offer(&mut w, ada, Some(bob), clauses);
    ok(&mut w, bob, "contract_sign", json!({ "contract": id, "stake": 40 }));
    let signed_at = w.world.frame() - 1;
    let frames = run_to(&mut w, signed_at + 5);
    let by: Vec<Value> =
        events(&frames, "society.contract_breached").iter().map(|e| e.data["by"].clone()).collect();
    assert_eq!(by, [json!(ada), json!(bob)]);
    w.step(1);
    // Each penalty goes to the other party.
    assert_eq!((w.cash(ada), w.cash(bob)), (Money(40), Money(100)));
    assert_eq!((w.attr(ada, "rep.standing"), w.attr(bob, "rep.standing")), (-50, -50));
    w.assert_conserved();
}

#[test]
fn the_offers_of_a_resident_that_leaves_the_world_end() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 1_000);
    let cyd = w.spawn("Cyd");
    let terms = sale(&w);
    let to_bob = offer(&mut w, ada, Some(bob), terms);
    let to_ada = offer(
        &mut w,
        bob,
        Some(ada),
        json!([{ "kind": "penalty", "by": "offerer", "amount": 500 }]),
    );
    let signed = offer(&mut w, bob, Some(cyd), bounty(cyd, 10));
    ok(&mut w, cyd, "contract_sign", json!({ "contract": signed }));
    assert_eq!(open_contracts(&w), 3);

    // Ada leaves the world. Her cash goes with her; her offer and the offer
    // to her end. The money that she locked has no owner now: the kernel
    // puts it on the unclaimed account. Bob gets his 500 back.
    w.despawn(ada);
    w.step(2);
    assert_eq!(open_contracts(&w), 1);
    assert_eq!((escrow(&w, to_bob), escrow(&w, to_ada)), (Money(0), Money(0)));
    assert_eq!((w.cash(bob), escrow(&w, signed)), (Money(500), Money(500)));
    assert_eq!(w.money(unsup_kernel::UNCLAIMED), Money(600 + 400));
    assert_eq!(notice(&w, bob, "society.contract_closed").data["reason"], json!("party_left"));
    w.assert_conserved();
}

#[test]
fn money_for_a_dead_party_goes_to_its_heir() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    let heir = w.spawn_at("Heir", FAR);
    w.give_item(bob, "bread", 3);
    let terms = sale(&w);
    let id = offer(&mut w, ada, Some(bob), terms);
    let pending =
        offer(&mut w, ada, None, json!([{ "kind": "penalty", "by": "offerer", "amount": 70 }]));
    ok(&mut w, bob, "contract_sign", json!({ "contract": id, "stake": 50 }));
    let signed_at = w.world.frame() - 1;
    w.set_pos(bob, Pos::new(DEPOT.x0, DEPOT.y0));
    ok(&mut w, bob, "contract_claim", json!({ "contract": id }));
    assert_eq!((w.cash(ada), w.cash(bob), escrow(&w, id)), (Money(530), Money(150), Money(350)));

    // The seller dies after the delivery, with a will. The heir gets the
    // cash that the seller carried.
    ok(&mut w, bob, "die_test", json!({ "heir": heir }));
    w.step(2);
    assert!(!w.is_alive(bob));
    assert_eq!(w.cash(heir), Money(150));
    // The buyer dies too, without a will: the city inherits. Her open offer
    // ends, and the money that it locked goes to the city.
    ok(&mut w, ada, "die_test", json!({}));
    w.step(2);
    assert_eq!((escrow(&w, pending), open_contracts(&w)), (Money(0), 1));
    assert_eq!(w.money(common::CITY), Money(530 + 70));

    // The engine carries the contract out to the end. The payment at the
    // deadline and the penalty of the seller go to its heir.
    let frames = run_to(&mut w, signed_at + 30);
    assert_eq!(events(&frames, "society.contract_fulfilled").len(), 1);
    w.step(1);
    assert_eq!(w.cash(heir), Money(150 + 300 + 50));
    assert_eq!((escrow(&w, id), open_contracts(&w)), (Money(0), 0));
    // Nothing went to the unclaimed account.
    assert_eq!(w.money(unsup_kernel::UNCLAIMED), Money(0));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.held, totals.money.in_flight), (1_100, 0));
}

#[test]
fn a_signer_can_have_a_limited_number_of_open_contracts() {
    let mut w = world();
    w.set_param("society.contracts_open_max", 1);
    let ada = rich(&mut w, "Ada", 100);
    let bob = rich(&mut w, "Bob", 100);
    let cyd = rich(&mut w, "Cyd", 100);
    w.step(1);
    let wait = json!([{ "kind": "pay_at_deadline", "by": "offerer", "amount": 10, "due_in": 50 }]);
    let first = offer(&mut w, ada, Some(cyd), wait.clone());
    let second = offer(&mut w, bob, Some(cyd), wait);
    ok(&mut w, cyd, "contract_sign", json!({ "contract": first }));
    // One open contract is the limit of this world. The stake comes back.
    assert_eq!(
        fail(&mut w, cyd, "contract_sign", json!({ "contract": second, "stake": 100 })),
        ErrorCode::Conflict
    );
    w.step(1);
    assert_eq!((w.cash(cyd), escrow(&w, second), open_contracts(&w)), (Money(100), Money(10), 2));
    w.assert_conserved();
}

#[test]
fn a_delivery_is_checked_once_at_a_time() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = rich(&mut w, "Bob", 100);
    w.give_item(bob, "bread", 5);
    let depot = site(&w, 0, "warehouse");
    let terms = sale(&w);
    let id = offer(&mut w, ada, Some(bob), terms);
    ok(&mut w, bob, "contract_sign", json!({ "contract": id, "stake": 50 }));
    w.set_pos(bob, Pos::new(DEPOT.x0, DEPOT.y0));

    // The resident claims, cancels the wait and claims again in one frame.
    // The first claim was stopped, so it moves nothing. The second one is
    // refused: the engine still checks the first. A failed action changes
    // nothing, so no goods move.
    let first = w.act(bob, "contract_claim", json!({ "contract": id }));
    w.act_ok(bob, "cancel", json!({}));
    let second = w.act(bob, "contract_claim", json!({ "contract": id }));
    w.step(3);
    let reports = w.reports(bob);
    let code = |action_id: u64| {
        let report = reports.iter().rev().find(|r| r.action_id == action_id).expect("a report");
        report.error.as_ref().map(|e| e.code)
    };
    assert_eq!(code(first.action_id), Some(ErrorCode::Interrupted));
    assert_eq!(code(second.action_id), Some(ErrorCode::Conflict));
    assert_eq!(
        (w.qty(Holder::Resident(bob), "bread"), w.qty(Holder::Site(depot), "bread")),
        (5, 0)
    );
    let seen = section(&w, bob).expect("a section");
    assert_eq!(seen["list"][0]["clauses"][2]["state"], json!("open"));

    // The check of the first claim is over, so the next claim delivers.
    // The goods move once.
    assert_eq!(
        ok(&mut w, bob, "contract_claim", json!({ "contract": id })),
        json!({ "contract": id, "clause": 2, "item": "bread", "qty": 3 })
    );
    assert_eq!(
        (w.qty(Holder::Resident(bob), "bread"), w.qty(Holder::Site(depot), "bread")),
        (2, 3)
    );
    let seen = section(&w, bob).expect("a section");
    assert_eq!(seen["list"][0]["clauses"][2]["state"], json!("done"));
    assert_eq!(fail(&mut w, bob, "contract_claim", json!({ "contract": id })), ErrorCode::Conflict);
    assert_eq!(w.qty(Holder::Site(depot), "bread"), 3);
    w.assert_conserved();
}
