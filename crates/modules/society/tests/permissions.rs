//! The mirror of memberships: what `realm.can` says in the zone of a member
//! after each change of a role, a grant or a membership. Also the grants of
//! government organizations and the offices that other modules fill.

mod common;

use common::{FAR, can, enlist, events, fail, found, notice, notice_kinds, ok, rich, world};
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, ErrorCode, OrgId, ResidentId, ZoneId};
use unsup_society::offices::{self, OfficeError};
use unsup_society::{Orgs, permissions};

const CITY: OrgId = OrgId::city(CityId(0));

/// Makes a resident the mayor of the city, as the politics module would:
/// through the functions for offices, in the global realm.
fn elect_mayor(w: &mut TestWorld<'_>, mayor: ResidentId) {
    // The global realm learns about a new resident in its next tick.
    w.step(1);
    offices::define_role(
        w.global(),
        CITY,
        "mayor",
        &["gov.appoint", "treasury.spend", "law.judge"],
    )
    .expect("the role is valid");
    offices::set_holders(w.global(), CITY, "mayor", &[mayor]).expect("the mayor is alive");
    w.world.settle();
    w.step(1);
}

#[test]
fn a_member_gains_and_loses_permissions_with_its_role_in_any_zone() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let near = w.spawn("Near");
    let far = w.spawn_at("Far", FAR);
    assert_eq!((w.zone_of(near), w.zone_of(far)), (ZoneId(0), ZoneId(3)));
    let org = found(&mut w, ada, "Acme");

    for member in [near, far] {
        assert!(!can(&w, member, org, "treasury.spend"));
        ok(&mut w, ada, "org_invite", json!({ "org": org, "resident": member }));
        ok(&mut w, member, "org_join", json!({ "org": org }));
        w.step(1);
        // A new member has the role `member`: no permission at all.
        let role = w.with_resident(member, |realm, idx| realm.memberships(idx).to_vec());
        assert_eq!(role.len(), 1);
        assert_eq!((role[0].org, role[0].role.as_str(), role[0].perms.len()), (org, "member", 0));

        // A role with the permission: the zone of the member knows it one
        // frame after the answer.
        ok(
            &mut w,
            ada,
            "org_define_role",
            json!({ "org": org, "role": "cashier", "permissions": ["treasury.spend"] }),
        );
        ok(
            &mut w,
            ada,
            "org_set_role",
            json!({ "org": org, "resident": member, "role": "cashier" }),
        );
        w.step(1);
        assert!(can(&w, member, org, "treasury.spend"));
        assert!(!can(&w, member, org, "org.manage"));

        // The role is defined again without the permission: every holder
        // loses it.
        ok(
            &mut w,
            ada,
            "org_define_role",
            json!({ "org": org, "role": "cashier", "permissions": ["org.invite"] }),
        );
        w.step(1);
        assert!(!can(&w, member, org, "treasury.spend"));
        assert!(can(&w, member, org, "org.invite"));

        // The membership ends: nothing is left.
        ok(&mut w, ada, "org_kick", json!({ "org": org, "resident": member }));
        w.step(1);
        assert!(!can(&w, member, org, "org.invite"));
        assert!(w.with_resident(member, |realm, idx| realm.memberships(idx).is_empty()));
    }
    w.assert_conserved();
}

#[test]
fn a_member_that_walks_to_another_zone_keeps_and_loses_permissions_there() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 1_000);
    let bob = w.spawn("Bob");
    let org = found(&mut w, ada, "Acme");
    enlist(&mut w, org, ada, bob);
    ok(&mut w, ada, "org_set_role", json!({ "org": org, "resident": bob, "role": "owner" }));
    w.step(1);
    assert!(can(&w, bob, org, "treasury.spend"));

    // The membership moves with the resident.
    w.set_pos(bob, FAR);
    assert_eq!(w.zone_of(bob), ZoneId(3));
    assert!(can(&w, bob, org, "treasury.spend"));

    // A change that is made while the member is away follows it.
    ok(&mut w, ada, "org_set_role", json!({ "org": org, "resident": bob, "role": "member" }));
    w.step(1);
    assert!(!can(&w, bob, org, "treasury.spend"));
    ok(&mut w, bob, "org_leave", json!({ "org": org }));
    w.step(1);
    assert!(w.with_resident(bob, |realm, idx| realm.memberships(idx).is_empty()));
    w.assert_conserved();
}

#[test]
fn a_power_of_the_state_works_only_while_the_organization_holds_a_grant() {
    let mut w = world();
    let mayor = w.spawn("Mayor");
    let chief = rich(&mut w, "Chief", 1_000);
    let officer = w.spawn_at("Officer", FAR);
    elect_mayor(&mut w, mayor);
    assert!(can(&w, mayor, CITY, "gov.appoint"));
    // A city holds `law.*` by itself, so the role of the mayor has it, with
    // the city as its source.
    assert!(can(&w, mayor, CITY, "law.judge"));
    assert!(can(&w, mayor, CITY, "law.judge@100"));
    assert_eq!(permissions::scoped("law.judge", CITY), "law.judge@100");

    let police = found(&mut w, chief, "City Police");
    enlist(&mut w, police, chief, officer);
    ok(
        &mut w,
        chief,
        "org_define_role",
        json!({ "org": police, "role": "constable", "permissions": ["law.arrest", "org.invite"] }),
    );
    ok(
        &mut w,
        chief,
        "org_set_role",
        json!({ "org": police, "resident": officer, "role": "constable" }),
    );
    w.step(1);
    // The role lists `law.arrest`, but the organization has no grant: the
    // permission has no effect. The ordinary permission works.
    assert!(!can(&w, officer, police, "law.arrest"));
    assert!(can(&w, officer, police, "org.invite"));

    // The city grants the power. The officer in the other zone has it in
    // the next frame, with the city as its source.
    w.act_ok(mayor, "org_grant", json!({ "org": CITY, "to": police, "permission": "law.arrest" }));
    let frames = w.step(3);
    assert_eq!(
        w.last_report(mayor).map(|r| r.result),
        Some(json!({ "org": CITY, "to": police, "permission": "law.arrest", "granted": true }))
    );
    assert!(can(&w, officer, police, "law.arrest"));
    assert!(can(&w, officer, police, "law.arrest@100"));
    assert!(!can(&w, officer, police, "law.arrest@1"), "the nation did not grant it");
    assert!(!can(&w, chief, police, "law.arrest"), "the role of the chief does not list it");
    let granted = events(&frames, "society.org_granted");
    assert_eq!(granted.len(), 1);
    assert_eq!(granted[0].data["permission"], json!("law.arrest"));
    let told = notice(&w, officer, "society.org_grant_changed");
    assert_eq!((told.data["granted"].clone(), told.data["by"].clone()), (json!(true), json!(CITY)));
    assert!(!notice_kinds(&w, chief).contains(&"society.org_grant_changed".to_string()));

    // The grant ends: the members lose the power.
    ok(
        &mut w,
        mayor,
        "org_grant",
        json!({ "org": CITY, "to": police, "permission": "law.arrest", "revoke": true }),
    );
    w.step(1);
    assert!(!can(&w, officer, police, "law.arrest"));
    assert!(!can(&w, officer, police, "law.arrest@100"));
    assert!(can(&w, officer, police, "org.invite"));
    assert_eq!(notice(&w, officer, "society.org_grant_changed").data["granted"], json!(false));
    w.assert_conserved();
}

#[test]
fn org_grant_refuses_what_a_government_cannot_give() {
    let mut w = world();
    let mayor = w.spawn("Mayor");
    let chief = rich(&mut w, "Chief", 2_000);
    let clerk = w.spawn("Clerk");
    elect_mayor(&mut w, mayor);
    let police = found(&mut w, chief, "City Police");
    let gang = found(&mut w, chief, "The Gang");
    let grant =
        |org: OrgId, to: OrgId, perm: &str| json!({ "org": org, "to": to, "permission": perm });

    // Only a government organization grants, and only through a member
    // with `gov.appoint`.
    assert_eq!(
        fail(&mut w, chief, "org_grant", grant(gang, police, "law.arrest")),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, chief, "org_grant", grant(CITY, police, "law.arrest")),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(OrgId(7_000_000), police, "law.arrest")),
        ErrorCode::NotFound
    );
    // Only powers of the state, and only those that the giver holds: a city
    // has no power over the bank.
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(CITY, police, "treasury.spend")),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(CITY, police, "law.fly")),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(CITY, police, "bank.central")),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(CITY, CITY, "law.arrest")),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(CITY, OrgId(7_000_000), "law.arrest")),
        ErrorCode::NotFound
    );

    ok(&mut w, mayor, "org_grant", grant(CITY, police, "law.arrest"));
    assert_eq!(
        fail(&mut w, mayor, "org_grant", grant(CITY, police, "law.arrest")),
        ErrorCode::Conflict
    );
    let revoke = json!({ "org": CITY, "to": police, "permission": "law.search", "revoke": true });
    assert_eq!(fail(&mut w, mayor, "org_grant", revoke), ErrorCode::Conflict);

    // The nation holds every power, also over the bank.
    offices::define_role(w.global(), OrgId::NATION, "president", &["gov.appoint"]).expect("valid");
    offices::set_holders(w.global(), OrgId::NATION, "president", &[clerk]).expect("alive");
    w.world.settle();
    w.step(1);
    ok(&mut w, clerk, "org_grant", grant(OrgId::NATION, police, "bank.central"));
    ok(&mut w, clerk, "org_grant", grant(OrgId::NATION, police, "law.arrest"));
    let orgs = w.global().res::<Orgs>();
    let record = orgs.get(police).expect("the police");
    assert_eq!(record.sources("law.arrest"), [OrgId::NATION, CITY]);
    assert_eq!(record.sources("bank.central"), [OrgId::NATION]);
    assert!(record.holds("law.arrest") && !record.holds("law.jail"));
    w.assert_conserved();
}

#[test]
fn other_modules_fill_the_offices_of_a_government() {
    let mut w = world();
    let old = w.spawn("Old");
    let new = w.spawn_at("New", FAR);
    let second = w.spawn("Second");
    // The global realm learns about a new resident in its next tick.
    w.step(1);

    // Errors change nothing.
    assert_eq!(
        offices::define_role(w.zone(ZoneId(0)), CITY, "mayor", &[]),
        Err(OfficeError::NotGlobal)
    );
    assert_eq!(
        offices::define_role(w.global(), OrgId(5_000_000), "mayor", &[]),
        Err(OfficeError::NotGovernment(OrgId(5_000_000)))
    );
    assert_eq!(
        offices::define_role(w.global(), OrgId::city(CityId(9)), "mayor", &[]),
        Err(OfficeError::NotGovernment(OrgId::city(CityId(9))))
    );
    assert_eq!(
        offices::define_role(w.global(), CITY, "The Mayor", &[]),
        Err(OfficeError::BadRoleName("The Mayor".into()))
    );
    assert_eq!(
        offices::define_role(w.global(), CITY, "mayor", &["gov.fly"]),
        Err(OfficeError::UnknownPermission("gov.fly".into()))
    );
    assert_eq!(
        offices::set_holders(w.global(), CITY, "mayor", &[old]),
        Err(OfficeError::UnknownRole("mayor".into()))
    );
    assert_eq!(
        offices::define_role(w.global(), CITY, "mayor", &["gov.sign", "treasury.spend"]),
        Ok(())
    );
    assert_eq!(
        offices::set_holders(w.global(), CITY, "mayor", &[ResidentId(999)]),
        Err(OfficeError::UnknownResident(ResidentId(999)))
    );
    assert_eq!(
        offices::set_holders(w.zone(ZoneId(0)), CITY, "mayor", &[old]),
        Err(OfficeError::NotGlobal)
    );
    assert_eq!(offices::holders(w.global(), CITY, "mayor"), []);

    // The first mayor.
    assert_eq!(offices::set_holders(w.global(), CITY, "mayor", &[old]), Ok(()));
    w.world.settle();
    w.step(1);
    assert!(can(&w, old, CITY, "gov.sign") && can(&w, old, CITY, "gov.sign@100"));
    assert_eq!(notice(&w, old, "society.org_role_changed").data["role"], json!("mayor"));
    assert_eq!(notice(&w, old, "society.org_role_changed").data["government"], json!("city"));

    // An election: the new mayor replaces the old one, in whatever zone.
    assert_eq!(offices::set_holders(w.global(), CITY, "mayor", &[new]), Ok(()));
    w.world.settle();
    w.step(1);
    assert!(!can(&w, old, CITY, "gov.sign"));
    assert!(can(&w, new, CITY, "gov.sign"));
    assert_eq!(notice_kinds(&w, old).last().map(String::as_str), Some("society.org_removed"));
    assert_eq!(offices::holders(w.global(), CITY, "mayor"), [new]);

    // A role with several seats, and a change of the powers of the role.
    assert_eq!(offices::define_role(w.global(), CITY, "council", &["gov.vote"]), Ok(()));
    assert_eq!(offices::set_holders(w.global(), CITY, "council", &[old, second]), Ok(()));
    assert_eq!(
        offices::define_role(w.global(), CITY, "council", &["gov.vote", "gov.propose"]),
        Ok(())
    );
    w.world.settle();
    w.step(1);
    assert!(can(&w, old, CITY, "gov.propose") && can(&w, second, CITY, "gov.vote"));
    assert!(!can(&w, new, CITY, "gov.vote"));
    // A vacant office.
    assert_eq!(offices::set_holders(w.global(), CITY, "mayor", &[]), Ok(()));
    w.world.settle();
    w.step(1);
    assert!(!can(&w, new, CITY, "gov.sign"));
    assert_eq!(w.global().res::<Orgs>().get(CITY).map(|o| o.member_count()), Some(2));
    w.assert_conserved();
}
