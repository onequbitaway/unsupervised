//! Relations: private labels, marriage, divorce and households, with their
//! effects on the `Kin` component and on the tenants of a home.

mod common;

use common::{
    FAR, FLAT, HOME, SHOP, events, fail, give_site, kin, notice, notice_kinds, ok, site, tenants,
    world,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Pos, ResidentId, ZoneId};

/// Returns the `relations` section of a resident.
fn relations(w: &TestWorld<'_>, r: ResidentId) -> Option<Value> {
    w.observe(r).sections.get("relations").cloned()
}

#[test]
fn relate_keeps_private_labels() {
    let mut w = world();
    w.set_param("society.relations_max", 3);
    // `relate` is free, and the kernel limits free actions per tick. This
    // test makes many of them in one tick.
    w.set_param("kernel.free_actions_per_tick", 100);
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.step(1);
    assert_eq!(relations(&w, ada), None, "a section with nothing to say is absent");

    let label = |who: u64, label: &str| json!({ "resident": who, "label": label });
    assert_eq!(
        ok(&mut w, ada, "relate", label(7, "friend")),
        json!({ "resident": 7, "label": "friend" })
    );
    ok(&mut w, ada, "relate", label(bob.0, "enemy"));
    ok(&mut w, ada, "relate", label(5, "friend"));
    assert_eq!(relations(&w, ada), Some(json!({ "labels": { "friend": [5, 7], "enemy": [bob] } })));
    // The label is private: the other resident sees and hears nothing.
    assert_eq!(relations(&w, bob), None);
    assert!(w.notices(bob).is_empty());

    assert_eq!(
        fail(&mut w, ada, "relate", label(1, "friend")),
        ErrorCode::InvalidArgs,
        "Ada is resident 1"
    );
    assert_eq!(fail(&mut w, ada, "relate", label(9, "lover")), ErrorCode::InvalidArgs);
    // Three labels are the limit of this world. A change of a label is not a
    // new label.
    assert_eq!(fail(&mut w, ada, "relate", label(9, "ally")), ErrorCode::Conflict);
    ok(&mut w, ada, "relate", label(bob.0, "rival"));
    assert_eq!(
        ok(&mut w, ada, "relate", label(7, "none")),
        json!({ "resident": 7, "label": null })
    );
    ok(&mut w, ada, "relate", label(9, "ally"));
    assert_eq!(
        relations(&w, ada),
        Some(json!({ "labels": { "friend": [5], "rival": [bob], "ally": [9] } }))
    );
    // To remove a label that is not there is fine.
    ok(&mut w, ada, "relate", label(77, "none"));

    // The labels move with the resident and survive a snapshot.
    w.set_pos(ada, FAR);
    w.snapshot_restore();
    assert_eq!(relations(&w, ada).map(|r| r["labels"]["rival"].clone()), Some(json!([bob])));
    w.assert_conserved();
}

#[test]
fn a_proposal_that_is_accepted_marries_both_residents_in_any_zone() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn_at("Bob", FAR);
    assert_eq!(w.zone_of(bob), ZoneId(3));
    assert_eq!((kin(&w, ada).spouse, kin(&w, bob).spouse), (None, None));

    let proposed = ok(&mut w, ada, "propose", json!({ "resident": bob }));
    let expires = proposed["expires"].as_u64().expect("a frame");
    assert_eq!(proposed, json!({ "to": bob, "expires": expires }));
    assert_eq!(expires, w.world.frame() - 1 + 1_440);
    let asked = notice(&w, bob, "society.proposal_received");
    assert_eq!(
        (asked.from, asked.data.clone()),
        (Some(ada), json!({ "ask": "marriage", "expires": expires }))
    );
    assert_eq!(
        relations(&w, bob),
        Some(json!({ "proposals": [{ "from": ada, "ask": "marriage", "expires": expires }] }))
    );

    w.act_ok(bob, "accept_proposal", json!({ "resident": ada }));
    let frames = w.step(2);
    assert_eq!(w.last_report(bob).map(|r| r.result), Some(json!({ "spouse": ada })));
    let married = events(&frames, "society.married");
    assert_eq!(married.len(), 1);
    assert_eq!(married[0].data, json!({ "a": ada, "b": bob }));
    // The kernel component of both has the spouse, in both zones.
    assert_eq!((kin(&w, ada).spouse, kin(&w, bob).spouse), (Some(bob), Some(ada)));
    assert_eq!(kin(&w, ada).successor(), Some(bob), "a spouse inherits when there is no will");
    assert_eq!(notice(&w, ada, "society.proposal_accepted").from, Some(bob));
    assert_eq!(relations(&w, bob), Some(json!({ "spouse": ada })), "the answered proposal is gone");
    assert_eq!(relations(&w, ada), Some(json!({ "spouse": bob })));
    w.assert_conserved();
}

#[test]
fn propose_and_accept_refuse_what_cannot_be() {
    let mut w = world();
    w.set_param("society.proposal_ttl", 10);
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    let dee = w.spawn("Dee");
    w.step(1);
    let with = |who: ResidentId| json!({ "resident": who });

    assert_eq!(fail(&mut w, ada, "propose", with(ada)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, ada, "propose", with(ResidentId(999))), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, bob, "accept_proposal", with(ada)),
        ErrorCode::NotFound,
        "nobody proposed"
    );
    assert_eq!(fail(&mut w, ada, "divorce", json!({})), ErrorCode::Conflict);

    // A proposal is for one resident. Another one cannot accept it.
    ok(&mut w, ada, "propose", with(bob));
    assert_eq!(fail(&mut w, cyd, "accept_proposal", with(ada)), ErrorCode::NotFound);
    // A proposal ends after its time.
    w.step(10);
    assert_eq!(fail(&mut w, bob, "accept_proposal", with(ada)), ErrorCode::NotFound);
    assert_eq!(common::kin(&w, bob).spouse, None);

    // A new proposal replaces the old one.
    ok(&mut w, ada, "propose", with(bob));
    ok(&mut w, ada, "propose", with(cyd));
    assert_eq!(fail(&mut w, bob, "accept_proposal", with(ada)), ErrorCode::NotFound);
    ok(&mut w, cyd, "accept_proposal", with(ada));

    // Married residents cannot propose, be proposed to, or accept.
    assert_eq!(fail(&mut w, ada, "propose", with(dee)), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, dee, "propose", with(cyd)), ErrorCode::Conflict);
    ok(&mut w, dee, "propose", with(bob));
    ok(&mut w, bob, "propose", with(dee));
    ok(&mut w, bob, "accept_proposal", with(dee));
    // Dee is married now, so its own open proposal is gone too.
    assert_eq!(fail(&mut w, dee, "accept_proposal", with(bob)), ErrorCode::NotFound);
    w.step(1);
    assert_eq!((kin(&w, bob).spouse, kin(&w, dee).spouse), (Some(dee), Some(bob)));
    w.assert_conserved();
}

#[test]
fn a_married_resident_cannot_accept_an_older_proposal() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    w.step(1);
    let with = |who: ResidentId| json!({ "resident": who });

    // Ada and Cyd both propose to Bob. Bob accepts Cyd.
    ok(&mut w, ada, "propose", with(bob));
    ok(&mut w, cyd, "propose", with(bob));
    ok(&mut w, bob, "accept_proposal", with(cyd));
    // The proposal of Ada is still open, but Bob is married now.
    assert_eq!(fail(&mut w, bob, "accept_proposal", with(ada)), ErrorCode::Conflict);
    w.step(1);
    assert_eq!(
        (kin(&w, ada).spouse, kin(&w, bob).spouse, kin(&w, cyd).spouse),
        (None, Some(cyd), Some(bob))
    );

    // After a divorce Bob is free to accept it.
    ok(&mut w, bob, "divorce", json!({}));
    ok(&mut w, bob, "accept_proposal", with(ada));
    w.step(1);
    assert_eq!(
        (kin(&w, ada).spouse, kin(&w, bob).spouse, kin(&w, cyd).spouse),
        (Some(bob), Some(ada), None)
    );
    w.assert_conserved();
}

#[test]
fn a_divorce_needs_one_spouse_and_clears_both() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn_at("Bob", FAR);
    ok(&mut w, ada, "propose", json!({ "resident": bob }));
    ok(&mut w, bob, "accept_proposal", json!({ "resident": ada }));
    w.step(1);
    assert_eq!(kin(&w, ada).spouse, Some(bob));

    w.act_ok(bob, "divorce", json!({}));
    let frames = w.step(3);
    assert_eq!(w.last_report(bob).map(|r| r.result), Some(json!({ "former_spouse": ada })));
    assert_eq!(events(&frames, "society.divorced")[0].data, json!({ "a": bob, "b": ada }));
    assert_eq!((kin(&w, ada).spouse, kin(&w, bob).spouse), (None, None));
    assert_eq!(notice(&w, ada, "society.divorce").from, Some(bob));
    assert_eq!(relations(&w, ada), None);
    assert_eq!(fail(&mut w, ada, "divorce", json!({})), ErrorCode::Conflict);
    // Both are free to marry again.
    ok(&mut w, ada, "propose", json!({ "resident": bob }));
    w.assert_conserved();
}

#[test]
fn a_spouse_that_leaves_the_world_leaves_a_widow() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn_at("Bob", FAR);
    let cyd = w.spawn("Cyd");
    ok(&mut w, ada, "propose", json!({ "resident": bob }));
    ok(&mut w, bob, "accept_proposal", json!({ "resident": ada }));
    w.step(1);
    assert_eq!(kin(&w, bob).spouse, Some(ada));

    w.despawn(ada);
    w.step(2);
    assert_eq!(kin(&w, bob).spouse, None);
    let widowed = notice(&w, bob, "society.widowed");
    assert_eq!((widowed.from, widowed.data.clone()), (None, json!({ "spouse": ada })));
    assert_eq!(relations(&w, bob), None);
    // Nobody can propose to a resident that left the world.
    assert_eq!(fail(&mut w, cyd, "propose", json!({ "resident": ada })), ErrorCode::NotFound);
    // The widow is free to marry again.
    ok(&mut w, cyd, "propose", json!({ "resident": bob }));
    ok(&mut w, bob, "accept_proposal", json!({ "resident": cyd }));
    w.step(1);
    assert_eq!((kin(&w, bob).spouse, kin(&w, cyd).spouse), (Some(cyd), Some(bob)));
    w.assert_conserved();
}

/// Puts a host in its home in zone 0 and lets it invite a guest.
fn invite(w: &mut TestWorld<'_>, host: ResidentId, guest: ResidentId) -> u64 {
    let home = site(w, 0, "home");
    give_site(w, 0, home, host);
    w.set_pos(host, Pos::new(HOME.x0 + 1, HOME.y0 + 1));
    let result = ok(w, host, "household_invite", json!({ "resident": guest }));
    w.step(1);
    result["household"].as_u64().expect("the result has the id")
}

#[test]
fn a_household_makes_its_members_tenants_of_the_home() {
    let mut w = world();
    let host = w.spawn("Host");
    let guest = w.spawn_at("Guest", FAR);
    let home = site(&w, 0, "home");
    assert_eq!((kin(&w, host).household, tenants(&w, 0, home)), (None, vec![]));

    let household = invite(&mut w, host, guest);
    assert!(household >= 1 << 20);
    // The host is in its household at once. The guest is only invited.
    assert_eq!((kin(&w, host).household, kin(&w, guest).household), (Some(household), None));
    let asked = notice(&w, guest, "society.proposal_received");
    assert_eq!(asked.from, Some(host));
    assert_eq!(
        (asked.data["ask"].clone(), asked.data["site"].clone()),
        (json!("household"), json!(home))
    );
    assert_eq!(
        relations(&w, guest).map(|r| r["proposals"][0]["ask"].clone()),
        Some(json!("household"))
    );
    assert_eq!(tenants(&w, 0, home), []);

    // The guest joins from another zone. The zone of the home adds it to
    // the tenants; the zone of the guest writes its `Kin`.
    w.act_ok(guest, "household_join", json!({ "host": host }));
    let frames = w.step(3);
    assert_eq!(
        w.last_report(guest).map(|r| r.result),
        Some(json!({ "household": household, "host": host, "site": home, "zone": 0 }))
    );
    assert_eq!(kin(&w, guest).household, Some(household));
    assert_eq!(tenants(&w, 0, home), [guest]);
    let joined = events(&frames, "society.household_joined");
    assert_eq!(joined.len(), 1);
    assert_eq!(
        joined[0].data,
        json!({ "household": household, "resident": guest, "host": host, "site": home })
    );
    assert_eq!(notice(&w, host, "society.household_joined").from, Some(guest));
    assert_eq!(relations(&w, guest), Some(json!({ "household": household })));

    // The guest leaves: it is no tenant any more. The host keeps its home
    // and its household.
    w.act_ok(guest, "household_leave", json!({}));
    let frames = w.step(3);
    assert_eq!(w.last_report(guest).map(|r| r.result), Some(json!({ "household": household })));
    assert_eq!((kin(&w, guest).household, kin(&w, host).household), (None, Some(household)));
    assert_eq!(tenants(&w, 0, home), []);
    assert_eq!(
        events(&frames, "society.household_left")[0].data,
        json!({ "household": household, "resident": guest, "host": host })
    );
    assert_eq!(fail(&mut w, guest, "household_leave", json!({})), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn a_household_ends_when_the_host_leaves_and_a_lease_survives_it() {
    let mut w = world();
    let host = w.spawn("Host");
    let lodger = w.spawn("Lodger");
    let renter = w.spawn("Renter");
    let home = site(&w, 0, "home");
    // The renter has a lease of its own in the same home: the economy
    // module wrote it.
    w.zone(ZoneId(0)).sites_mut().get_mut(home).expect("the home").add_tenant(renter);

    let household = invite(&mut w, host, lodger);
    ok(&mut w, lodger, "household_join", json!({ "host": host }));
    ok(&mut w, host, "household_invite", json!({ "resident": renter }));
    ok(&mut w, renter, "household_join", json!({ "host": host }));
    w.step(1);
    assert_eq!(tenants(&w, 0, home), [renter, lodger]);
    assert_eq!(kin(&w, renter).household, Some(household));

    w.act_ok(host, "household_leave", json!({}));
    let frames = w.step(3);
    assert_eq!(
        events(&frames, "society.household_dissolved")[0].data,
        json!({ "household": household, "host": host, "cause": "left" })
    );
    for member in [host, lodger, renter] {
        assert_eq!(kin(&w, member).household, None);
    }
    // The lodger was a tenant only through the household. The renter keeps
    // the lease that it had before.
    assert_eq!(tenants(&w, 0, home), [renter]);
    assert_eq!(
        notice(&w, lodger, "society.household_dissolved").data,
        json!({ "household": household, "cause": "left" })
    );
    w.assert_conserved();
}

#[test]
fn household_verbs_refuse_what_the_rules_forbid() {
    let mut w = world();
    w.set_param("society.household_members_max", 3);
    let host = w.spawn("Host");
    let guest = w.spawn("Guest");
    let other = w.spawn("Other");
    let third = w.spawn("Third");
    let fourth = w.spawn("Fourth");
    let home = site(&w, 0, "home");
    let invite_args = |who: ResidentId| json!({ "resident": who });

    // The host must stand in a home that it owns or rents.
    assert_eq!(
        fail(&mut w, host, "household_invite", invite_args(guest)),
        ErrorCode::NotHere,
        "in the street"
    );
    w.set_pos(host, Pos::new(HOME.x0, HOME.y0));
    assert_eq!(
        fail(&mut w, host, "household_invite", invite_args(guest)),
        ErrorCode::NotHere,
        "not its home"
    );
    let shop = site(&w, 0, "shop");
    give_site(&w, 0, shop, host);
    w.set_pos(host, Pos::new(SHOP.x0, SHOP.y0));
    assert_eq!(
        fail(&mut w, host, "household_invite", invite_args(guest)),
        ErrorCode::NotHere,
        "a shop is no home"
    );
    // A tenant can be a host too.
    w.zone(ZoneId(0)).sites_mut().get_mut(home).expect("the home").add_tenant(host);
    w.set_pos(host, Pos::new(HOME.x0, HOME.y0));
    assert_eq!(fail(&mut w, host, "household_invite", invite_args(host)), ErrorCode::InvalidArgs);
    assert_eq!(
        fail(&mut w, host, "household_invite", invite_args(ResidentId(999))),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, guest, "household_join", json!({ "host": host })),
        ErrorCode::NotAllowed,
        "no invitation"
    );

    ok(&mut w, host, "household_invite", invite_args(guest));
    assert_eq!(
        fail(&mut w, other, "household_join", json!({ "host": host })),
        ErrorCode::NotAllowed
    );
    ok(&mut w, guest, "household_join", json!({ "host": host }));
    assert_eq!(
        fail(&mut w, host, "household_invite", invite_args(guest)),
        ErrorCode::Conflict,
        "a member already"
    );
    // Only the host invites.
    w.set_pos(guest, Pos::new(HOME.x0, HOME.y0));
    assert_eq!(fail(&mut w, guest, "household_invite", invite_args(other)), ErrorCode::Conflict);
    // Three members are the limit, with the open invitations.
    ok(&mut w, host, "household_invite", invite_args(other));
    assert_eq!(fail(&mut w, host, "household_invite", invite_args(third)), ErrorCode::Conflict);

    // A resident is in one household. It must leave before it joins another.
    let flat = site(&w, 3, "apartment");
    give_site(&w, 3, flat, fourth);
    w.set_pos(fourth, Pos::new(FLAT.x0, FLAT.y0));
    ok(&mut w, fourth, "household_invite", invite_args(guest));
    assert_eq!(
        fail(&mut w, guest, "household_join", json!({ "host": fourth })),
        ErrorCode::Conflict
    );
    // A household with a lodger does not move by an invitation from
    // another home.
    give_site(&w, 3, flat, host);
    w.set_pos(host, Pos::new(FLAT.x0, FLAT.y0));
    assert_eq!(fail(&mut w, host, "household_invite", invite_args(third)), ErrorCode::Conflict);
    assert_eq!(tenants(&w, 0, home), [host, guest]);
    w.assert_conserved();
}

#[test]
fn a_household_that_is_full_takes_no_more_members() {
    let mut w = world();
    w.set_param("society.household_members_max", 3);
    let host = w.spawn("Host");
    let one = w.spawn("One");
    let two = w.spawn("Two");
    let three = w.spawn("Three");
    let home = site(&w, 0, "home");
    w.step(1);
    let household = invite(&mut w, host, one);
    ok(&mut w, host, "household_invite", json!({ "resident": two }));
    // The host and two open invitations make three: the limit.
    assert_eq!(
        fail(&mut w, host, "household_invite", json!({ "resident": three })),
        ErrorCode::Conflict
    );

    // A smaller limit comes while both invitations are open. The first
    // guest fills the household. The second one is refused.
    w.set_param("society.household_members_max", 2);
    ok(&mut w, one, "household_join", json!({ "host": host }));
    assert_eq!(fail(&mut w, two, "household_join", json!({ "host": host })), ErrorCode::Conflict);
    w.step(1);
    assert_eq!(tenants(&w, 0, home), [one]);
    assert_eq!((kin(&w, one).household, kin(&w, two).household), (Some(household), None));
    w.assert_conserved();
}

#[test]
fn a_household_ends_when_the_host_loses_the_home_or_leaves_the_world() {
    let mut w = world();
    let host = w.spawn("Host");
    let lodger = w.spawn("Lodger");
    let home = site(&w, 0, "home");
    let household = invite(&mut w, host, lodger);
    ok(&mut w, lodger, "household_join", json!({ "host": host }));
    w.step(1);
    assert_eq!(tenants(&w, 0, home), [lodger]);

    // The eviction of another tenant, or from another site, changes nothing.
    ok(&mut w, host, "evict_test", json!({ "site": home, "tenant": 999 }));
    ok(&mut w, host, "evict_test", json!({ "site": 12_345, "tenant": host }));
    w.step(2);
    assert_eq!(kin(&w, lodger).household, Some(household));

    // The host sells the home: the economy module says so with an event.
    ok(&mut w, host, "sell_test", json!({ "site": home }));
    let frames = w.step(3);
    assert_eq!(events(&frames, "society.household_dissolved")[0].data["cause"], json!("home_lost"));
    assert_eq!((kin(&w, host).household, kin(&w, lodger).household), (None, None));
    assert_eq!(tenants(&w, 0, home), []);

    // A new household. This time the lodger is evicted: only it leaves.
    let household = invite(&mut w, host, lodger);
    ok(&mut w, lodger, "household_join", json!({ "host": host }));
    ok(&mut w, host, "evict_test", json!({ "site": home, "tenant": lodger }));
    w.step(3);
    assert_eq!((kin(&w, host).household, kin(&w, lodger).household), (Some(household), None));

    // The host leaves the world: the household ends.
    ok(&mut w, host, "household_invite", json!({ "resident": lodger }));
    ok(&mut w, lodger, "household_join", json!({ "host": host }));
    w.step(1);
    assert_eq!(tenants(&w, 0, home), [lodger]);
    w.despawn(host);
    w.step(3);
    assert_eq!(kin(&w, lodger).household, None);
    assert_eq!(tenants(&w, 0, home), []);
    assert_eq!(
        notice_kinds(&w, lodger).last().map(String::as_str),
        Some("society.household_dissolved")
    );
    w.assert_conserved();
}
