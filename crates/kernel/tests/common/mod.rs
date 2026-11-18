//! A small demo module that the kernel tests drive. It uses every kind of
//! thing that a real module registers.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    ActionError, AttrDef, AttrShow, Audience, Begin, Builder, Component, CondDef, Dest, EventMeta,
    Holder, Message, Money, NoArgs, Plugin, Pos, Rect, ResidentId, Resource, Scope, Site, SiteId,
    Stage, Untrusted, VerbSpec, ZoneId,
};

/// The data file of the demo module, in the documented format.
pub const DATA: &str = r#"
[params."demo.step_ticks"]
value = 1
min = 1
max = 60
policy = false
summary = "Ticks that one step takes."

[params."demo.work_pay"]
value = 500
min = 0
max = 100000
policy = true
summary = "Cents that one `work` action pays."

[params."demo.decay"]
value = 1
min = 0
max = 10
policy = false
summary = "Energy that a resident loses each tick."

[[items]]
name = "bread"
weight = 1
tags = ["food"]
attrs = { nutrition = 30 }
summary = "A loaf of bread."

[[items]]
name = "coal"
weight = 5
tags = ["fuel"]
summary = "A lump of coal."
"#;

/// The account in the global realm where `deposit` puts money.
pub const BANK: Holder = Holder::Account(77);

/// The site where new residents appear.
pub const PLAZA: Rect = Rect::new(28, 28, 37, 37);

/// Per-resident state of the demo module.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Walker {
    pub steps: u32,
    pub arrivals: u32,
    pub departures: u32,
    pub spawned: bool,
}

impl Component for Walker {
    const NAME: &'static str = "demo.walker";
}

/// Per-zone state of the demo module.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ZoneLog {
    pub worked: u32,
    pub hello_back: Vec<u32>,
    pub despawned: Vec<ResidentId>,
    pub interrupted: Vec<String>,
    pub conditions_ended: Vec<(ResidentId, String, bool)>,
    pub resident_messages: Vec<(ResidentId, u32)>,
}

impl Resource for ZoneLog {
    const NAME: &'static str = "demo.zone_log";
}

/// Global state of the demo module.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalLog {
    pub pings: u32,
    pub hellos: Vec<(ZoneId, u32)>,
    pub events: Vec<(String, u64)>,
    pub ticks: u64,
}

impl Resource for GlobalLog {
    const NAME: &'static str = "demo.global_log";
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub n: u32,
}

impl Message for Hello {
    const KIND: &'static str = "demo.hello";
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloBack {
    pub n: u32,
}

impl Message for HelloBack {
    const KIND: &'static str = "demo.hello_back";
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Poke {
    pub n: u32,
}

impl Message for Poke {
    const KIND: &'static str = "demo.poke";
}

/// Arguments of `step`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StepArgs {
    /// Columns to move: -1, 0 or 1.
    pub dx: i32,
    /// Rows to move: -1, 0 or 1.
    pub dy: i32,
}

/// Arguments of `pay`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PayArgs {
    /// The resident that gets the money.
    pub to: ResidentId,
    /// Cents to pay.
    pub amount: i64,
}

/// Arguments of verbs that take a duration.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicksArgs {
    /// Duration in ticks.
    pub ticks: u32,
}

/// Arguments of verbs that carry text.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TextArgs {
    /// The text.
    pub text: String,
}

/// Arguments of `whisper`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WhisperArgs {
    /// The residents that hear it.
    pub to: Vec<ResidentId>,
    /// The text.
    pub text: String,
}

/// Arguments of `ping`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PingArgs {
    /// A number that comes back.
    pub n: u32,
}

/// Arguments of `accept`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AcceptArgs {
    /// The resident whose offer is accepted.
    pub resident: ResidentId,
    /// The action id of the offer.
    pub action_id: u64,
}

/// Arguments of verbs that take an amount of money.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AmountArgs {
    /// Cents.
    pub amount: i64,
}

/// Arguments of `jump`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct JumpArgs {
    /// The zone to go to.
    pub zone: ZoneId,
    /// Column in that zone.
    pub x: i32,
    /// Row in that zone.
    pub y: i32,
}

/// Arguments of `leave`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct LeaveArgs {
    /// The resident that inherits. The unclaimed account when absent.
    pub heir: Option<ResidentId>,
}

fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "demo", summary, free: false, allowed_in: &[] }
}

fn free(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "demo", summary, free: true, allowed_in: &[] }
}

/// The demo module.
pub struct Demo;

impl Plugin for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn build(&self, b: &mut Builder) {
        b.load_data(DATA);
        let step_ticks = b.param_id("demo.step_ticks");
        let work_pay = b.param_id("demo.work_pay");
        let decay = b.param_id("demo.decay");
        let bread = b.item_id("bread");
        let energy =
            b.attr(AttrDef::new("energy").range(0, 100).default(50).summary("How rested."));
        b.attr(AttrDef::new("skill").range(0, 1_000).show(AttrShow::Full).summary("How skilled."));
        b.attr(AttrDef::new("secret").show(AttrShow::Hidden));
        let jailed = b.condition(CondDef::new("jailed").blocking().summary("In jail."));
        b.condition(CondDef::new("tired").summary("Tired."));
        let wages = b.faucet("demo.wages");
        let grant = b.faucet("demo.start");
        let eaten = b.sink("demo.eaten");
        let tax = b.sink("demo.tax");

        b.component::<Walker>();

        match b.scope() {
            Scope::Zone => {
                b.resource::<ZoneLog>();
                b.on_worldgen(0, |realm, gen_| {
                    let info = realm.zone_info().expect("zone hooks run in zones");
                    assert!(gen_.layout.zone(info.id).is_some());
                    realm.tiles_mut().fill(info.bounds, 1);
                    if info.id == ZoneId(0) {
                        let id = SiteId(realm.alloc_id());
                        realm.sites_mut().insert(
                            Site::new(id, "plaza", "Plaza", PLAZA).with_tag(Site::SPAWN_TAG),
                        );
                        realm.tiles_mut().fill(PLAZA, 2);
                    }
                    // Tell the global realm that the zone exists.
                    realm.send(Dest::Global, &Hello { n: info.id.0 });
                });
                b.system(Stage::Update, 0, "demo.decay", move |realm| {
                    let d = realm.param(decay);
                    for value in realm.attrs_mut(energy).iter_mut() {
                        *value = (*value - d).max(0);
                    }
                });
                b.every(10, Stage::Post, 0, "demo.skill", move |realm| {
                    let skill = realm.defs().attr_id("skill").expect("registered above");
                    for idx in realm.indices() {
                        if realm.activity(idx).is_some_and(|a| a.verb == "work") {
                            realm.attr_add(idx, skill, 1);
                        }
                    }
                });
                b.on_message::<HelloBack>(|realm, _from, msg| {
                    realm.res_mut::<ZoneLog>().hello_back.push(msg.n);
                });
                b.on_resident_message::<Poke>(|realm, idx, _from, msg| {
                    let id = realm.id_of(idx);
                    realm.res_mut::<ZoneLog>().resident_messages.push((id, msg.n));
                });
                b.on_event("demo.worked", |realm, _event| {
                    realm.res_mut::<ZoneLog>().worked += 1;
                });
                b.on_event("kernel.despawned", |realm, event| {
                    if let Some(actor) = event.actor {
                        realm.res_mut::<ZoneLog>().despawned.push(actor);
                    }
                });
                b.on_event("kernel.action_interrupted", |realm, event| {
                    let text = format!(
                        "{}:{}",
                        event.data["verb"].as_str().unwrap_or("?"),
                        event.data["reason"].as_str().unwrap_or("?")
                    );
                    realm.res_mut::<ZoneLog>().interrupted.push(text);
                });
                b.on_event("kernel.condition_ended", |realm, event| {
                    let actor = event.actor.expect("the event names the resident");
                    let name = event.data["condition"].as_str().unwrap_or("?").to_string();
                    let expired = event.data["expired"].as_bool().unwrap_or(false);
                    realm.res_mut::<ZoneLog>().conditions_ended.push((actor, name, expired));
                });
                b.on_spawn(0, move |realm, idx| {
                    let id = realm.id_of(idx);
                    realm.ledger_mut().mint(grant, Holder::Resident(id), Money(1_000));
                    realm.col_mut::<Walker>()[idx].spawned = true;
                });
                b.on_arrive(|realm, idx| realm.col_mut::<Walker>()[idx].arrivals += 1);
                b.on_depart(|realm, idx| realm.col_mut::<Walker>()[idx].departures += 1);
                b.observe("demo", 0, |realm, idx, opts| {
                    let walker = realm.col::<Walker>()[idx].clone();
                    let mut out = json!({ "steps": walker.steps });
                    if opts.is_full() {
                        out["arrivals"] = json!(walker.arrivals);
                    }
                    Some(out)
                });
            }
            Scope::Global => {
                b.resource::<GlobalLog>();
                b.system(Stage::Update, 0, "demo.count", |realm| {
                    realm.res_mut::<GlobalLog>().ticks += 1
                });
                b.on_message::<Hello>(|realm, from, msg| {
                    let zone = from.zone().expect("zones send hello");
                    realm.res_mut::<GlobalLog>().hellos.push((zone, msg.n));
                    realm.send(Dest::Zone(zone), &HelloBack { n: msg.n + 100 });
                });
                b.on_event("demo.", |realm, event| {
                    realm.res_mut::<GlobalLog>().events.push((event.kind.clone(), event.frame));
                });
                b.observe_global("bank", 0, |realm, resident, _opts| {
                    let pings = realm.res::<GlobalLog>().pings;
                    let known = realm.directory().get(resident).map(|entry| entry.name.clone());
                    Some(json!({ "deposits": realm.ledger().money(BANK), "pings": pings, "name": known }))
                });
            }
        }

        b.verb::<StepArgs>(
            spec("step", "Walk one tile."),
            move |realm, _ctx, args| {
                if args.dx.abs() > 1 || args.dy.abs() > 1 {
                    return Err(ActionError::invalid_args("A step is at most one tile."));
                }
                Ok(Begin::Busy { ticks: realm.param(step_ticks) as u32 })
            },
            |realm, ctx, args| {
                let to = realm.pos(ctx.idx).offset(args.dx, args.dy);
                realm
                    .set_pos(ctx.idx, to)
                    .map_err(|e| ActionError::not_here(format!("You cannot go there: {e}.")))?;
                realm.col_mut::<Walker>()[ctx.idx].steps += 1;
                Ok(json!({ "pos": to }))
            },
        );
        b.instant_verb::<PayArgs>(
            spec("pay", "Give money to another resident."),
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
                realm.notify(
                    args.to,
                    "demo.paid",
                    Some(ctx.resident),
                    json!({ "amount": args.amount }),
                );
                Ok(json!({ "paid": args.amount }))
            },
        );
        b.verb::<TicksArgs>(
            spec("work", "Work for pay."),
            |_realm, _ctx, args| Ok(Begin::Busy { ticks: args.ticks }),
            move |realm, ctx, _args| {
                let pay = Money(realm.param(work_pay));
                realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), pay);
                realm.emit(
                    EventMeta::public("demo.worked", 5).actor(ctx.resident).at(realm.pos(ctx.idx)),
                    &json!({ "pay": pay }),
                );
                Ok(json!({ "pay": pay }))
            },
        );
        b.verb::<TicksArgs>(
            VerbSpec {
                allowed_in: &["jailed", "not_a_condition"],
                ..spec("nap", "Sleep a little.")
            },
            |_realm, _ctx, args| Ok(Begin::Busy { ticks: args.ticks }),
            move |realm, ctx, _args| Ok(json!({ "energy": realm.attr_add(ctx.idx, energy, 20) })),
        );
        b.instant_verb::<NoArgs>(spec("eat", "Eat a loaf of bread."), move |realm, ctx, _args| {
            realm.ledger_mut().consume(Holder::Resident(ctx.resident), eaten, bread, 1)?;
            Ok(json!({ "energy": realm.attr_add(ctx.idx, energy, 30) }))
        });
        b.instant_verb::<AmountArgs>(spec("pay_tax", "Burn money."), move |realm, ctx, args| {
            realm.ledger_mut().burn(Holder::Resident(ctx.resident), tax, Money(args.amount))?;
            Ok(Value::Null)
        });
        b.instant_verb::<TextArgs>(
            free("shout", "Say something that everyone within 5 tiles hears."),
            |realm, ctx, args| {
                realm.emit(
                    EventMeta::public("demo.shout", 1)
                        .actor(ctx.resident)
                        .audience(Audience::Nearby(5)),
                    &Untrusted::new(ctx.resident, args.text.clone()),
                );
                Ok(Value::Null)
            },
        );
        b.instant_verb::<WhisperArgs>(
            free("whisper", "Say something to chosen residents."),
            |realm, ctx, args| {
                realm.emit(
                    EventMeta::silent("demo.whisper")
                        .actor(ctx.resident)
                        .audience(Audience::Private(args.to.clone())),
                    &Untrusted::new(ctx.resident, args.text.clone()),
                );
                Ok(Value::Null)
            },
        );
        b.verb::<TicksArgs>(
            spec("offer", "Offer something and wait for someone to accept."),
            |_realm, _ctx, args| Ok(Begin::Wait { timeout: args.ticks }),
            |_realm, _ctx, _args| Ok(Value::Null),
        );
        b.instant_verb::<AcceptArgs>(
            spec("accept", "Accept the offer of another resident."),
            |realm, ctx, args| {
                realm.complete_action(
                    args.resident,
                    args.action_id,
                    Ok(json!({ "accepted_by": ctx.resident })),
                );
                Ok(Value::Null)
            },
        );
        b.verb::<NoArgs>(
            free("bad_free", "A free verb that wrongly takes time."),
            |_realm, _ctx, _args| Ok(Begin::Busy { ticks: 3 }),
            |_realm, _ctx, _args| Ok(Value::Null),
        );
        b.instant_verb::<JumpArgs>(spec("jump", "Travel to a zone."), |realm, ctx, args| {
            realm
                .teleport(ctx.idx, args.zone, Pos::new(args.x, args.y))
                .map_err(|e| ActionError::not_found(format!("You cannot travel there: {e}.")))?;
            Ok(Value::Null)
        });
        b.instant_verb::<LeaveArgs>(
            spec("leave", "Leave the world for ever."),
            |realm, ctx, args| {
                let heir = args.heir.map_or(unsup_kernel::UNCLAIMED, Holder::Resident);
                realm.despawn(ctx.idx, heir, "left");
                Ok(Value::Null)
            },
        );
        b.instant_verb::<AmountArgs>(
            spec("deposit", "Put money in the bank."),
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
        b.instant_verb::<PayArgs>(
            free("poke", "Send a number to another resident, wherever it is."),
            |realm, _ctx, args| {
                realm.send(Dest::Resident(args.to), &Poke { n: args.amount as u32 });
                Ok(Value::Null)
            },
        );
        b.instant_verb::<NoArgs>(
            spec("jail_me", "Go to jail for 5 ticks."),
            move |realm, ctx, _args| {
                realm.set_condition(ctx.idx, jailed, Some(realm.tick() + 5));
                Ok(Value::Null)
            },
        );

        b.global_verb::<PingArgs>(
            spec("ping", "Ask the global realm for an answer."),
            |realm, ctx, args| {
                if args.n == 13 {
                    return Err(ActionError::conflict("Thirteen is unlucky."));
                }
                let mut log = realm.res_mut::<GlobalLog>();
                log.pings += 1;
                Ok(json!({ "pong": args.n, "count": log.pings, "zone": ctx.zone }))
            },
        );
        b.global_verb::<PingArgs>(
            free("note", "Leave a note in the global realm."),
            |realm, _ctx, args| {
                realm.res_mut::<GlobalLog>().pings += args.n;
                Ok(json!({ "noted": args.n }))
            },
        );
        b.global_verb::<AmountArgs>(
            spec("set_pay", "Change what `work` pays, everywhere."),
            move |realm, _ctx, args| {
                Ok(json!({ "pay": realm.set_param(work_pay, args.amount, None) }))
            },
        );
        b.global_verb::<AmountArgs>(
            spec("withdraw", "Take money out of the bank."),
            |realm, ctx, args| {
                realm.wire_money(
                    BANK,
                    Dest::Resident(ctx.resident),
                    Holder::Resident(ctx.resident),
                    Money(args.amount),
                    "withdraw",
                )?;
                Ok(json!({ "withdrawn": args.amount }))
            },
        );
    }
}

/// Makes a test world with the demo module.
pub fn world() -> TestWorld<'static> {
    TestWorld::new(&[&Demo])
}

/// Returns the per-resident state of the demo module.
pub fn walker(world: &TestWorld<'_>, r: ResidentId) -> Walker {
    world.with_resident(r, |realm, idx| realm.col::<Walker>()[idx].clone())
}

/// Sends `step` requests until the resident has walked `n` tiles in one
/// direction, one frame per tile.
pub fn walk(world: &mut TestWorld<'_>, r: ResidentId, dx: i32, dy: i32, n: u32) {
    for _ in 0..n {
        world.act_ok(r, "step", json!({ "dx": dx, "dy": dy }));
        world.step(1);
    }
}
