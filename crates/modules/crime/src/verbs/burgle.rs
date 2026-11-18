//! The verb `burgle`: the till and the stock of a site that is not yours.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{ActionError, Begin, Builder, Holder, Idx, ItemId, Realm, Site};

use super::{learn, outcome, spec};
use crate::calc;
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::loot;
use crate::names::tag;
use crate::powers;

/// Arguments of `burgle`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct BurgleArgs {
    /// Name of the only good to take from the stock. Without it, you take
    /// the goods with the highest price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

/// Returns the site to burgle: the site where the resident stands, when
/// it is not its own.
fn target_site(realm: &Realm, idx: Idx) -> Result<Site, ActionError> {
    let Some(site) = powers::site_here(realm, idx) else {
        return Err(ActionError::not_here("You are not inside a site. Walk into one first."));
    };
    if powers::controls(realm, idx, &site) {
        return Err(ActionError::not_allowed("This site is yours. You cannot burgle it."));
    }
    Ok(site)
}

/// Returns the good that the arguments name.
fn wanted_item(realm: &Realm, args: &BurgleArgs) -> Result<Option<ItemId>, ActionError> {
    match &args.item {
        None => Ok(None),
        Some(name) => match realm.defs().item_id(name) {
            Some(item) => Ok(Some(item)),
            None => Err(ActionError::not_found(format!("No item is named `{name}`."))),
        },
    }
}

/// Returns the number of other residents inside a site that are awake.
fn occupants(realm: &Realm, h: &Handles, site: &Site, burglar: Idx) -> i64 {
    let reach = site.rect.width().max(site.rect.height());
    let count = realm
        .nearby(site.rect.centre(), reach)
        .into_iter()
        .filter(|&idx| {
            idx != burglar
                && site.rect.contains(realm.pos(idx))
                && !realm.has_condition(idx, h.asleep)
        })
        .count();
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Returns true when a resident carries an item with the tag `tool`.
fn has_tool(realm: &Realm, idx: Idx) -> bool {
    let carried = realm.ledger().items_of(Holder::Resident(realm.id_of(idx)));
    carried.iter().any(|&(item, qty)| qty > 0 && loot::has_tag(realm, item, tag::TOOL))
}

/// Returns the chance in basis points that the burglary works.
fn chance(realm: &Realm, h: &Handles, burglar: Idx, site: &Site) -> u32 {
    let p = &h.p;
    let mut raw = realm.param(p.burgle_base)
        + calc::level(realm, burglar, h.stealth) * realm.param(p.stealth_level);
    if calc::is_night(realm, h) {
        raw += realm.param(p.night_bonus);
    }
    match occupants(realm, h, site, burglar) {
        0 => raw += realm.param(p.burgle_empty),
        awake => raw -= awake.saturating_mul(realm.param(p.burgle_occupant)),
    }
    if has_tool(realm, burglar) {
        raw += realm.param(p.burgle_tool);
    }
    if site.has_tag(tag::PUBLIC) {
        raw -= realm.param(p.burgle_public);
    }
    calc::clamp_chance(realm, h, raw)
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<BurgleArgs>(
        spec(
            "burgle",
            "Steal from the till and the stock of the site where you stand. It is easier at night, in an empty site and with a tool. It is a crime.",
        ),
        move |realm, ctx, args| {
            target_site(realm, ctx.idx)?;
            wanted_item(realm, args)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.burgle_ticks) })
        },
        move |realm, ctx, args| {
            let site = target_site(realm, ctx.idx)?;
            let only = wanted_item(realm, args)?;
            let success = calc::roll(realm, chance(realm, &h, ctx.idx, &site));
            let case = commit::new_case(realm);
            let from = Holder::Site(site.id);
            let mut haul = loot::Haul::default();
            if success {
                let till = realm.ledger().money(from).cents();
                let cash = calc::percent(till, realm.param(h.p.burgle_till_pct));
                let units = realm.param(h.p.burgle_units_max);
                let goods = match only {
                    Some(item) => vec![(item, units.min(realm.ledger().qty(from, item)))],
                    None => loot::pick(realm, from, units),
                };
                haul = loot::take(realm, &h, case, from, ctx.idx, cash, &goods);
            }
            let (victim, owner) = powers::site_victim(&site);
            let deed = Deed {
                kind: CrimeKind::Burglary,
                perp: ctx.idx,
                pos: realm.pos(ctx.idx),
                site: Some(site.id),
                victim: Some(victim),
                target: None,
                owner,
                value: haul.value,
                failed: !success,
                force: false,
            };
            let filed = commit::file(realm, &h, case, &deed, Finder::Nobody);
            learn(realm, &h, ctx.idx, h.stealth, success);
            let mut out = outcome(success, filed);
            out.insert("site".into(), json!(site.id));
            out.insert("cash".into(), json!(haul.cash));
            out.insert("goods".into(), haul.goods_json(realm));
            out.insert("value".into(), json!(haul.value));
            Ok(Value::Object(out))
        },
    );
}
