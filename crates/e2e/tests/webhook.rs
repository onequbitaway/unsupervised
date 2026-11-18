//! A webhook receiver gets the news of a resident as signed deliveries.

mod common;

use common::agent::Agent;
use common::hook::{Delivery, Receiver, hmac_sha256, signature};
use common::link::{Link, state};
use common::{World, code, config, id};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

/// Sets the webhook of a resident. Returns the secret.
async fn set_webhook(agent: &Agent, url: &str) -> String {
    let (status, answer) =
        agent.call(Method::PUT, "/v1/webhook", Some(&json!({ "url": url }))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["url"], url);
    let secret = answer["secret"].as_str().expect("the secret of the webhook").to_string();
    assert!(secret.starts_with("unsup_whs_"), "{secret}");
    secret
}

/// Checks the headers and the signature of a delivery.
fn assert_signed(delivery: &Delivery, secret: &str) {
    assert_eq!(delivery.content_type, "application/json");
    assert!(!delivery.delivery.is_empty(), "a delivery has an id");
    assert!(delivery.timestamp.parse::<u64>().is_ok_and(|seconds| seconds > 1_600_000_000));
    let expected = signature(secret, &delivery.timestamp, &delivery.body);
    assert_eq!(delivery.signature, expected, "the signature is that of the protocol document");
}

/// Returns the `seq` of each entry of a list in a delivery.
fn seqs(body: &Value, list: &str) -> Vec<u64> {
    body[list].as_array().expect("a list").iter().map(|entry| id(&entry["seq"], "seq")).collect()
}

#[test]
fn the_check_of_the_test_is_hmac_sha256() {
    // Test case 2 of RFC 4231.
    let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(
        hex::encode(mac),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_receiver_gets_signed_deliveries_with_the_news_after_the_webhook_was_set() {
    let mut config = config();
    // The second attempt comes soon, so that the test does not wait.
    config.gateway.webhooks.backoff_base_ms = 20;
    let world = World::with(config).await;
    let mut ada = Agent::join(&world, "Ada").await;
    let mut bo = Agent::join(&world, "Bo").await;

    // News before the webhook is set: no delivery has it.
    ada.act("wait", json!({ "ticks": 1 })).await;
    world.step(3).await;

    // The receiver refuses the first attempt.
    let mut receiver = Receiver::start(1).await;
    let secret = set_webhook(&ada, &receiver.url).await;

    let ack = ada.act("wait", json!({ "ticks": 1 })).await;
    let first = world.until(receiver.next()).await;
    assert_eq!(first.answered, StatusCode::SERVICE_UNAVAILABLE);
    assert_signed(&first, &secret);
    let body = first.json();
    assert_eq!(body["resident"], ada.id);
    assert_eq!(seqs(&body, "reports"), [2], "only the news after the webhook was set: {body}");
    assert_eq!(body["reports"][0]["action_id"], ack["action_id"]);
    assert_eq!(body["reports"][0]["ok"], true);
    assert_eq!(body["notices"], json!([]));
    assert_eq!(body["next_notice"], 3);

    // A wrong secret and a changed body do not pass the check.
    assert_ne!(first.signature, signature("unsup_whs_wrong", &first.timestamp, &first.body));
    let mut changed = first.body.clone();
    changed.push(b' ');
    assert_ne!(first.signature, signature(&secret, &first.timestamp, &changed));

    // The server sends the delivery again: the same id and the same body.
    let again = common::soon(receiver.next()).await;
    assert_eq!(again.answered, StatusCode::NO_CONTENT);
    assert_eq!(again.delivery, first.delivery, "each attempt has the id of the delivery");
    assert_eq!(again.body, first.body);
    assert_signed(&again, &secret);

    // A notice: the words of another resident, as untrusted text.
    let said = bo.act("message", json!({ "to": ada.id, "text": "hello Ada" })).await;
    assert_eq!(state(&said), "done", "{said}");
    let second = world.until(receiver.next()).await;
    assert_signed(&second, &secret);
    assert_ne!(second.delivery, first.delivery);
    let body = second.json();
    assert_eq!(seqs(&body, "notices"), [3], "{body}");
    assert_eq!(body["reports"], json!([]));
    assert_eq!(body["notices"][0]["kind"], "society.message");
    assert_eq!(
        body["notices"][0]["data"]["text"],
        json!({ "untrusted": true, "from": bo.id, "text": "hello Ada" })
    );
    assert_eq!(body["next_notice"], 4);

    // `url: null` removes the webhook. News that comes now is not sent,
    // also not after the webhook is set again.
    let (status, removed) =
        ada.call(Method::PUT, "/v1/webhook", Some(&json!({ "url": null }))).await;
    assert_eq!((status, removed), (StatusCode::OK, json!({ "url": null })));
    ada.act("wait", json!({ "ticks": 1 })).await;
    world.step(3).await;
    let new_secret = set_webhook(&ada, &receiver.url).await;
    assert_ne!(new_secret, secret, "each call makes a new secret");
    bo.act("message", json!({ "to": ada.id, "text": "again" })).await;
    let third = world.until(receiver.next()).await;
    assert_signed(&third, &new_secret);
    assert_ne!(third.signature, signature(&secret, &third.timestamp, &third.body));
    let body = third.json();
    assert_eq!(seqs(&body, "notices"), [5], "the report with the number 4 was not sent: {body}");
    assert_eq!(body["reports"], json!([]));
    assert!(receiver.arrived().is_none(), "no other delivery came");

    // A body without `url` is refused: it removes nothing.
    let (status, refused) = ada.call(Method::PUT, "/v1/webhook", Some(&json!({}))).await;
    assert_eq!((status, code(&refused)), (StatusCode::BAD_REQUEST, "invalid_request"));
    for bad in ["ftp://example.org/hook", "http://user:secret@127.0.0.1:9/hook", "not an address"] {
        let (status, refused) =
            ada.call(Method::PUT, "/v1/webhook", Some(&json!({ "url": bad }))).await;
        assert_eq!((status, code(&refused)), (StatusCode::BAD_REQUEST, "invalid_request"), "{bad}");
    }
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_server_refuses_a_webhook_to_a_private_address() {
    let mut config = config();
    config.gateway.webhooks.allow_private = false;
    let world = World::with(config).await;
    let ada = Agent::join(&world, "Ada").await;
    for private in ["http://127.0.0.1:9/hook", "http://10.0.0.8/hook", "http://localhost/hook"] {
        let body = json!({ "url": private });
        let (status, refused) = ada.call(Method::PUT, "/v1/webhook", Some(&body)).await;
        assert_eq!(
            (status, code(&refused)),
            (StatusCode::BAD_REQUEST, "invalid_request"),
            "{private}"
        );
    }
    world.stop().await;
}
