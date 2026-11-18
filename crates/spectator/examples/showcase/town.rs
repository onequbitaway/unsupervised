//! The module of the showcase world: residents that walk by themselves,
//! and verbs that make the events that the viewer shows.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, Audience, Begin, Builder, Component, EventMeta, Holder, Plugin, Pos, Realm, Rect,
    ResidentId, Scope, Stage, TileMap, UNCLAIMED, Untrusted, VerbSpec,
};

use crate::map::{self, walkable};
use crate::numbers::Numbers;

/// The walk of one resident: the tiles that it still has to step on, the
/// last one first.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stroll {
    path: Vec<Pos>,
}

impl Component for Stroll {
    const NAME: &'static str = "showcase.stroll";
}

/// Arguments of the busy verbs.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicksArgs {
    /// Duration in ticks.
    pub ticks: u32,
}

/// Arguments of `say`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SayArgs {
    /// The words.
    pub text: String,
}

/// Arguments of verbs that name another resident.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct OtherArgs {
    /// The other resident.
    pub other: ResidentId,
}

/// Arguments of verbs that take none.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Nothing {}

fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "showcase", summary, free: false, allowed_in: &[] }
}

/// Finds the shortest walk from `from` to `to` inside `bounds`, at most
/// `limit` tiles long. The answer has the last step first; it is empty when
/// there is no walk.
fn find_path(tiles: &TileMap, bounds: Rect, from: Pos, to: Pos, limit: usize) -> Vec<Pos> {
    let width = bounds.width() as usize;
    let slot = |p: Pos| p.y.abs_diff(bounds.y0) as usize * width + p.x.abs_diff(bounds.x0) as usize;
    let mut came: Vec<Option<Pos>> = vec![None; width * bounds.height() as usize];
    let mut queue = VecDeque::from([from]);
    came[slot(from)] = Some(from);
    while let Some(at) = queue.pop_front() {
        if at == to {
            let mut path = Vec::new();
            let mut step = to;
            while step != from && path.len() <= limit {
                path.push(step);
                step = came[slot(step)].unwrap_or(from);
            }
            return if path.len() > limit { Vec::new() } else { path };
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let next = at.offset(dx, dy);
            if bounds.contains(next)
                && came[slot(next)].is_none()
                && tiles.get(next).is_some_and(walkable)
            {
                came[slot(next)] = Some(at);
                queue.push_back(next);
            }
        }
    }
    Vec::new()
}

/// Chooses where an idle resident walks to: a tile of this district, or
/// the end of the avenue towards a neighbour district.
fn choose_walk(realm: &Realm, from: Pos, numbers: &Numbers) -> Vec<Pos> {
    let Some(info) = realm.zone_info() else {
        return Vec::new();
    };
    let b = info.bounds;
    let c = b.centre();
    let mut rng = realm.rng();
    // The tile behind the border when the walk leaves the district.
    let mut beyond = None;
    let target = if rng.chance(u64::from(numbers.walk.leave_zone_percent), 100) {
        let exits = [
            (Pos::new(c.x, b.y0), Pos::new(c.x, b.y0 - 1)),
            (Pos::new(b.x1 - 1, c.y), Pos::new(b.x1, c.y)),
            (Pos::new(c.x, b.y1 - 1), Pos::new(c.x, b.y1)),
            (Pos::new(b.x0, c.y), Pos::new(b.x0 - 1, c.y)),
        ];
        let open: Vec<(Pos, Pos)> = exits
            .into_iter()
            .filter(|(_, out)| {
                realm.layout().zone_at(*out).is_some_and(|z| {
                    realm.layout().zone(z).is_some_and(|other| other.city == info.city)
                })
            })
            .collect();
        match rng.pick(&open) {
            Some((edge, out)) => {
                beyond = Some(*out);
                *edge
            }
            None => return Vec::new(),
        }
    } else {
        Pos::new(
            rng.range(i64::from(b.x0), i64::from(b.x1)) as i32,
            rng.range(i64::from(b.y0), i64::from(b.y1)) as i32,
        )
    };
    drop(rng);
    let tiles = realm.tiles();
    if !tiles.get(target).is_some_and(walkable) {
        return Vec::new();
    }
    let mut path = find_path(&tiles, b, from, target, numbers.walk.max_path as usize);
    if let Some(out) = beyond
        && (!path.is_empty() || from == target)
    {
        path.insert(0, out);
    }
    path
}

/// Moves every idle resident one tile along its walk, and starts new walks.
fn stroll(realm: &Realm, numbers: &Numbers) {
    let all: Vec<_> = realm.indices().collect();
    for idx in all {
        if realm.activity(idx).is_some() {
            continue;
        }
        let next = realm.col_mut::<Stroll>()[idx].path.pop();
        match next {
            Some(next) => {
                if realm.set_pos(idx, next).is_err() {
                    realm.col_mut::<Stroll>()[idx].path.clear();
                }
            }
            None => {
                let start = realm.rng().chance(u64::from(numbers.walk.start_per_mille), 1_000);
                if start {
                    let path = choose_walk(realm, realm.pos(idx), numbers);
                    realm.col_mut::<Stroll>()[idx].path = path;
                }
            }
        }
    }
}

/// The showcase module.
pub struct Town {
    /// The numbers of `data/showcase.toml`.
    pub numbers: Numbers,
}

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "showcase"
    }

    fn build(&self, b: &mut Builder) {
        let drama = self.numbers.drama.clone();
        b.component::<Stroll>();
        if b.scope() == Scope::Zone {
            b.on_worldgen(0, |realm, world| {
                let Some(info) = realm.zone_info() else { return };
                let number = world
                    .layout
                    .city(info.city)
                    .and_then(|city| city.zones.iter().position(|zone| *zone == info.id))
                    .unwrap_or(0);
                map::generate(realm, number as u32);
            });
            let numbers = self.numbers.clone();
            b.system(Stage::Update, 0, "showcase.stroll", move |realm| stroll(realm, &numbers));
        }

        // Verbs that only take time. The viewer shows them as the activity.
        for (verb, summary) in [
            ("eat", "Eat a meal."),
            ("rest", "Rest for a while."),
            ("sleep", "Sleep."),
            ("train", "Train a skill."),
        ] {
            b.verb::<TicksArgs>(
                spec(verb, summary),
                |_, _, args| Ok(Begin::Busy { ticks: args.ticks.max(1) }),
                |_, _, _| Ok(Value::Null),
            );
        }
        let shift = drama.shift_completed;
        b.verb::<TicksArgs>(
            spec("work", "Work a shift."),
            |_, _, args| Ok(Begin::Busy { ticks: args.ticks.max(1) }),
            move |realm, ctx, _| {
                let pos = realm.pos(ctx.idx);
                let site = realm.sites().at(pos).map(|site| site.id);
                realm.emit(
                    EventMeta::public("economy.shift_completed", shift).actor(ctx.resident).at(pos),
                    &json!({
                        "resident": ctx.resident, "site": site, "wage": 1_200, "tax": 120,
                        "skill": "labor",
                    }),
                );
                Ok(Value::Null)
            },
        );
        let said = drama.said;
        b.instant_verb::<SayArgs>(spec("say", "Say something aloud."), move |realm, ctx, args| {
            realm.emit(
                EventMeta::public("society.said", said)
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx))
                    .audience(Audience::Nearby(6)),
                &json!({ "text": Untrusted::new(ctx.resident, args.text.clone()) }),
            );
            Ok(Value::Null)
        });
        let theft = drama.theft_committed;
        b.instant_verb::<OtherArgs>(
            spec("rob", "Take money from a resident."),
            move |realm, ctx, args| {
                if realm.idx_of(args.other).is_none() || args.other == ctx.resident {
                    return Err(ActionError::not_found("The victim is not in this district."));
                }
                realm.emit(
                    EventMeta::public("crime.theft_committed", theft)
                        .actor(ctx.resident)
                        .at(realm.pos(ctx.idx)),
                    &json!({ "thief": ctx.resident, "victim": args.other, "amount": 4_500 }),
                );
                Ok(Value::Null)
            },
        );
        let married = drama.married;
        b.instant_verb::<OtherArgs>(spec("marry", "Marry a resident."), move |realm, ctx, args| {
            if realm.idx_of(args.other).is_none() || args.other == ctx.resident {
                return Err(ActionError::not_found("The partner is not in this district."));
            }
            realm.emit(
                EventMeta::public("society.married", married)
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "a": ctx.resident, "b": args.other }),
            );
            Ok(Value::Null)
        });
        let opened = drama.business_opened;
        b.instant_verb::<Nothing>(
            spec("open_business", "Open a business in the nearest building."),
            move |realm, ctx, _| {
                let pos = realm.pos(ctx.idx);
                let sites = realm.sites();
                let Some(site) = sites.within(pos, 12).into_iter().find(|s| s.kind != "park")
                else {
                    return Err(ActionError::not_found("No building is near."));
                };
                realm.emit(
                    EventMeta::public("economy.business_opened", opened)
                        .actor(ctx.resident)
                        .at(site.rect.centre()),
                    &json!({
                        "site": site.id, "owner": Holder::Resident(ctx.resident), "kind": site.kind,
                    }),
                );
                Ok(Value::Null)
            },
        );
        let died = drama.resident_died;
        b.instant_verb::<Nothing>(spec("die", "Leave the world."), move |realm, ctx, _| {
            realm.emit(
                EventMeta::public("life.resident_died", died)
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({
                    "resident": ctx.resident, "cause": "old_age", "age": 81, "heir": null,
                    "estate": 0,
                }),
            );
            realm.despawn(ctx.idx, UNCLAIMED, "old_age");
            Ok(Value::Null)
        });
        let election = drama.election_decided;
        b.global_verb::<Nothing>(
            spec("win_election", "Win the election for mayor."),
            move |realm, ctx, _| {
                let city = realm.layout().zone(ctx.zone).map(|zone| zone.city);
                realm.emit(
                    EventMeta::public("politics.election_decided", election).actor(ctx.resident),
                    &json!({ "city": city, "office": "mayor", "winner": ctx.resident }),
                );
                Ok(Value::Null)
            },
        );
    }
}
