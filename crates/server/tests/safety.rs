//! Moderation and abuse controls of a whole server, with the real game
//! modules: speech, gifts, shifts and the feed for viewers.

mod common;

use common::{ADMIN, Client, SHIFT_PAY, advance, config, earn, send};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use unsup_gateway::{AdminToken, GatewayConfig};
use unsup_server::{Server, ServerConfig};

/// A server with the rules of `data/defaults.toml` for text, an admin
/// token, and the gateway values in `overrides`.
async fn server(overrides: &str) -> (Server, Client) {
    let mut config = config();
    let text = format!("admin_token = \"{ADMIN}\"\n{overrides}");
    let mut table: toml::Table = toml::from_str(&text).expect("TOML");
    // The policy of the server for text, as `ServerConfig::default()`
    // has it, below the values of the test.
    let defaults: toml::Table = toml::from_str(unsup_server::DEFAULTS).expect("the defaults");
    let mut gateway = defaults["gateway"].as_table().expect("a table").clone();
    unsup_gateway::merge_tables(&mut gateway, std::mem::take(&mut table));
    config.gateway = GatewayConfig::from_table(gateway).expect("a gateway configuration");
    assert_eq!(config.gateway.admin_token, Some(AdminToken::new(ADMIN).expect("a long token")));
    let server = unsup_server::start(config).await.expect("the server starts");
    let client = Client::of(&server);
    (server, client)
}

/// Sends one action and returns the status and the answer.
async fn try_act(client: &Client, key: &str, verb: &str, args: Value) -> (StatusCode, Value) {
    send(client.post("/v1/act", &json!({ "verb": verb, "args": args })).bearer_auth(key)).await
}

/// Returns the public events of the feed whose kind starts with `kind`.
async fn feed(client: &Client, kind: &str) -> Vec<Value> {
    let (status, page) = send(client.get(&format!("/v1/feed?kind={kind}&limit=500"))).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page["events"].as_array().expect("events").clone()
}

/// Returns the notices of a resident.
async fn notices(client: &Client, key: &str) -> Vec<Value> {
    let (status, seen) = send(client.get("/v1/observe?since=1").bearer_auth(key)).await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    seen["notices"].as_array().expect("notices").clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_server_has_the_rules_of_a_public_world() {
    let policy = ServerConfig::default().gateway.moderation;
    assert_eq!((policy.max_repeat, policy.texts_per_minute, policy.text_burst), (12, 30, 10));
    assert_eq!(policy.links, unsup_gateway::LinkRule::Refuse);
    // The gateway alone has no such rules.
    let neutral = GatewayConfig::default().moderation;
    assert_eq!((neutral.max_repeat, neutral.texts_per_minute), (0, 0));
    assert_eq!(neutral.links, unsup_gateway::LinkRule::Allow);
    assert_eq!(policy.text_verbs, ["say", "message", "org_say", "post", "org_create"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn moderated_text_is_refused_and_never_reaches_a_notice_or_an_event() {
    let (server, client) =
        server("[moderation]\nwords = [\"scamcoin\"]\npatterns = [\"double your*money\"]").await;
    let token = client.sign_up("Ada").await;
    let (_, speaker) = client.resident(&token, "Speaker").await;
    let (listener_id, listener) = client.resident(&token, "Listener").await;
    // Both go to the depot and earn there: the speaker has the fee of an
    // organization, and the listener is in earshot.
    earn(&server, &client, &[&speaker, &listener]).await;

    let refused = [
        ("say", json!({ "text": "buy SCAMCOIN now" }), "word"),
        ("say", json!({ "text": "Double your\nmoney today" }), "pattern"),
        ("say", json!({ "text": "see https://example.com/win" }), "link"),
        ("say", json!({ "text": "visit example.org" }), "link"),
        ("say", json!({ "text": "wooooooooooooooow" }), "repeated"),
        ("post", json!({ "text": "scamcoin for everybody" }), "word"),
        ("message", json!({ "to": listener_id, "text": "psst, scamcoin" }), "word"),
        ("org_create", json!({ "name": "Scamcoin Traders" }), "word"),
        ("org_create", json!({ "name": "www.example.com" }), "link"),
    ];
    for (verb, args, rule) in &refused {
        let (status, answer) = try_act(&client, &speaker, verb, args.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{verb} {args}: {answer}");
        assert_eq!(answer["error"]["code"], "text_refused", "{verb} {args}");
        let message = answer["error"]["message"].as_str().expect("a message");
        let expected = match *rule {
            "word" => "The text holds a word that this server does not permit.",
            "pattern" => "The text matches a pattern that this server does not permit.",
            "link" => "The text holds a link. This server takes no links.",
            _ => "The text repeats one character more than 12 times in a row.",
        };
        let field = if *verb == "org_create" { "name" } else { "text" };
        assert_eq!(message, format!("`args.{field}`: {expected}"), "{verb} {args}");
    }
    // The names of residents pass the same filter.
    let named =
        client.post("/v1/residents", &json!({ "name": "Scamcoin Sam" })).bearer_auth(&token);
    let (status, answer) = send(named).await;
    assert_eq!(
        (status, answer["error"]["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("text_refused"))
    );

    // Clean text of every kind goes through.
    assert_eq!(client.accepted(&speaker, "say", json!({ "text": "good morning" })).await, "done");
    assert_eq!(
        client
            .accepted(
                &speaker,
                "message",
                json!({ "to": listener_id, "text": "a word in private" })
            )
            .await,
        "done"
    );
    assert_eq!(
        client.accepted(&speaker, "post", json!({ "text": "bread is cheap today" })).await,
        "started"
    );
    advance(&server, 10).await;
    // The organization is made in the global realm: the answer comes
    // in a report.
    assert_eq!(
        client.accepted(&speaker, "org_create", json!({ "name": "Honest Traders" })).await,
        "pending"
    );
    advance(&server, 20).await;

    // The listener heard the clean speech and nothing else.
    let heard: Vec<(String, String)> = notices(&client, &listener)
        .await
        .iter()
        .map(|notice| {
            let text = notice["data"]["text"]["text"].as_str().expect("untrusted text");
            (notice["kind"].as_str().expect("a kind").to_string(), text.to_string())
        })
        .collect();
    assert_eq!(
        heard,
        [
            ("society.said".to_string(), "good morning".to_string()),
            ("society.message".to_string(), "a word in private".to_string()),
        ]
    );
    // The feed for viewers has the clean speech, the clean post and the
    // organization, and not one refused text.
    let society = feed(&client, "society.").await;
    let shown: Vec<(&str, &Value)> = society
        .iter()
        .map(|event| {
            let data = &event["data"];
            let text = if data["text"].is_object() {
                &data["text"]["text"]
            } else {
                &data["name"]["text"]
            };
            (event["kind"].as_str().expect("a kind"), text)
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("society.said", &json!("good morning")),
            ("society.posted", &json!("bread is cheap today")),
            ("society.org_created", &json!("Honest Traders")),
        ]
    );
    let everything = serde_json::to_string(&feed(&client, "").await).expect("JSON").to_lowercase();
    for word in ["scamcoin", "example", "double your", "wooo"] {
        assert!(!everything.contains(word), "the feed holds {word:?}");
    }
    let dashboard = client.dashboard().await;
    assert_eq!(
        dashboard["moderation"]["refused_by_rule"],
        json!({ "link": 3, "pattern": 1, "repeated_character": 1, "word": 5 })
    );
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resident_cannot_flood_the_square() {
    let (server, client) = server("[moderation]\ntexts_per_minute = 6\ntext_burst = 3").await;
    let token = client.sign_up("Ada").await;
    let (_, key) = client.resident(&token, "Loud").await;
    for n in 0..3 {
        let text = format!("hear me, number {n}");
        assert_eq!(client.accepted(&key, "say", json!({ "text": text })).await, "done");
        advance(&server, 1).await;
    }
    let (status, answer) = try_act(&client, &key, "say", json!({ "text": "and again" })).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{answer}");
    assert_eq!(
        answer,
        json!({ "error": { "code": "rate_limited",
            "message": "Too many texts. Try again in 10 seconds." } })
    );
    // Other actions go on.
    assert_eq!(client.accepted(&key, "wait", json!({ "ticks": 1 })).await, "started");
    let said = feed(&client, "society.said").await;
    assert_eq!(said.len(), 3);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn young_residents_that_give_their_pay_to_one_receiver_are_flagged() {
    let limits = "[limits]\nresidents_per_owner = 0\n[abuse.funnel]\nmin_givers = 3";
    let (server, client) = server(limits).await;
    let boss_token = client.sign_up("Boss").await;
    let (boss_id, boss) = client.resident(&boss_token, "Boss").await;
    let farm_token = client.sign_up("Farmer").await;
    let mut mules = Vec::new();
    for name in ["Mule A", "Mule B", "Mule C"] {
        mules.push(client.resident(&farm_token, name).await);
    }
    let trader_token = client.sign_up("Trader").await;
    let (trader_id, trader) = client.resident(&trader_token, "Trader").await;
    let (friend_id, friend) = client.resident(&trader_token, "Friend").await;

    // Everybody works one starter shift at the depot.
    let mut workers: Vec<&str> = mules.iter().map(|(_, key)| key.as_str()).collect();
    workers.extend([boss.as_str(), trader.as_str(), friend.as_str()]);
    earn(&server, &client, &workers).await;

    let watched = |client: &Client| {
        let request = client.get("/admin/flags").bearer_auth(ADMIN);
        async move {
            let (status, flags) = send(request).await;
            assert_eq!(status, StatusCode::OK, "{flags}");
            flags
        }
    };
    // Waits until the detector saw the frame, then returns the flags.
    let flags_at = |frame: u64| {
        let (client, watched) = (&client, &watched);
        async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                let flags = watched(client).await;
                if flags["watched_frame"] == frame {
                    return flags["flags"].clone();
                }
                assert!(std::time::Instant::now() < deadline, "the detector is late: {flags}");
                tokio::task::yield_now().await;
            }
        }
    };

    // Normal trade: the trader buys grain at the depot, and gives a part
    // of it to one friend, who gives money back. Two residents, both
    // ways: that is no funnel.
    assert_eq!(
        client.accepted(&trader, "buy", json!({ "item": "grain", "qty": 2 })).await,
        "started"
    );
    advance(&server, 10).await;
    assert_eq!(client.myself(&trader).await["items"]["grain"], 2);
    let gift = json!({ "to": friend_id, "item": "grain", "qty": 1 });
    assert_eq!(client.accepted(&trader, "give", gift).await, "started");
    let pay = json!({ "to": trader_id, "money": 150 });
    assert_eq!(client.accepted(&friend, "give", pay).await, "started");
    let frame = advance(&server, 5).await;
    assert_eq!(flags_at(frame).await, json!([]));
    let sold = feed(&client, "economy.goods_sold").await;
    assert_eq!(sold.len(), 1, "the purchase is a public event, and not a gift");

    // The funnel: each mule gives its whole pay to the boss.
    for (index, (_, key)) in mules.iter().enumerate() {
        let all = json!({ "to": boss_id, "money": SHIFT_PAY });
        assert_eq!(client.accepted(key, "give", all).await, "started");
        let frame = advance(&server, 3).await;
        let flags = flags_at(frame).await;
        assert_eq!(flags.as_array().map(Vec::len), Some(usize::from(index == 2)), "{flags}");
    }
    let flags = watched(&client).await["flags"].clone();
    let flag = &flags[0];
    assert_eq!((flag["kind"].clone(), flag["receiver"].clone()), (json!("funnel"), json!(boss_id)));
    assert_eq!((flag["money"].clone(), flag["goods"].clone()), (json!(3 * SHIFT_PAY), json!(0)));
    assert_eq!(flag["receiver_owner"], 1);
    let givers: Vec<(&Value, &Value, &Value)> = flag["givers"]
        .as_array()
        .expect("givers")
        .iter()
        .map(|giver| (&giver["resident"], &giver["owner"], &giver["money"]))
        .collect();
    let expected: Vec<(Value, Value, Value)> =
        mules.iter().map(|(id, _)| (json!(id), json!(2), json!(SHIFT_PAY))).collect();
    assert_eq!(givers, expected.iter().map(|(a, b, c)| (a, b, c)).collect::<Vec<_>>());
    assert_eq!(flag["evidence"].as_array().map(Vec::len), Some(3));
    // Every giver was young: it appeared less than two game days ago.
    for giver in flag["givers"].as_array().expect("givers") {
        assert!(
            giver["age_minutes"].as_u64().is_some_and(|age| age > 300 && age < 2_880),
            "{giver}"
        );
    }

    // The detector flags and does nothing else: the boss has the money,
    // nobody has a sanction, and every key works.
    assert_eq!(client.cash(&boss).await, 4 * SHIFT_PAY);
    let (_, sanctions) = client.admin(Method::GET, "/admin/sanctions", None).await;
    assert_eq!(sanctions, json!({ "sanctions": [] }));
    for (_, key) in &mules {
        assert_eq!(client.accepted(key, "wait", json!({ "ticks": 1 })).await, "started");
    }
    assert_eq!(client.dashboard().await["abuse"]["open_flags"], 1);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_ups_of_one_address_and_residents_of_one_owner_have_limits() {
    let limits = "[limits]\nsignups_per_hour = 60\nsignup_burst = 3\nresidents_per_owner = 2";
    let (server, client) = server(limits).await;
    let mut tokens = Vec::new();
    for name in ["One", "Two", "Three"] {
        tokens.push(client.sign_up(name).await);
    }
    let fourth =
        client.post("/v1/owners", &json!({ "name": "Four" })).send().await.expect("answer");
    assert_eq!(fourth.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(fourth.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("60"));
    let answer: Value = fourth.json().await.expect("JSON");
    assert_eq!(answer["error"]["code"], "rate_limited");
    let (_, stats) = client.admin(Method::GET, "/admin/stats", None).await;
    assert_eq!(stats["store"]["owners"], 3, "the fourth owner was not made");

    // An owner has two residents at most.
    client.resident(&tokens[0], "A").await;
    client.resident(&tokens[0], "B").await;
    let third = client.post("/v1/residents", &json!({ "name": "C" })).bearer_auth(&tokens[0]);
    let (status, answer) = send(third).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::FORBIDDEN,
            json!({ "error": { "code": "resident_limit",
                "message": "An owner has at most 2 residents in the world at a time." } })
        )
    );
    // The limit counts for each owner.
    client.resident(&tokens[1], "A of Two").await;
    assert_eq!(server.sim.stats().await.expect("stats").residents, 3);
    server.stop().await.expect("the server stops");
}
