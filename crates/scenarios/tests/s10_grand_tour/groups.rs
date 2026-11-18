//! A guild, its contracts and its missions.

use serde_json::json;
use unsup_kernel::{Holder, OrgId};
use unsup_scenarios::{Game, Republic};

use crate::Tour;

/// Day 7: Ann founds a guild with a treasurer and a treasury. One member
/// is removed and one leaves. The mayor grants the guild a power of the
/// law.
pub fn guild(g: &mut Game, cast: &Republic) -> OrgId {
    let [ann, ben, cal, dee] = cast.citizens;
    let (judy, bram) = (cast.judge, cast.banker);
    let created = g.act(ann, "org_create", json!({ "name": "Tour Guild" }));
    let org = OrgId(created["org"].as_u64().expect("an organization id"));
    for member in [ben, cal, dee, judy, bram] {
        g.act(ann, "org_invite", json!({ "org": org, "resident": member }));
        g.act(member, "org_join", json!({ "org": org }));
    }
    g.act(
        ann,
        "org_define_role",
        json!({ "org": org, "role": "treasurer", "permissions": ["treasury.spend"] }),
    );
    g.act(ann, "org_set_role", json!({ "org": org, "resident": ben, "role": "treasurer" }));
    g.act(ann, "org_deposit", json!({ "org": org, "amount": 4_000 }));
    g.act(dee, "org_say", json!({ "org": org, "text": "The guild meets at the exchange." }));
    assert_eq!(
        g.act(ann, "org_kick", json!({ "org": org, "resident": judy }))["removed"],
        json!(judy)
    );
    assert_eq!(g.act(bram, "org_leave", json!({ "org": org }))["dissolved"], false);
    g.step(2);
    assert_eq!(g.money(Holder::Org(org)), 4_000);
    assert_eq!(g.section(ann, "orgs").expect("an orgs section")["member_of"][0]["members"], 4);

    // The mayor lets the guild search for the city. A member with a role
    // that lists the power has it one frame later.
    let city = OrgId::city(crate::CITY);
    let granted = g.act(
        cast.mayor,
        "org_grant",
        json!({ "org": city, "to": org, "permission": "law.search" }),
    );
    assert_eq!(granted["granted"], true);
    g.act(
        ann,
        "org_define_role",
        json!({ "org": org, "role": "watchman", "permissions": ["law.search"] }),
    );
    g.act(ann, "org_set_role", json!({ "org": org, "resident": dee, "role": "watchman" }));
    g.step(1);
    assert!(g.world.with_resident(dee, |realm, idx| realm.can(idx, org, "law.search")));
    org
}

/// Day 8: one contract that is kept, one offer that is rejected and one
/// that is taken back.
pub fn contracts(g: &mut Game, tour: &Tour) {
    let [_, ben, cal, dee] = tour.cast.citizens;
    let sale = |to: Option<unsup_kernel::ResidentId>| {
        json!({
            "to": to,
            "clauses": [
                { "kind": "pay_at_signing", "by": "offerer", "amount": 500 },
                { "kind": "deliver", "by": "counterparty", "item": "vegetables", "qty": 3, "due_in": 120 },
            ],
        })
    };
    let dee_cash = g.cash(dee);
    let kept = g.act(dee, "contract_offer", sale(Some(cal)))["contract"].as_u64().expect("an id");
    let rejected =
        g.act(dee, "contract_offer", sale(Some(ben)))["contract"].as_u64().expect("an id");
    let withdrawn = g.act(dee, "contract_offer", sale(None))["contract"].as_u64().expect("an id");
    assert_eq!(g.cash(dee), dee_cash - 1_500);
    assert_eq!(g.act(cal, "contract_view", json!({ "contract": kept }))["your_part"], 0);
    g.act(cal, "contract_sign", json!({ "contract": kept }));
    assert_eq!(
        g.act(ben, "contract_reject", json!({ "contract": rejected }))["status"],
        "rejected"
    );
    assert_eq!(
        g.act(dee, "contract_cancel", json!({ "contract": withdrawn }))["status"],
        "cancelled"
    );
    g.walk_to(cal, g.pos(dee).offset(1, 0));
    g.act(cal, "contract_claim", json!({ "contract": kept }));
    g.step(2);
    assert_eq!(g.last_event("society.contract_fulfilled").data["contract"], kept);
    // The two offers that ended gave their money back.
    assert_eq!(g.cash(dee), dee_cash - 500);
}

/// Day 8: a team mission for the guild. Two members accept, one gives up,
/// and the shift of the other completes it.
pub fn missions(g: &mut Game, tour: &Tour) {
    let [ann, ben, cal, _] = tour.cast.citizens;
    let guild = tour.guild;
    let posted = g.act(
        ann,
        "mission_post",
        json!({
            "title": "A shift for the guild",
            "objectives": [{ "event": "economy.shift_completed", "count": 1, "actor": "team" }],
            "mode": "team",
            "org": guild,
            "reward": 600,
            "deadline_in": 600,
            "takers": 2,
        }),
    );
    let mission = posted["mission"].as_u64().expect("a mission id");
    let open = g.act(ben, "mission_list", json!({ "mode": "team" }));
    assert_eq!(open["missions"][0]["mission"], mission);
    g.act(ben, "mission_accept", json!({ "mission": mission, "org": guild }));
    g.act(cal, "mission_accept", json!({ "mission": mission, "org": guild }));
    assert_eq!(g.act(cal, "mission_abandon", json!({ "mission": mission }))["status"], "abandoned");
    let hall = g.site("city_hall");
    g.walk(ben, &hall);
    let cash = g.cash(ben);
    g.act(ben, "work", json!({}));
    g.step(1);
    assert_eq!(g.cash(ben), cash + 2_520 + 600);
    assert_eq!(g.act(ben, "mission_claim", json!({ "mission": mission }))["paid"], 600);
    let board = g.act(ben, "leaderboard", json!({ "board": "missions" }));
    assert_eq!(board["top"][0]["resident"], json!(ben));
}
