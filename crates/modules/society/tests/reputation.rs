//! Reputation: the attribute `rep.standing` and the verb `opine`.

mod common;

use common::{FAR, count_notices, events, fail, notice, ok, rich, spawn_owned, world};
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, ResidentId, ZoneId};

#[test]
fn rep_standing_is_declared_as_the_game_design_says() {
    let w = world();
    let catalogue = w.world.catalogue();
    let attr = catalogue.attrs.iter().find(|a| a.name == "rep.standing").expect("the attribute");
    assert_eq!((attr.min, attr.max, attr.default), (Some(-1_000), Some(1_000), 0));
}

#[test]
fn praise_and_blame_move_the_standing_of_a_resident_in_any_zone() {
    let mut w = world();
    w.set_param("society.opinion_min_age", 0);
    let ada = spawn_owned(&mut w, "Ada", 1);
    let bob = spawn_owned(&mut w, "Bob", 2);
    let cyd = spawn_owned(&mut w, "Cyd", 3);
    w.set_pos(bob, FAR);
    assert_eq!(w.zone_of(bob), ZoneId(3));
    w.step(1);
    let weight = w.param("society.opinion_weight");
    assert_eq!(weight, 5);

    let said = ok(&mut w, ada, "opine", json!({ "resident": bob, "verdict": "praise" }));
    assert_eq!(said, json!({ "about": bob, "verdict": "praise" }));
    assert_eq!(w.attr(bob, "rep.standing"), 0, "the opinion is on its way");
    let frames = w.step(2);
    assert_eq!(w.attr(bob, "rep.standing"), 5);
    assert_eq!(w.attr(ada, "rep.standing"), 0);
    let given = events(&frames, "society.opinion_given");
    assert_eq!(given.len(), 1);
    assert_eq!(given[0].data, json!({ "from": ada, "about": bob, "verdict": "praise" }));
    let received = notice(&w, bob, "society.opinion_received");
    assert_eq!((received.from, received.data["verdict"].clone()), (Some(ada), json!("praise")));

    ok(&mut w, cyd, "opine", json!({ "resident": bob, "verdict": "condemn" }));
    ok(&mut w, cyd, "opine", json!({ "resident": ada, "verdict": "condemn" }));
    w.step(2);
    assert_eq!((w.attr(bob, "rep.standing"), w.attr(ada, "rep.standing")), (0, -5));

    // The standing stays inside its bounds.
    w.set_attr(bob, "rep.standing", 998);
    w.step(1_440);
    ok(&mut w, ada, "opine", json!({ "resident": bob, "verdict": "praise" }));
    w.step(2);
    assert_eq!(w.attr(bob, "rep.standing"), 1_000);
    w.assert_conserved();
}

#[test]
fn opine_has_limits() {
    let mut w = world();
    w.set_param("society.opinions_per_day", 2);
    let ada = spawn_owned(&mut w, "Ada", 1);
    let twin = spawn_owned(&mut w, "Twin", 1);
    let bob = spawn_owned(&mut w, "Bob", 2);
    let cyd = spawn_owned(&mut w, "Cyd", 3);
    let dee = spawn_owned(&mut w, "Dee", 4);
    w.step(1);
    let praise = |who: u64| json!({ "resident": who, "verdict": "praise" });

    assert_eq!(fail(&mut w, ada, "opine", praise(ada.0)), ErrorCode::InvalidArgs);
    assert_eq!(
        fail(&mut w, ada, "opine", json!({ "resident": bob, "verdict": "love" })),
        ErrorCode::InvalidArgs
    );
    // A resident must live one game day before its opinion counts.
    assert_eq!(w.param("society.opinion_min_age"), 1_440);
    assert_eq!(fail(&mut w, ada, "opine", praise(bob.0)), ErrorCode::NotAllowed);
    w.step(1_438);
    assert_eq!(fail(&mut w, ada, "opine", praise(bob.0)), ErrorCode::NotAllowed);
    w.step(1);

    // One opinion about a resident in one day, and two opinions in all.
    ok(&mut w, ada, "opine", praise(bob.0));
    assert_eq!(fail(&mut w, ada, "opine", praise(bob.0)), ErrorCode::RateLimited);
    ok(&mut w, ada, "opine", praise(cyd.0));
    assert_eq!(fail(&mut w, ada, "opine", praise(dee.0)), ErrorCode::RateLimited);
    w.step(2);
    assert_eq!(
        (w.attr(bob, "rep.standing"), w.attr(cyd, "rep.standing"), w.attr(dee, "rep.standing")),
        (5, 5, 0)
    );

    // The next day the limits start again.
    w.step(1_440);
    ok(&mut w, ada, "opine", praise(bob.0));
    w.step(2);
    assert_eq!(w.attr(bob, "rep.standing"), 10);

    // An owner cannot praise its own residents: the global realm knows the
    // owners and refuses. So does it for a resident that does not exist.
    ok(&mut w, twin, "opine", praise(ada.0));
    ok(&mut w, twin, "opine", praise(999));
    w.step(2);
    assert_eq!(w.attr(ada, "rep.standing"), 0);
    assert_eq!(count_notices(&w, twin, "society.opinion_refused"), 2);
    let reasons: Vec<_> = w.notices(twin).into_iter().map(|n| n.data).collect();
    assert_eq!(
        reasons,
        [
            json!({ "about": ada, "reason": "same_owner" }),
            json!({ "about": 999, "reason": "no_such_resident" })
        ]
    );
    assert_eq!(count_notices(&w, ada, "society.opinion_received"), 0);
    w.assert_conserved();
}

/// Makes and fulfils the smallest contract: one cent from `payer` to
/// `other` at the signing. It ends when it is signed.
fn small_deal(w: &mut TestWorld<'_>, payer: ResidentId, other: ResidentId) {
    let clauses = json!([{ "kind": "pay_at_signing", "by": "offerer", "amount": 1 }]);
    let offer = ok(w, payer, "contract_offer", json!({ "to": other, "clauses": clauses }));
    let signed = ok(w, other, "contract_sign", json!({ "contract": offer["contract"] }));
    assert_eq!(signed["status"], json!("ended"));
    w.step(1);
}

#[test]
fn kept_contracts_raise_the_standing_a_limited_number_of_times_in_one_day() {
    let mut w = world();
    assert_eq!(w.param("society.rep_contract_rewards_per_day"), 2);
    assert_eq!(w.param("society.rep_contract_fulfilled"), 10);
    let ada = rich(&mut w, "Ada", 100);
    let bob = w.spawn_at("Bob", FAR);
    let cyd = w.spawn("Cyd");
    w.step(1);
    let standing =
        |w: &TestWorld<'_>| [ada, bob, cyd].map(|resident| w.attr(resident, "rep.standing"));

    small_deal(&mut w, ada, bob);
    assert_eq!(standing(&w), [10, 10, 0]);
    small_deal(&mut w, ada, bob);
    assert_eq!(standing(&w), [20, 20, 0]);
    // The third contract of the day is kept like the others, but it gives
    // no standing: many small contracts do not build a name.
    small_deal(&mut w, ada, bob);
    assert_eq!(standing(&w), [20, 20, 0]);
    // The limit is per resident. Cyd has its first reward of the day.
    small_deal(&mut w, ada, cyd);
    assert_eq!(standing(&w), [20, 20, 10]);
    assert_eq!(w.cash(ada), unsup_kernel::Money(96));

    // The next day the count starts again.
    w.step(1_440);
    small_deal(&mut w, ada, bob);
    assert_eq!(standing(&w), [30, 30, 10]);
    w.assert_conserved();
}
