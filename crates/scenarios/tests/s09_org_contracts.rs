//! Story 9: an organization and two contracts.
//!
//! Ann founds a guild. It has roles: Ben becomes the treasurer and can
//! spend the treasury, a plain member cannot. Then the guild buys food
//! with two contracts. Cal signs one and delivers: the escrow pays him
//! back his penalty. Otto signs the other and does not deliver: at the
//! deadline the engine gives his penalty to the buyer.
//!
//! The story starts in the republic (`unsup_scenarios::republic`). All the
//! money of the story is wages that the residents earned there.

use serde_json::{Value, json};
use unsup_kernel::{CityId, ErrorCode, Holder, OrgId, ResidentId};
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic_in_republic};

const CITY: CityId = CityId(0);

/// A contract of the buyer: 1,500 at the signing for 10 vegetables in two
/// hours, with a penalty of 800 on the seller.
fn purchase(seller: ResidentId) -> Value {
    json!({
        "to": seller,
        "clauses": [
            { "kind": "pay_at_signing", "by": "offerer", "amount": 1_500 },
            { "kind": "deliver", "by": "counterparty", "item": "vegetables", "qty": 10, "due_in": 120 },
            { "kind": "penalty", "by": "counterparty", "amount": 800 },
        ],
    })
}

fn story(g: &mut Game, republic: &Republic) {
    let [ann, ben, cal, dee] = republic.citizens;
    let otto = republic.officer;
    let treasury = g.city_treasury(CITY);
    let start = [ann, ben, cal, dee, otto].map(|r| g.cash(r));
    assert_eq!(start, [12_600, 12_600, 12_600, 12_600, 17_600]);

    // The guild. The fee of 1,000 goes to the city.
    let created = g.act(ann, "org_create", json!({ "name": "Granary Guild" }));
    let org = OrgId(created["org"].as_u64().expect("an organization id"));
    assert_eq!(created, json!({ "org": org, "role": "owner", "fee": 1_000 }));
    g.step(1);
    assert_eq!((g.cash(ann), g.city_treasury(CITY)), (start[0] - 1_000, treasury + 1_000));
    assert_eq!(
        g.refused(dee, "org_create", json!({ "name": " granary guild " })),
        ErrorCode::Conflict
    );
    assert_eq!(g.cash(dee), start[3], "a refused request gives the fee back");

    // Members come by invitation.
    assert_eq!(g.refused(ben, "org_join", json!({ "org": org })), ErrorCode::NotAllowed);
    g.act(ann, "org_invite", json!({ "org": org, "resident": ben }));
    g.act(ann, "org_invite", json!({ "org": org, "resident": dee }));
    assert_eq!(g.notice(ben, "society.org_invited").data["org"], json!(org));
    assert_eq!(
        g.act(ben, "org_join", json!({ "org": org })),
        json!({ "org": org, "role": "member", "members": 2 })
    );
    assert_eq!(g.act(dee, "org_join", json!({ "org": org }))["members"], 3);

    // Roles. A treasurer can spend. A member cannot. The zone of each
    // member has the role one frame later.
    let role = g.act(
        ann,
        "org_define_role",
        json!({ "org": org, "role": "treasurer", "permissions": ["treasury.spend"] }),
    );
    assert_eq!(
        role,
        json!({ "org": org, "role": "treasurer", "permissions": ["treasury.spend"], "inactive": [] })
    );
    g.act(ann, "org_set_role", json!({ "org": org, "resident": ben, "role": "treasurer" }));
    g.step(1);
    let can = |g: &Game, r: ResidentId, perm: &str| {
        g.world.with_resident(r, |realm, idx| realm.can(idx, org, perm))
    };
    assert_eq!(
        [
            can(g, ann, "treasury.spend"),
            can(g, ben, "treasury.spend"),
            can(g, dee, "treasury.spend"),
            can(g, ben, "org.manage")
        ],
        [true, true, false, false]
    );
    assert_eq!(g.notice(ben, "society.org_role_changed").data["role"], "treasurer");

    // The treasury. Everybody can pay in. Only the treasurer and the owner
    // pay out.
    g.act(ann, "org_deposit", json!({ "org": org, "amount": 5_000 }));
    g.act(dee, "org_deposit", json!({ "org": org, "amount": 1_000 }));
    g.step(1);
    assert_eq!(g.money(Holder::Org(org)), 6_000);
    let pay = json!({ "org": org, "amount": 4_600, "to_resident": dee });
    assert_eq!(g.refused(dee, "org_pay", pay.clone()), ErrorCode::NotAllowed);
    assert_eq!(g.refused(cal, "org_pay", pay.clone()), ErrorCode::NotAllowed);
    assert_eq!(
        g.refused(ben, "org_pay", json!({ "org": org, "amount": 6_001, "to_resident": dee })),
        ErrorCode::Insufficient
    );
    // The treasurer gives Dee the money for the purchases of the guild.
    assert_eq!(
        g.act(ben, "org_pay", pay),
        json!({ "org": org, "amount": 4_600, "treasury": 1_400 })
    );
    g.step(1);
    assert_eq!((g.money(Holder::Org(org)), g.cash(dee)), (1_400, start[3] - 1_000 + 4_600));
    assert_eq!(g.notice(dee, "society.org_payment").data["amount"], 4_600);
    let seen = g.section(ben, "orgs").expect("an orgs section");
    assert_eq!(
        (
            seen["member_of"][0]["role"].as_str(),
            seen["member_of"][0]["treasury"].as_i64(),
            seen["member_of"][0]["members"].as_i64()
        ),
        (Some("treasurer"), Some(1_400), Some(3))
    );
    // The members talk in private.
    // The text goes through the global realm: it arrives two frames later.
    g.act(dee, "org_say", json!({ "org": org, "text": "Two sellers. I pay at the signing." }));
    g.step(2);
    assert_eq!(
        g.notice(ann, "society.org_message").data["text"]["text"],
        "Two sellers. I pay at the signing."
    );
    assert!(g.notices(cal, "society.org_message").is_empty());

    // Two offers. Each locks the 1,500 of the buyer in its own escrow.
    let dee_cash = g.cash(dee);
    let first = g.act(dee, "contract_offer", purchase(cal));
    let second = g.act(dee, "contract_offer", purchase(otto));
    let (kept, broken) =
        (first["contract"].as_u64().expect("an id"), second["contract"].as_u64().expect("an id"));
    assert_eq!(first["locked"], 1_500);
    assert_eq!(
        (g.cash(dee), g.money(Holder::Escrow(kept)), g.money(Holder::Escrow(broken))),
        (dee_cash - 3_000, 1_500, 1_500)
    );
    // Each seller reads its offer: its part is the penalty of 800.
    let read = g.act(cal, "contract_view", json!({ "contract": kept }));
    assert_eq!(
        (read["you_are"].as_str(), read["your_part"].as_i64(), read["status"].as_str()),
        (Some("counterparty"), Some(800), Some("offered"))
    );
    // An offer is for the resident that it names.
    assert_eq!(
        g.refused(ben, "contract_sign", json!({ "contract": kept, "stake": 800 })),
        ErrorCode::NotAllowed
    );
    // A stake below the part is refused, and no money stays behind.
    assert_eq!(
        g.refused(cal, "contract_sign", json!({ "contract": kept, "stake": 799 })),
        ErrorCode::Insufficient
    );
    assert_eq!(g.cash(cal), start[2]);

    // Both sign in the same frame. Each seller locks 800 and gets 1,500.
    let signed = g.act_all(&[
        (cal, "contract_sign", json!({ "contract": kept, "stake": 800 })),
        (otto, "contract_sign", json!({ "contract": broken, "stake": 800 })),
    ]);
    assert_eq!(signed[0], json!({ "contract": kept, "locked": 800, "status": "active" }));
    let signing = g.last_event("society.contract_signed").frame;
    g.step(1);
    assert_eq!((g.cash(cal), g.cash(otto)), (start[2] + 700, start[4] + 700));
    assert_eq!((g.money(Holder::Escrow(kept)), g.money(Holder::Escrow(broken))), (800, 800));
    let deadline = signing + 120;
    assert_eq!(
        g.section(cal, "contracts").expect("a contracts section")["list"][0]["next_due"],
        deadline
    );

    // Cal delivers. He must stand next to the buyer: from the city hall
    // the goods do not reach her.
    let hall = g.site("city_hall");
    g.walk(cal, &hall);
    assert_eq!(g.refused(cal, "contract_claim", json!({ "contract": kept })), ErrorCode::NotHere);
    g.walk_to(cal, g.pos(dee).offset(1, 0));
    let (cal_food, dee_food) = (g.carried(cal, "vegetables"), g.carried(dee, "vegetables"));
    let delivered = g.act(cal, "contract_claim", json!({ "contract": kept }));
    assert_eq!(
        delivered,
        json!({ "contract": kept, "clause": 1, "item": "vegetables", "qty": 10 })
    );
    g.step(2);
    assert_eq!(
        (g.carried(cal, "vegetables"), g.carried(dee, "vegetables")),
        (cal_food - 10, dee_food + 10)
    );
    // The contract is fulfilled: the penalty goes back, and both parties
    // gain standing.
    assert_eq!(
        g.last_event("society.contract_fulfilled").data,
        json!({ "contract": kept, "parties": [dee, cal] })
    );
    assert_eq!((g.cash(cal), g.money(Holder::Escrow(kept))), (start[2] + 1_500, 0));
    assert_eq!(
        g.notice(cal, "society.contract_ended").data,
        json!({ "contract": kept, "outcome": "fulfilled" })
    );
    assert_eq!((g.attr(cal, "rep.standing"), g.attr(dee, "rep.standing")), (10, 10));

    // Otto does nothing. One frame before the deadline the contract runs.
    g.run_to_frame(deadline - 1);
    assert!(g.events_of("society.contract_breached").is_empty());
    assert_eq!(g.money(Holder::Escrow(broken)), 800);
    // At the deadline the engine judges: Otto breached. His penalty goes
    // to the buyer, and he loses standing. He keeps the 1,500 that he got
    // at the signing.
    g.step(1);
    let breached = g.last_event("society.contract_breached");
    assert_eq!(breached.frame, deadline);
    assert_eq!(breached.data, json!({ "contract": broken, "by": otto, "parties": [dee, otto] }));
    g.step(1);
    assert_eq!(g.money(Holder::Escrow(broken)), 0);
    assert_eq!(g.cash(otto), start[4] + 1_500 - 800);
    assert_eq!(g.cash(dee), dee_cash - 3_000 + 800);
    assert_eq!(
        g.notice(otto, "society.contract_ended").data,
        json!({ "contract": broken, "outcome": "breached", "by": [otto] })
    );
    assert_eq!((g.attr(otto, "rep.standing"), g.attr(dee, "rep.standing")), (-50, 10));
    assert_eq!(g.section(otto, "contracts"), None);
    // A contract that ended takes no delivery.
    assert_eq!(
        g.refused(otto, "contract_claim", json!({ "contract": broken })),
        ErrorCode::NotFound
    );

    // The story made no money and lost none: the five residents, the guild
    // and the city fee add up to what the residents had.
    let now: i64 = [ann, ben, cal, dee, otto].iter().map(|r| g.cash(*r)).sum();
    assert_eq!(now + g.money(Holder::Org(org)) + 1_000, start.iter().sum::<i64>());
    g.audit();
}

#[test]
fn a_guild_with_roles_and_a_treasury_keeps_one_contract_and_loses_one() {
    assert_deterministic_in_republic(&Setup::town(1), story);
}
