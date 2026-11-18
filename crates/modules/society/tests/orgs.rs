//! Organizations: creation, members, roles, the reserved organizations and
//! what happens when a member leaves the world.

mod common;

use common::{CITY, can, enlist, events, fail, found, notice, ok, rich, world};
use serde_json::json;
use unsup_kernel::protocol::SpawnSpec;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    CityConfig, CityId, ErrorCode, Holder, Money, OrgId, OwnerId, Pos, ResidentId, Untrusted,
    WorldConfig,
};
use unsup_society::Orgs;

#[test]
fn a_founder_pays_the_fee_and_owns_the_organization() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_500);
    let fee = w.param("society.org_create_fee");
    assert_eq!(fee, 1_000);

    let ack = w.act(ada, "org_create", json!({ "name": "Acme Bakery" }));
    // The fee leaves the pocket at once and is in flight to the global realm.
    assert_eq!(w.cash(ada), Money(500));
    let frames = w.step(2);
    let report = w.last_report(ada).expect("the answer is a report");
    assert_eq!((report.action_id, report.ok), (ack.action_id, true));
    let org = OrgId(report.result["org"].as_u64().expect("an id"));
    assert!(org.0 >= 1 << 20, "new ids are above the reserved ids");
    assert_eq!(report.result, json!({ "org": org, "role": "owner", "fee": 1_000 }));

    // The fee is in the treasury of the city of the founder.
    assert_eq!(w.money(CITY), Money(1_000));
    assert_eq!(w.money(Holder::Org(org)), Money(0));
    w.assert_conserved();

    // The public event names the organization. The name is untrusted text.
    let created = events(&frames, "society.org_created");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].actor, Some(ada));
    assert_eq!(created[0].data["org"], json!(org));
    assert_eq!(created[0].data["founder"], json!(ada));
    let name: Untrusted =
        serde_json::from_value(created[0].data["name"].clone()).expect("untrusted");
    assert_eq!((name.from, name.text.as_str()), (ada, "Acme Bakery"));

    // The zone of the founder knows the membership.
    for perm in ["org.manage", "org.invite", "treasury.spend", "site.manage", "job.hire"] {
        assert!(can(&w, ada, org, perm), "{perm}");
    }
    assert!(!can(&w, ada, org, "law.arrest"));
    let orgs = w.global().res::<Orgs>();
    let record = orgs.get(org).expect("the registry has the organization");
    assert_eq!(
        (record.name(), record.founder(), record.member_count()),
        ("Acme Bakery", Some(ada), 1)
    );
    assert_eq!(record.role_of(ada), Some("owner"));
}

#[test]
fn org_create_refuses_bad_names_poor_founders_and_taken_names() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 2_500);
    let bob = rich(&mut w, "Bob", 999);

    assert_eq!(fail(&mut w, ada, "org_create", json!({ "name": "  " })), ErrorCode::InvalidArgs);
    let long = "x".repeat(41);
    assert_eq!(fail(&mut w, ada, "org_create", json!({ "name": long })), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, ada, "org_create", json!({ "name": "a\nb" })), ErrorCode::InvalidArgs);
    // One cent short: refused in the zone, nothing moves.
    assert_eq!(fail(&mut w, bob, "org_create", json!({ "name": "Bobs" })), ErrorCode::Insufficient);
    assert_eq!(w.cash(bob), Money(999));

    found(&mut w, ada, "Acme");
    assert_eq!(w.cash(ada), Money(1_500));
    // The same name, with another case and spaces around it: refused by the
    // registry, and the fee comes back.
    assert_eq!(fail(&mut w, ada, "org_create", json!({ "name": " ACME " })), ErrorCode::Conflict);
    w.step(1);
    assert_eq!(w.cash(ada), Money(1_500));
    assert_eq!(w.money(CITY), Money(1_000));
    w.assert_conserved();
}

#[test]
fn a_resident_can_be_in_a_limited_number_of_organizations() {
    let mut w = world();
    w.set_param("society.memberships_max", 2);
    w.set_param("society.org_create_fee", 0);
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.step(1);
    let one = found(&mut w, ada, "One");
    found(&mut w, ada, "Two");
    assert_eq!(fail(&mut w, ada, "org_create", json!({ "name": "Three" })), ErrorCode::Conflict);

    // The limit also stops a join.
    found(&mut w, bob, "Bob 1");
    found(&mut w, bob, "Bob 2");
    ok(&mut w, ada, "org_invite", json!({ "org": one, "resident": bob }));
    assert_eq!(fail(&mut w, bob, "org_join", json!({ "org": one })), ErrorCode::Conflict);
    assert_eq!(w.global().res::<Orgs>().membership_count(bob), 2);
    w.assert_conserved();
}

#[test]
fn invite_join_leave_and_kick() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let cyd = w.spawn_at("Cyd", common::FAR);
    let org = found(&mut w, ada, "Acme");

    // An invitation is a notice with the name as untrusted text.
    ok(&mut w, ada, "org_invite", json!({ "org": org, "resident": cyd }));
    let invited = notice(&w, cyd, "society.org_invited");
    assert_eq!(invited.from, Some(ada));
    assert_eq!(invited.data["org"], json!(org));
    assert_eq!(invited.data["name"], json!({ "untrusted": true, "from": ada, "text": "Acme" }));
    assert!(!can(&w, cyd, org, "org.invite"));

    let joined = ok(&mut w, cyd, "org_join", json!({ "org": org }));
    assert_eq!(joined, json!({ "org": org, "role": "member", "members": 2 }));
    assert_eq!(
        w.global().res::<Orgs>().get(org).and_then(|o| o.role_of(cyd).map(str::to_string)),
        Some("member".into())
    );

    // A member without `org.invite` cannot invite; without `org.manage` it cannot kick.
    assert_eq!(
        fail(&mut w, cyd, "org_invite", json!({ "org": org, "resident": bob })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, cyd, "org_kick", json!({ "org": org, "resident": ada })),
        ErrorCode::NotAllowed
    );

    // The manager removes the member. The member gets a notice.
    let frames = {
        w.act_ok(ada, "org_kick", json!({ "org": org, "resident": cyd }));
        w.step(2)
    };
    assert_eq!(w.last_report(ada).map(|r| r.result), Some(json!({ "org": org, "removed": cyd })));
    assert_eq!(notice(&w, cyd, "society.org_kicked").from, Some(ada));
    let left = events(&frames, "society.org_left");
    assert_eq!(left.len(), 1);
    assert_eq!(
        (left[0].data["resident"].clone(), left[0].data["cause"].clone()),
        (json!(cyd), json!("kicked"))
    );
    assert_eq!(w.global().res::<Orgs>().get(org).map(|o| o.member_count()), Some(1));

    // Bob joins and leaves by itself.
    enlist(&mut w, org, ada, bob);
    assert_eq!(
        ok(&mut w, bob, "org_leave", json!({ "org": org })),
        json!({ "org": org, "dissolved": false })
    );
    assert_eq!(w.global().res::<Orgs>().get(org).map(|o| o.member_count()), Some(1));
    w.assert_conserved();
}

#[test]
fn invite_and_join_refuse_what_the_registry_forbids() {
    let mut w = world();
    w.set_param("society.org_invites_max", 2);
    w.set_param("society.org_members_max", 2);
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    let dee = w.spawn("Dee");
    let eve = w.spawn("Eve");
    w.step(1);
    let org = found(&mut w, ada, "Acme");
    let invite = |who: ResidentId| json!({ "org": org, "resident": who });

    assert_eq!(
        fail(&mut w, ada, "org_invite", json!({ "org": 5_000_000, "resident": bob })),
        ErrorCode::NotFound
    );
    assert_eq!(fail(&mut w, ada, "org_invite", invite(ResidentId(999))), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, ada, "org_invite", invite(ada)), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, bob, "org_join", json!({ "org": org })), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, bob, "org_join", json!({ "org": 5_000_000 })), ErrorCode::NotFound);

    ok(&mut w, ada, "org_invite", invite(bob));
    assert_eq!(fail(&mut w, ada, "org_invite", invite(bob)), ErrorCode::Conflict);
    ok(&mut w, ada, "org_invite", invite(cyd));
    // Two invitations are open; that is the limit.
    assert_eq!(fail(&mut w, ada, "org_invite", invite(dee)), ErrorCode::Conflict);

    ok(&mut w, bob, "org_join", json!({ "org": org }));
    assert_eq!(fail(&mut w, bob, "org_join", json!({ "org": org })), ErrorCode::Conflict);
    // Two members are the limit of this world.
    assert_eq!(fail(&mut w, cyd, "org_join", json!({ "org": org })), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, eve, "org_leave", json!({ "org": org })), ErrorCode::Conflict);
    // No organization has this id: every verb says so.
    let nowhere = 5_000_000;
    assert_eq!(fail(&mut w, ada, "org_leave", json!({ "org": nowhere })), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, ada, "org_kick", json!({ "org": nowhere, "resident": bob })),
        ErrorCode::NotFound
    );
    let role = json!({ "org": nowhere, "role": "clerk", "permissions": [] });
    assert_eq!(fail(&mut w, ada, "org_define_role", role), ErrorCode::NotFound);
    let set = json!({ "org": nowhere, "resident": bob, "role": "member" });
    assert_eq!(fail(&mut w, ada, "org_set_role", set), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, ada, "org_kick", invite(ada)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, ada, "org_kick", invite(eve)), ErrorCode::NotFound);
    w.assert_conserved();
}

/// Steps the world until `frame` frames have run.
fn run_to(w: &mut TestWorld<'_>, frame: u64) {
    let left = frame.checked_sub(w.world.frame()).expect("the frame is not in the past");
    w.step(u32::try_from(left).expect("a short wait"));
}

#[test]
fn an_invitation_ends_after_some_time() {
    let mut w = world();
    w.set_param("society.org_invite_ttl", 30);
    w.set_param("society.org_invites_max", 2);
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    let dee = w.spawn("Dee");
    let eve = w.spawn("Eve");
    let org = found(&mut w, ada, "Acme");
    let invite = |who: ResidentId| json!({ "org": org, "resident": who });

    // The answer and the notice say when the invitation ends: 30 frames
    // after the frame in which the global realm made it.
    let invited = ok(&mut w, ada, "org_invite", invite(cyd));
    let ends = w.world.frame() - 1 + 30;
    assert_eq!(invited, json!({ "org": org, "invited": cyd, "expires": ends }));
    assert_eq!(notice(&w, cyd, "society.org_invited").data["expires"], json!(ends));
    // Bob is invited two frames later.
    assert_eq!(ok(&mut w, ada, "org_invite", invite(bob))["expires"], json!(ends + 2));
    assert_eq!(fail(&mut w, ada, "org_invite", invite(dee)), ErrorCode::Conflict);

    // Cyd asks one frame after the end of its invitation: too late. Bob
    // asks in the last frame of its own: in time.
    run_to(&mut w, ends);
    let late = w.act(cyd, "org_join", json!({ "org": org }));
    w.step(1);
    let in_time = w.act(bob, "org_join", json!({ "org": org }));
    w.step(2);
    let report = |r: ResidentId, action_id: u64| {
        w.reports(r).into_iter().find(|report| report.action_id == action_id).expect("a report")
    };
    assert_eq!(
        report(cyd, late.action_id).error.map(|e| e.code),
        Some(ErrorCode::NotAllowed),
        "the invitation of Cyd ended"
    );
    assert_eq!(
        report(bob, in_time.action_id).result,
        json!({ "org": org, "role": "member", "members": 2 })
    );
    // An invitation that ended is not in the observation.
    assert!(!w.observe(cyd).sections.contains_key("orgs"));

    // It does not count for the limit, and it does not stop a new one.
    ok(&mut w, ada, "org_invite", invite(dee));
    ok(&mut w, ada, "org_invite", invite(cyd));
    assert_eq!(fail(&mut w, ada, "org_invite", invite(eve)), ErrorCode::Conflict);
    ok(&mut w, cyd, "org_join", json!({ "org": org }));
    w.assert_conserved();
}

#[test]
fn the_registry_forgets_invitations_that_ended() {
    let mut w = world();
    w.set_param("society.org_invite_ttl", 30);
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let org = found(&mut w, ada, "Acme");
    let invited = ok(&mut w, ada, "org_invite", json!({ "org": org, "resident": bob }));
    let ends = invited["expires"].as_u64().expect("a frame");
    let count = |w: &TestWorld<'_>| w.global().res::<Orgs>().get(org).map(|o| o.invite_count());
    assert!(ends < 59, "the invitation ends before the first clean-up");

    // The clean-up runs once in each game hour. Until then the record keeps
    // the invitation that ended.
    run_to(&mut w, 59);
    assert_eq!(count(&w), Some(1));
    run_to(&mut w, 60);
    assert_eq!(count(&w), Some(0));
    // The invitation is gone for the resident too.
    assert_eq!(fail(&mut w, bob, "org_join", json!({ "org": org })), ErrorCode::NotAllowed);
    w.assert_conserved();
}

#[test]
fn an_open_organization_needs_no_invitation() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let result = ok(&mut w, ada, "org_create", json!({ "name": "Dockers Union", "open": true }));
    let union = OrgId(result["org"].as_u64().expect("an id"));
    assert_eq!(ok(&mut w, bob, "org_join", json!({ "org": union }))["members"], json!(2));
    w.assert_conserved();
}

#[test]
fn the_last_manager_cannot_leave_others_behind_and_the_last_member_gets_the_treasury() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_700);
    let bob = w.spawn("Bob");
    let org = found(&mut w, ada, "Acme");
    enlist(&mut w, org, ada, bob);
    ok(&mut w, ada, "org_deposit", json!({ "org": org, "amount": 700 }));
    assert_eq!((w.cash(ada), w.money(Holder::Org(org))), (Money(0), Money(700)));

    // Ada is the only manager. She must hand the organization over first.
    assert_eq!(fail(&mut w, ada, "org_leave", json!({ "org": org })), ErrorCode::Conflict);
    ok(&mut w, ada, "org_set_role", json!({ "org": org, "resident": bob, "role": "owner" }));
    assert_eq!(ok(&mut w, ada, "org_leave", json!({ "org": org }))["dissolved"], json!(false));
    w.step(1);
    assert!(!can(&w, ada, org, "org.manage"));

    // Bob is the last member. When he leaves, the organization ends and the
    // treasury is his.
    w.act_ok(bob, "org_leave", json!({ "org": org }));
    let frames = w.step(3);
    let dissolved = events(&frames, "society.org_dissolved");
    assert_eq!(dissolved.len(), 1);
    assert_eq!(
        (dissolved[0].data["treasury"].clone(), dissolved[0].data["to"].clone()),
        (json!(700), json!(bob))
    );
    assert_eq!((w.cash(bob), w.money(Holder::Org(org))), (Money(700), Money(0)));
    assert!(w.global().res::<Orgs>().get(org).is_none());
    // The name is free again.
    w.give_money(ada, Money(1_000));
    found(&mut w, ada, "Acme");
    w.assert_conserved();
}

#[test]
fn roles_are_defined_and_set_within_the_rules() {
    let mut w = world();
    w.set_param("society.org_roles_max", 3);
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    w.step(1);
    let org = found(&mut w, ada, "Acme");
    enlist(&mut w, org, ada, bob);
    let role =
        |name: &str, perms: &[&str]| json!({ "org": org, "role": name, "permissions": perms });

    // Permissions come back sorted and once each. A power of the state is
    // listed as without effect, because the organization has no grant.
    let made = ok(
        &mut w,
        ada,
        "org_define_role",
        role("clerk", &["treasury.spend", "law.arrest", "org.invite", "org.invite"]),
    );
    assert_eq!(
        made,
        json!({ "org": org, "role": "clerk", "permissions": ["law.arrest", "org.invite", "treasury.spend"], "inactive": ["law.arrest"] })
    );

    assert_eq!(fail(&mut w, bob, "org_define_role", role("boss", &[])), ErrorCode::NotAllowed);
    assert_eq!(
        fail(&mut w, ada, "org_define_role", role("Not A Name", &[])),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(&mut w, ada, "org_define_role", role("clerk", &["fly"])),
        ErrorCode::InvalidArgs
    );
    // `owner`, `member` and `clerk` are three roles: the limit of this world.
    assert_eq!(fail(&mut w, ada, "org_define_role", role("fourth", &[])), ErrorCode::Conflict);
    // The only manager cannot take `org.manage` from its own role.
    assert_eq!(
        fail(&mut w, ada, "org_define_role", role("owner", &["org.invite"])),
        ErrorCode::Conflict
    );

    let set = |who: ResidentId, name: &str| json!({ "org": org, "resident": who, "role": name });
    assert_eq!(fail(&mut w, bob, "org_set_role", set(bob, "owner")), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, ada, "org_set_role", set(cyd, "clerk")), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, ada, "org_set_role", set(bob, "nothing")), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, ada, "org_set_role", set(ada, "member")), ErrorCode::Conflict);

    assert_eq!(
        ok(&mut w, ada, "org_set_role", set(bob, "clerk")),
        json!({ "org": org, "resident": bob, "role": "clerk" })
    );
    let changed = notice(&w, bob, "society.org_role_changed");
    assert_eq!((changed.from, changed.data["role"].clone()), (Some(ada), json!("clerk")));
    w.assert_conserved();
}

#[test]
fn the_reserved_organizations_exist_from_the_start_without_members() {
    let w = world();
    let orgs = w.global().res::<Orgs>();
    assert_eq!(orgs.len(), 3);
    for id in [OrgId::NATION, OrgId::CENTRAL_BANK, OrgId::city(CityId(0))] {
        let org = orgs.get(id).expect("a reserved organization");
        assert!(org.is_government());
        assert_eq!((org.member_count(), org.name(), org.founder()), (0, "", None));
    }
    assert!(orgs.get(OrgId::city(CityId(1))).is_none(), "the world has one city");
}

#[test]
fn each_city_has_a_government_that_gets_the_fees_of_its_founders() {
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("Ashford", Pos::new(0, 0), 1),
            CityConfig::new("Brindle", Pos::new(1_000, 0), 1),
        ],
        ..WorldConfig::default()
    };
    let mut w = TestWorld::with_config(&common::plugins(), config);
    let (first, second) = (OrgId::city(CityId(0)), OrgId::city(CityId(1)));
    {
        let orgs = w.global().res::<Orgs>();
        // The nation, the central bank and one government for each city.
        assert_eq!(orgs.len(), 4);
        for id in [first, second] {
            let org = orgs.get(id).expect("the government of a city");
            assert!(org.is_government());
            assert_eq!(org.member_count(), 0);
        }
    }

    // A founder in the second city pays the fee to that city.
    let spec =
        SpawnSpec { owner: OwnerId(1), name: "Bea".into(), model: None, city: Some(CityId(1)) };
    let bea = w.world.spawn(spec).expect("the spawn is valid");
    w.give_money(bea, Money(1_000));
    found(&mut w, bea, "Brindle Mill");
    assert_eq!(
        (w.money(Holder::Org(first)), w.money(Holder::Org(second)), w.cash(bea)),
        (Money(0), Money(1_000), Money(0))
    );
    w.assert_conserved();
}

#[test]
fn a_member_that_leaves_the_world_leaves_its_organizations() {
    let mut w = world();
    w.set_param("society.org_invites_max", 1);
    let ada = rich(&mut w, "Ada", 1_300);
    let bob = w.spawn("Bob");
    let cyd = w.spawn_at("Cyd", common::FAR);
    let dee = w.spawn("Dee");
    let eve = w.spawn("Eve");
    let org = found(&mut w, ada, "Acme");
    enlist(&mut w, org, ada, cyd);
    enlist(&mut w, org, ada, bob);
    ok(&mut w, ada, "org_deposit", json!({ "org": org, "amount": 300 }));
    ok(&mut w, ada, "org_invite", json!({ "org": org, "resident": dee }));

    // The invited resident dies: its invitation no longer counts.
    w.despawn(dee);
    w.step(1);
    ok(&mut w, ada, "org_invite", json!({ "org": org, "resident": eve }));

    // The only manager dies. The member with the lowest id takes its role.
    assert!(!can(&w, bob, org, "org.manage"));
    w.despawn(ada);
    let frames = w.step(2);
    assert_eq!(events(&frames, "society.org_left").len(), 1);
    assert!(can(&w, bob, org, "org.manage"));
    assert!(!can(&w, cyd, org, "org.manage"));
    assert_eq!(notice(&w, bob, "society.org_role_changed").data["role"], json!("owner"));
    {
        let orgs = w.global().res::<Orgs>();
        let record = orgs.get(org).expect("the organization lives on");
        assert_eq!(record.members().collect::<Vec<_>>(), [(bob, "owner"), (cyd, "member")]);
        assert_eq!(orgs.membership_count(ada), 0);
    }

    // The last members die. The organization ends, and the nation gets the
    // treasury: nobody is left to take it.
    w.despawn(cyd);
    w.step(1);
    assert_eq!(w.global().res::<Orgs>().get(org).map(|o| o.member_count()), Some(1));
    w.despawn(bob);
    let frames = w.step(1);
    let dissolved = events(&frames, "society.org_dissolved");
    assert_eq!(dissolved.len(), 1);
    assert_eq!(dissolved[0].data["to"], json!(null));
    assert!(w.global().res::<Orgs>().get(org).is_none());
    assert_eq!(w.money(Holder::Org(org)), Money(0));
    assert_eq!(w.money(Holder::Org(OrgId::NATION)), Money(300));
    w.assert_conserved();
}
