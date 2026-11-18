//! Walking: the verb `move_to` and the system that moves walkers one tile
//! at a time.
//!
//! A walk is planned zone by zone. In the zone of the target, the path goes
//! to the target tile. In another zone, the path goes to the edge of the
//! zone that is on the way, and one more step crosses the border; the kernel
//! moves the resident to the next zone and the walk goes on there. Every
//! tile at the edge of a zone is a road, so the step across is always
//! possible.
//!
//! A walk inside one zone is a busy action: its length is known when it
//! starts, and the kernel ends it. A walk to another zone is a waiting
//! action: the life module ends it on arrival.
//!
//! Only the residents that walk are touched in a tick. The zone keeps their
//! ids in the resource [`Walkers`].

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{
    ActionCtx, ActionError, Begin, Builder, Event, Idx, Layout, Pos, Realm, RealmId, ResidentId,
    Resource, Scope, SiteId, Stage, TileMap, VerbSpec, ZoneInfo,
};

use crate::body::Body;
use crate::handles::Handles;
use crate::names::tile;
use crate::path::{self, EAST, Goal, NORTH, NoRoute, SOUTH, Scratch, WEST};
use crate::sites::{self, SiteAt};
use crate::tiles::Costs;

/// What a walk does next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Phase {
    /// The resident follows the path.
    #[default]
    Walking,
    /// The resident is in a new zone and needs a path.
    Plan,
    /// The walk waits for the global realm to say where the site is.
    Lookup,
}

/// A walk in progress. It is part of [`Body`], so it moves with the
/// resident to the next zone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Walk {
    /// Id of the action `move_to`.
    #[serde(rename = "a")]
    pub(crate) action: u64,
    /// The tile where the walk ends.
    #[serde(rename = "t")]
    pub(crate) target: Pos,
    /// The tile where the walk put the resident last. A resident that is
    /// somewhere else was moved by something else, and the walk stops.
    #[serde(rename = "o", default)]
    pub(crate) at: Pos,
    /// The site that the request named.
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    pub(crate) site: Option<SiteId>,
    /// The steps in the current zone, as directions.
    #[serde(rename = "p", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) path: Vec<u8>,
    /// Position of the next step in `path`.
    #[serde(rename = "c", default)]
    pub(crate) cursor: u32,
    /// Tick of the zone at which the next step is made. After an arrival
    /// in a new zone it is the tick of the arrival.
    #[serde(rename = "d", default)]
    pub(crate) due: u64,
    /// Steps made so far, in all zones.
    #[serde(rename = "n", default)]
    pub(crate) steps: u32,
    #[serde(rename = "f", default)]
    pub(crate) phase: Phase,
    /// True when the action waits and the life module ends it. False when
    /// the kernel ends it after a known number of ticks.
    #[serde(rename = "w", default)]
    pub(crate) waits: bool,
}

/// The residents of a zone that walk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Walkers {
    active: BTreeSet<ResidentId>,
}

impl Walkers {
    /// Returns the number of residents that walk in this zone.
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Returns true when no resident walks in this zone.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

impl Resource for Walkers {
    const NAME: &'static str = "life.walkers";
}

/// The buffers of the path search of a zone. They hold no state: a snapshot
/// writes nothing for them.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PathBuffers {
    #[serde(skip)]
    scratch: Scratch,
}

impl Resource for PathBuffers {
    const NAME: &'static str = "life.path_buffers";
}

/// Arguments of `move_to`. Give `x` and `y`, or `site`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MoveToArgs {
    /// Column of the target tile. The tile must be in your city and
    /// walkable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    /// Row of the target tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    /// Id of a site in your city. The walk ends inside the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteId>,
}

/// The path through one zone.
struct Leg {
    steps: Vec<u8>,
    /// Ticks of all steps.
    ticks: u32,
    /// Ticks of the first step.
    first: u32,
    /// True when the leg ends at the target. False when it ends with the
    /// step into the next zone.
    last: bool,
}

/// What one tick did to a walk.
enum Outcome {
    /// The walk goes on.
    Going,
    /// The resident is at the target.
    Arrived(Value),
    /// The walk cannot go on.
    Failed(ActionError),
}

/// Returns the result of a walk that ended at `pos`.
fn arrival(pos: Pos, steps: u32, ticks: u64, site: Option<SiteId>) -> Value {
    let mut out = json!({ "x": pos.x, "y": pos.y, "steps": steps, "ticks": ticks });
    if let Some(site) = site {
        out["site"] = json!(site);
    }
    out
}

/// Returns the ticks since the running action of a resident started.
fn elapsed(realm: &Realm, idx: Idx) -> u64 {
    realm.activity(idx).map_or(0, |activity| realm.tick().since(activity.started))
}

/// The error of a walk whose resident is not where the walk left it:
/// another action or another module moved it.
fn moved(pos: Pos) -> ActionError {
    ActionError::conflict(format!("You were moved during the walk. You are at {pos}."))
}

fn no_route(error: NoRoute, target: Pos, limit: usize) -> ActionError {
    match error {
        NoRoute::Unreachable => {
            ActionError::not_found(format!("There is no way on foot to {target}."))
        }
        NoRoute::TooFar => ActionError::not_found(format!(
            "No way to {target} was found: the search looked at {limit} tiles. Walk to a nearer place first."
        )),
    }
}

/// Returns the ticks of a step onto a tile of this zone, or the cost of the
/// step across the border when the tile is outside.
fn step_ticks(tiles: &TileMap, costs: &Costs, pos: Pos) -> Option<u32> {
    match tiles.get(pos) {
        Some(code) => costs.of(code),
        None => Some(costs.border()),
    }
}

/// What a path search needs to know about the zone.
#[derive(Clone, Copy)]
struct Ground<'a> {
    tiles: &'a TileMap,
    costs: &'a Costs,
    /// The greatest number of tiles that one search looks at.
    limit: usize,
    layout: &'a Layout,
    zone: &'a ZoneInfo,
}

/// Plans the path of a walk through one zone, from `from` towards `target`.
/// The target is in the same city.
fn plan_leg(
    ground: &Ground<'_>,
    scratch: &mut Scratch,
    from: Pos,
    target: Pos,
) -> Result<Leg, ActionError> {
    let Ground { tiles, costs, limit, layout, zone } = *ground;
    let first = |steps: &[u8]| {
        steps
            .first()
            .and_then(|&dir| step_ticks(tiles, costs, path::step(from, dir)))
            .unwrap_or(costs.border())
    };
    if zone.bounds.contains(target) {
        let code = tiles.get(target).unwrap_or(tile::WALL);
        if costs.of(code).is_none() {
            let name = tile::name(code);
            return Err(ActionError::not_allowed(format!(
                "You cannot stand on the tile {target}: it is {name}. Choose a walkable tile."
            ))
            .with_details(json!({ "tile": name })));
        }
        let route = path::find(tiles, costs, from, Goal::Tile(target), limit, scratch)
            .map_err(|error| no_route(error, target, limit))?;
        let first = first(&route.steps);
        return Ok(Leg { steps: route.steps, ticks: route.ticks, first, last: true });
    }
    let Some(there) = layout.zone_at(target).and_then(|id| layout.zone(id)) else {
        return Err(ActionError::invalid_args(format!("The tile {target} is outside the world.")));
    };
    // The edges of this zone that are on the way to the zone of the target.
    let sides =
        [there.row < zone.row, there.col > zone.col, there.row > zone.row, there.col < zone.col];
    let route =
        path::find(tiles, costs, from, Goal::Edge { toward: target, sides }, limit, scratch)
            .map_err(|error| no_route(error, target, limit))?;
    // The step that leaves the zone. At a corner, it goes along the axis on
    // which the target is farther away.
    let bounds = zone.bounds;
    let end = route.end;
    let across = [
        (NORTH, end.y == bounds.y0, end.y.abs_diff(target.y)),
        (EAST, end.x == bounds.x1 - 1, end.x.abs_diff(target.x)),
        (SOUTH, end.y == bounds.y1 - 1, end.y.abs_diff(target.y)),
        (WEST, end.x == bounds.x0, end.x.abs_diff(target.x)),
    ]
    .into_iter()
    .filter(|&(dir, at_edge, _)| at_edge && sides[usize::from(dir)])
    .max_by_key(|&(dir, _, distance)| (distance, std::cmp::Reverse(dir)));
    let Some((dir, _, _)) = across else {
        return Err(no_route(NoRoute::Unreachable, target, limit));
    };
    let mut steps = route.steps;
    steps.push(dir);
    let first = first(&steps);
    Ok(Leg { steps, ticks: route.ticks.saturating_add(costs.border()), first, last: false })
}

/// The greatest number of tiles that one path search looks at.
fn search_limit(realm: &Realm, h: &Handles) -> usize {
    usize::try_from(realm.param(h.p.path_search_max)).unwrap_or(0)
}

/// `begin` of `move_to`: finds the target and the first path.
fn begin(
    realm: &Realm,
    h: &Handles,
    ctx: &ActionCtx,
    args: &MoveToArgs,
) -> Result<Begin, ActionError> {
    let Some(zone) = realm.zone_info() else {
        return Err(ActionError::internal("`move_to` needs a zone."));
    };
    let here = realm.pos(ctx.idx);
    let tick = realm.tick().0;
    let timeout = u32::try_from(realm.param(h.p.walk_timeout)).unwrap_or(1);

    let (target, site) = match (args.x, args.y, args.site) {
        (Some(x), Some(y), None) => (Pos::new(x, y), None),
        (None, None, Some(site)) => {
            let rect = realm.sites().get(site).map(|site| site.rect);
            let Some(rect) = rect else {
                // The site is in another district, if it exists at all. The
                // global realm knows; the walk starts when it answers.
                sites::ask(realm, ctx.resident, ctx.action_id, site);
                realm.col_mut::<Body>()[ctx.idx].walk = Some(Box::new(Walk {
                    action: ctx.action_id,
                    target: here,
                    at: here,
                    site: Some(site),
                    phase: Phase::Lookup,
                    waits: true,
                    ..Walk::default()
                }));
                return Ok(Begin::Wait { timeout });
            };
            let Some(tile) = sites::standing_tile(&realm.tiles(), rect) else {
                return Err(ActionError::not_found(format!(
                    "The site {site} has no tile where you can stand."
                )));
            };
            (tile, Some(site))
        }
        _ => {
            return Err(ActionError::invalid_args("Give `x` and `y`, or `site`."));
        }
    };
    if target == here {
        return Ok(Begin::Done(arrival(here, 0, 0, site)));
    }
    let layout = realm.layout();
    let Some(there) = layout.zone_at(target).and_then(|id| layout.zone(id)) else {
        return Err(ActionError::invalid_args(format!("The tile {target} is outside the world.")));
    };
    if there.city != zone.city {
        return Err(ActionError::not_allowed(format!(
            "The tile {target} is in another city. Go to a station and use `travel`."
        )));
    }

    let costs = Costs::read(realm, &h.p);
    let leg = {
        let tiles = realm.tiles();
        let ground =
            Ground { tiles: &tiles, costs: &costs, limit: search_limit(realm, h), layout, zone };
        let mut buffers = realm.res_mut::<PathBuffers>();
        plan_leg(&ground, &mut buffers.scratch, here, target)?
    };
    let begin = if leg.last { Begin::Busy { ticks: leg.ticks } } else { Begin::Wait { timeout } };
    realm.col_mut::<Body>()[ctx.idx].walk = Some(Box::new(Walk {
        action: ctx.action_id,
        target,
        at: here,
        site,
        path: leg.steps,
        cursor: 0,
        due: tick + u64::from(leg.first),
        steps: 0,
        phase: Phase::Walking,
        waits: !leg.last,
    }));
    realm.res_mut::<Walkers>().active.insert(ctx.resident);
    Ok(begin)
}

/// `finish` of `move_to`: the kernel calls it when a walk inside one zone
/// is over. The system made every step but the last one.
fn finish(realm: &Realm, h: &Handles, ctx: &ActionCtx) -> Result<Value, ActionError> {
    let walk = realm.col_mut::<Body>()[ctx.idx].walk.take();
    realm.res_mut::<Walkers>().active.remove(&ctx.resident);
    let Some(walk) = walk.filter(|walk| walk.action == ctx.action_id) else {
        return Err(ActionError::conflict("The walk was stopped."));
    };
    let costs = Costs::read(realm, &h.p);
    let mut pos = realm.pos(ctx.idx);
    if pos != walk.at {
        return Err(moved(pos));
    }
    let mut steps = walk.steps;
    let mut blocked = None;
    {
        let tiles = realm.tiles();
        for &dir in walk.path.iter().skip(walk.cursor as usize) {
            let next = path::step(pos, dir);
            if tiles.get(next).and_then(|code| costs.of(code)).is_none() {
                blocked = Some(next);
                break;
            }
            pos = next;
            steps += 1;
        }
    }
    realm
        .set_pos(ctx.idx, pos)
        .map_err(|error| ActionError::conflict(format!("You cannot go to {pos}: {error}.")))?;
    if let Some(tile) = blocked {
        return Err(ActionError::conflict(format!("The way is blocked at {tile}.")));
    }
    if pos != walk.target {
        return Err(moved(pos));
    }
    Ok(arrival(pos, steps, realm.tick().since(ctx.started), walk.site))
}

/// Moves one walker forward by at most one step.
fn advance(
    realm: &Realm,
    ground: &Ground<'_>,
    scratch: &mut Scratch,
    idx: Idx,
    walk: &mut Walk,
) -> Outcome {
    let Ground { tiles, costs, .. } = *ground;
    let tick = realm.tick().0;
    let here = realm.pos(idx);
    if here != walk.at {
        return Outcome::Failed(moved(here));
    }
    match walk.phase {
        Phase::Lookup => return Outcome::Going,
        Phase::Plan => {
            if here == walk.target {
                return Outcome::Arrived(arrival(here, walk.steps, elapsed(realm, idx), walk.site));
            }
            match plan_leg(ground, scratch, here, walk.target) {
                Ok(leg) => {
                    walk.path = leg.steps;
                    walk.cursor = 0;
                    walk.due += u64::from(leg.first);
                    walk.phase = Phase::Walking;
                }
                Err(error) => return Outcome::Failed(error),
            }
        }
        Phase::Walking => {}
    }
    if walk.due > tick {
        return Outcome::Going;
    }
    let Some(&dir) = walk.path.get(walk.cursor as usize) else {
        return Outcome::Failed(ActionError::internal("The walk has no next step."));
    };
    let last = walk.cursor as usize + 1 == walk.path.len();
    if last && !walk.waits {
        // The kernel ends a walk inside one zone at this tick or the next,
        // and `finish` makes the last step.
        return Outcome::Going;
    }
    let next = path::step(here, dir);
    if step_ticks(tiles, costs, next).is_none() {
        return Outcome::Failed(ActionError::conflict(format!("The way is blocked at {next}.")));
    }
    if let Err(error) = realm.set_pos(idx, next) {
        return Outcome::Failed(ActionError::conflict(format!(
            "You cannot go to {next}: {error}."
        )));
    }
    walk.at = next;
    walk.cursor += 1;
    walk.steps += 1;
    if let Some(&after) = walk.path.get(walk.cursor as usize) {
        let ticks = step_ticks(tiles, costs, path::step(next, after)).unwrap_or(costs.border());
        walk.due = tick + u64::from(ticks);
        return Outcome::Going;
    }
    if next == walk.target {
        return Outcome::Arrived(arrival(next, walk.steps, elapsed(realm, idx), walk.site));
    }
    // The step left the zone. The walk goes on after the arrival in the
    // next zone.
    walk.path.clear();
    walk.cursor = 0;
    walk.phase = Phase::Plan;
    Outcome::Going
}

/// The system that moves every walker of the zone. It runs every tick and
/// touches only the residents that walk.
fn step_walkers(realm: &Realm, h: &Handles) {
    let ids: Vec<ResidentId> = {
        let walkers = realm.res::<Walkers>();
        if walkers.active.is_empty() {
            return;
        }
        walkers.active.iter().copied().collect()
    };
    let Some(zone) = realm.zone_info() else {
        return;
    };
    let costs = Costs::read(realm, &h.p);
    let mut gone = Vec::new();
    let mut ended = Vec::new();
    {
        let mut bodies = realm.col_mut::<Body>();
        let tiles = realm.tiles();
        let ground = Ground {
            tiles: &tiles,
            costs: &costs,
            limit: search_limit(realm, h),
            layout: realm.layout(),
            zone,
        };
        let mut buffers = realm.res_mut::<PathBuffers>();
        for id in ids {
            let Some(idx) = realm.idx_of(id) else {
                gone.push(id);
                continue;
            };
            let Some(walk) = bodies[idx].walk.as_deref_mut() else {
                gone.push(id);
                continue;
            };
            let (action, waits) = (walk.action, walk.waits);
            let result = match advance(realm, &ground, &mut buffers.scratch, idx, walk) {
                Outcome::Going => continue,
                Outcome::Arrived(value) => Ok(value),
                Outcome::Failed(error) => Err(error),
            };
            bodies[idx].walk = None;
            gone.push(id);
            ended.push((id, idx, action, waits, result));
        }
    }
    if !gone.is_empty() {
        let mut walkers = realm.res_mut::<Walkers>();
        for id in &gone {
            walkers.active.remove(id);
        }
    }
    for (id, idx, action, waits, result) in ended {
        if waits {
            realm.complete_action(id, action, result);
        } else if let Err(error) = result {
            realm.interrupt(idx, &error.message);
        }
    }
}

/// A resident arrived from another zone. A walk goes on from here.
fn arrived(realm: &Realm, idx: Idx) {
    let tick = realm.tick().0;
    let mut bodies = realm.col_mut::<Body>();
    if let Some(walk) = bodies[idx].walk.as_deref_mut()
        && walk.phase != Phase::Lookup
    {
        walk.path.clear();
        walk.cursor = 0;
        walk.phase = Phase::Plan;
        walk.due = tick;
        realm.res_mut::<Walkers>().active.insert(realm.id_of(idx));
    }
}

/// A resident leaves the zone or the world.
fn left(realm: &Realm, idx: Idx) {
    let id = realm.id_of(idx);
    let mut walkers = realm.res_mut::<Walkers>();
    if !walkers.active.is_empty() {
        walkers.active.remove(&id);
    }
}

/// The kernel stopped an action (cancel, interrupt, timeout). A stopped
/// `move_to` leaves the resident where it is.
fn interrupted(realm: &Realm, event: &Event) {
    if event.data["verb"].as_str() != Some("move_to") {
        return;
    }
    let Some(idx) = event.actor.and_then(|id| realm.idx_of(id)) else {
        return;
    };
    let action = event.data["action_id"].as_u64();
    let mut bodies = realm.col_mut::<Body>();
    if bodies[idx].walk.as_ref().is_some_and(|walk| Some(walk.action) == action) {
        bodies[idx].walk = None;
        realm.res_mut::<Walkers>().active.remove(&realm.id_of(idx));
    }
}

/// The global realm answered where a site is. The walk starts, or fails
/// when the site does not exist or is in another city.
pub(crate) fn site_found(realm: &Realm, idx: Idx, _from: RealmId, message: SiteAt) {
    let id = realm.id_of(idx);
    let tick = realm.tick().0;
    let city = realm.zone_info().map(|zone| zone.city);
    let verdict = {
        let mut bodies = realm.col_mut::<Body>();
        let Some(walk) = bodies[idx]
            .walk
            .as_deref_mut()
            .filter(|walk| walk.action == message.action && walk.phase == Phase::Lookup)
        else {
            return;
        };
        match message.place {
            None => Err(ActionError::not_found(format!("No site has the id {}.", message.site))),
            Some(place) if Some(place.city) != city => Err(ActionError::not_allowed(format!(
                "The site {} is in another city. Go to a station and use `travel`.",
                message.site
            ))),
            Some(place) => {
                walk.target = place.tile;
                walk.at = realm.pos(idx);
                walk.phase = Phase::Plan;
                walk.due = tick;
                Ok(())
            }
        }
    };
    match verdict {
        Ok(()) => {
            realm.res_mut::<Walkers>().active.insert(id);
        }
        Err(error) => {
            realm.col_mut::<Body>()[idx].walk = None;
            realm.complete_action(id, message.action, Err(error));
        }
    }
}

/// Registers the verb, the system and the hooks of walking.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.resource::<Walkers>();
        b.resource::<PathBuffers>();
        b.system(Stage::Update, 0, "life.walk", move |realm| step_walkers(realm, &h));
        b.on_arrive(arrived);
        b.on_depart(left);
        b.on_despawn(left);
        b.on_event("kernel.action_interrupted", interrupted);
    }
    b.verb::<MoveToArgs>(
        VerbSpec {
            verb: "move_to",
            module: "life",
            summary: "Walk to a tile or a site of your city, one tile at a time. Roads are fast, trees are slow.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| begin(realm, &h, ctx, args),
        move |realm, ctx, _args| finish(realm, &h, ctx),
    );
}
