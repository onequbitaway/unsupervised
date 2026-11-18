//! The verbs `vandalize` and `repair`.
//!
//! Vandalism destroys a part of the stock of a site through the sink
//! `destruction` and lowers the value of the site. The lost value is in
//! the site attribute `damage` until somebody repairs the site. A repair
//! costs money, which leaves the world through the sink `repair.materials`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{
    ActionError, Begin, Builder, EventMeta, Holder, Idx, Money, NoArgs, Realm, Site, SiteId,
};

use super::{learn, outcome, spec};
use crate::calc;
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::loot;
use crate::names::{event, key};
use crate::powers;

/// Returns the value that one act of vandalism can still take from a
/// site: `crime.vandalize_value_bp` of the value of the site without
/// damage, while the damage is below `crime.vandalize_damage_max_bp` of
/// that value.
fn value_loss(realm: &Realm, h: &Handles, site: &Site) -> i64 {
    let value = site.attr(key::VALUE).max(0);
    let damage = site.attr(key::DAMAGE).max(0);
    let whole = value.saturating_add(damage);
    let room = (calc::share(whole, realm.param(h.p.vandalize_damage_max)) - damage).max(0);
    calc::share(whole, realm.param(h.p.vandalize_value)).min(room).min(value)
}

/// Returns the site to damage: the site where the resident stands, when
/// it is not its own and something is left to damage.
fn target_site(realm: &Realm, h: &Handles, idx: Idx) -> Result<Site, ActionError> {
    let Some(site) = powers::site_here(realm, idx) else {
        return Err(ActionError::not_here("You are not inside a site. Walk into one first."));
    };
    if powers::controls(realm, idx, &site) {
        return Err(ActionError::not_allowed("This site is yours. You cannot vandalize it."));
    }
    let stock = realm.ledger().items_of(Holder::Site(site.id)).iter().any(|&(_, qty)| qty > 0);
    if !stock && value_loss(realm, h, &site) == 0 {
        return Err(ActionError::conflict("Nothing is left to damage at this site."));
    }
    Ok(site)
}

/// Destroys a part of each good in the stock of a site. Returns the goods
/// by name and their normal price.
fn destroy_stock(realm: &Realm, h: &Handles, site: SiteId) -> (Map<String, Value>, i64) {
    let holder = Holder::Site(site);
    let percent = realm.param(h.p.vandalize_stock_pct);
    let stock = realm.ledger().items_of(holder);
    let mut destroyed = Map::new();
    let mut value = 0i64;
    for (item, qty) in stock {
        let units = calc::percent_up(qty, percent).min(qty);
        if units > 0 && realm.ledger_mut().consume(holder, h.destroyed, item, units).is_ok() {
            destroyed.insert(realm.defs().item_name(item).to_string(), json!(units));
            value = value.saturating_add(loot::value_of(realm, item, units));
        }
    }
    (destroyed, value)
}

/// Moves value of a site between the attributes `value` and `damage`.
/// A positive `loss` damages the site, a negative one repairs it.
fn shift_value(realm: &Realm, site: SiteId, loss: i64) {
    let mut sites = realm.sites_mut();
    let Some(entry) = sites.get_mut(site) else {
        return;
    };
    let value = entry.attr(key::VALUE);
    let damage = entry.attr(key::DAMAGE);
    entry.set_attr(key::VALUE, value - loss);
    if damage + loss > 0 {
        entry.set_attr(key::DAMAGE, damage + loss);
    } else {
        entry.attrs.remove(key::DAMAGE);
    }
}

/// Arguments of `repair`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct RepairArgs {
    /// Cents of site value to give back. Without it, you repair all the
    /// damage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
}

/// Returns the value that a repair gives back and what it costs.
fn repair_plan(
    realm: &Realm,
    h: &Handles,
    idx: Idx,
    args: &RepairArgs,
) -> Result<(Site, i64, i64), ActionError> {
    if args.amount.is_some_and(|amount| amount < 1) {
        return Err(ActionError::invalid_args("`amount` must be at least 1."));
    }
    let Some(site) = powers::site_here(realm, idx) else {
        return Err(ActionError::not_here("You are not inside a site. Walk into one first."));
    };
    let damage = site.attr(key::DAMAGE).max(0);
    if damage == 0 {
        return Err(ActionError::conflict("This site is not damaged."));
    }
    let amount = args.amount.unwrap_or(damage).min(damage);
    // The cost rounds up, so that no repair is free.
    let cost =
        calc::div_up(amount.saturating_mul(realm.param(h.p.repair_cost).max(0)), calc::WHOLE_BP);
    let cash = realm.ledger().money(Holder::Resident(realm.id_of(idx))).cents();
    if cash < cost {
        return Err(ActionError::insufficient("You do not have the cash for this repair.")
            .with_details(json!({ "needed": cost, "available": cash })));
    }
    Ok((site, amount, cost))
}

/// Registers the two verbs.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<NoArgs>(
        spec(
            "vandalize",
            "Damage the site where you stand: a part of its stock is destroyed and its value falls until somebody repairs it. It is a crime.",
        ),
        move |realm, ctx, _args| {
            target_site(realm, &h, ctx.idx)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.vandalize_ticks) })
        },
        move |realm, ctx, _args| {
            let site = target_site(realm, &h, ctx.idx)?;
            let mut raw = realm.param(h.p.vandalize_base)
                + calc::level(realm, ctx.idx, h.stealth) * realm.param(h.p.stealth_level);
            if calc::is_night(realm, &h) {
                raw += realm.param(h.p.night_bonus);
            }
            let success = calc::roll(realm, calc::clamp_chance(realm, &h, raw));
            let case = commit::new_case(realm);
            let mut destroyed = Map::new();
            let mut goods_value = 0;
            let mut loss = 0;
            if success {
                (destroyed, goods_value) = destroy_stock(realm, &h, site.id);
                loss = value_loss(realm, &h, &site);
                if loss > 0 {
                    shift_value(realm, site.id, loss);
                }
            }
            let (victim, owner) = powers::site_victim(&site);
            let total = goods_value.saturating_add(loss);
            let deed = Deed {
                kind: CrimeKind::Vandalism,
                perp: ctx.idx,
                pos: realm.pos(ctx.idx),
                site: Some(site.id),
                victim: Some(victim),
                target: None,
                owner,
                value: total,
                failed: !success,
                force: false,
            };
            let filed = commit::file(realm, &h, case, &deed, Finder::Nobody);
            learn(realm, &h, ctx.idx, h.stealth, success);
            let mut out = outcome(success, filed);
            out.insert("site".into(), json!(site.id));
            out.insert("destroyed".into(), Value::Object(destroyed));
            out.insert("damage".into(), json!(loss));
            out.insert("value".into(), json!(total));
            Ok(Value::Object(out))
        },
    );

    b.verb::<RepairArgs>(
        spec(
            "repair",
            "Repair the damage of the site where you stand. You pay for the materials. The value of the site comes back.",
        ),
        move |realm, ctx, args| {
            repair_plan(realm, &h, ctx.idx, args)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.repair_ticks) })
        },
        move |realm, ctx, args| {
            let (site, amount, cost) = repair_plan(realm, &h, ctx.idx, args)?;
            realm.ledger_mut().burn(Holder::Resident(ctx.resident), h.repair, Money(cost))?;
            shift_value(realm, site.id, -amount);
            realm.attr_add(ctx.idx, h.labor, realm.param(h.p.repair_xp).max(0));
            let left = site.attr(key::DAMAGE) - amount;
            realm.emit(
                EventMeta::public(event::REPAIRED, h.drama(realm, h.p.drama_repaired))
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "site": site.id, "by": ctx.resident, "value": amount, "cost": cost }),
            );
            Ok(json!({ "site": site.id, "repaired": amount, "cost": cost, "damage": left }))
        },
    );
}
