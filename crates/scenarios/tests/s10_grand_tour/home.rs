//! A home: a lease, two households and a marriage.

use serde_json::json;
use unsup_kernel::{Holder, Kin, ResidentId};
use unsup_scenarios::Game;
use unsup_scenarios::republic::grant;

use crate::Tour;

fn kin(g: &Game, r: ResidentId) -> Kin {
    g.world.with_resident(r, |realm, idx| realm.col::<Kin>()[idx].clone())
}

/// Ben buys a home and lets it to Dee. Dee takes Cal in and then ends her
/// lease. Ben and Dee marry, live together and part again.
pub fn run(g: &mut Game, tour: &Tour) {
    let [_, ben, cal, dee] = tour.cast.citizens;
    let home = g
        .sites_in("home", crate::CITY)
        .into_iter()
        .min_by_key(|home| (home.attr("value"), home.id))
        .expect("a home");
    grant(g, &tour.cast, ben, home.attr("value") + 10_000);
    g.walk(ben, &home);
    g.act(ben, "buy_property", json!({}));
    g.act(ben, "till_fund", json!({ "amount": 10_000 }));

    // The lease: 500 for a day. The first day is paid into the till.
    assert_eq!(g.act(ben, "rent_out", json!({ "rent": 500 }))["rent"], 500);
    g.walk_all(&[dee, cal], &home);
    assert_eq!(g.act(dee, "rent", json!({ "max_rent": 500 }))["rent"], 500);
    assert_eq!(g.site_now(home.id).tenants, [dee]);
    assert_eq!(g.money(Holder::Site(home.id)), 10_500);

    // The tenant is the host of a household: Cal lives with her.
    g.act(dee, "household_invite", json!({ "resident": cal }));
    let household = g.act(cal, "household_join", json!({ "host": dee }))["household"].clone();
    g.step(1);
    assert_eq!(g.site_now(home.id).tenants, [dee, cal]);
    assert_eq!(kin(g, cal).household, household.as_u64());
    // The tenant ends her lease by herself. The economy module says so
    // with an event, and the household ends with the lease: its member is
    // no longer a tenant of the home.
    g.act(dee, "end_lease", json!({}));
    assert_eq!(
        g.last_event("economy.lease_ended").data,
        json!({ "site": home.id, "tenant": dee, "by": "tenant" })
    );
    g.step(2);
    assert_eq!(g.site_now(home.id).tenants, []);
    assert_eq!((kin(g, dee).household, kin(g, cal).household), (None, None));
    assert_eq!(g.last_event("society.household_dissolved").data["cause"], "home_lost");

    // The owner and Dee marry.
    g.act(ben, "propose", json!({ "resident": dee }));
    assert_eq!(g.act(dee, "accept_proposal", json!({ "resident": ben })), json!({ "spouse": ben }));
    // His household: Dee joins and leaves again.
    g.act(ben, "household_invite", json!({ "resident": dee }));
    g.act(dee, "household_join", json!({ "host": ben }));
    g.step(1);
    assert_eq!(g.site_now(home.id).tenants, [dee]);
    // Talk at home: aloud, by message and on the public feed.
    assert_eq!(g.act(ben, "say", json!({ "text": "Welcome home." })), json!({ "heard_by": 2 }));
    assert_eq!(
        g.act(dee, "message", json!({ "to": cal, "text": "Come by tomorrow." }))["delivered"],
        true
    );
    assert!(g.act(ben, "post", json!({ "text": "Home 2 has a household." }))["post"].is_u64());
    // A private label and a public opinion.
    g.act(dee, "relate", json!({ "resident": ben, "label": "friend" }));
    g.act(dee, "opine", json!({ "resident": ben, "verdict": "praise" }));
    g.step(2);
    assert_eq!(g.attr(ben, "rep.standing"), 5);
    assert_eq!(
        g.section(dee, "relations").expect("relations")["labels"],
        json!({ "friend": [ben] })
    );
    // The end of it.
    g.act(dee, "household_leave", json!({}));
    assert_eq!(g.act(dee, "divorce", json!({})), json!({ "former_spouse": ben }));
    g.step(2);
    assert_eq!(g.site_now(home.id).tenants, []);
    assert_eq!((kin(g, ben).spouse, kin(g, dee).spouse), (None, None));
}
