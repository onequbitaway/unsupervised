//! A small town module and helpers that the tests and the examples of the
//! runtime share.

#![allow(dead_code)]

pub mod cluster;
pub mod roads;
pub mod script;

use std::hint::black_box;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, AttrDef, Begin, Builder, Component, Dest, Holder, Message, Money, Plugin, Pos,
    RealmId, ResidentId, Resource, Scope, Stage, VerbSpec, World, WorldConfig, ZoneId,
};
use unsup_protocol::{ActionRequest, AdminCommand, CityId, OwnerId, SimError, SpawnSpec};
use unsup_runtime::{Plugins, SimHandle};

/// The data file of the town module.
const DATA: &str = r#"
[params."town.heavy_zone"]
value = 0
min = 0
max = 10000
policy = false
summary = "Number of the zone whose tick is slow, plus one. 0: no zone is slow."

[params."town.heavy_rounds"]
value = 0
min = 0
max = 1000000000
policy = false
summary = "Rounds of busy work in each tick of the slow zone."
"#;

/// Per-resident state of the town module.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    /// How many times the resident chopped wood.
    pub chops: u64,
}

impl Component for Tally {
    const NAME: &'static str = "town.tally";
}

/// Global state of the town module.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Census {
    /// Ticks of the global realm.
    pub ticks: u64,
}

impl Resource for Census {
    const NAME: &'static str = "town.census";
}

/// A message that comes from outside the world: the town hall orders a
/// bonus for one resident.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decree {
    /// Title of the decree.
    pub title: String,
    /// The resident that gets the bonus and a notice.
    pub to: ResidentId,
    /// Cents of the bonus.
    pub bonus: i64,
}

impl Message for Decree {
    const KIND: &'static str = "town.decree";
}

/// One decree that the global realm received.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Posted {
    /// The frame in which the handler ran.
    pub frame: u64,
    /// The realm that the envelope names as its sender.
    pub from: RealmId,
    /// Title of the decree.
    pub title: String,
}

/// Global state of the town module: the decrees, in the order of arrival.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Board {
    /// The decrees.
    pub posted: Vec<Posted>,
}

impl Resource for Board {
    const NAME: &'static str = "town.board";
}

/// Arguments of `work`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WorkArgs {
    /// Duration in ticks.
    ticks: u32,
}

/// Arguments of `tell`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct TellArgs {
    /// The resident that gets the notice.
    to: ResidentId,
    /// The kind of the notice.
    kind: String,
}

/// Arguments of `travel`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct TravelArgs {
    /// The zone to go to.
    zone: ZoneId,
}

/// Arguments of `pay`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct PayArgs {
    /// The resident that gets the money.
    to: ResidentId,
    /// Cents to pay.
    amount: i64,
}

/// Arguments of verbs that take none.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Nothing {}

fn verb(verb: &'static str, summary: &'static str, free: bool) -> VerbSpec {
    VerbSpec { verb, module: "town", summary, free, allowed_in: &[] }
}

/// A small module. `work` pays after some ticks. `chop` pays one cent at
/// once. `tell` writes a notice to a resident in any zone. `travel` moves
/// the resident to another zone. `pay` sends money to a resident in any
/// zone. `crash` panics. The global realm takes the message [`Decree`]:
/// it writes the decree on the [`Board`], pays the bonus and tells the
/// resident.
pub struct Town {
    /// Cents that one `work` action pays.
    pub wage: i64,
}

impl Default for Town {
    fn default() -> Town {
        Town { wage: 500 }
    }
}

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        b.load_data(DATA);
        let heavy_zone = b.param_id("town.heavy_zone");
        let heavy_rounds = b.param_id("town.heavy_rounds");
        let wages = b.faucet("town.wages");
        let start = b.faucet("town.start");
        let bonuses = b.faucet("town.bonuses");
        let energy = b.attr(AttrDef::new("energy").range(0, 1_000).default(1_000));
        let wage = self.wage;
        b.component::<Tally>();
        match b.scope() {
            Scope::Zone => {
                b.on_spawn(0, move |realm, idx| {
                    let id = realm.id_of(idx);
                    realm.ledger_mut().mint(start, Holder::Resident(id), Money(100));
                });
                b.system(Stage::Update, 0, "town.tire", move |realm| {
                    for value in realm.attrs_mut(energy).iter_mut() {
                        *value = (*value - 1).max(0);
                    }
                });
                // Busy work that changes nothing: it makes one zone slow,
                // for the tests of time dilation.
                b.system(Stage::Update, 1, "town.burn", move |realm| {
                    let zone = realm.zone_info().map_or(0, |info| i64::from(info.id.0) + 1);
                    if realm.param(heavy_zone) != zone {
                        return;
                    }
                    let mut x = 0x9e37_79b9_7f4a_7c15_u64;
                    for _ in 0..realm.param(heavy_rounds) {
                        x = black_box(x ^ (x << 13) ^ (x >> 7)).wrapping_mul(0x2545_f491_4f6c_dd1d);
                    }
                    black_box(x);
                });
            }
            Scope::Global => {
                b.resource::<Census>();
                b.system(Stage::Update, 0, "town.census", |realm| {
                    realm.res_mut::<Census>().ticks += 1;
                });
                b.resource::<Board>();
                b.on_message::<Decree>(move |realm, from, decree| {
                    realm.res_mut::<Board>().posted.push(Posted {
                        frame: realm.frame(),
                        from,
                        title: decree.title.clone(),
                    });
                    // The bonus is made here and wired to the resident, so
                    // that the message moves value across realms.
                    let bank = Holder::Account(1);
                    let to = Holder::Resident(decree.to);
                    realm.ledger_mut().mint(bonuses, bank, Money(decree.bonus));
                    let wired = realm.wire_money(
                        bank,
                        Dest::Resident(decree.to),
                        to,
                        Money(decree.bonus),
                        "bonus",
                    );
                    let data = json!({ "title": decree.title, "paid": wired.is_ok() });
                    realm.notify(decree.to, "town.decree", None, data);
                });
            }
        }
        b.verb::<WorkArgs>(
            verb("work", "Work for pay.", false),
            |_, _, args| Ok(Begin::Busy { ticks: args.ticks }),
            move |realm, ctx, _| {
                realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), Money(wage));
                Ok(json!({ "pay": wage }))
            },
        );
        b.instant_verb::<Nothing>(
            verb("chop", "Chop wood for one cent.", false),
            move |realm, ctx, _| {
                realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), Money(1));
                let mut tally = realm.col_mut::<Tally>();
                tally[ctx.idx].chops += 1;
                Ok(json!({ "chops": tally[ctx.idx].chops }))
            },
        );
        b.instant_verb::<TellArgs>(
            verb("tell", "Tell a resident something.", true),
            |realm, ctx, args| {
                realm.notify(args.to, &args.kind, Some(ctx.resident), json!({ "hello": true }));
                Ok(Value::Null)
            },
        );
        b.instant_verb::<TravelArgs>(verb("travel", "Go to a zone.", false), |realm, ctx, args| {
            let bounds = realm
                .layout()
                .zone(args.zone)
                .map(|zone| zone.bounds)
                .ok_or_else(|| ActionError::not_found("No zone has this number."))?;
            // Each resident has its own place in a zone, so that a
            // crowd does not stand on one tile.
            let spread = ctx.resident.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 20;
            let (width, height) = (u64::from(bounds.width()), u64::from(bounds.height()));
            let place = Pos::new(
                bounds.x0 + (spread % width) as i32,
                bounds.y0 + (spread / width % height) as i32,
            );
            realm
                .teleport(ctx.idx, args.zone, place)
                .map_err(|e| ActionError::not_found(format!("You cannot go there: {e}.")))?;
            Ok(json!({ "pos": place }))
        });
        b.instant_verb::<PayArgs>(
            verb("pay", "Give money to a resident.", false),
            |realm, ctx, args| {
                let from = Holder::Resident(ctx.resident);
                let to = Holder::Resident(args.to);
                if realm.idx_of(args.to).is_some() {
                    realm.ledger_mut().transfer_money(from, to, Money(args.amount), "gift")?;
                } else {
                    realm.wire_money(
                        from,
                        Dest::Resident(args.to),
                        to,
                        Money(args.amount),
                        "gift",
                    )?;
                }
                Ok(json!({ "paid": args.amount }))
            },
        );
        b.instant_verb::<Nothing>(verb("crash", "A bug.", true), |_, _, _| {
            panic!("the town module has a bug")
        });
    }
}

/// Returns the town module as the plugin list of a world.
pub fn plugins() -> Plugins {
    Arc::new(vec![Box::new(Town::default())])
}

/// Makes a new world from a plugin list: one city of four zones.
pub fn new_world(plugins: &[&dyn Plugin]) -> World {
    World::new(plugins, WorldConfig::with_seed(3))
}

/// Makes a new world with the town module.
pub fn world() -> World {
    new_world(&[&Town::default()])
}

/// Returns the request to create a resident.
pub fn spec(name: &str) -> SpawnSpec {
    SpawnSpec { owner: OwnerId(1), name: name.to_string(), model: None, city: Some(CityId(0)) }
}

/// Returns an action request.
pub fn act(verb: &str, args: Value) -> ActionRequest {
    ActionRequest::new(verb, args)
}

/// Number of residents that the script creates.
pub const SCRIPT_RESIDENTS: u64 = 6;

/// Sends the inputs of round `n` of a fixed script, without the frame at
/// its end. The same rounds give the same inputs in every run: the first
/// rounds create residents, and every round makes each resident work, chop
/// wood, tell, travel to another zone or pay, in a pattern that depends
/// only on `n`. Some requests are refused by the world; a refused request
/// is an input like any other.
pub async fn play_inputs(handle: &SimHandle, n: u64) {
    if n < SCRIPT_RESIDENTS {
        handle.spawn(spec(&format!("R{n}"))).await.expect("spawn");
    }
    let residents = (n + 1).min(SCRIPT_RESIDENTS);
    for r in 1..=residents {
        let other = ResidentId(r % residents + 1);
        let request = match (n + r) % 5 {
            0 => act("work", json!({ "ticks": 1 + n % 3 })),
            1 => act("chop", json!({})),
            2 => act("tell", json!({ "to": other, "kind": "town.greeting" })),
            3 => act("travel", json!({ "zone": (n + r) % 4 })),
            _ => act("pay", json!({ "to": other, "amount": 7 })),
        };
        // The answer can be a refusal of the world: the resident is busy
        // and its queue is full, or it does not exist after a rollback.
        // That is part of the script. A runtime that does not take the
        // call is not.
        match handle.act(ResidentId(r), request).await {
            Ok(_) | Err(SimError::UnknownResident) => {}
            Err(error) => panic!("the runtime did not take the act: {error}"),
        }
    }
    if n % 4 == 3 {
        let command = AdminCommand::SetParam {
            name: "kernel.queue_max".into(),
            value: 4 + (n % 3) as i64,
            city: None,
        };
        handle.admin(command).await.expect("admin");
    }
}

/// Plays the rounds `from..to` of the script, each with its frame.
pub async fn play(handle: &SimHandle, from: u64, to: u64) {
    for n in from..to {
        play_inputs(handle, n).await;
        handle.step(1).await.expect("step");
    }
}

/// Returns the state hash of a world as text.
pub fn hash_of(world: &World) -> String {
    hex(&world.state_hash())
}

/// Writes bytes as lower-case hexadecimal text.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
