//! A second small module, for the tests and the example of the cluster:
//! residents that walk tile by tile across zone borders, shout to those
//! near them, use a bank in the global realm and leave the world.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, Audience, Begin, Builder, CityConfig, Component, Dest, EventMeta, Holder, Message,
    Money, Plugin, Pos, ResidentId, Resource, Scope, Stage, UNCLAIMED, VerbSpec, WorldConfig,
    ZoneId,
};
use unsup_runtime::Plugins;

use super::Town;

/// The data file of the roads module.
const DATA: &str = r#"
[params."roads.walk_timeout"]
value = 100000
min = 1
max = 1000000
policy = false
summary = "Ticks after which a walk that did not arrive fails."

[params."roads.shout_radius"]
value = 6
min = 0
max = 64
policy = false
summary = "Tiles from which a shout is heard."
"#;

/// The account of the bank in the global realm.
pub const BANK: Holder = Holder::Account(7);

/// Per-resident state of the roads module. It moves with the resident
/// from zone to zone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Route {
    /// Where the resident walks to. `None`: it does not walk.
    pub to: Option<Pos>,
    /// The action that the walk belongs to.
    pub action: u64,
    /// Tiles that the resident ever walked.
    pub steps: u64,
    /// Sum of the numbers that other residents sent with `poke`.
    pub pokes: u64,
}

impl Component for Route {
    const NAME: &'static str = "roads.route";
}

/// Global state of the roads module.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vault {
    /// How many times a resident took money out of the bank.
    pub withdrawals: u64,
}

impl Resource for Vault {
    const NAME: &'static str = "roads.vault";
}

/// State of a zone: the numbers that arrived with `signal`, in the order
/// of arrival.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    /// The numbers.
    pub heard: Vec<u64>,
}

impl Resource for Signals {
    const NAME: &'static str = "roads.signals";
}

/// A number that a zone gets from a resident in any zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// The number.
    pub n: u64,
}

impl Message for Signal {
    const KIND: &'static str = "roads.signal";
}

/// A number that one resident sends to another, wherever it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Poke {
    /// The number.
    pub n: u64,
}

impl Message for Poke {
    const KIND: &'static str = "roads.poke";
}

/// Arguments of `walk`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WalkArgs {
    /// Column of the tile to walk to.
    x: i32,
    /// Row of the tile to walk to.
    y: i32,
}

/// Arguments of `deposit` and `withdraw`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct AmountArgs {
    /// Cents.
    amount: i64,
}

/// Arguments of `poke`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct PokeArgs {
    /// The resident that gets the number.
    to: ResidentId,
    /// The number.
    n: u64,
}

/// Arguments of `call`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct CallArgs {
    /// The resident that gets the notice.
    to: ResidentId,
    /// Ticks until the notice is sent.
    ticks: u32,
}

/// Arguments of `signal`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct SignalArgs {
    /// The zone that gets the number.
    zone: ZoneId,
    /// The number.
    n: u64,
    /// Ticks until the number is sent.
    ticks: u32,
}

/// Arguments of `leave`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct LeaveArgs {
    /// The resident that gets the money of the one that leaves.
    heir: Option<ResidentId>,
}

/// Arguments of verbs that take none.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Nothing {}

fn verb(verb: &'static str, summary: &'static str, free: bool) -> VerbSpec {
    VerbSpec { verb, module: "roads", summary, free, allowed_in: &[] }
}

/// The roads module. `walk` moves a resident one tile per tick to a tile
/// anywhere in the world. `shout` is heard by every resident near, also
/// across a zone border. `deposit` wires money to the bank in the global
/// realm; `withdraw` is a verb of the global realm that wires it back and
/// sends a receipt. `poke` sends a module message to a resident in any
/// zone. `call` writes a notice to a resident in any zone, and `signal`
/// sends a module message to a zone; both do it in a tick, after some
/// time. `leave` takes the resident out of the world.
pub struct Roads;

impl Plugin for Roads {
    fn name(&self) -> &'static str {
        "roads"
    }

    fn build(&self, b: &mut Builder) {
        b.load_data(DATA);
        let walk_timeout = b.param_id("roads.walk_timeout");
        let shout_radius = b.param_id("roads.shout_radius");
        b.component::<Route>();
        match b.scope() {
            Scope::Zone => {
                b.system(Stage::Update, 10, "roads.walk", |realm| {
                    let mut moves = Vec::new();
                    {
                        let routes = realm.col::<Route>();
                        for idx in realm.indices() {
                            if let Some(to) = routes[idx].to {
                                let pos = realm.pos(idx);
                                let next =
                                    pos.offset((to.x - pos.x).signum(), (to.y - pos.y).signum());
                                moves.push((idx, next, next == to));
                            }
                        }
                    }
                    for (idx, next, arrived) in moves {
                        // The target is in the world, so every tile on the
                        // way is in it too.
                        if realm.set_pos(idx, next).is_err() {
                            continue;
                        }
                        let action = {
                            let mut routes = realm.col_mut::<Route>();
                            let route = &mut routes[idx];
                            route.steps += 1;
                            if arrived {
                                route.to = None;
                            }
                            route.action
                        };
                        if arrived {
                            realm.complete_action(
                                realm.id_of(idx),
                                action,
                                Ok(json!({ "pos": next })),
                            );
                        }
                    }
                });
                // A walk that was cancelled or that ran out of time stops.
                b.on_event("kernel.action_interrupted", |realm, event| {
                    if event.data["verb"] != "walk" {
                        return;
                    }
                    let Some(idx) = event.actor.and_then(|actor| realm.idx_of(actor)) else {
                        return;
                    };
                    let mut routes = realm.col_mut::<Route>();
                    if Some(routes[idx].action) == event.data["action_id"].as_u64() {
                        routes[idx].to = None;
                    }
                });
                b.on_resident_message::<Poke>(|realm, idx, _from, poke| {
                    realm.col_mut::<Route>()[idx].pokes += poke.n;
                });
                b.resource::<Signals>();
                b.on_message::<Signal>(|realm, _from, signal| {
                    realm.res_mut::<Signals>().heard.push(signal.n);
                });
            }
            Scope::Global => {
                b.resource::<Vault>();
            }
        }
        b.verb::<WalkArgs>(
            verb("walk", "Walk to a tile, one tile per tick.", false),
            move |realm, ctx, args| {
                let to = Pos::new(args.x, args.y);
                if realm.layout().zone_at(to).is_none() {
                    return Err(ActionError::not_found("No zone has this tile."));
                }
                if realm.pos(ctx.idx) == to {
                    return Ok(Begin::Done(json!({ "pos": to })));
                }
                let mut routes = realm.col_mut::<Route>();
                routes[ctx.idx].to = Some(to);
                routes[ctx.idx].action = ctx.action_id;
                let timeout = u32::try_from(realm.param(walk_timeout)).unwrap_or(u32::MAX);
                Ok(Begin::Wait { timeout })
            },
            |_, _, _| Ok(Value::Null),
        );
        b.instant_verb::<Nothing>(
            verb("shout", "Shout. Every resident near hears it.", true),
            move |realm, ctx, _| {
                let radius = u32::try_from(realm.param(shout_radius)).unwrap_or(0);
                realm.emit(
                    EventMeta::public("roads.shout", 1)
                        .actor(ctx.resident)
                        .audience(Audience::Nearby(radius)),
                    &json!({ "who": ctx.resident }),
                );
                Ok(Value::Null)
            },
        );
        b.instant_verb::<AmountArgs>(
            verb("deposit", "Put money in the bank.", false),
            |realm, ctx, args| {
                realm.wire_money(
                    Holder::Resident(ctx.resident),
                    Dest::Global,
                    BANK,
                    Money(args.amount),
                    "deposit",
                )?;
                Ok(json!({ "deposited": args.amount }))
            },
        );
        b.global_verb::<AmountArgs>(
            verb("withdraw", "Take money out of the bank.", false),
            |realm, ctx, args| {
                realm.wire_money(
                    BANK,
                    Dest::Resident(ctx.resident),
                    Holder::Resident(ctx.resident),
                    Money(args.amount),
                    "withdraw",
                )?;
                realm.res_mut::<Vault>().withdrawals += 1;
                realm.notify(ctx.resident, "roads.receipt", None, json!({ "amount": args.amount }));
                Ok(json!({ "withdrawn": args.amount }))
            },
        );
        b.instant_verb::<PokeArgs>(
            verb("poke", "Send a number to a resident in any zone.", true),
            |realm, _, args| {
                match realm.idx_of(args.to) {
                    // A resident of this zone gets the number at once.
                    Some(idx) => realm.col_mut::<Route>()[idx].pokes += args.n,
                    None => realm.send(Dest::Resident(args.to), &Poke { n: args.n }),
                }
                Ok(Value::Null)
            },
        );
        b.verb::<CallArgs>(
            verb("call", "Call a resident in any zone. The call takes some ticks.", false),
            |_, _, args| Ok(Begin::Busy { ticks: args.ticks }),
            |realm, ctx, args| {
                // The notice is sent in a tick, so the router of the frame
                // delivers it, with what the other realms sent in the frame.
                realm.notify(args.to, "roads.call", Some(ctx.resident), Value::Null);
                Ok(Value::Null)
            },
        );
        b.verb::<SignalArgs>(
            verb("signal", "Send a number to a zone after some ticks.", false),
            |realm, _, args| {
                if realm.layout().zone(args.zone).is_none() {
                    return Err(ActionError::not_found("No zone has this number."));
                }
                Ok(Begin::Busy { ticks: args.ticks })
            },
            |realm, _, args| {
                // The message is sent in a tick, so the router of the frame
                // delivers it.
                realm.send(Dest::Zone(args.zone), &Signal { n: args.n });
                Ok(Value::Null)
            },
        );
        b.instant_verb::<LeaveArgs>(
            verb("leave", "Leave the world for ever.", false),
            |realm, ctx, args| {
                let heir = args.heir.map_or(UNCLAIMED, Holder::Resident);
                realm.despawn(ctx.idx, heir, "left");
                Ok(Value::Null)
            },
        );
    }
}

/// Returns the town module and the roads module as the plugin list of a
/// world.
pub fn plugins() -> Plugins {
    Arc::new(vec![Box::new(Town::default()), Box::new(Roads)])
}

/// Returns a world of one city with `districts` zones per side and
/// `zone_size` tiles per zone side. Small zones make short walks cross
/// borders.
pub fn world_config(seed: u64, districts: u32, zone_size: u32) -> WorldConfig {
    WorldConfig {
        seed,
        cities: vec![CityConfig::new("Crossing", Pos::new(0, 0), districts)],
        zone_size,
        ..WorldConfig::default()
    }
}
