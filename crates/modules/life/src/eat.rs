//! Food and medicine: the verbs `eat` and `use_item`.
//!
//! The life module does not define goods. It reads tags and attributes: an
//! item with the tag `food` and the attribute `nutrition` can be eaten, and
//! an item with the tag `medicine` and the attribute `healing` can heal.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{
    ActionCtx, ActionError, Begin, Builder, EventMeta, Holder, Idx, ItemId, Realm, VerbSpec,
};

use crate::care::BODY_CARE;
use crate::handles::Handles;
use crate::names::{attr, key, tag};

/// Arguments of `eat`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EatArgs {
    /// Name of the item to eat. It must have the tag `food`. Without it,
    /// you eat the food that you carry which fits your hunger best.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

/// Arguments of `use_item`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct UseItemArgs {
    /// Name of the item to use. It must have the tag `medicine`.
    pub item: String,
}

/// Returns the food points of one unit of an item, or `None` when the item
/// is not food.
fn nutrition(realm: &Realm, item: ItemId) -> Option<i64> {
    let def = realm.defs().item(item)?;
    let points = def.attr_value(key::NUTRITION);
    (def.has_tag(tag::FOOD) && points > 0).then_some(points)
}

/// Returns the food that a resident carries which fits its hunger best: the
/// one with the most food points that are not wasted, else the one with the
/// fewest food points. Ties go to the lower item id.
pub(crate) fn best_food(realm: &Realm, h: &Handles, idx: Idx) -> Option<ItemId> {
    let hunger = attr::NEED_MAX - realm.attr(idx, h.food);
    let carried = realm.ledger().items_of(Holder::Resident(realm.id_of(idx)));
    let foods: Vec<(ItemId, i64)> = carried
        .into_iter()
        .filter(|&(_, qty)| qty > 0)
        .filter_map(|(item, _)| nutrition(realm, item).map(|points| (item, points)))
        .collect();
    let fits = foods
        .iter()
        .filter(|&&(_, points)| points <= hunger)
        .max_by_key(|&&(item, points)| (points, std::cmp::Reverse(item)));
    let smallest = foods.iter().min_by_key(|&&(item, points)| (points, item));
    fits.or(smallest).map(|&(item, _)| item)
}

/// Finds the item of an `eat` request and checks that the resident carries
/// it.
fn food_for(
    realm: &Realm,
    h: &Handles,
    idx: Idx,
    wanted: Option<&str>,
) -> Result<ItemId, ActionError> {
    let Some(name) = wanted else {
        return best_food(realm, h, idx)
            .ok_or_else(|| ActionError::insufficient("You carry no food."));
    };
    let Some(item) = realm.defs().item_id(name) else {
        return Err(ActionError::not_found(format!("No item is named `{name}`.")));
    };
    if nutrition(realm, item).is_none() {
        return Err(ActionError::not_allowed(format!("`{name}` is not food.")));
    }
    if realm.ledger().qty(Holder::Resident(realm.id_of(idx)), item) < 1 {
        return Err(ActionError::insufficient(format!("You carry no `{name}`.")));
    }
    Ok(item)
}

/// A resident eats one unit of a food item that it carries. Returns the
/// result of the meal.
pub(crate) fn eat_one(
    realm: &Realm,
    h: &Handles,
    idx: Idx,
    item: ItemId,
) -> Result<Value, ActionError> {
    let resident = realm.id_of(idx);
    let Some(points) = nutrition(realm, item) else {
        return Err(ActionError::not_allowed("That item is not food."));
    };
    realm.ledger_mut().consume(Holder::Resident(resident), h.eaten, item, 1)?;
    let food = realm.attr_add(idx, h.food, points);
    realm.attr_add(idx, h.mood, realm.param(h.p.meal_mood));
    let name = realm.defs().item_name(item).to_string();
    realm.emit(
        EventMeta::public("life.meal_eaten", h.drama(realm, h.p.drama_meal))
            .actor(resident)
            .at(realm.pos(idx)),
        &json!({ "resident": resident, "item": name, "nutrition": points }),
    );
    Ok(json!({ "item": name, "nutrition": points, "food": food }))
}

/// Finds the item of a `use_item` request. Returns the item and the health
/// points of one unit.
fn medicine_for(realm: &Realm, name: &str) -> Result<(ItemId, i64), ActionError> {
    let defs = realm.defs();
    let Some(item) = defs.item_id(name) else {
        return Err(ActionError::not_found(format!("No item is named `{name}`.")));
    };
    let healing = defs
        .item(item)
        .filter(|def| def.has_tag(tag::MEDICINE))
        .map_or(0, |def| def.attr_value(key::HEALING));
    if healing <= 0 {
        return Err(ActionError::not_allowed(format!(
            "`{name}` cannot be used. Only medicine can."
        )));
    }
    Ok((item, healing))
}

/// Registers the verbs `eat` and `use_item`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<EatArgs>(
        VerbSpec {
            verb: "eat",
            module: "life",
            summary: "Eat one unit of food that you carry.",
            free: false,
            allowed_in: BODY_CARE,
        },
        move |realm, ctx: &ActionCtx, args| {
            if realm.attr(ctx.idx, h.food) >= attr::NEED_MAX {
                return Err(ActionError::conflict("You are not hungry: your food is full."));
            }
            food_for(realm, &h, ctx.idx, args.item.as_deref())?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.eat_ticks) })
        },
        move |realm, ctx, args| {
            // The food can be gone by now: given away, stolen or spoiled.
            let item = food_for(realm, &h, ctx.idx, args.item.as_deref())?;
            eat_one(realm, &h, ctx.idx, item)
        },
    );
    b.verb::<UseItemArgs>(
        VerbSpec {
            verb: "use_item",
            module: "life",
            summary: "Use one unit of an item that you carry. Medicine gives health back.",
            free: false,
            allowed_in: BODY_CARE,
        },
        move |realm, ctx: &ActionCtx, args| {
            let (item, _) = medicine_for(realm, &args.item)?;
            if realm.attr(ctx.idx, h.health) >= attr::NEED_MAX {
                return Err(ActionError::conflict("You are healthy: your health is full."));
            }
            if realm.ledger().qty(Holder::Resident(ctx.resident), item) < 1 {
                return Err(ActionError::insufficient(format!("You carry no `{}`.", args.item)));
            }
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.use_item_ticks) })
        },
        move |realm, ctx, args| {
            let (item, healing) = medicine_for(realm, &args.item)?;
            realm.ledger_mut().consume(Holder::Resident(ctx.resident), h.used, item, 1)?;
            let health = realm.attr_add(ctx.idx, h.health, healing);
            realm.emit(
                EventMeta::public("life.item_used", h.drama(realm, h.p.drama_item_used))
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "resident": ctx.resident, "item": args.item, "healing": healing }),
            );
            Ok(json!({ "item": args.item, "healing": healing, "health": health }))
        },
    );
}
