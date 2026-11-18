//! Story 6: a family.
//!
//! Ann owns a home. She proposes to Ben, he accepts, and he joins her
//! household: now the home is his home too. Then Ann dies. She left no
//! will, so her spouse inherits: the cash and the goods on her, the money
//! in her bank account and the home.
//!
//! Four modules settle one death. The life module gives the heir what the
//! resident carried. The finance module gives it the bank balance. The
//! economy module gives it the home. The society module ends the marriage
//! and the household.
//!
//! The story starts in the republic (`unsup_scenarios::republic`). The
//! president pays the price of the home from the national treasury.

use serde_json::{Value, json};
use unsup_kernel::{ErrorCode, Holder, Kin, ResidentId};
use unsup_scenarios::republic::grant;
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic_in_republic};

fn kin(g: &Game, r: ResidentId) -> Kin {
    g.world.with_resident(r, |realm, idx| realm.col::<Kin>()[idx].clone())
}

fn relations(g: &Game, r: ResidentId) -> Value {
    g.section(r, "relations").unwrap_or_default()
}

fn story(g: &mut Game, republic: &Republic) {
    let [ann, ben, cal, _] = republic.citizens;
    let (ann_cash, ben_cash) = (g.cash(ann), g.cash(ben));
    let (ann_food, ben_food) = (g.carried(ann, "vegetables"), g.carried(ben, "vegetables"));
    assert_eq!((ann_cash, ben_cash, ann_food, ben_food), (12_600, 12_600, 23, 23));

    // Ann buys the cheapest home of the city and puts 10,000 in its till
    // for the property tax.
    let home = g
        .sites("home")
        .into_iter()
        .min_by_key(|home| (home.attr("value"), home.id))
        .expect("a home");
    let price = home.attr("value");
    grant(g, republic, ann, price + 10_000);
    g.walk(ann, &home);
    g.act(ann, "buy_property", json!({}));
    g.act(ann, "till_fund", json!({ "amount": 10_000 }));
    // She keeps 5,000 in the bank.
    let bank = g.site("bank");
    g.walk(ann, &bank);
    g.act(ann, "open_account", json!({}));
    g.act(ann, "deposit", json!({ "amount": 5_000 }));
    let ann_cash = ann_cash - 100 - 5_000;
    assert_eq!(g.cash(ann), ann_cash);

    // The proposal. Ben sees it in his observation and gets a notice.
    let proposal = g.act(ann, "propose", json!({ "resident": ben }));
    let expires = proposal["expires"].as_u64().expect("the end of the proposal");
    assert_eq!(proposal, json!({ "to": ben, "expires": expires }));
    g.step(1);
    assert_eq!(
        relations(g, ben),
        json!({ "proposals": [{ "from": ann, "ask": "marriage", "expires": expires }] })
    );
    assert_eq!(g.notice(ben, "society.proposal_received").from, Some(ann));
    // Nobody can accept a proposal that was not made.
    assert_eq!(g.refused(cal, "accept_proposal", json!({ "resident": ann })), ErrorCode::NotFound);

    // The marriage.
    assert_eq!(g.act(ben, "accept_proposal", json!({ "resident": ann })), json!({ "spouse": ann }));
    assert_eq!(g.last_event("society.married").data, json!({ "a": ann, "b": ben }));
    g.step(1);
    assert_eq!((kin(g, ann).spouse, kin(g, ben).spouse), (Some(ben), Some(ann)));
    assert_eq!(relations(g, ann), json!({ "spouse": ben }));
    // A married resident cannot marry again.
    assert_eq!(g.refused(cal, "propose", json!({ "resident": ann })), ErrorCode::Conflict);

    // The household. The host invites from inside her home, and only from
    // there. Before he joins, Ben sleeps there like a stranger.
    assert_eq!(g.refused(ann, "household_invite", json!({ "resident": ben })), ErrorCode::NotHere);
    g.walk(ann, &home);
    g.walk(ben, &home);
    assert_eq!(g.act(ben, "inspect", json!({ "site": home.id }))["sleep"], "poor");
    let invitation = g.act(ann, "household_invite", json!({ "resident": ben }));
    let household = invitation["household"].as_u64().expect("a household id");
    assert_eq!(
        (invitation["invited"].as_u64(), invitation["site"].as_u64()),
        (Some(ben.0), Some(home.id.0))
    );
    let joined = g.act(ben, "household_join", json!({ "host": ann }));
    assert_eq!(
        joined,
        json!({ "household": household, "host": ann, "site": home.id, "zone": g.zone_of_site(home.id) })
    );
    g.step(1);
    assert_eq!((kin(g, ann).household, kin(g, ben).household), (Some(household), Some(household)));
    assert_eq!(relations(g, ben), json!({ "spouse": ann, "household": household }));
    // The home is his home now: he is a tenant and sleeps well there.
    assert_eq!(g.site_now(home.id).tenants, [ben]);
    let slept = g.act(ben, "sleep", json!({ "ticks": 30 }));
    assert_eq!(slept["quality"], "good");
    assert_eq!(g.section(ben, "home").expect("a home section")["site"], json!(home.id));

    // The death. Cal attacks Ann in the street until she is dead. An
    // attack can fail; a failed one hurts the attacker.
    let park = g.site("park");
    g.walk(ann, &park);
    g.walk_to(cal, g.pos(ann).offset(1, 0));
    let (mut hits, mut misses) = (0, 0);
    let last = loop {
        let attack = g.act(cal, "assault", json!({ "target": ann }));
        if attack["success"] == true {
            hits += 1;
        } else {
            misses += 1;
        }
        if attack["fatal"] == true {
            break attack;
        }
        assert!(hits + misses < 30, "thirty attacks did not kill");
    };
    // 1,000 health: six hits of 150 and a last one that takes the rest.
    assert_eq!((hits, misses, last["damage"].as_i64()), (7, 2, Some(100)));
    assert_eq!(g.attr(cal, "need.health"), 1_000 - 2 * 60);
    assert!(!g.is_alive(ann));
    let died = g.last_event("life.resident_died");
    assert_eq!(
        died.data,
        json!({ "resident": ann, "cause": "violence", "age": 18, "heir": { "Resident": ben }, "estate": ann_cash })
    );
    let homicide = g.last_event("crime.assault_committed");
    assert_eq!(
        (homicide.frame, homicide.data["kind"].as_str(), homicide.data["fatal"].as_bool()),
        (died.frame, Some("homicide"), Some(true))
    );

    // The estate. Ben was in the same district, so he has it at once: the
    // cash and the vegetables that Ann carried.
    g.step(2);
    assert_eq!(g.carried(ben, "vegetables"), ben_food + ann_food);
    let inheritance = g.notice(ben, "life.inheritance");
    assert_eq!(
        inheritance.data,
        json!({ "from": ann, "money": ann_cash, "items": { "vegetables": ann_food } })
    );
    // The bank balance. Ben has no account, so it comes as cash.
    let settled = g.last_event("finance.estate_settled");
    assert_eq!(
        settled.data,
        json!({ "resident": ann, "heir": { "Resident": ben }, "balance": 5_000, "loan_unpaid": 0, "shares": 0 })
    );
    assert_eq!(g.cash(ben), ben_cash + ann_cash + 5_000);
    assert_eq!(g.money(Holder::Account(ann.0)), 0);
    // The home, with the money in its till.
    let inherited = g.last_event("economy.property_inherited");
    assert_eq!(
        inherited.data,
        json!({ "site": home.id, "city": 0, "from": ann, "to": { "Resident": ben } })
    );
    let now = g.site_now(home.id);
    assert_eq!((now.owner, now.tenants.len()), (Some(Holder::Resident(ben)), 0));
    assert_eq!(
        g.section(ben, "property").expect("a property section")["owned"],
        json!([{ "site": home.id, "kind": "home", "zone": g.zone_of_site(home.id), "till": 10_000 }])
    );
    assert_eq!(g.act(ben, "inspect", json!({ "site": home.id }))["sleep"], "good");

    // The marriage and the household ended.
    assert_eq!(g.notice(ben, "society.widowed").data, json!({ "spouse": ann }));
    assert_eq!(
        g.notice(ben, "society.household_dissolved").data,
        json!({ "household": household, "cause": "death" })
    );
    assert_eq!(
        g.last_event("society.household_dissolved").data,
        json!({ "household": household, "host": ann, "cause": "death" })
    );
    assert_eq!(kin(g, ben), Kin::default());
    assert_eq!(g.section(ben, "relations"), None);
    // A widower can marry again.
    assert_eq!(g.act(ben, "propose", json!({ "resident": cal }))["to"], json!(cal));

    // Nothing of the estate was lost and nothing was made.
    g.audit();
    assert_eq!(g.money(Holder::Resident(ann)), 0);
}

#[test]
fn a_spouse_inherits_the_cash_the_account_and_the_home() {
    assert_deterministic_in_republic(&Setup::town(1), story);
}
