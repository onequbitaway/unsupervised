//! Moderation over real HTTP: every text passes the filters before it
//! enters the simulation, on every way in.

mod common;

use std::sync::Arc;

use common::market::{market, market_world};
use common::{ADMIN, Harness, code, config, send};
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use unsup_gateway::{FilterFuture, Gateway, Refusal, Text, TextFilter, TextUse};
use unsup_runtime::RuntimeConfig;
use unsup_store::MemoryStore;

const RULES: &str = r#"
[moderation]
max_text_bytes = 40
max_repeat = 4
links = "refuse"
words = ["Badword", "very bad"]
patterns = ["call ###-####", "free*money"]
"#;

/// Makes `from` tell `to` a text, and returns the status and the answer.
async fn tell(h: &Harness, key: &str, to: u64, text: &str) -> (StatusCode, Value) {
    h.act(key, "tell", json!({ "to": to, "text": text })).await
}

/// Returns the texts of the notices in the inbox of a resident.
async fn heard(h: &Harness, key: &str) -> Vec<String> {
    let (status, inbox) = send(h.get("/v1/observe?since=1", key)).await;
    assert_eq!(status, StatusCode::OK, "{inbox}");
    inbox["notices"]
        .as_array()
        .expect("notices")
        .iter()
        .map(|notice| notice["data"]["text"].as_str().expect("a text").to_string())
        .collect()
}

#[tokio::test]
async fn each_rule_refuses_its_text_and_nothing_else() {
    let h = market(config(RULES)).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;

    let refused = [
        ("this is a BADWORD!", "The text holds a word that this server does not permit."),
        ("a very   Bad thing", "The text holds a word that this server does not permit."),
        // A character without width does not hide a word.
        ("bad\u{200b}word", "The text holds a word that this server does not permit."),
        ("Call 555-0199 today", "The text matches a pattern that this server does not permit."),
        ("FREE and easy\nmoney", "The text matches a pattern that this server does not permit."),
        ("see example.com", "The text holds a link. This server takes no links."),
        ("https://x.test/a", "The text holds a link. This server takes no links."),
        ("write to ada@mail.example.org", "The text holds a link. This server takes no links."),
        ("aaaaa", "The text repeats one character more than 4 times in a row."),
        (
            "This text has forty-one bytes in its body",
            "The text has 41 bytes. This server takes at most 40.",
        ),
        (
            "ring\u{7}",
            "The text holds a control character. Only line breaks and tabs are permitted.",
        ),
        (
            "left\u{202e}right",
            "The text holds a control character. Only line breaks and tabs are permitted.",
        ),
    ];
    for (text, reason) in refused {
        let (status, answer) = tell(&h, &bob_key, ada, text).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text:?}: {answer}");
        assert_eq!(
            answer,
            json!({ "error": { "code": "text_refused",
                "message": format!("`args.text`: {reason}") } }),
            "{text:?}"
        );
    }

    // Text that looks like a refused one and is not.
    let allowed = [
        "badwords are fine",
        "a very good, bad thing",
        "call 555-019 now",
        "free the mice",
        "It ends. Then it starts.",
        "aaaa",
        "This text has forty bytes in its body ..",
        "two\nlines\tand a tab",
    ];
    for text in allowed {
        let (status, answer) = tell(&h, &bob_key, ada, text).await;
        assert_eq!(status, StatusCode::OK, "{text:?}: {answer}");
        assert_eq!(answer["acks"][0]["status"]["state"], "done", "{text:?}: {answer}");
        // The simulation takes a few free actions in one tick.
        h.step(1).await;
    }
    // An argument that is not speech passes the same rules.
    let (status, answer) =
        h.act(&bob_key, "tell", json!({ "to": ada, "text": "ok", "kind": "badword" })).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));
    assert_eq!(
        answer["error"]["message"],
        "`args.kind`: The text holds a word that this server does not permit."
    );

    // One bad action stops a whole plan: the good action before it is
    // not sent.
    let plan = json!({ "plan": [
        { "verb": "tell", "args": { "to": ada, "text": "first of a plan" } },
        { "verb": "tell", "args": { "to": ada, "text": "second has a badword" } },
    ] });
    let (status, answer) = send(h.post("/v1/act", &bob_key, &plan)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(
        answer["error"]["message"],
        "Action 2: `args.text`: The text holds a word that this server does not permit."
    );

    // The inbox of Ada holds the allowed texts and not one refused text.
    h.step(1).await;
    assert_eq!(heard(&h, &ada_key).await, allowed);
}

#[tokio::test]
async fn a_refused_text_reaches_no_notice_and_no_public_event() {
    let h = market(config(RULES)).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;
    let mut frames = h.sim.frames();

    // A notice for one resident, and a public event for the viewers.
    let (status, _) = tell(&h, &bob_key, ada, "you are a badword").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, answer) = h.act(&bob_key, "shout", json!({ "text": "BADWORD for all" })).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));
    h.done(&bob_key, "shout", json!({ "text": "good morning" })).await;
    let (status, _) = tell(&h, &bob_key, ada, "hello Ada").await;
    assert_eq!(status, StatusCode::OK);
    h.step(1).await;

    assert_eq!(heard(&h, &ada_key).await, ["hello Ada"]);
    // The frame is what the stream for viewers is made from.
    let frame = frames.recv().await.expect("the frame");
    let shouted: Vec<&Value> = frame
        .events
        .iter()
        .filter(|event| event.kind == "market.shouted")
        .map(|event| &event.data)
        .collect();
    assert_eq!(
        shouted,
        [&json!({ "text": { "untrusted": true, "from": bob, "text": "good morning" } })]
    );
    let all = serde_json::to_string(&*frame).expect("JSON").to_lowercase();
    assert!(!all.contains("badword"), "{all}");
}

#[tokio::test]
async fn names_of_owners_residents_and_models_pass_the_rules() {
    let h = market(config(&format!("{RULES}\n[limits]\nsignups_per_hour = 0"))).await;
    let sign_up = h.http.post(h.url("/v1/owners")).json(&json!({ "name": "Mr Badword" }));
    let (status, answer) = send(sign_up).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(
        answer["error"],
        json!({ "code": "text_refused",
            "message": "`name`: The text holds a word that this server does not permit." })
    );

    let (_, token) = h.owner("Ada").await;
    let cases = [
        (json!({ "name": "badword" }), "`name`: The text holds a word"),
        (json!({ "name": "visit example.com" }), "`name`: The text holds a link."),
        (json!({ "name": "Ada", "model": "badword-9000" }), "`model`: The text holds a word"),
        (json!({ "name": "Zzzzzzz" }), "`name`: The text repeats one character"),
    ];
    for (body, start) in cases {
        let (status, answer) = send(h.post("/v1/residents", &token, &body)).await;
        assert_eq!((status, code(&answer)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));
        let message = answer["error"]["message"].as_str().expect("a message");
        assert!(message.starts_with(start), "{body}: {message}");
    }
    // Nothing was made: the world has no resident and no spawn event.
    let (_, listed) = send(h.get("/v1/residents", &token)).await;
    assert_eq!(listed["residents"], json!([]));
    assert_eq!(h.sim.stats().await.expect("stats").residents, 0);
    let (status, made) =
        send(h.post("/v1/residents", &token, &json!({ "name": "Ada", "model": "good-1" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
}

#[tokio::test]
async fn the_text_limit_counts_speech_of_one_resident() {
    let rules = "[moderation]\nwords = [\"badword\"]\ntexts_per_minute = 1\ntext_burst = 2";
    let h = market(config(rules)).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;

    // A refused text takes nothing from the limit.
    let (status, _) = tell(&h, &bob_key, ada, "badword").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    for text in ["one", "two"] {
        let (status, _) = tell(&h, &bob_key, ada, text).await;
        assert_eq!(status, StatusCode::OK, "{text}");
    }
    let third = json!({ "verb": "tell", "args": { "to": ada, "text": "three" } });
    let response = h.post("/v1/act", &bob_key, &third).send().await.expect("answer");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("60"));
    let answer: Value = response.json().await.expect("JSON");
    assert_eq!(
        answer,
        json!({ "error": { "code": "rate_limited",
            "message": "Too many texts. Try again in 60 seconds." } })
    );
    // Actions without speech have no such limit, and each resident has
    // its own.
    h.step(1).await;
    for _ in 0..3 {
        h.done(&bob_key, "earn", json!({ "cents": 1 })).await;
    }
    let (status, _) = tell(&h, &ada_key, ada, "my own limit").await;
    assert_eq!(status, StatusCode::OK);
    h.step(1).await;
    assert_eq!(heard(&h, &ada_key).await, ["one", "two", "my own limit"]);
}

/// Opens the agent WebSocket of a resident.
async fn socket(
    h: &Harness,
    key: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://{}/v1/ws", h.addr).into_client_request().expect("a request");
    let value = format!("Bearer {key}").parse().expect("a header value");
    request.headers_mut().insert("Authorization", value);
    tokio_tungstenite::connect_async(request).await.expect("the socket opens").0
}

#[tokio::test]
async fn every_way_in_has_the_filter() {
    let h = market(config(RULES)).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let bad = json!({ "to": ada, "text": "a badword" });
    let expected = "`args.text`: The text holds a word that this server does not permit.";

    // The WebSocket: one action and a plan.
    let mut ws = socket(&h, &bob_key).await;
    let frames = [
        json!({ "op": "act", "request": { "verb": "tell", "args": bad } }),
        json!({ "op": "plan", "requests": [{ "verb": "tell", "args": bad }] }),
    ];
    for frame in frames {
        ws.send(Message::Text(frame.to_string().into())).await.expect("the frame is sent");
        let answer = common::soon(ws.next()).await.expect("open").expect("a frame");
        let Message::Text(text) = answer else { panic!("a text frame, not {answer:?}") };
        let answer: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            answer,
            json!({ "op": "error", "error": { "code": "text_refused", "message": expected } })
        );
    }

    // The MCP tools `act`, `plan` and `set_routines`.
    let routine = json!({ "name": "talk", "when": { "type": "always" },
        "then": { "verb": "tell", "args": bad } });
    let calls = [
        ("act", json!({ "verb": "tell", "args": bad }), expected.to_string()),
        ("plan", json!({ "actions": [{ "verb": "tell", "args": bad }] }), expected.to_string()),
        (
            "set_routines",
            json!({ "routines": [routine] }),
            "`args.routines[0].then.args.text`: The text holds a word that this server does not \
             permit."
                .to_string(),
        ),
    ];
    for (tool, arguments, message) in calls {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments,
                "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                           "io.modelcontextprotocol/clientCapabilities": {} } } });
        let request = h
            .post("/mcp", &bob_key, &body)
            .header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "tools/call")
            .header("Mcp-Name", tool);
        let (status, answer) = send(request).await;
        assert_eq!(status, StatusCode::OK, "{tool}: {answer}");
        assert_eq!(answer["result"]["isError"], true, "{tool}: {answer}");
        assert_eq!(
            answer["result"]["content"][0]["text"],
            format!("text_refused: {message}"),
            "{tool}"
        );
    }

    // The routines over HTTP, and the verb `set_routines` itself. The
    // label of a routine is text too.
    let (status, answer) =
        send(h.put("/v1/routines", &bob_key, &json!({ "routines": [routine] }))).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));
    let labelled = json!({ "name": "badword", "when": { "type": "always" },
        "then": { "verb": "work", "args": { "ticks": 1 } } });
    let (status, answer) = h.act(&bob_key, "set_routines", json!({ "routines": [labelled] })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(
        answer["error"]["message"],
        "`args.routines[0].name`: The text holds a word that this server does not permit."
    );

    // Nothing of all this is in the world: no notice, no routine.
    h.step(3).await;
    assert_eq!(heard(&h, &ada_key).await, Vec::<String>::new());
    let (_, bob) = send(h.get("/v1/observe?detail=full&sections=self", &bob_key)).await;
    assert_eq!(bob["sections"]["self"]["routines"], json!([]));

    // The dashboard counts what the filters refused, by rule.
    let (status, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(status, StatusCode::OK, "{dashboard}");
    assert_eq!(dashboard["moderation"]["refused"], 7);
    assert_eq!(dashboard["moderation"]["refused_by_rule"], json!({ "word": 7 }));
}

/// A filter of another kind, as a classifier with a model would be: it
/// answers later, and it refuses with its own rule name.
struct Classifier;

impl TextFilter for Classifier {
    fn name(&self) -> &str {
        "classifier"
    }

    fn check<'a>(&'a self, text: &'a Text<'a>) -> FilterFuture<'a> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            let speech = matches!(text.usage, TextUse::Speech { verb: "tell" });
            if speech && text.text.contains("mean") {
                return Err(Refusal::new("toxic", "The text is unkind."));
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn another_filter_plugs_in_behind_the_rules() {
    let (sim, runtime) = unsup_runtime::start(market_world(), RuntimeConfig::manual());
    let store = Arc::new(MemoryStore::new());
    let gateway = Gateway::builder(sim.clone(), store.clone(), config(RULES))
        .filter(Arc::new(Classifier))
        .start()
        .await
        .expect("the gateway starts");
    let h = Harness::around(sim, runtime, store, gateway).await;
    let (ada, ada_key) = h.agent("Ada").await;

    let (status, answer) = tell(&h, &ada_key, ada, "you are mean").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(
        answer["error"],
        json!({ "code": "text_refused", "message": "`args.text`: The text is unkind." })
    );
    // The rules come first: their refusal is the answer.
    let (_, answer) = tell(&h, &ada_key, ada, "a mean badword").await;
    assert_eq!(
        answer["error"]["message"],
        "`args.text`: The text holds a word that this server does not permit."
    );
    // The classifier sees speech only: the same word in a name passes.
    let (_, token) = h.owner("Owner").await;
    let (status, _) = send(h.post("/v1/residents", &token, &json!({ "name": "Meanie" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = tell(&h, &ada_key, ada, "you are kind").await;
    assert_eq!(status, StatusCode::OK);

    let (_, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(dashboard["moderation"]["refused_by_rule"], json!({ "toxic": 1, "word": 1 }));
}

#[tokio::test]
async fn a_rule_that_cannot_be_used_stops_the_configuration() {
    let cases = [
        ("patterns = [\"*\"]", "`moderation.patterns`: the pattern `*` matches every text"),
        ("words = [\"!!!\"]", "`moderation.words` holds `!!!`, which has no letter and no digit"),
        ("links = \"maybe\"", "unknown variant `maybe`"),
    ];
    for (line, reason) in cases {
        let text = format!("[moderation]\n{line}");
        let error = unsup_gateway::GatewayConfig::from_toml(&text).expect_err("a refusal");
        assert!(error.0.contains(reason), "{line}: {}", error.0);
    }
}
