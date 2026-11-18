//! Operations over real HTTP: the funnel detector and its flag list, the
//! alert list, the dashboard, and the rollback of the world.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::market::{Market, market, market_world};
use common::{ADMIN, Deadline, Harness, Town, agent_socket, code, config, next_frame, send};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use unsup_kernel::{Input, Plugin, World, WorldConfig};
use unsup_protocol::FrameOutput;
use unsup_runtime::{
    DurableConfig, DurableJournal, Journal, JournalError, MemStorage, RuntimeConfig,
};
use unsup_store::{LocalSignUp, MemoryStore, Store};

const FUNNEL: &str = "[abuse.funnel]\nmin_givers = 3\nyoung_minutes = 100\nwindow_minutes = 50";

/// Makes `count` residents, each of its own owner. Returns ids and keys.
async fn agents(h: &Harness, prefix: &str, count: usize) -> Vec<(u64, String)> {
    let mut made = Vec::new();
    for index in 0..count {
        made.push(h.agent(&format!("{prefix}{index}")).await);
    }
    made
}

#[tokio::test]
async fn a_scripted_funnel_is_flagged_with_its_evidence_and_normal_trade_is_not() {
    let h = market(config(&format!("{FUNNEL}\n[limits]\nsignups_per_hour = 0"))).await;
    let (boss_owner, boss_token) = h.owner("Boss").await;
    let (boss, boss_key) = h.resident(&boss_token, "Boss").await;
    let (shop, shop_key) = h.agent("Shop").await;
    let mules = agents(&h, "Mule", 3).await;
    let buyers = agents(&h, "Buyer", 4).await;

    // Normal trade: four young residents earn and buy from the same shop,
    // and the shop pays one of them back. One friend gives one gift.
    for (_, key) in &buyers {
        h.done(key, "earn", json!({ "cents": 500 })).await;
        h.done(key, "buy", json!({ "to": shop, "cents": 300 })).await;
    }
    h.done(&shop_key, "buy", json!({ "to": buyers[0].0, "cents": 100 })).await;
    h.done(&buyers[1].1, "gift", json!({ "to": buyers[0].0, "cents": 50 })).await;
    let flags = h.step_and_watch(1).await;
    assert_eq!(flags, json!({ "watched_frame": 1, "flags": [] }), "trade is not a funnel");

    // The funnel: three young residents earn and give all to the boss.
    for (index, (_, key)) in mules.iter().enumerate() {
        h.done(key, "earn", json!({ "cents": 400 })).await;
        h.done(key, "gift", json!({ "to": boss, "cents": 400 })).await;
        let flags = h.step_and_watch(1).await;
        let count = flags["flags"].as_array().expect("flags").len();
        assert_eq!(count, usize::from(index == 2), "after {} mules: {flags}", index + 1);
    }
    let (_, boss_seen) = send(h.get("/v1/observe", &boss_key)).await;
    assert_eq!(boss_seen["sections"]["self"]["cash"], 1200, "the detector moved nothing");

    let (status, listed) = send(h.get("/admin/flags", ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    let owner_of = |resident: u64| {
        h.store
            .resident(unsup_protocol::ResidentId(resident))
            .expect("store")
            .expect("a record")
            .owner
    };
    // Mule number n gave after frame n + 1, and it appeared at frame 0.
    let givers: Vec<Value> = (0..3)
        .map(|n| {
            let resident = mules[n].0;
            json!({ "resident": resident, "owner": owner_of(resident), "age_minutes": n + 1,
                    "gifts": 1, "money": 400, "goods": 0 })
        })
        .collect();
    let evidence: Vec<Value> = (0..3)
        .map(|n| {
            json!({ "frame": n + 1, "from": mules[n].0, "to": boss, "money": 400, "goods": 0,
                    "age_minutes": n + 1 })
        })
        .collect();
    assert_eq!(
        listed,
        json!({ "watched_frame": 4, "flags": [{
            "id": 1,
            "kind": "funnel",
            "receiver": boss,
            "receiver_owner": boss_owner,
            "first_frame": 1,
            "last_frame": 3,
            "money": 1200,
            "goods": 0,
            "givers": givers,
            "evidence": evidence,
            "dismissed": false,
        }] })
    );

    // The detector only flags: nobody is muted, frozen or banned.
    let (_, sanctions) = send(h.get("/admin/sanctions", ADMIN)).await;
    assert_eq!(sanctions, json!({ "sanctions": [] }));
    h.done(&boss_key, "gift", json!({ "to": shop, "cents": 1 })).await;

    // An administrator closes the flag.
    let (status, answer) = send(h.request(Method::POST, "/admin/flags/1/dismiss", ADMIN)).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    let (status, answer) = send(h.request(Method::POST, "/admin/flags/1/dismiss", ADMIN)).await;
    assert_eq!((status, code(&answer)), (StatusCode::NOT_FOUND, "not_found"));
    let (_, listed) = send(h.get("/admin/flags", ADMIN)).await;
    assert_eq!(listed["flags"][0]["dismissed"], true);
    let (_, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(dashboard["abuse"]["open_flags"], 0);
    assert_eq!(dashboard["abuse"]["flags"], listed["flags"]);
}

#[tokio::test]
async fn residents_from_before_the_start_of_the_gateway_have_their_true_age() {
    let h = market(config(FUNNEL)).await;
    let (boss, _) = h.agent("Boss").await;
    let young = agents(&h, "Young", 2).await;
    let (old, old_key) = h.agent("Old").await;
    h.step(1).await;

    // A new gateway: its detector saw nobody appear.
    let (sim, runtime, store) = h.into_parts();
    let h = Harness::serve(sim, runtime, store, config(FUNNEL), vec![Arc::new(LocalSignUp)]).await;
    let (late, late_key) = h.agent("Late").await;
    for (_, key) in &young {
        h.done(key, "earn", json!({ "cents": 10 })).await;
        h.done(key, "gift", json!({ "to": boss, "cents": 10 })).await;
    }
    let flags = h.step_and_watch(1).await;
    assert_eq!(flags["flags"], json!([]), "two givers are not enough");
    h.done(&late_key, "earn", json!({ "cents": 10 })).await;
    h.done(&late_key, "gift", json!({ "to": boss, "cents": 10 })).await;
    let flags = h.step_and_watch(1).await;
    let givers: Vec<(u64, u64)> = flags["flags"][0]["givers"]
        .as_array()
        .unwrap_or_else(|| panic!("a flag: {flags}"))
        .iter()
        .map(|g| (g["resident"].as_u64().expect("id"), g["age_minutes"].as_u64().expect("age")))
        .collect();
    // The young residents appeared at frame 0 and gave after frame 1.
    // The detector asked the simulation for their age. The late one
    // appeared after frame 1 and gave after frame 2.
    assert_eq!(givers, [(young[0].0, 1), (young[1].0, 1), (late, 1)]);

    // At frame 100 the residents of frame 0 are old, and the late one is
    // young for one more minute. Two old givers and one young giver to
    // another receiver: that is one giver, not three.
    h.step(97).await;
    let target = young[0].0;
    for key in [&young[1].1, &old_key, &late_key] {
        h.done(key, "earn", json!({ "cents": 10 })).await;
        h.done(key, "gift", json!({ "to": target, "cents": 10 })).await;
    }
    let flags = h.step_and_watch(1).await;
    assert_eq!(flags["watched_frame"], 101);
    let receivers: Vec<&Value> =
        flags["flags"].as_array().expect("flags").iter().map(|flag| &flag["receiver"]).collect();
    assert_eq!(receivers, [&json!(boss)], "{flags}");
    // The detector asked for the age of the old resident too: it is in
    // no flag, and the gift of the young one alone stays below the
    // threshold. A second young giver would not change that.
    assert_eq!(flags["flags"][0]["givers"].as_array().map(Vec::len), Some(3));
    let _ = old;
}

/// A journal that refuses every input while its switch is on.
struct Faulty(Arc<AtomicBool>);

impl Journal for Faulty {
    fn record(&mut self, _input: &Input) -> Result<(), JournalError> {
        if self.0.load(Ordering::SeqCst) {
            return Err(JournalError { message: "the disk is full".into() });
        }
        Ok(())
    }

    fn frame_done(&mut self, _world: &World, _frame: &FrameOutput) {}
}

fn alert<'a>(list: &'a Value, kind: &str) -> &'a Value {
    list["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .find(|alert| alert["kind"] == kind)
        .unwrap_or_else(|| panic!("no alert {kind} in {list}"))
}

#[tokio::test]
async fn journal_errors_overload_and_failed_audits_are_alerts() {
    let broken = Arc::new(AtomicBool::new(false));
    let runtime_config = RuntimeConfig {
        journal: Box::new(Faulty(Arc::clone(&broken))),
        // One waiting call fills the runtime.
        max_waiters: 1,
        ..RuntimeConfig::manual()
    };
    let (sim, runtime) = unsup_runtime::start(market_world(), runtime_config);
    let store = Arc::new(MemoryStore::new());
    // The watcher does not look by itself here: each request looks.
    let quiet = config("[ops]\nwatch_interval_ms = 0\n[limits]\nwaits_per_resident = 0");
    let h = Harness::serve(sim, runtime, store, quiet, vec![Arc::new(LocalSignUp)]).await;
    let (_, key) = h.agent("Ada").await;

    let (status, list) = send(h.get("/admin/alerts", ADMIN)).await;
    assert_eq!((status, &list), (StatusCode::OK, &json!({ "alerts": [] })));

    // The journal fails: the action is refused, and the alert says why.
    broken.store(true, Ordering::SeqCst);
    let (status, answer) = h.act(&key, "earn", json!({ "cents": 5 })).await;
    assert_eq!((status, code(&answer)), (StatusCode::SERVICE_UNAVAILABLE, "overloaded"));
    broken.store(false, Ordering::SeqCst);
    let (_, list) = send(h.get("/admin/alerts", ADMIN)).await;
    let journal = alert(&list, "journal_error");
    assert_eq!(journal["count"], 1);
    assert_eq!(
        journal["message"],
        "The journal could not record 1 inputs. The world did not apply them. 1 inputs were \
         refused since the start. Look at the disk of the data directory."
    );
    assert_eq!(list["alerts"].as_array().map(Vec::len), Some(1), "{list}");

    // Overload: a second waiting call finds the runtime full.
    let waiting = tokio::spawn(send(h.get("/v1/wait?timeout=60", &key)));
    let deadline = Deadline::start();
    while h.sim.stats().await.expect("stats").waiting != 1 {
        deadline.again().await;
    }
    let (status, answer) = send(h.get("/v1/wait?timeout=60", &key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::SERVICE_UNAVAILABLE, "overloaded"));
    let (_, list) = send(h.get("/admin/alerts", ADMIN)).await;
    let overload = alert(&list, "overload");
    assert_eq!(overload["count"], 1);
    let message = overload["message"].as_str().expect("a message");
    assert!(message.starts_with("The simulation refused 1 calls because"), "{message}");
    // A look without news raises nothing again.
    let (_, again) = send(h.get("/admin/alerts", ADMIN)).await;
    assert_eq!(alert(&again, "overload")["count"], 1);
    assert_eq!(alert(&again, "journal_error")["count"], 1);

    // The host raises what only it can see, for example a failed audit.
    h.gateway.alerts().raise("audit_failed", "The audit of the world failed at frame 7.");
    let (_, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    let failed = alert(&dashboard, "audit_failed");
    assert_eq!(failed["message"], "The audit of the world failed at frame 7.");
    let kinds: Vec<&Value> =
        dashboard["alerts"].as_array().expect("alerts").iter().map(|a| &a["kind"]).collect();
    assert_eq!(kinds.len(), 3, "{kinds:?}");
    assert_eq!(dashboard["journal"]["errors"], 1);
    assert_eq!(dashboard["sim"]["overloaded"], 1);

    // The world stops while the gateway runs: that is an alert too.
    h.step(1).await;
    drop(waiting);
    let (sim, runtime, _) = h_parts(&h);
    drop(sim);
    runtime.shutdown().expect("the world stops");
    let (_, list) = send(h.get("/admin/alerts", ADMIN)).await;
    let stopped = alert(&list, "simulation_stopped");
    assert_eq!(stopped["count"], 1);
}

/// Takes the runtime out of a harness that stays in use.
fn h_parts(h: &Harness) -> (unsup_runtime::SimHandle, unsup_runtime::RuntimeJoin, ()) {
    let (sim, runtime) = h.take_runtime();
    (sim, runtime, ())
}

#[tokio::test]
async fn the_dashboard_shows_the_simulation_the_economy_and_the_host() {
    let h = market(config("")).await;
    let (_, key) = h.agent("Ada").await;
    let (status, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(status, StatusCode::OK, "{dashboard}");
    assert_eq!(dashboard["economy"], Value::Null, "no module published statistics yet");
    assert_eq!(dashboard["host"], Value::Null, "the host said nothing yet");
    assert_eq!(dashboard["sim"]["residents"], 1);
    assert_eq!(
        dashboard["store"],
        json!({ "owners": 1, "residents": 1, "retired": 0, "webhooks": 0 })
    );
    // This runtime has no journal: nothing is recorded.
    assert_eq!(dashboard["journal"], json!({ "records": 0, "not_on_disk": 0, "errors": 0 }));
    assert_eq!(dashboard["snapshots"]["written"], 0);
    assert_eq!(
        dashboard["moderation"],
        json!({
            // The name of the owner and the name of the resident.
            "checked": 2,
            "refused": 0,
            "refused_by_rule": {},
            "muted_requests": 0,
            "banned_requests": 0,
            "sanctions": { "muted": 0, "frozen": 0, "banned": 0 },
        })
    );

    // A module publishes statistics two times: the newest one shows.
    h.done(&key, "publish", json!({ "cents": 1000 })).await;
    h.step_and_watch(1).await;
    h.done(&key, "publish", json!({ "cents": 2500 })).await;
    h.step_and_watch(1).await;
    h.gateway.set_host_info(|| json!({ "data_dir": "/srv/world", "recovered": true }));
    let (_, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(dashboard["economy"]["data"], json!({ "money_supply": 2500 }));
    // The module published after frame 1. The watcher got it with frame 2.
    assert_eq!(dashboard["economy"]["frame"], 1);
    assert_eq!(dashboard["host"], json!({ "data_dir": "/srv/world", "recovered": true }));
    assert_eq!(dashboard["abuse"]["watched_frame"], 2);
    assert_eq!(dashboard["sim"]["frame"], 2);
}

/// Starts a gateway in front of a world with the durable journal in
/// memory: a snapshot every two frames.
async fn durable(overrides: &str) -> Harness {
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(Town), Box::new(Market)];
    let durable =
        DurableConfig { snapshot_every: 2, keep_snapshots: 10, ..DurableConfig::default() };
    let opened =
        DurableJournal::open(Arc::new(MemStorage::new()), Arc::new(plugins), durable, |plugins| {
            World::new(plugins, WorldConfig::with_seed(3))
        })
        .expect("a new world");
    let runtime_config =
        RuntimeConfig { journal: Box::new(opened.journal), ..RuntimeConfig::manual() };
    let (sim, runtime) = unsup_runtime::start(opened.world, runtime_config);
    let store = Arc::new(MemoryStore::new());
    Harness::serve(sim, runtime, store, config(overrides), vec![Arc::new(LocalSignUp)]).await
}

async fn rollback(h: &Harness, frame: u64) -> (StatusCode, Value) {
    send(h.post("/admin/rollback", ADMIN, &json!({ "frame": frame }))).await
}

#[tokio::test]
async fn a_rollback_puts_the_world_back_and_ends_the_records_of_the_lost_residents() {
    let h = durable("[limits]\nsignups_per_hour = 0").await;
    let (_, ada_token) = h.owner("Ada").await;
    let (ada, ada_key) = h.resident(&ada_token, "Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;
    h.done(&ada_key, "earn", json!({ "cents": 100 })).await;
    // Frame 2 has a snapshot: Ada has 100 cents, Bob is frozen later.
    h.step(2).await;

    // After the snapshot: Ada earns more, a third resident appears, Bob
    // is frozen, Ada is muted, and Bob leaves the world.
    h.done(&ada_key, "earn", json!({ "cents": 900 })).await;
    let (cy, cy_key) = h.agent("Cy").await;
    let (status, _) = send(h.request(Method::POST, "/admin/residents/1/freeze", ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(h.request(Method::POST, "/admin/residents/1/mute", ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) =
        send(h.request(Method::POST, &format!("/admin/residents/{bob}/despawn"), ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    h.step(1).await;
    assert_eq!((ada, bob, cy), (1, 2, 3));
    let (_, seen) = send(h.get("/v1/observe", &ada_key)).await;
    assert_eq!(seen["sections"]["self"]["cash"], 1000);
    assert_eq!(seen["frame"], 3);

    // Back to the snapshot at or before frame 3: that of frame 2.
    let (status, answer) = rollback(&h, 3).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer,
        json!({
            "ok": true,
            "frame": 2,
            // One action, one spawn, one freeze, one despawn, one frame.
            "dropped_inputs": 5,
            // Cy did not exist at frame 2: its record and its key end.
            "retired": [cy],
            // Bob is in the world again, and nobody has a key for it.
            "orphans": [bob],
            // The freeze of Ada is from after the snapshot. It is back.
            "refrozen": 1,
        })
    );
    let (_, seen) = send(h.get("/v1/observe", &ada_key)).await;
    assert_eq!(
        (seen["frame"].clone(), seen["sections"]["self"]["cash"].clone()),
        (json!(2), json!(100))
    );
    assert_eq!(seen["sections"]["self"]["conditions"], json!({ "frozen": null }));
    let (_, answer) = h.act(&ada_key, "earn", json!({ "cents": 1 })).await;
    assert_eq!(answer["acks"][0]["status"]["error"]["code"], "blocked", "{answer}");
    // The mute is not in the world. It stays as it was.
    let (_, sanctions) = send(h.get("/admin/sanctions", ADMIN)).await;
    let kinds: Vec<&Value> =
        sanctions["sanctions"].as_array().expect("a list").iter().map(|s| &s["kind"]).collect();
    assert_eq!(kinds, ["mute", "freeze"]);

    // The key of Cy is unknown, and the key of Bob ended at the despawn.
    for key in [&cy_key, &bob_key] {
        let (status, answer) = send(h.get("/v1/me", key)).await;
        assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    }
    // The world gives the id of Cy to the next resident. The old key of
    // Cy does not reach it.
    let (dee, dee_key) = h.agent("Dee").await;
    assert_eq!(dee, cy, "the id is used again");
    let (status, me) = send(h.get("/v1/me", &dee_key)).await;
    assert_eq!((status, me["name"].clone()), (StatusCode::OK, json!("Dee")));
    let (status, _) = send(h.get("/v1/me", &cy_key)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The dashboard counts the rollback.
    let (_, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(dashboard["snapshots"]["rollbacks"], 1);
    assert_eq!(dashboard["sim"]["frame"], 2);

    // A frame without a snapshot at or before it does not exist here:
    // frame 0 has the first snapshot. The body must name a frame.
    let (status, answer) = rollback(&h, 0).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["frame"], 0);
    assert_eq!(h.sim.stats().await.expect("stats").residents, 0, "the world of frame 0 is empty");
    let (status, answer) = send(h.post("/admin/rollback", ADMIN, &json!({}))).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
}

#[tokio::test]
async fn a_rollback_that_cannot_be_done_is_refused_with_its_reason() {
    // A runtime without a journal keeps no snapshots.
    let h = market(config("")).await;
    let (_, key) = h.agent("Ada").await;
    let (status, answer) = rollback(&h, 0).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::CONFLICT,
            json!({ "error": { "code": "refused",
                "message": "The runtime keeps no snapshots, so it cannot roll back." } })
        )
    );
    let (status, _) = send(h.get("/v1/me", &key)).await;
    assert_eq!(status, StatusCode::OK, "nothing changed");
}

#[tokio::test]
async fn cursors_follow_the_world_back_after_a_rollback() {
    let h = durable("").await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    // The world of the snapshot at frame 2 has one notice for Ada.
    h.tell(&bob_key, ada, "before").await;
    h.step(2).await;
    let mcp = |tool: &'static str, arguments: Value| {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments,
                "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                           "io.modelcontextprotocol/clientCapabilities": {} } } });
        let request = h
            .post("/mcp", &ada_key, &body)
            .header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "tools/call")
            .header("Mcp-Name", tool);
        async move {
            let (status, answer) = send(request).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            answer["result"]["content"][0]["text"].as_str().expect("a text").to_string()
        }
    };
    let mut ws = agent_socket(&h, &ada_key, "?since=1").await;
    assert_eq!(next_frame(&mut ws).await["notice"]["data"]["text"], "before");

    // Three more notices after the snapshot. The socket and the MCP
    // cursor of Ada move behind all four.
    for text in ["one", "two", "three"] {
        h.tell(&bob_key, ada, text).await;
        h.step(1).await;
        assert_eq!(next_frame(&mut ws).await["notice"]["data"]["text"], text);
    }
    let seen = mcp("observe", json!({ "sections": [] })).await;
    assert!(seen.contains("text: three") && seen.contains("seq: 4"), "{seen}");

    let (status, answer) = rollback(&h, 2).await;
    assert_eq!((status, answer["frame"].clone()), (StatusCode::OK, json!(2)));
    // The inbox of Ada ends at 2 now. The next notice gets the number 2
    // again. A cursor that stayed at 5 would hide it.
    h.tell(&bob_key, ada, "again").await;
    h.step(1).await;
    let pushed = next_frame(&mut ws).await;
    assert_eq!(
        (pushed["notice"]["seq"].clone(), pushed["notice"]["data"]["text"].clone()),
        (json!(2), json!("again"))
    );
    let seen = mcp("observe", json!({ "sections": [] })).await;
    assert!(seen.contains("text: again") && seen.contains("seq: 2"), "{seen}");
    // The notice from before the snapshot does not come a second time.
    assert!(!seen.contains("before"), "{seen}");
    let seen = mcp("observe", json!({ "sections": [] })).await;
    assert_eq!(seen, "time: year 1, day 1, 00:03 (tick 3)\nnews: none\n");
}
