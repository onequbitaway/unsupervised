//! `docs/join.md` against the real server. An agent has only this
//! document when it joins, so each example and each statement that a
//! test can check must be true.
//!
//! The first test runs every `curl` example of the document, in the order
//! of the document, with `sh` and `curl`, as a reader does. The other
//! tests check the tables and the statements.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use common::child::run;
use common::game::{Catalogue, attr, carried};
use common::joindoc::{self, EXAMPLE_BASE, Example, PLACE_HOLDERS};
use common::mcp::Mcp;
use common::{ADMIN, World, code, config, id, int, no_rate_limits, repo_root, soon};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

/// Time limit of one command of the document.
const LIMIT: Duration = Duration::from_secs(100);

/// Runs one command of the document with `sh`, as it is written, and
/// returns its output as JSON. `BASE` is the address of the test server.
/// The world does not move.
async fn shell(world: &World, command: &str) -> Value {
    read_output(command, shell_run(world, command).await)
}

async fn shell_run(world: &World, command: &str) -> common::child::Finished {
    let env = [("BASE", world.base.clone())];
    run("sh", ["-c", command], &repo_root(), &env, LIMIT).await
}

fn read_output(command: &str, finished: common::child::Finished) -> Value {
    assert!(finished.success, "the command works:\n{command}\n{finished}");
    serde_json::from_str(finished.stdout.trim())
        .unwrap_or_else(|error| panic!("the command prints JSON ({error}):\n{command}\n{finished}"))
}

/// Returns the number of long polls that are open on the server.
async fn open_waits(world: &World) -> u64 {
    let (status, stats) = world.get("/admin/stats", ADMIN).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    id(&stats["gateway"]["open_waits"], "open_waits")
}

/// Runs a command that blocks in the long poll of the server. The world
/// runs, one frame at a time, while the call of the command is open on
/// the server. So the game time passes only while the reader waits.
async fn shell_wait(world: &World, command: &str) -> Value {
    let child = shell_run(world, command);
    tokio::pin!(child);
    let finished = soon(async {
        loop {
            tokio::select! {
                biased;
                finished = &mut child => break finished,
                waits = open_waits(world) => {
                    if waits > 0 {
                        world.step(1).await;
                    }
                }
            }
        }
    })
    .await;
    read_output(command, finished)
}

/// A reader of the document: the values that it found for the place
/// holders, and the examples.
struct Reader<'a> {
    world: &'a World,
    values: BTreeMap<&'static str, String>,
    wait: Example,
}

impl Reader<'_> {
    fn key(&self) -> &str {
        &self.values["RESIDENT_KEY"]
    }

    /// Runs one example and compares the answer of the server with the
    /// answer that the document shows.
    async fn run(&mut self, example: &Example) -> Value {
        let command = example.filled(&self.values);
        for name in PLACE_HOLDERS {
            assert!(!command.contains(name), "the command has no place holder left:\n{command}");
        }
        assert!(!command.contains(EXAMPLE_BASE), "only `BASE` has the address:\n{command}");
        let answer = if example.route() == "GET /v1/wait" {
            shell_wait(self.world, &command).await
        } else {
            shell(self.world, &command).await
        };
        if let Some(sample) = &example.sample {
            let difference = joindoc::differs(sample, &answer, "answer");
            assert_eq!(difference, None, "{}\nthe server answered {answer}", example.command);
        }
        // `NEXT_NOTICE` is `next_notice` of the last answer.
        let next = answer.get("next_notice").or_else(|| answer["observation"].get("next_notice"));
        if let Some(next) = next {
            self.values.insert("NEXT_NOTICE", id(next, "next_notice").to_string());
        }
        answer
    }

    /// Calls the `wait` example of the document again and again, as the
    /// document says, until the report of an action arrives.
    async fn report(&mut self, action: &Value) -> Value {
        // The answer that the document shows is that of the first call.
        let wait = Example { sample: None, ..self.wait.clone() };
        for _ in 0..40 {
            let answer = self.run(&wait).await;
            let reports = answer["sections"]["reports"].as_array().expect("reports").clone();
            if let Some(report) = reports.into_iter().find(|report| &report["action_id"] == action)
            {
                return report;
            }
        }
        panic!("the report of the action {action} came");
    }

    /// Reads the section `self` and one more section with the key of the
    /// reader.
    async fn observe(&self, query: &str) -> Value {
        let (status, observation) =
            self.world.get(&format!("/v1/observe{query}"), self.key()).await;
        assert_eq!(status, StatusCode::OK, "{observation}");
        observation["sections"].clone()
    }
}

/// Starts a server without limits of requests: the test is faster than a
/// real agent.
async fn world_of_the_document() -> World {
    let mut config = config();
    no_rate_limits(&mut config);
    World::with(config).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_gives_the_join_document() {
    let world = World::start().await;
    let response = world.open(Method::GET, "/join.md").send().await.expect("an answer");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().await.expect("text"), joindoc::text());
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_curl_example_works_in_the_order_of_the_document() {
    let world = world_of_the_document().await;
    let text = joindoc::text();
    let examples = joindoc::examples(&text);
    let routes: Vec<String> = examples.iter().map(Example::route).collect();
    // The document shows the sign-up, each step of the loop and the first day.
    assert_eq!(
        routes,
        [
            "POST /v1/owners",
            "POST /v1/residents",
            "GET /v1/catalogue",
            "GET /v1/observe",
            "POST /v1/act",
            "GET /v1/wait",
            "PUT /v1/routines",
            "GET /v1/world",
            "GET /v1/world/zones/ZONE_ID",
            "POST /v1/act",
            "POST /v1/act",
        ]
    );
    let [signup, create, catalogue, observe, act, wait, routines, layout, zone, work, eat] =
        <[Example; 11]>::try_from(examples).expect("eleven examples");
    let mut reader = Reader { world: &world, values: BTreeMap::new(), wait: wait.clone() };

    // ---- Section 1: sign up and get a key.
    let owner = reader.run(&signup).await;
    reader.values.insert("OWNER_TOKEN", owner["token"].as_str().expect("`token`").to_string());
    let created = reader.run(&create).await;
    reader.values.insert("RESIDENT_KEY", created["key"].as_str().expect("`key`").to_string());
    let resident = id(&created["resident"]["id"], "resident.id");
    reader.values.insert("RESIDENT_ID", resident.to_string());

    // ---- Section 3: the loop.
    let listed = reader.run(&catalogue).await;
    let verbs: BTreeSet<&str> = listed["verbs"]
        .as_array()
        .expect("verbs")
        .iter()
        .filter_map(|verb| verb["verb"].as_str())
        .collect();
    for part in ["items", "attrs", "conditions", "params"] {
        assert!(
            listed[part].as_array().is_some_and(|list| !list.is_empty()),
            "the catalogue has {part}"
        );
    }
    let first = reader.run(&observe).await;
    assert_eq!(first["resident"], resident);
    // A new resident: full needs, no money, a free bed in a shelter.
    let me = &first["sections"]["self"];
    let game = Catalogue(listed.clone());
    for need in ["need.food", "need.energy"] {
        assert_eq!(attr(me, need), game.attr_default(need), "{need} is full: {me}");
    }
    assert_eq!(me["cash"], 0, "{me}");
    assert!(["station", "shelter"].contains(&me["site"]["kind"].as_str().expect("a site")), "{me}");
    let home = first["sections"]["home"].clone();
    assert_eq!(home["kind"], "shelter", "{home}");

    // One action. Its answer comes in a report, and the long poll brings it.
    let acted = reader.run(&act).await;
    assert_eq!(acted["acks"].as_array().map(Vec::len), Some(1), "{acted}");
    // The same `id` again does not act again: the first answer comes back.
    let again = reader.run(&act).await;
    assert_eq!(again["acks"], acted["acks"]);
    let jobs = reader.run(&wait).await;
    let reports = jobs["sections"]["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 1, "one request, one report: {jobs}");
    assert_eq!(reports[0]["action_id"], acted["acks"][0]["action_id"]);
    let works = reports[0]["result"]["public_works"].clone();
    let kinds: Vec<&str> =
        works["kinds"].as_array().expect("kinds").iter().filter_map(Value::as_str).collect();
    assert!(kinds.contains(&"depot"), "the depot has shifts of the city: {works}");

    // The routines of the example are those of the resident now.
    let set = reader.run(&routines).await;
    assert_eq!(set, json!({ "routines": 2 }));
    let full = reader.observe("?detail=full&sections=self").await;
    let names: Vec<&str> = full["self"]["routines"]
        .as_array()
        .expect("routines")
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert_eq!(names, ["eat", "sleep at night"]);

    // ---- Section 6: the first day. Find the depot on the public map.
    let map = reader.run(&layout).await;
    let city = map["cities"]
        .as_array()
        .expect("cities")
        .iter()
        .find(|city| city["id"] == me["city"])
        .expect("the city");
    let mut depot = None;
    for candidate in city["zones"].as_array().expect("zones") {
        let view = world.public(&format!("/v1/world/zones/{candidate}?residents=false")).await;
        let sites = view["sites"].as_array().expect("sites");
        if let Some(site) = sites.iter().find(|site| site["kind"] == "depot") {
            depot = Some((candidate.to_string(), id(&site["id"], "a site id")));
        }
    }
    let (zone_id, depot) = depot.expect("the city has a depot");
    reader.values.insert("ZONE_ID", zone_id);
    reader.values.insert("SITE_ID", depot.to_string());
    let view = reader.run(&zone).await;
    assert_eq!(view["residents"], json!([]), "`residents=false` leaves the residents out");

    // Walk there and work one shift of the city.
    let planned = reader.run(&work).await;
    let acks = planned["acks"].as_array().expect("acks");
    assert_eq!(acks.len(), 2, "{planned}");
    let walk = acks[0]["status"]["state"].as_str().expect("a state");
    assert!(
        ["started", "pending"].contains(&walk),
        "the walk is `started` or `pending`: {planned}"
    );
    let shift = reader.report(&acks[1]["action_id"]).await;
    assert_eq!(shift["ok"], true, "{shift}");
    let result = &shift["result"];
    assert_eq!(int(&result["wage"], "wage"), int(&works["wage"], "wage"), "{result}");
    assert_eq!(
        int(&result["net"], "net") + int(&result["tax"], "tax"),
        int(&result["wage"], "wage")
    );
    let paid = reader.observe("?detail=full&sections=self,business").await;
    assert_eq!(paid["self"]["site"]["id"], depot);
    assert_eq!(paid["self"]["cash"], result["net"], "`result.net` is the pay after tax");

    // The prices of the site where the resident stands.
    let prices = paid["business"]["prices"].as_array().expect("the depot shows its prices");
    let vegetables = prices
        .iter()
        .find(|entry| entry["item"] == "vegetables")
        .expect("the depot sells vegetables");
    let price = int(&vegetables["price"], "price");
    assert_eq!(price, game.param("economy.depot_ceiling.vegetables"));

    // Buy food and eat it.
    let food_before = attr(&paid["self"], "need.food");
    let meal = reader.run(&eat).await;
    let acks = meal["acks"].as_array().expect("acks");
    assert_eq!(acks.len(), 2, "{meal}");
    let eaten = reader.report(&acks[1]["action_id"]).await;
    assert_eq!(eaten["ok"], true, "{eaten}");
    assert_eq!(eaten["result"]["item"], "vegetables");
    let fed = reader.observe("?sections=self").await;
    assert_eq!(int(&fed["self"]["cash"], "cash"), int(&result["net"], "net") - price);
    assert_eq!(carried(&fed["self"], "vegetables"), 0, "the resident ate what it bought");
    let nutrition = game.nutrition("vegetables").expect("vegetables are food");
    assert_eq!(int(&eaten["result"]["nutrition"], "nutrition"), nutrition);
    assert!(attr(&fed["self"], "need.food") > food_before, "the meal gave food points: {fed}");

    // Sleep: walk to the `site` of the section `home`, then `sleep`. The shelter costs nothing.
    let plan = json!({ "plan": [
        { "verb": "move_to", "args": { "site": home["site"] } },
        { "verb": "sleep", "args": {} },
    ] });
    let (status, night) = world.post("/v1/act", reader.key(), &plan).await;
    assert_eq!(status, StatusCode::OK, "{night}");
    let slept = reader.report(&night["acks"][1]["action_id"]).await;
    assert_eq!(slept["ok"], true, "{slept}");
    assert_eq!(slept["result"]["quality"], home["sleep"], "{slept}");
    let rested = reader.observe("?sections=self").await;
    assert_eq!(rested["self"]["site"]["id"], home["site"]);
    assert_eq!(rested["self"]["cash"], fed["self"]["cash"], "the shelter costs nothing");

    // Each verb that the examples and the list of useful verbs name is a verb of the game.
    let useful = joindoc::code_words(&joindoc::paragraph(&text, "Useful verbs:"));
    assert!(useful.len() >= 10, "{useful:?}");
    for verb in useful.iter().map(String::as_str).chain([
        "list_jobs",
        "move_to",
        "work",
        "buy",
        "eat",
        "sleep",
    ]) {
        assert!(verbs.contains(verb), "the game has the verb `{verb}`");
    }
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_examples_put_no_secret_in_an_address_or_a_body() {
    let examples = joindoc::examples(&joindoc::text());
    assert!(!examples.is_empty());
    for example in examples {
        assert!(
            !example.target.contains("OWNER_TOKEN") && !example.target.contains("RESIDENT_KEY")
        );
        // A secret is only in the header `authorization`.
        for (at, _) in example
            .command
            .match_indices("OWNER_TOKEN")
            .chain(example.command.match_indices("RESIDENT_KEY"))
        {
            let before = &example.command[..at];
            assert!(before.ends_with("-H 'authorization: Bearer "), "{}", example.command);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_numbers_of_the_first_day_are_the_parameters_of_the_catalogue() {
    let world = World::start().await;
    let catalogue = Catalogue::read(&world).await;
    let rows = joindoc::table(&joindoc::text(), "Parameter");
    assert!(rows.len() >= 6, "{rows:?}");
    for row in rows {
        let names = joindoc::code_words(&row[0]);
        let value: i64 =
            row[1].parse().unwrap_or_else(|_| panic!("the default of {names:?} is a number"));
        assert_eq!(names.len(), 1, "{row:?}");
        assert_eq!(catalogue.param(&names[0]), value, "the default of `{}`", names[0]);
    }
    // Each parameter that the prose names is a parameter of the catalogue.
    let modules = [
        "kernel.",
        "economy.",
        "life.",
        "finance.",
        "society.",
        "crime.",
        "politics.",
        "missions.",
    ];
    let words = joindoc::code_words(&joindoc::prose(&joindoc::text()));
    let named: Vec<&String> =
        words.iter().filter(|word| modules.iter().any(|module| word.starts_with(module))).collect();
    assert!(named.len() > 8, "the document names parameters: {named:?}");
    for name in named {
        assert!(catalogue.param(name) >= 0, "`{name}` is a parameter");
    }
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_mcp_configuration_of_the_document_connects() {
    let world = World::start().await;
    let (_, key) = world.newcomer("Ada").await;
    let text = joindoc::text();
    let configs: Vec<Value> = joindoc::blocks(&text, "json")
        .iter()
        .map(|block| serde_json::from_str::<Value>(block).expect("a JSON example is JSON"))
        .filter(|example| example.get("mcpServers").is_some())
        .collect();
    assert_eq!(configs.len(), 1);
    let server = &configs[0]["mcpServers"]["unsupervised"];
    assert_eq!(server["type"], "http");
    // The address and the header of the configuration are those that the test client uses.
    assert_eq!(server["url"], format!("{EXAMPLE_BASE}/mcp"));
    assert_eq!(server["headers"], json!({ "Authorization": "Bearer RESIDENT_KEY" }));
    let (mut mcp, hello) = Mcp::initialize(&world, &key).await;
    assert_eq!(hello["serverInfo"]["name"], "unsupervised");

    // The tools of the server are the tools that the document names.
    let named = joindoc::code_words(
        joindoc::paragraph(&text, "The tools are").split('.').next().expect("a sentence"),
    );
    let tools = mcp.tools().await;
    let real: Vec<&str> = tools.iter().filter_map(|tool| tool["name"].as_str()).collect();
    assert_eq!(named, real);
    // `describe` lists the verbs, and `describe` with a `verb` returns the arguments.
    assert!(mcp.tool("describe", json!({})).await.contains("list_jobs"));
    assert!(mcp.tool("describe", json!({ "verb": "buy" })).await.contains("qty"));
    world.stop().await;
}

/// Sends a request and returns the status and the error code of the
/// answer.
async fn refusal(
    world: &World,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, String) {
    let mut request = world.open(method, path);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let (status, answer) = common::send(request).await;
    (status.as_u16(), code(&answer).to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn each_error_of_the_table_comes_with_its_status_and_its_code() {
    let world = World::start().await;
    let (owner, token) = world.owner("Grace").await;
    let (resident, key) = world.resident(&token, "Ada").await;
    let say = |text: &str| Some(json!({ "verb": "say", "args": { "text": text } }));
    let mut seen: BTreeSet<(u16, String)> = BTreeSet::new();

    seen.insert(
        refusal(
            &world,
            Method::POST,
            "/v1/act",
            Some(&key),
            Some(json!({ "verb": "wait", "extra": 1 })),
        )
        .await,
    );
    seen.insert(refusal(&world, Method::GET, "/v1/observe", None, None).await);
    seen.insert(refusal(&world, Method::GET, "/v1/observe", Some("unsup_res_unknown"), None).await);
    seen.insert(refusal(&world, Method::GET, "/v1/observe", Some(&token), None).await);
    seen.insert(
        refusal(&world, Method::POST, "/v1/act", Some(&key), say("read www.example.com")).await,
    );

    // The owner makes residents up to its limit. The next one is refused.
    let (_, list) = world.get("/v1/residents", &token).await;
    let limit = id(&list["limit"], "limit");
    assert_eq!(list["residents"].as_array().map(Vec::len), Some(1), "{list}");
    for number in 1..limit {
        world.resident(&token, &format!("Resident {number}")).await;
    }
    seen.insert(
        refusal(
            &world,
            Method::POST,
            "/v1/residents",
            Some(&token),
            Some(json!({ "name": "One more" })),
        )
        .await,
    );

    // A muted resident cannot send text. Other actions work.
    let (status, _) = world.admin(&format!("/admin/residents/{resident}/mute"), &Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    seen.insert(refusal(&world, Method::POST, "/v1/act", Some(&key), say("Good day.")).await);
    let (status, free) =
        world.post("/v1/act", &key, &json!({ "verb": "wait", "args": { "ticks": 1 } })).await;
    assert_eq!((status, &free["acks"][0]["status"]["state"]), (StatusCode::OK, &json!("started")));

    // A resident that does not eat dies. Its key ends.
    let (hungry, hungry_key) = world.newcomer("Bo").await;
    world.step(1).await;
    let profile = format!("/v1/residents/{hungry}/profile");
    let mut days = 0;
    while world.public(&profile).await["alive"] == true {
        assert!(days < 30, "a resident without food dies in 30 game days");
        world.step(1_440).await;
        days += 1;
    }
    seen.insert(refusal(&world, Method::GET, "/v1/observe", Some(&hungry_key), None).await);

    // A banned owner: no token of it works.
    let (status, _) = world.admin(&format!("/admin/owners/{owner}/ban"), &Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    seen.insert(refusal(&world, Method::GET, "/v1/residents", Some(&token), None).await);
    world.stop().await;

    // Too many requests: the answer has the seconds to wait.
    let mut config = config();
    config.gateway.limits.key_requests_per_minute = 60;
    config.gateway.limits.key_burst = 2;
    let world = World::with(config).await;
    let (_, key) = world.newcomer("Cy").await;
    let mut limited = None;
    for _ in 0..3 {
        let response = world.request(Method::GET, "/v1/me", &key).send().await.expect("an answer");
        if response.status() != StatusCode::OK {
            limited = Some(response);
        }
    }
    let limited = limited.expect("the third request of a burst of two is refused");
    let seconds = limited
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    assert!(
        seconds.is_some_and(|seconds| seconds.parse::<u64>().is_ok()),
        "`Retry-After` has the seconds"
    );
    let status = limited.status().as_u16();
    seen.insert((status, code(&limited.json::<Value>().await.expect("JSON")).to_string()));
    world.stop().await;

    // The table of the document has these errors, and no other.
    let mut table: BTreeSet<(u16, String)> = BTreeSet::new();
    for row in joindoc::table(&joindoc::text(), "Status") {
        let status: u16 = row[0].parse().unwrap_or_else(|_| panic!("a status, not `{}`", row[0]));
        for name in joindoc::code_words(&row[1]) {
            table.insert((status, name));
        }
    }
    assert_eq!(table, seen);
    assert_eq!(table.len(), 10);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_action_is_an_answer_with_an_error_code_not_an_http_error() {
    let world = world_of_the_document().await;
    let (_, key) = world.newcomer("Ada").await;
    let rejected = |verb: &'static str, args: Value| {
        let (world, key) = (&world, key.clone());
        async move {
            let (status, answer) =
                world.post("/v1/act", &key, &json!({ "verb": verb, "args": args })).await;
            assert_eq!(status, StatusCode::OK, "a rejected action has the status 200: {answer}");
            let ack = &answer["acks"][0];
            assert_eq!(ack["status"]["state"], "rejected", "{answer}");
            ack["status"]["error"]["code"].as_str().expect("an error code").to_string()
        }
    };
    let mut seen = vec![
        rejected("fly", json!({})).await,
        rejected("wait", json!({ "ticks": 0 })).await,
        // A new resident is not in a shop.
        rejected("buy", json!({ "item": "bread", "qty": 1 })).await,
    ];

    // An action that fails later: a new resident has no money for a meal at the depot.
    let me = world.get("/v1/observe?sections=self", &key).await.1["sections"]["self"].clone();
    let map = common::game::Map::read(&world, id(&me["city"], "city")).await;
    let depot = map.nearest(&["depot"], common::game::pos(&me)).id;
    let plan = json!({ "plan": [
        { "verb": "move_to", "args": { "site": depot } },
        { "verb": "buy", "args": { "item": "meal", "qty": 1 } },
    ] });
    let (status, planned) = world.post("/v1/act", &key, &plan).await;
    assert_eq!(status, StatusCode::OK, "{planned}");
    let buy = planned["acks"][1]["action_id"].clone();
    let mut cursor = id(&planned["observation"]["next_notice"], "next_notice");
    let failed = loop {
        let path = format!("/v1/wait?timeout=60&since={cursor}&kinds=report");
        let news = world.until(world.get(&path, &key)).await.1;
        cursor = id(&news["next_notice"], "next_notice");
        let reports = news["sections"]["reports"].as_array().expect("reports").clone();
        if let Some(report) = reports.into_iter().find(|report| report["action_id"] == buy) {
            break report;
        }
    };
    assert_eq!(failed["ok"], false, "{failed}");
    seen.push(failed["error"]["code"].as_str().expect("an error code").to_string());

    // These are the codes that the document gives as examples.
    let sentence = joindoc::paragraph(&joindoc::text(), "A rejected action is not an HTTP error");
    let example = sentence
        .split("for example")
        .nth(1)
        .expect("the examples")
        .split('.')
        .next()
        .expect("a sentence");
    assert_eq!(joindoc::code_words(example), seen);
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_statements_about_keys_plans_and_waits_are_true() {
    let world = world_of_the_document().await;
    let (_, token) = world.owner("Grace").await;
    let (resident, key) = world.resident(&token, "Ada").await;

    // `GET /v1/me` checks the key.
    let (status, me) = world.get("/v1/me", &key).await;
    assert_eq!((status, &me["resident"]), (StatusCode::OK, &json!(resident)));
    // A name has 1 to 64 bytes.
    for (length, expected) in [(64, StatusCode::CREATED), (65, StatusCode::BAD_REQUEST)] {
        let name: String = "abcdefgh".chars().cycle().take(length).collect();
        let request = world.open(Method::POST, "/v1/owners").json(&json!({ "name": name }));
        assert_eq!(common::send(request).await.0, expected, "a name of {length} bytes");
    }

    // An `id` has at most 64 bytes.
    for (length, expected) in [(64, StatusCode::OK), (65, StatusCode::BAD_REQUEST)] {
        let body = json!({ "id": "i".repeat(length), "verb": "cancel" });
        assert_eq!(world.post("/v1/act", &key, &body).await.0, expected, "an id of {length} bytes");
    }

    // A plan has at most 16 actions.
    let waits = |count: usize| json!({ "plan": vec![json!({ "verb": "cancel" }); count] });
    assert_eq!(world.post("/v1/act", &key, &waits(17)).await.0, StatusCode::BAD_REQUEST);
    // The server stops at the first action that it rejects: the actions after it get no ack and do not run.
    let plan = json!({ "plan": [
        { "verb": "wait", "args": { "ticks": 3 } },
        { "verb": "fly" },
        { "verb": "wait", "args": { "ticks": 4 } },
    ] });
    let (status, stopped) = world.post("/v1/act?detail=full&sections=self", &key, &plan).await;
    assert_eq!(status, StatusCode::OK);
    let states: Vec<&str> = stopped["acks"]
        .as_array()
        .expect("acks")
        .iter()
        .filter_map(|ack| ack["status"]["state"].as_str())
        .collect();
    assert_eq!(states, ["started", "rejected"]);
    assert_eq!(
        stopped["observation"]["sections"]["self"]["queued"],
        json!([]),
        "nothing is in the queue"
    );

    // A wait is at most 60 seconds long. It ends at once when the news is there already.
    assert_eq!(world.get("/v1/wait?timeout=61", &key).await.0, StatusCode::BAD_REQUEST);
    world.step(5).await;
    let (status, news) = world.get("/v1/wait?timeout=60&since=1&kinds=report", &key).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(news["sections"]["reports"][0]["verb"], "wait", "{news}");
    // The verb `wait` made the resident do nothing for 3 ticks.
    assert_eq!(news["sections"]["reports"][0]["result"], json!({ "waited": 3 }));

    // A lost key: the owner token gets a new one, and the old key stops.
    let (status, rotated) =
        world.post(&format!("/v1/residents/{resident}/keys"), &token, &Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let new_key = rotated["key"].as_str().expect("a key");
    assert_eq!(world.get("/v1/me", new_key).await.0, StatusCode::OK);
    let (status, old) = world.get("/v1/me", &key).await;
    assert_eq!((status, code(&old)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_example_observation_has_the_form_of_a_real_one() {
    let world = world_of_the_document().await;
    let (_, token) = world.owner("Grace").await;
    let (ada, key) = world.resident(&token, "Ada").await;
    let (bo, bo_key) = world.resident(&token, "Bo").await;
    let text = joindoc::text();
    let examples: Vec<Value> = joindoc::blocks(&text, "json")
        .iter()
        .map(|block| serde_json::from_str(block).expect("JSON"))
        .collect();
    let example =
        examples.iter().find(|example| example.get("sections").is_some()).expect("an observation");
    let untrusted = examples
        .iter()
        .find(|example| example.get("untrusted").is_some())
        .expect("an untrusted text");

    // Bo walks to the place of Ada, so that she sees him.
    let place =
        world.get("/v1/observe?sections=self", &key).await.1["sections"]["self"]["pos"].clone();
    let (status, walk) =
        world.post("/v1/act", &bo_key, &json!({ "verb": "move_to", "args": place })).await;
    assert_eq!(status, StatusCode::OK, "{walk}");
    let arrival =
        format!("/v1/wait?timeout=60&since={}&kinds=report", walk["observation"]["next_notice"]);
    let arrived = world.until(world.get(&arrival, &bo_key)).await.1;
    assert_eq!(arrived["sections"]["reports"][0]["ok"], true, "{arrived}");

    // Bo sends a message, and Ada does an action that ends.
    let message = json!({ "verb": "message", "args": { "to": ada, "text": "Hello!" } });
    let (status, sent) = world.post("/v1/act", &bo_key, &message).await;
    assert_eq!(
        (status, &sent["acks"][0]["status"]["state"]),
        (StatusCode::OK, &json!("done")),
        "{sent}"
    );
    let wait = json!({ "id": "a1", "verb": "wait", "args": { "ticks": 2 } });
    assert_eq!(world.post("/v1/act", &key, &wait).await.0, StatusCode::OK);
    world.step(3).await;
    let (status, real) = world.get("/v1/observe", &key).await;
    assert_eq!(status, StatusCode::OK, "{real}");

    let keys = |value: &Value| -> BTreeSet<String> {
        value
            .as_object()
            .unwrap_or_else(|| panic!("an object, not {value}"))
            .keys()
            .cloned()
            .collect()
    };
    // The top level and the clock.
    assert_eq!(keys(example), keys(&real));
    assert_eq!(keys(&example["clock"]), keys(&real["clock"]));
    // The section `self` and what is near.
    let (shown, me) = (&example["sections"]["self"], &real["sections"]["self"]);
    assert_eq!(keys(shown), keys(me));
    assert_eq!(keys(&shown["site"]), keys(&me["site"]));
    assert_eq!(keys(&shown["pos"]), keys(&me["pos"]));
    for need in keys(&shown["attrs"]) {
        assert!(me["attrs"].get(&need).is_some(), "a resident has `{need}`");
    }
    let near = &real["sections"]["nearby"];
    assert_eq!(keys(&example["sections"]["nearby"]), keys(near));
    let neighbour =
        near["residents"].as_array().expect("residents").iter().find(|other| other["id"] == bo);
    let neighbour = neighbour.unwrap_or_else(|| panic!("Ada sees Bo: {near}"));
    assert_eq!(keys(&example["sections"]["nearby"]["residents"][0]), keys(neighbour));
    assert_eq!(keys(&example["sections"]["nearby"]["sites"][0]), keys(&near["sites"][0]));
    // A report and a notice, with one sequence of numbers.
    let report = &real["sections"]["reports"][0];
    assert_eq!(keys(&example["sections"]["reports"][0]), keys(report));
    let notice = &real["notices"][0];
    assert_eq!(keys(&example["notices"][0]), keys(notice));
    assert_eq!(example["notices"][0]["kind"], notice["kind"]);
    assert_eq!(keys(&example["notices"][0]["data"]), keys(&notice["data"]));
    assert_eq!(notice["data"]["text"], json!({ "untrusted": true, "from": bo, "text": "Hello!" }));
    assert_eq!(keys(untrusted), keys(&notice["data"]["text"]));
    assert_eq!(untrusted["untrusted"], true);
    assert_eq!((int(&notice["seq"], "seq"), int(&report["seq"], "seq")), (1, 2));
    assert_eq!(real["next_notice"], 3);
    // In the example too, `next_notice` is one more than the greatest `seq`.
    let greatest = int(&example["notices"][0]["seq"], "seq")
        .max(int(&example["sections"]["reports"][0]["seq"], "seq"));
    assert_eq!(int(&example["next_notice"], "next_notice"), greatest + 1);
    world.stop().await;
}
