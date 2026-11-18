//! Appointments: the mayor fills the police and the court of its city, the
//! president fills the central bank.

mod common;

use common::{
    CITY, can, error_of, event, fail, holders, notices, ok, overrides, plugins,
    plugins_with_finance, quiet_config, rich, seat, section,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, OrgId, ResidentId, WorldConfig};
use unsup_politics::roles;

/// Makes a world whose first city has a mayor.
fn city_with_mayor(config: WorldConfig) -> (TestWorld<'static>, ResidentId) {
    let mut w = TestWorld::with_config(&plugins(), config);
    let mayor = rich(&mut w, "Mayor", 0);
    seat(&mut w, CITY, roles::MAYOR, &[mayor]);
    (w, mayor)
}

/// Returns the organization in which a resident has a permission, as its
/// zone sees it.
fn source_of(w: &TestWorld<'_>, r: ResidentId, perm: &str) -> Option<OrgId> {
    w.with_resident(r, |realm, idx| realm.has_perm(idx, perm))
}

#[test]
fn an_appointment_gives_the_powers_of_the_post_and_a_dismissal_takes_them() {
    let (mut w, mayor) = city_with_mayor(quiet_config());
    let pat = rich(&mut w, "Pat", 0);
    let jo = rich(&mut w, "Jo", 0);
    let chief = rich(&mut w, "Chief", 0);
    w.step(1);
    assert!(!can(&w, pat, CITY, "law.arrest"));

    // A police officer can arrest and search in the name of the city.
    w.act_ok(mayor, "appoint", json!({ "post": "police_officer", "resident": pat }));
    let frames = w.step(2);
    let facts = json!({
        "gov": "city", "city": 0, "post": "police_officer", "resident": pat, "by": mayor,
        "org": CITY,
    });
    assert_eq!(w.last_report(mayor).expect("the appointment reports").result, facts);
    let appointed = event(&frames, "politics.official_appointed");
    assert_eq!(appointed.actor, Some(mayor));
    assert_eq!(appointed.data, facts);
    let told = notices(&w, pat, "politics.appointed");
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].from, &told[0].data), (Some(mayor), &facts));
    assert!(can(&w, pat, CITY, "law.arrest"));
    assert!(can(&w, pat, CITY, "law.search"));
    assert!(!can(&w, pat, CITY, "law.judge"));
    assert!(!can(&w, pat, CITY, "law.jail"));
    assert!(!can(&w, pat, CITY, "gov.appoint"));
    // Zone code finds the power and the city that gives it.
    assert_eq!(source_of(&w, pat, "law.arrest"), Some(CITY));
    assert_eq!(source_of(&w, pat, "law.arrest@100"), Some(CITY));
    assert_eq!(source_of(&w, pat, "law.arrest@1"), None, "the nation did not give it");
    assert_eq!(holders(&w, CITY, roles::POLICE_OFFICER), [pat]);
    assert_eq!(section(&w, pat)["offices"], json!([{ "city": 0, "office": "police_officer" }]));

    // A judge can judge and jail. A police chief can arrest, search and
    // jail.
    ok(&mut w, mayor, "appoint", json!({ "post": "judge", "resident": jo, "city": 0 }));
    ok(&mut w, mayor, "appoint", json!({ "post": "police_chief", "resident": chief }));
    for (who, perms) in [
        (
            jo,
            [("law.judge", true), ("law.jail", true), ("law.arrest", false), ("law.search", false)],
        ),
        (
            chief,
            [("law.judge", false), ("law.jail", true), ("law.arrest", true), ("law.search", true)],
        ),
    ] {
        for (perm, has) in perms {
            assert_eq!(can(&w, who, CITY, perm), has, "resident {who}, {perm}");
        }
    }

    // A dismissal takes the powers away.
    w.act_ok(mayor, "dismiss", json!({ "post": "police_officer", "resident": pat }));
    let frames = w.step(2);
    let dismissed = event(&frames, "politics.official_dismissed");
    assert_eq!(dismissed.data["resident"], json!(pat));
    assert_eq!(dismissed.data["post"], "police_officer");
    assert_eq!(notices(&w, pat, "politics.dismissed").len(), 1);
    assert!(!can(&w, pat, CITY, "law.arrest"));
    assert!(!can(&w, pat, CITY, "law.search"));
    assert_eq!(source_of(&w, pat, "law.arrest"), None);
    assert!(holders(&w, CITY, roles::POLICE_OFFICER).is_empty());
    assert_eq!(section(&w, pat)["offices"], Value::Null);
    // The others keep their posts.
    assert!(can(&w, jo, CITY, "law.judge") && can(&w, chief, CITY, "law.arrest"));
    w.assert_conserved();
}

#[test]
fn each_refusal_of_the_appointment_verbs_has_its_code() {
    let data = overrides(&[("politics.police_officers_max", 1)]);
    let mut config = quiet_config();
    config.data.push(data);
    let (mut w, mayor) = city_with_mayor(config);
    let member = rich(&mut w, "Member", 0);
    let pat = rich(&mut w, "Pat", 0);
    let sam = rich(&mut w, "Sam", 0);
    seat(&mut w, CITY, roles::COUNCILLOR, &[member]);
    let post = |post: &str, who: ResidentId| json!({ "post": post, "resident": who });

    // Only the executive appoints.
    for who in [member, pat] {
        assert_eq!(fail(&mut w, who, "appoint", post("judge", sam)), ErrorCode::NotAllowed);
    }
    // A mayor fills the posts of its city, not the central bank.
    assert_eq!(fail(&mut w, mayor, "appoint", post("central_banker", pat)), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, mayor, "appoint", post("judge", mayor)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, mayor, "appoint", post("judge", ResidentId(999))), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, mayor, "appoint", json!({ "post": "judge", "resident": pat, "city": 9 })),
        ErrorCode::NotFound
    );
    let unknown = w.act_err(mayor, "appoint", post("king", pat));
    assert_eq!(unknown.code, ErrorCode::InvalidArgs);

    ok(&mut w, mayor, "appoint", post("police_officer", pat));
    assert_eq!(fail(&mut w, mayor, "appoint", post("police_officer", pat)), ErrorCode::Conflict);
    // A resident has one role in the government of a city: an appointment
    // must not take another office away.
    assert_eq!(fail(&mut w, mayor, "appoint", post("judge", pat)), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, mayor, "appoint", post("judge", member)), ErrorCode::Conflict);
    assert!(can(&w, member, CITY, "gov.vote"), "the councillor kept its seat");
    // The post has one seat in this world.
    let full = error_of(&mut w, mayor, "appoint", post("police_officer", sam));
    assert_eq!(full.code, ErrorCode::Conflict);
    assert_eq!(full.details, json!({ "seats": 1, "holders": [pat] }));
    ok(&mut w, mayor, "appoint", post("police_chief", sam));
    let other = rich(&mut w, "Other", 0);
    w.step(1);
    assert_eq!(fail(&mut w, mayor, "appoint", post("police_chief", other)), ErrorCode::Conflict);

    // dismiss
    assert_eq!(fail(&mut w, member, "dismiss", post("police_officer", pat)), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, mayor, "dismiss", post("judge", pat)), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, mayor, "dismiss", post("police_officer", other)), ErrorCode::NotFound);
    assert!(can(&w, pat, CITY, "law.arrest"), "a refused dismissal changes nothing");
    w.assert_conserved();
}

#[test]
fn the_president_appoints_the_central_banker() {
    let mut w = TestWorld::with_config(&plugins_with_finance(), quiet_config());
    let president = rich(&mut w, "President", 0);
    let banker = rich(&mut w, "Banker", 0);
    seat(&mut w, OrgId::NATION, roles::PRESIDENT, &[president]);
    assert!(!can(&w, banker, OrgId::CENTRAL_BANK, "bank.central"));
    let refused = w.act_err(banker, "central_bank_set_rate", json!({ "rate_bp": 700 }));
    assert_eq!(refused.code, ErrorCode::NotAllowed);

    let result =
        ok(&mut w, president, "appoint", json!({ "post": "central_banker", "resident": banker }));
    assert_eq!(
        result,
        json!({
            "gov": "nation", "post": "central_banker", "resident": banker, "by": president,
            "org": OrgId::CENTRAL_BANK,
        })
    );
    assert!(can(&w, banker, OrgId::CENTRAL_BANK, "bank.central"));
    assert_eq!(holders(&w, OrgId::CENTRAL_BANK, roles::CENTRAL_BANKER), [banker]);
    assert_eq!(section(&w, banker)["offices"], json!([{ "office": "central_banker" }]));
    // The finance module obeys the banker.
    assert_eq!(w.param("finance.base_rate_bp"), 500);
    let rate = ok(&mut w, banker, "central_bank_set_rate", json!({ "rate_bp": 700 }));
    assert_eq!(rate, json!({ "rate_bp": 700, "old_bp": 500 }));
    assert_eq!(w.param("finance.base_rate_bp"), 700);

    ok(&mut w, president, "dismiss", json!({ "post": "central_banker", "resident": banker }));
    assert!(!can(&w, banker, OrgId::CENTRAL_BANK, "bank.central"));
    let refused = w.act_err(banker, "central_bank_set_rate", json!({ "rate_bp": 900 }));
    assert_eq!(refused.code, ErrorCode::NotAllowed);
    assert_eq!(w.param("finance.base_rate_bp"), 700);
    w.assert_conserved();
}
