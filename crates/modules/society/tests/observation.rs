//! What agents read: the catalogue of verbs and the observation sections
//! `orgs`, `contracts` and `relations`.

mod common;

use common::{enlist, found, ok, rich, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::{Detail, ObserveOpts, VerbScope};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, OrgId, ResidentId};
use unsup_society::{offices, permissions};

fn section(w: &TestWorld<'_>, r: ResidentId, name: &str, detail: Detail) -> Option<Value> {
    let opts = ObserveOpts { detail, ..ObserveOpts::default() };
    w.observe_with(r, &opts).sections.get(name).cloned()
}

#[test]
fn the_catalogue_has_every_verb_of_the_module_with_a_schema() {
    let w = world();
    let catalogue = w.world.catalogue();
    let mut verbs: Vec<(&str, bool, VerbScope)> = catalogue
        .verbs
        .iter()
        .filter(|v| v.module == "society")
        .map(|v| (v.verb.as_str(), v.free, v.scope))
        .collect();
    verbs.sort_by_key(|(verb, _, _)| *verb);
    let (zone, global) = (VerbScope::Zone, VerbScope::Global);
    let expected = [
        ("accept_proposal", false, global),
        ("contract_cancel", false, global),
        ("contract_claim", false, zone),
        ("contract_offer", false, zone),
        ("contract_reject", false, global),
        ("contract_sign", false, zone),
        ("contract_view", true, global),
        ("divorce", false, global),
        ("household_invite", false, zone),
        ("household_join", false, global),
        ("household_leave", false, global),
        ("message", true, zone),
        ("opine", true, zone),
        ("org_create", false, zone),
        ("org_define_role", false, global),
        ("org_deposit", false, zone),
        ("org_grant", false, global),
        ("org_invite", false, global),
        ("org_join", false, global),
        ("org_kick", false, global),
        ("org_leave", false, global),
        ("org_pay", false, zone),
        ("org_say", true, zone),
        ("org_set_role", false, global),
        ("post", false, zone),
        ("propose", false, global),
        ("relate", true, zone),
        ("say", true, zone),
    ];
    assert_eq!(verbs, expected);
    for verb in catalogue.verbs.iter().filter(|v| v.module == "society") {
        assert!(verb.summary.ends_with('.'), "{}: {}", verb.verb, verb.summary);
        assert!(verb.args_schema.is_object(), "{}", verb.verb);
    }
    // The schema of an offer explains every kind of clause to an agent.
    let offer = catalogue.verbs.iter().find(|v| v.verb == "contract_offer").expect("the verb");
    let schema = offer.args_schema.to_string();
    for kind in ["pay_at_signing", "pay_at_deadline", "pay_on_event", "deliver", "penalty"] {
        assert!(schema.contains(kind), "{kind}");
    }
    assert!(schema.contains("Game minutes from the signing to the deadline."));
    // Every permission of the game design is a valid permission.
    assert_eq!(permissions::PERMISSIONS.len(), 14);
    assert!(permissions::is_known("law.jail") && !permissions::is_known("law.fly"));
    assert!(permissions::is_sovereign("bank.central") && !permissions::is_sovereign("job.hire"));
}

#[test]
fn every_entry_of_the_data_file_is_a_parameter_with_bounds() {
    let w = world();
    let catalogue = w.world.catalogue();
    let ours: Vec<_> = catalogue.params.iter().filter(|p| p.name.starts_with("society.")).collect();
    // The catalogue has one parameter for each table of the data file.
    let data = include_str!("../data/balance.toml");
    let tables = data.lines().filter(|line| line.starts_with("[params.\"society.")).count();
    assert!(tables > 0, "the data file has parameters");
    assert_eq!(ours.len(), tables);
    for param in &ours {
        assert!(!param.summary.is_empty(), "{}", param.name);
        let (min, max) = (param.min.expect("a minimum"), param.max.expect("a maximum"));
        assert!(min <= param.value && param.value <= max, "{}", param.name);
    }
    // A law can change the fee for a new organization.
    let policy: Vec<_> = ours.iter().filter(|p| p.policy).map(|p| p.name.as_str()).collect();
    assert_eq!(policy, ["society.org_create_fee"]);
}

#[test]
fn a_resident_with_nothing_has_no_sections() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.step(1);
    for name in ["orgs", "contracts", "relations"] {
        assert_eq!(section(&w, ada, name, Detail::Full), None, "{name}");
    }
    w.assert_conserved();
}

#[test]
fn the_orgs_section_lists_memberships_and_invitations() {
    let mut w = world();
    let ada = rich(&mut w, "Ada", 2_500);
    let bob = w.spawn("Bob");
    let acme = found(&mut w, ada, "Acme");
    let union = found(&mut w, ada, "Union");
    enlist(&mut w, acme, ada, bob);
    ok(&mut w, ada, "org_deposit", json!({ "org": acme, "amount": 500 }));
    ok(&mut w, ada, "org_invite", json!({ "org": union, "resident": bob }));
    let city = OrgId::city(CityId(0));
    offices::define_role(w.global(), city, "mayor", &["gov.sign"]).expect("valid");
    offices::set_holders(w.global(), city, "mayor", &[bob]).expect("alive");
    w.world.settle();
    w.step(1);

    let name = |text: &str| json!({ "untrusted": true, "from": ada, "text": text });
    let brief = section(&w, bob, "orgs", Detail::Brief).expect("a section");
    // The invitation was made two frames ago, in the frame before the answer.
    let expires = w.world.frame() - 2 + 10_080;
    assert_eq!(w.param("society.org_invite_ttl"), 10_080);
    assert_eq!(
        brief,
        json!({
            "member_of": [
                { "org": city, "government": "city", "city": 0, "role": "mayor", "members": 1, "treasury": 2_000 },
                { "org": acme, "name": name("Acme"), "role": "member", "members": 2, "treasury": 500 },
            ],
            "invites": [{ "org": union, "name": name("Union"), "expires": expires }],
        })
    );
    // The section is small: near 150 tokens for a resident in two
    // organizations.
    assert!(brief.to_string().len() < 420, "{} bytes", brief.to_string().len());

    let full = section(&w, bob, "orgs", Detail::Full).expect("a section");
    assert_eq!(full["member_of"][0]["permissions"], json!(["gov.sign", "gov.sign@100"]));
    assert_eq!(
        full["member_of"][1]["roles"]["owner"],
        json!(["job.hire", "org.invite", "org.manage", "site.manage", "treasury.spend"])
    );
    assert_eq!(full["member_of"][1]["open"], json!(false));
    // Ada is the owner of both.
    let hers = section(&w, ada, "orgs", Detail::Brief).expect("a section");
    assert_eq!(hers["member_of"].as_array().map(Vec::len), Some(2));
    assert_eq!(hers.get("invites"), None);
    assert_eq!((hers.get("more_orgs"), hers.get("more_invites")), (None, None));
    w.assert_conserved();
}

#[test]
fn the_lists_of_the_orgs_section_are_bounded() {
    let mut w = world();
    w.set_param("society.org_create_fee", 0);
    w.set_param("society.brief_list_max", 2);
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.step(1);
    let ids: Vec<OrgId> =
        ["One", "Two", "Three"].iter().map(|name| found(&mut w, ada, name)).collect();
    for &org in &ids {
        ok(&mut w, ada, "org_invite", json!({ "org": org, "resident": bob }));
    }
    let listed = |section: &Value, list: &str| -> Vec<Value> {
        section[list].as_array().expect("a list").iter().map(|e| e["org"].clone()).collect()
    };

    // Brief detail: two entries of each list, and a count of the rest.
    let hers = section(&w, ada, "orgs", Detail::Brief).expect("a section");
    assert_eq!(listed(&hers, "member_of"), [json!(ids[0]), json!(ids[1])]);
    assert_eq!(hers["more_orgs"], json!(1));
    let his = section(&w, bob, "orgs", Detail::Brief).expect("a section");
    assert_eq!(listed(&his, "invites"), [json!(ids[0]), json!(ids[1])]);
    assert_eq!(his["more_invites"], json!(1));
    assert_eq!(his.get("member_of"), None);

    // Full detail: every organization of the resident. The invitations
    // stay bounded, because others decide how many there are.
    let hers = section(&w, ada, "orgs", Detail::Full).expect("a section");
    assert_eq!(listed(&hers, "member_of"), [json!(ids[0]), json!(ids[1]), json!(ids[2])]);
    assert_eq!(hers.get("more_orgs"), None);
    let his = section(&w, bob, "orgs", Detail::Full).expect("a section");
    assert_eq!(listed(&his, "invites").len(), 2);
    assert_eq!(his["more_invites"], json!(1));
    w.assert_conserved();
}

#[test]
fn the_contracts_section_is_short_and_bounded() {
    let mut w = world();
    w.set_param("society.contracts_open_max", 2);
    let ada = rich(&mut w, "Ada", 100);
    let bob = rich(&mut w, "Bob", 100);
    let cyd = rich(&mut w, "Cyd", 100);
    w.step(1);
    let clauses = json!([
        { "kind": "pay_at_signing", "by": "offerer", "amount": 10 },
        { "kind": "penalty", "by": "counterparty", "amount": 7 },
    ]);
    let mut ids = Vec::new();
    for from in [ada, bob, bob, cyd] {
        let result = ok(
            &mut w,
            from,
            "contract_offer",
            json!({ "to": if from == ada { bob } else { ada }, "clauses": clauses, "expires_in": 500 }),
        );
        ids.push(result["contract"].as_u64().expect("an id"));
    }
    let expires = |i: usize| w.world.frame() - 1 - 2 * (3 - i as u64) + 500;

    // Ada offered one contract and three offers name her. The list shows
    // two, the limit of this world, and counts the rest.
    let seen = section(&w, ada, "contracts", Detail::Brief).expect("a section");
    assert_eq!(
        seen,
        json!({
            "more": 2,
            "list": [
                { "contract": ids[0], "you_are": "offerer", "with": bob, "status": "offered", "expires": expires(0), "your_part": 10, "clauses": 2 },
                { "contract": ids[1], "you_are": "counterparty", "with": bob, "status": "offered", "expires": expires(1), "your_part": 7, "clauses": 2 },
            ],
        })
    );
    assert!(seen.to_string().len() < 320, "{} bytes", seen.to_string().len());
    let full = section(&w, ada, "contracts", Detail::Full).expect("a section");
    assert_eq!(
        full["list"][0]["clauses"][1],
        json!({ "kind": "penalty", "by": "counterparty", "amount": 7, "state": "open" })
    );

    // The brief list has its own, smaller limit. The full list keeps the
    // limit of open contracts.
    w.set_param("society.brief_list_max", 1);
    let seen = section(&w, ada, "contracts", Detail::Brief).expect("a section");
    assert_eq!(seen["more"], json!(3));
    assert_eq!(seen["list"].as_array().map(Vec::len), Some(1));
    assert_eq!(seen["list"][0]["contract"], json!(ids[0]));
    let full = section(&w, ada, "contracts", Detail::Full).expect("a section");
    assert_eq!((full["more"].clone(), full["list"].as_array().map(Vec::len)), (json!(2), Some(2)));
    w.assert_conserved();
}
