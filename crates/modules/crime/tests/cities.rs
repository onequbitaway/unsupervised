//! Powers of the law count only in the city that gave them, and the law
//! sites of a city are found also when they come later.

mod common;

use common::*;
use serde_json::json;
use unsup_crime::CrimePlugin;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    CityConfig, CityId, ErrorCode, Holder, OrgId, Pos, Rect, ResidentId, Site, SiteId, WorldConfig,
};

static CRIME: CrimePlugin = CrimePlugin;

/// A place in the first city and a place in the second city.
const IN_A: Pos = Pos::new(10, 10);
const IN_B: Pos = Pos::new(210, 10);

/// A world with two cities of one zone each, and only this module. The
/// city watch sees every crime.
fn two_cities() -> TestWorld<'static> {
    let rules = with(&[&[ALWAYS, WATCH_SEES_ALL, ("crime.watch_arrest_bp", 0)], &BLIND]);
    let cfg = WorldConfig {
        cities: vec![
            CityConfig::new("Aston", Pos::new(0, 0), 1),
            CityConfig::new("Bray", Pos::new(200, 0), 1),
        ],
        ..config(5, &rules)
    };
    let mut w = TestWorld::with_config(&[&CRIME], cfg);
    w.step(2);
    w
}

/// Writes a membership as the society module would: the permission by its
/// name, and one time for the government that it comes from.
fn grant(w: &mut TestWorld<'_>, resident: ResidentId, source: OrgId, perms: &[&str]) {
    let mut names: Vec<String> = Vec::new();
    for perm in perms {
        names.push((*perm).to_string());
        names.push(format!("{perm}@{}", source.0));
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    w.global().grant(resident, source, "office", &names);
    w.world.settle();
    w.step(1);
}

#[test]
fn a_power_of_one_city_has_no_effect_in_another_city() {
    let mut w = two_cities();
    let (city_a, city_b) = (OrgId::city(CityId(0)), OrgId::city(CityId(1)));
    let bo = w.spawn_at("Bo", IN_A);
    w.give_money(bo, unsup_kernel::Money(4_000));
    let cy = w.spawn_at("Cy", IN_A.offset(1, 0));
    let eve = w.spawn_at("Eve", IN_A.offset(0, 1));
    grant(&mut w, eve, city_a, &["law.arrest", "law.search"]);
    // A judge of the other city, and a judge of the nation.
    let ben = w.spawn_at("Ben", IN_B.offset(5, 5));
    grant(&mut w, ben, city_b, &["law.judge"]);
    let nat = w.spawn_at("Nat", IN_B.offset(6, 5));
    grant(&mut w, nat, OrgId::NATION, &["law.judge"]);

    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));

    // The thief and the officer go to the other city. There the officer
    // of the first city is a resident like every other.
    w.set_pos(cy, IN_B);
    w.set_pos(eve, IN_B.offset(1, 0));
    // Eve stood next to the theft, so its `law` section has the case. It
    // has no powers in this city.
    let section = law(&w, eve).expect("the officer saw the theft");
    assert_eq!(section["known_cases"][0]["case"], json!(case));
    assert_eq!(section.get("powers_here"), None);
    for verb in ["arrest", "search"] {
        assert_eq!(fail(&mut w, eve, verb, json!({ "target": cy })), ErrorCode::NotAllowed);
    }
    assert_eq!(fail(&mut w, eve, "patrol", json!({})), ErrorCode::NotAllowed);
    assert!(!w.has_condition(cy, "detained"));
    // The docket of the second city does not have the case of the first.
    assert_eq!(ok(&mut w, eve, "case_list", json!({}))["cases"], json!([]));

    // Back in the first city the arrest works.
    w.set_pos(cy, IN_A.offset(1, 0));
    w.set_pos(eve, IN_A.offset(0, 1));
    assert_eq!(
        law(&w, eve).expect("an officer")["powers_here"],
        json!(["law.arrest", "law.search"])
    );
    ok(&mut w, eve, "arrest", json!({ "target": cy }));

    // A charge from the other city does not find the custody.
    w.set_pos(eve, IN_B.offset(1, 0));
    grant(&mut w, eve, city_b, &["law.arrest"]);
    assert_eq!(
        fail(&mut w, eve, "charge", json!({ "target": cy, "case": case })),
        ErrorCode::NotFound
    );
    w.set_pos(eve, IN_A.offset(0, 1));
    grant(&mut w, eve, city_a, &["law.arrest", "law.search"]);
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));

    // The judge of the other city cannot rule. The judge of the nation
    // can, from where it stands.
    let acquit = json!({ "case": case, "guilty": false });
    assert_eq!(fail(&mut w, ben, "rule_case", acquit.clone()), ErrorCode::NotAllowed);
    assert_eq!(case_view(&mut w, bo, case)["status"], json!("trial"));
    assert_eq!(ok(&mut w, nat, "rule_case", acquit)["guilty"], json!(false));
    w.step(1);
    assert!(!w.has_condition(cy, "detained"));
    w.assert_conserved();
}

#[test]
fn a_jail_that_is_built_later_is_found_within_one_game_hour() {
    let mut w = two_cities();
    let eve = w.spawn_at("Eve", IN_A.offset(0, 1));
    grant(&mut w, eve, OrgId::city(CityId(0)), &["law.arrest", "law.jail"]);
    let cy = w.spawn_at("Cy", IN_A);
    let dee = w.spawn_at("Dee", IN_A.offset(1, 1));

    // No jail: the first suspect is held where it stands.
    w.set_condition(cy, "wanted", None);
    assert_eq!(ok(&mut w, eve, "arrest", json!({ "target": cy }))["jail"], json!(null));
    assert_eq!(w.pos(cy), IN_A);

    // Somebody builds a jail in the first city and one in the second.
    let rect = Rect::sized(40, 40, 6, 6);
    let mut ids = Vec::new();
    for (zone, corner) in [(0u32, rect), (1u32, Rect::sized(240, 40, 6, 6))] {
        let realm = w.zone(unsup_kernel::ZoneId(zone));
        let id = SiteId(realm.alloc_id());
        realm.sites_mut().insert(
            Site::new(id, "jail", "Jail", corner)
                .with_owner(Holder::Org(OrgId::city(CityId(zone as u16))))
                .with_tag("public"),
        );
        ids.push(id);
    }
    w.world.settle();
    w.step(61);

    // The next suspect goes to the jail of this city, not of the other.
    w.set_condition(dee, "wanted", None);
    assert_eq!(ok(&mut w, eve, "arrest", json!({ "target": dee }))["jail"], json!(ids[0]));
    assert_eq!(w.pos(dee), rect.centre());

    // `law.jail` does not free a suspect, and `law.arrest` is needed for
    // that. An officer with `law.jail` only is refused.
    let gus = w.spawn_at("Gus", rect.centre().offset(1, 0));
    grant(&mut w, gus, OrgId::city(CityId(0)), &["law.jail"]);
    assert_eq!(fail(&mut w, gus, "release", json!({ "target": dee })), ErrorCode::NotAllowed);
    assert!(w.has_condition(dee, "detained"));
    w.assert_conserved();
}
