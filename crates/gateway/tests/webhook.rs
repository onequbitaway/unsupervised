//! Webhooks: deliveries to a local receiver with a verified signature,
//! retries, the bounded queue, and the addresses that the server refuses.

mod common;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode as Status, Uri, header};
use axum::response::{IntoResponse, Response};
use common::{ADMIN, Deadline, Harness, code, config, send, soon, world};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc};
use unsup_gateway::webhook::sign::{
    DELIVERY_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER, sign, verify,
};
use unsup_protocol::{OwnerId, ResidentId, SpawnSpec};
use unsup_store::{
    LocalSignUp, MemoryStore, NewOwner, NewResident, Secret, Store, Token, TokenKind, unix_now,
};

const LOCAL: &str = "[webhooks]\nallow_private = true\nbackoff_base_ms = 5\nbackoff_max_ms = 10";

/// One request that the receiver got.
#[derive(Debug)]
struct Delivery {
    path: String,
    headers: HeaderMap,
    body: Bytes,
}

impl Delivery {
    fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_else(|| panic!("{name}"))
    }

    fn timestamp(&self) -> u64 {
        self.header(TIMESTAMP_HEADER).parse().expect("a timestamp")
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }

    /// Returns true when the signature header fits the secret.
    fn signed_with(&self, secret: &str) -> bool {
        verify(secret, self.timestamp(), &self.body, self.header(SIGNATURE_HEADER))
    }
}

struct Shared {
    requests: mpsc::UnboundedSender<Delivery>,
    /// The statuses of the next answers. 200 when it is empty.
    statuses: Mutex<VecDeque<Status>>,
    /// The handler takes one permit before it answers.
    gate: Semaphore,
}

/// A local HTTP server that records what it gets.
struct Receiver {
    addr: SocketAddr,
    requests: mpsc::UnboundedReceiver<Delivery>,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn receive(
    State(shared): State<Arc<Shared>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let _ = shared.requests.send(Delivery { path: uri.path().to_string(), headers, body });
    let permit = shared.gate.acquire().await.expect("the gate is not closed");
    permit.forget();
    let status = shared.statuses.lock().expect("lock").pop_front().unwrap_or(Status::OK);
    if status.is_redirection() {
        return (status, [(header::LOCATION, "/elsewhere")]).into_response();
    }
    status.into_response()
}

impl Receiver {
    /// Starts a receiver that answers with `statuses`, then with 200.
    /// `open` is the number of requests that it answers at once; each
    /// further one waits for [`Receiver::release`].
    async fn start(statuses: &[Status], open: usize) -> Receiver {
        let (tx, requests) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            requests: tx,
            statuses: Mutex::new(statuses.iter().copied().collect()),
            gate: Semaphore::new(open),
        });
        let app = axum::Router::new().fallback(receive).with_state(Arc::clone(&shared));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().expect("an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the receiver runs");
        });
        Receiver { addr, requests, shared, task }
    }

    /// A receiver that answers every request at once.
    async fn open(statuses: &[Status]) -> Receiver {
        Receiver::start(statuses, Semaphore::MAX_PERMITS).await
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Waits for the next request.
    async fn next(&mut self) -> Delivery {
        soon(self.requests.recv()).await.expect("the receiver runs")
    }

    /// Lets `count` waiting requests get their answer.
    fn release(&self, count: usize) {
        self.shared.gate.add_permits(count);
    }
}

/// Sets the webhook of a resident and returns the signing secret.
async fn set_webhook(h: &Harness, key: &str, url: &str) -> String {
    let (status, body) = send(h.put("/v1/webhook", key, &json!({ "url": url }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], url);
    body["secret"].as_str().expect("a secret").to_string()
}

/// Reads the webhook numbers of the administrator until `done` holds.
async fn webhook_stats(h: &Harness, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Deadline::start();
    loop {
        let (_, stats) = send(h.get("/admin/stats", ADMIN)).await;
        let webhooks = stats["gateway"]["webhooks"].clone();
        if done(&webhooks) {
            return webhooks;
        }
        deadline.again().await;
    }
}

#[tokio::test]
async fn a_delivery_arrives_with_a_signature_that_verifies() {
    let h = Harness::with_toml(LOCAL).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;
    let mut receiver = Receiver::open(&[]).await;

    let secret = set_webhook(&h, &ada_key, &receiver.url("/hook/ada?x=1")).await;
    assert!(secret.starts_with("unsup_whs_") && secret.len() == 74, "{secret}");

    let before = unix_now();
    h.tell(&bob_key, ada, "hello Ada").await;
    h.step(1).await;
    let delivery = receiver.next().await;
    assert_eq!(delivery.path, "/hook/ada");
    assert_eq!(delivery.header("content-type"), "application/json");
    assert_eq!(
        delivery.header("user-agent"),
        concat!("unsupervised-webhook/", env!("CARGO_PKG_VERSION"))
    );
    let id = delivery.header(DELIVERY_HEADER);
    assert!(id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
    assert!((before..=unix_now()).contains(&delivery.timestamp()));

    // The signature fits the secret, the time and the exact body.
    let signature = delivery.header(SIGNATURE_HEADER);
    assert_eq!(signature, sign(&secret, delivery.timestamp(), &delivery.body));
    assert!(signature.starts_with("v1=") && signature.len() == 67, "{signature}");
    assert!(delivery.signed_with(&secret));
    assert!(!delivery.signed_with("unsup_whs_another_secret"));
    assert!(!verify(&secret, delivery.timestamp() + 1, &delivery.body, signature));
    let mut changed = delivery.body.to_vec();
    changed[0] ^= 1;
    assert!(!verify(&secret, delivery.timestamp(), &changed, signature));

    assert_eq!(
        delivery.json(),
        json!({
            "resident": ada,
            "notices": [{
                "seq": 1,
                "tick": 0,
                "kind": "town.told",
                "from": bob,
                "data": { "untrusted": true, "from": bob, "text": "hello Ada" },
            }],
            "reports": [],
            "next_notice": 2,
        })
    );

    // The next delivery has only what is new: here the report of an
    // action.
    let work = json!({ "verb": "work", "args": { "ticks": 1 }, "id": "w" });
    let (_, answer) = send(h.post("/v1/act", &ada_key, &work)).await;
    h.step(1).await;
    let second = receiver.next().await;
    assert_ne!(second.header(DELIVERY_HEADER), id);
    let body = second.json();
    assert_eq!((body["notices"].clone(), body["next_notice"].clone()), (json!([]), json!(3)));
    assert_eq!(body["reports"][0]["action_id"], answer["acks"][0]["action_id"]);
    assert_eq!(body["reports"][0]["request_id"], "w");
    assert!(second.signed_with(&secret));

    let stats = webhook_stats(&h, |w| w["delivered"] == 2).await;
    assert_eq!(
        stats,
        json!({ "webhooks": 1, "delivered": 2, "failed_attempts": 0, "queue_full": 0, "abandoned": 0 })
    );
    let (_, all) = send(h.get("/admin/stats", ADMIN)).await;
    assert_eq!(all["store"]["webhooks"], 1);
}

#[tokio::test]
async fn a_new_secret_replaces_the_old_and_null_removes_the_webhook() {
    let h = Harness::with_toml(LOCAL).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let mut receiver = Receiver::open(&[]).await;

    let first = set_webhook(&h, &ada_key, &receiver.url("/one")).await;
    let second = set_webhook(&h, &ada_key, &receiver.url("/two")).await;
    assert_ne!(first, second);
    h.tell(&bob_key, ada, "to the second address").await;
    h.step(1).await;
    let delivery = receiver.next().await;
    assert_eq!(delivery.path, "/two");
    assert!(delivery.signed_with(&second) && !delivery.signed_with(&first));
    webhook_stats(&h, |w| w["delivered"] == 1).await;

    // `null` removes the webhook: news that comes now is not delivered.
    let (status, body) = send(h.put("/v1/webhook", &ada_key, &json!({ "url": null }))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "url": null })));
    assert_eq!(h.store.webhook(ResidentId(ada)), Ok(None));
    h.tell(&bob_key, ada, "nobody listens").await;
    h.step(1).await;

    // A webhook that is set again gets only later news. The first
    // request is that news, so the one before was never sent.
    set_webhook(&h, &ada_key, &receiver.url("/three")).await;
    h.tell(&bob_key, ada, "after the new start").await;
    h.step(1).await;
    let delivery = receiver.next().await;
    assert_eq!(delivery.path, "/three");
    let texts: Vec<Value> = delivery.json()["notices"]
        .as_array()
        .expect("notices")
        .iter()
        .map(|n| n["data"]["text"].clone())
        .collect();
    assert_eq!(texts, [json!("after the new start")]);

    for body in [json!({}), json!({ "url": 5 }), json!({ "url": null, "secret": "mine" })] {
        let (status, answer) = send(h.put("/v1/webhook", &ada_key, &body)).await;
        assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"), "{body}");
    }
}

#[tokio::test]
async fn a_failed_delivery_is_sent_again_with_the_same_id() {
    let h = Harness::with_toml(LOCAL).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let mut receiver =
        Receiver::open(&[Status::INTERNAL_SERVER_ERROR, Status::SERVICE_UNAVAILABLE]).await;
    let secret = set_webhook(&h, &ada_key, &receiver.url("/hook")).await;

    h.tell(&bob_key, ada, "important").await;
    h.step(1).await;
    let first = receiver.next().await;
    let second = receiver.next().await;
    let third = receiver.next().await;
    // The same delivery three times: one id, one body, each one signed.
    assert_eq!(second.header(DELIVERY_HEADER), first.header(DELIVERY_HEADER));
    assert_eq!(third.header(DELIVERY_HEADER), first.header(DELIVERY_HEADER));
    assert_eq!((&second.body, &third.body), (&first.body, &first.body));
    assert!(
        first.signed_with(&secret) && second.signed_with(&secret) && third.signed_with(&secret)
    );

    let stats = webhook_stats(&h, |w| w["delivered"] == 1).await;
    assert_eq!(
        (stats["failed_attempts"].clone(), stats["abandoned"].clone()),
        (json!(2), json!(0))
    );

    // The delivery was accepted, so the next one holds only new news.
    h.tell(&bob_key, ada, "next").await;
    h.step(1).await;
    let next = receiver.next().await;
    assert_ne!(next.header(DELIVERY_HEADER), first.header(DELIVERY_HEADER));
    assert_eq!(next.json()["notices"].as_array().map(Vec::len), Some(1));
    assert_eq!(next.json()["notices"][0]["data"]["text"], "next");
}

#[tokio::test]
async fn after_the_last_attempt_the_webhook_rests_and_starts_again() {
    let h = Harness::with_toml(&format!("{LOCAL}\nattempts = 2")).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    // A redirect is a failure too: the server never follows one.
    let mut receiver = Receiver::open(&[Status::BAD_GATEWAY, Status::FOUND]).await;
    set_webhook(&h, &ada_key, &receiver.url("/hook")).await;

    h.tell(&bob_key, ada, "kept in the inbox").await;
    h.step(1).await;
    let first = receiver.next().await;
    let second = receiver.next().await;
    assert_eq!(second.header(DELIVERY_HEADER), first.header(DELIVERY_HEADER));
    // After two attempts the server gives the delivery up. It rests,
    // then sends what the inbox holds as a new delivery.
    let third = receiver.next().await;
    assert_eq!(third.path, "/hook", "the redirect was not followed");
    assert_ne!(third.header(DELIVERY_HEADER), first.header(DELIVERY_HEADER));
    assert_eq!(third.json()["notices"][0]["data"]["text"], "kept in the inbox");
    let stats = webhook_stats(&h, |w| w["delivered"] == 1).await;
    assert_eq!(
        (stats["failed_attempts"].clone(), stats["abandoned"].clone()),
        (json!(2), json!(1))
    );
}

#[tokio::test]
async fn deliveries_wait_in_a_bounded_queue() {
    // One worker and no place to wait.
    let h = Harness::with_toml(&format!("{LOCAL}\nworkers = 1\nqueue = 0\nattempts = 50")).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (cy, cy_key) = h.agent("Cy").await;
    let (_, bob_key) = h.agent("Bob").await;
    // The receiver holds every request until the test lets it go.
    let mut receiver = Receiver::start(&[], 0).await;
    set_webhook(&h, &ada_key, &receiver.url("/ada")).await;
    set_webhook(&h, &cy_key, &receiver.url("/cy")).await;

    h.tell(&bob_key, ada, "for Ada").await;
    h.tell(&bob_key, cy, "for Cy").await;
    h.step(1).await;
    // One delivery holds the worker. The other finds the queue full and
    // tries again later.
    let first = receiver.next().await;
    let stats = webhook_stats(&h, |w| w["queue_full"].as_u64() >= Some(1)).await;
    assert_eq!(stats["delivered"], 0);
    receiver.release(2);
    let second = receiver.next().await;
    let mut paths = [first.path.as_str(), second.path.as_str()];
    paths.sort_unstable();
    assert_eq!(paths, ["/ada", "/cy"]);
    let stats = webhook_stats(&h, |w| w["delivered"] == 2).await;
    assert_eq!(
        (stats["failed_attempts"].clone(), stats["abandoned"].clone()),
        (json!(0), json!(0))
    );
}

#[tokio::test]
async fn a_private_address_is_refused() {
    // The defaults: no private addresses.
    let h = Harness::start().await;
    let (_, key) = h.agent("Ada").await;
    let put = |url: String| send(h.put("/v1/webhook", &key, &json!({ "url": url })));
    let private = "`url` points at a private, loopback or reserved address. The server sends \
                   webhooks only to public addresses.";

    let receiver = Receiver::open(&[]).await;
    let refused = [
        receiver.url("/hook"),
        "http://127.0.0.1/hook".to_string(),
        "http://localhost:9/hook".to_string(),
        "https://[::1]/hook".to_string(),
        "http://10.1.2.3/".to_string(),
        "http://172.16.0.1/".to_string(),
        "http://192.168.1.10:8080/hook".to_string(),
        "http://169.254.169.254/latest/meta-data/".to_string(),
        "http://100.64.0.1/".to_string(),
        "http://0.0.0.0/".to_string(),
        "http://[fd12:3456::1]/".to_string(),
        "http://[fe80::1]/".to_string(),
        "http://[::ffff:127.0.0.1]/".to_string(),
        // Other spellings of 127.0.0.1.
        "http://2130706433/".to_string(),
        "http://0x7f.0.0.1/".to_string(),
        "http://127.1/".to_string(),
    ];
    for url in refused {
        let (status, answer) = put(url.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}");
        assert_eq!(
            answer,
            json!({ "error": { "code": "invalid_request", "message": private } }),
            "{url}"
        );
    }

    let wrong = [
        ("ftp://example.org/hook", "`url` must start with `http://` or `https://`."),
        ("file:///etc/passwd", "`url` must start with `http://` or `https://`."),
        ("javascript:alert(1)", "`url` must start with `http://` or `https://`."),
        ("https://user:secret@8.8.8.8/hook", "`url` must not hold a user name or a password."),
        ("example.org/hook", "`url` is not an address: relative URL without a base."),
        ("http://", "`url` is not an address: empty host."),
    ];
    for (url, message) in wrong {
        let (status, answer) = put(url.to_string()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}");
        assert_eq!(answer["error"]["message"], message, "{url}");
    }
    let (status, answer) = put(format!("https://8.8.8.8/{}", "a".repeat(2048))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(answer["error"]["message"], "`url` has at most 2048 bytes.");
    // Nothing was stored.
    assert_eq!(h.store.webhooks(), Ok(Vec::new()));

    // A public address is taken. The server sends nothing until there is
    // news, and this test makes none.
    let (status, answer) = put("https://8.8.8.8/hook".to_string()).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(h.store.webhooks().expect("list").len(), 1);
}

#[tokio::test]
async fn a_stored_webhook_cannot_reach_a_private_address_later() {
    // A server ran with private addresses allowed and stored two
    // webhooks. Now it starts again with the strict rules.
    let (sim, runtime) = world();
    let spec =
        |name: &str| SpawnSpec { owner: OwnerId(1), name: name.into(), model: None, city: None };
    let by_address = sim.spawn(spec("ByAddress")).await.expect("spawn");
    let by_name = sim.spawn(spec("ByName")).await.expect("spawn");
    let teller = sim.spawn(spec("Teller")).await.expect("spawn");

    let mut receiver = Receiver::open(&[]).await;
    let store = Arc::new(MemoryStore::new());
    let token = Token::generate(TokenKind::Owner).expect("random bytes");
    let owner = NewOwner { name: "Ada".into(), provider: "local".into(), subject: "ada".into() };
    let owner = store.create_owner(&owner, &token.hash(), &token.hint(), 1).expect("owner").id;
    let mut keys = Vec::new();
    for (resident, name) in [(by_address, "ByAddress"), (by_name, "ByName"), (teller, "Teller")] {
        let key = Token::generate(TokenKind::Resident).expect("random bytes");
        let new = NewResident { resident, owner, name: name.into(), model: None };
        store.create_resident(&new, &key.hash(), &key.hint(), 1).expect("resident");
        keys.push(key.reveal().to_string());
    }
    let secret = Secret::generate_webhook().expect("random bytes");
    let port = receiver.addr.port();
    store.set_webhook(by_address, &receiver.url("/literal"), &secret, 1).expect("webhook");
    store
        .set_webhook(by_name, &format!("http://localhost:{port}/name"), &secret, 1)
        .expect("webhook");

    let strict = config("[webhooks]\nattempts = 1\nbackoff_max_ms = 60000");
    let h = Harness::serve(sim, runtime, store, strict, vec![Arc::new(LocalSignUp)]).await;
    // The address that is an IP is refused at the start. The name is
    // kept: what it points at is checked for every connection.
    let stats = webhook_stats(&h, |_| true).await;
    assert_eq!(stats["webhooks"], 1);

    h.tell(&keys[2], by_address.0, "to the address").await;
    h.tell(&keys[2], by_name.0, "to the name").await;
    h.step(1).await;
    let stats = webhook_stats(&h, |w| w["abandoned"] == 1).await;
    assert_eq!(
        (stats["failed_attempts"].clone(), stats["delivered"].clone()),
        (json!(1), json!(0))
    );
    // Not one request reached the local receiver.
    assert!(receiver.requests.try_recv().is_err());
}
