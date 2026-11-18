//! A second small module for the tests of moderation and of the abuse
//! detectors: residents get money, give it away, buy, and speak in
//! public.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_gateway::GatewayConfig;
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionCtx, ActionError, Builder, EventMeta, Holder, Money, Plugin, Realm, ResidentId, VerbSpec,
    World, WorldConfig,
};
use unsup_protocol::Untrusted;
use unsup_runtime::RuntimeConfig;
use unsup_store::{LocalSignUp, MemoryStore};

use super::{Harness, Town, send};

/// Arguments of `earn` and of `publish`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Amount {
    /// Cents.
    cents: i64,
}

/// Arguments of `gift` and of `buy`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Pay {
    /// The resident that gets the money.
    to: ResidentId,
    /// Cents.
    cents: i64,
}

/// Arguments of `shout`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Shout {
    /// What everybody can read.
    text: String,
}

fn free(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "market", summary, free: true, allowed_in: &[] }
}

/// Moves cash from the acting resident to another one.
fn pay(realm: &Realm, ctx: &ActionCtx, args: &Pay, reason: &str) -> Result<(), ActionError> {
    let (from, to) = (Holder::Resident(ctx.resident), Holder::Resident(args.to));
    realm.ledger_mut().transfer_money(from, to, Money(args.cents), reason)?;
    Ok(())
}

/// The module: `earn` makes money, `gift` gives it away with the public
/// event of the life module, `buy` pays a seller with the public event
/// of a sale, `shout` is a public event with text, and `publish` is the
/// public event with economy statistics.
pub struct Market;

impl Plugin for Market {
    fn name(&self) -> &'static str {
        "market"
    }

    fn build(&self, b: &mut Builder) {
        let wages = b.faucet("market.wages");
        b.instant_verb::<Amount>(free("earn", "Get money."), move |realm, ctx, args| {
            realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), Money(args.cents));
            Ok(Value::Null)
        });
        b.instant_verb::<Pay>(free("gift", "Give money away."), |realm, ctx, args| {
            pay(realm, ctx, args, "gift")?;
            let gift = json!({
                "from": ctx.resident, "to": args.to, "money": args.cents, "item": null, "qty": 0,
            });
            realm.emit(EventMeta::public("life.gift_given", 5).actor(ctx.resident), &gift);
            Ok(Value::Null)
        });
        b.instant_verb::<Pay>(free("buy", "Pay a seller for goods."), |realm, ctx, args| {
            pay(realm, ctx, args, "sale")?;
            let sale = json!({
                "seller": Holder::Resident(args.to), "buyer": ctx.resident, "item": "bread",
                "qty": 1, "unit_price": args.cents,
            });
            realm.emit(EventMeta::public("economy.goods_sold", 5).actor(ctx.resident), &sale);
            Ok(Value::Null)
        });
        b.instant_verb::<Shout>(
            free("shout", "Say something to the world."),
            |realm, ctx, args| {
                let text = Untrusted::new(ctx.resident, args.text.clone()).to_value();
                realm.emit(
                    EventMeta::public("market.shouted", 20).actor(ctx.resident),
                    &json!({ "text": text }),
                );
                Ok(Value::Null)
            },
        );
        b.instant_verb::<Amount>(free("publish", "Publish statistics."), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("finance.stats_published", 0).actor(ctx.resident),
                &json!({ "money_supply": args.cents }),
            );
            Ok(Value::Null)
        });
    }
}

/// Makes the world of the market tests.
pub fn market_world() -> World {
    World::new(&[&Town, &Market], WorldConfig::with_seed(3))
}

/// Starts a gateway in front of the market world.
pub async fn market(config: GatewayConfig) -> Harness {
    let (sim, runtime) = unsup_runtime::start(market_world(), RuntimeConfig::manual());
    let providers = vec![Arc::new(LocalSignUp) as _];
    Harness::serve(sim, runtime, Arc::new(MemoryStore::new()), config, providers).await
}

impl Harness {
    /// Sends one action and returns the status and the answer.
    pub async fn act(&self, key: &str, verb: &str, args: Value) -> (reqwest::StatusCode, Value) {
        send(self.post("/v1/act", key, &json!({ "verb": verb, "args": args }))).await
    }

    /// Sends one action that the simulation must finish at once.
    pub async fn done(&self, key: &str, verb: &str, args: Value) {
        let (status, body) = self.act(key, verb, args).await;
        assert_eq!(status, reqwest::StatusCode::OK, "{verb}: {body}");
        assert_eq!(body["acks"][0]["status"]["state"], "done", "{verb}: {body}");
    }

    /// Runs frames, then waits until the watcher of the gateway handled
    /// the last one. Returns the flag list of that moment.
    pub async fn step_and_watch(&self, frames: u32) -> Value {
        let frame = self.sim.step(frames).await.expect("the world steps");
        let deadline = super::Deadline::start();
        loop {
            let (status, flags) = send(self.get("/admin/flags", super::ADMIN)).await;
            assert_eq!(status, reqwest::StatusCode::OK, "{flags}");
            if flags["watched_frame"] == frame {
                return flags;
            }
            deadline.again().await;
        }
    }
}
