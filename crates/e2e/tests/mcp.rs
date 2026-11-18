//! An agent over MCP, with a small client in the test: the handshake,
//! the tool list, each tool, and the first day of a newcomer through
//! tool calls. A tool result is text, as a language model reads it.

mod common;

use common::game::{Catalogue, Map};
use common::mcp::{Mcp, SESSION, STATELESS, block, line, number, report_of};
use common::{World, id};
use reqwest::StatusCode;
use serde_json::{Value, json};

/// The six tools, in the order of the protocol document.
const TOOLS: [&str; 6] = ["observe", "act", "plan", "wait", "describe", "set_routines"];

/// Calls the tool `wait` until the report of an action is there. The
/// world runs while the tool waits. Returns the report and the text of
/// the tool result that had it.
async fn wait_for(world: &World, mcp: &mut Mcp, action: i64) -> (String, String) {
    for _ in 0..8 {
        let news =
            world.until(mcp.tool("wait", json!({ "timeout": 50, "kinds": ["report"] }))).await;
        if let Some(report) = report_of(&news, action) {
            return (report, news);
        }
    }
    panic!("the report of the action {action} came");
}

/// Calls `act` for an action that takes game time, then waits for its
/// report. Returns the report.
async fn act_and_wait(world: &World, mcp: &mut Mcp, verb: &str, args: Value) -> String {
    let text = mcp.tool("act", json!({ "verb": verb, "args": args })).await;
    let ack = line(&text, "ack:");
    assert!(ack.contains(&format!("verb: {verb}")), "{ack}");
    assert!(!ack.contains("state: rejected") && !ack.contains("state: done"), "{ack}");
    let action = number(ack, "action_id").expect("the acknowledgement has the action id");
    let (report, _) = wait_for(world, mcp, action).await;
    assert!(report.contains("ok: true"), "`{verb}` went well:\n{report}");
    report
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newcomer_lives_its_first_day_over_mcp() {
    let world = World::start().await;
    let (resident, key) = world.newcomer("Ada").await;

    // The handshake.
    let (mut mcp, hello) = Mcp::initialize(&world, &key).await;
    assert_eq!(hello["protocolVersion"], SESSION);
    assert_eq!(hello["serverInfo"]["name"], "unsupervised");
    assert_eq!(hello["capabilities"], json!({ "tools": {} }));
    let instructions = hello["instructions"].as_str().expect("instructions for the model");
    assert!(instructions.contains("untrusted"), "the instructions name untrusted text");
    assert_eq!(mcp.result("ping", json!({})).await, json!({}));

    // The tool list.
    let tools = mcp.tools().await;
    let names: Vec<&str> =
        tools.iter().map(|tool| tool["name"].as_str().expect("a name")).collect();
    assert_eq!(names, TOOLS);
    for tool in &tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(tool["description"].as_str().is_some_and(|text| !text.is_empty()), "{tool}");
    }

    // `describe`: the verbs of the real game, and the arguments of one.
    let catalogue = Catalogue::read(&world).await;
    let verbs = mcp.tool("describe", json!({})).await;
    for verb in catalogue.verbs() {
        assert!(verbs.contains(&format!(" {verb}")), "`describe` lists `{verb}`");
    }
    assert!(verbs.contains("list_jobs (free)"), "a free verb is marked:\n{verbs}");
    let help = mcp.tool("describe", json!({ "verb": "buy" })).await;
    assert!(help.starts_with("buy (economy): "), "{help}");
    for argument in ["item", "qty", "max_price"] {
        assert!(help.contains(&format!("  {argument} (")), "`buy` takes `{argument}`:\n{help}");
    }
    let unknown = mcp.call("describe", json!({ "verb": "fly" })).await;
    assert!(unknown.is_error, "{}", unknown.text);

    // `observe`: a newcomer.
    let seen = mcp.tool("observe", json!({})).await;
    assert!(seen.starts_with("time: year 1, day 1, "), "{seen}");
    let me = block(&seen, "self");
    assert!(me.contains("  name: Ada\n"), "{me}");
    assert_eq!(number(me, "id"), Some(resident as i64), "{me}");
    assert!(seen.contains("news: none"), "a newcomer has no news:\n{seen}");
    let cash = number(me, "cash").expect("the observation shows the cash");
    let city = number(me, "city").expect("the observation shows the city");
    let place = line(me, "pos:");
    let (x, y) = (number(place, "x").expect("x"), number(place, "y").expect("y"));
    let brief = mcp.tool("observe", json!({ "sections": ["self"] })).await;
    assert!(!brief.contains("nearby:"), "only the section that was asked for:\n{brief}");

    // Find paid work. `list_jobs` is free; its answer is a report.
    let text = mcp.tool("act", json!({ "verb": "list_jobs" })).await;
    assert!(line(&text, "ack:").contains("state: pending"), "{text}");
    let listing = number(line(&text, "ack:"), "action_id").expect("an action id");
    let (jobs, _) = wait_for(&world, &mut mcp, listing).await;
    assert!(jobs.contains("verb: list_jobs"), "{jobs}");
    let wage = number(&jobs, "wage").expect("the starter wage");
    let minutes = number(&jobs, "minutes").expect("the length of a starter shift");
    assert!(jobs.contains("open: true"), "{jobs}");
    let kinds = line(&jobs, "kinds:");
    let map = Map::read(&world, city as u64).await;
    let depot = map.nearest(&["depot"], (x, y)).id;
    assert!(kinds.contains("depot"), "the depot has starter shifts: {kinds}");

    // Walk to the depot and work one shift.
    let walked = act_and_wait(&world, &mut mcp, "move_to", json!({ "site": depot })).await;
    assert_eq!(number(&walked, "site"), Some(depot as i64), "{walked}");
    let text = mcp.tool("act", json!({ "verb": "work", "args": {} })).await;
    let ack = line(&text, "ack:");
    assert!(ack.contains("state: started"), "{ack}");
    assert_eq!(number(ack, "ticks"), Some(minutes), "{ack}");
    // The fresh observation of `act` shows the running action.
    assert!(text.contains("verb: work"), "{text}");
    let action = number(ack, "action_id").expect("an action id");
    let (paid, news) = wait_for(&world, &mut mcp, action).await;
    assert_eq!(number(&paid, "wage"), Some(wage), "{paid}");
    let net = number(&paid, "net").expect("the pay after tax");
    assert_eq!(number(&paid, "tax"), Some(wage - net), "{paid}");
    // The result of `wait` is an observation: it shows the new cash.
    assert_eq!(number(block(&news, "self"), "cash"), Some(cash + net), "the pay is in the pocket");

    // `plan`: buy food and eat it. The price comes from the depot.
    let prices = mcp.tool("observe", json!({ "detail": "full", "sections": ["business"] })).await;
    let (food, price) = prices
        .lines()
        .filter_map(|line| {
            let item = line.split("item: ").nth(1)?.split([',', '}']).next()?.trim();
            catalogue.nutrition(item)?;
            Some((item.to_string(), number(line, "price")?))
        })
        .min_by_key(|(item, price)| (*price, item.clone()))
        .expect("the depot sells food");
    assert!(price <= cash + net, "one shift pays for the cheapest food ({food}, {price})");
    let text = mcp
        .tool(
            "plan",
            json!({ "actions": [
                { "verb": "buy", "args": { "item": food, "qty": 1 } },
                { "verb": "eat", "args": { "item": food } },
            ] }),
        )
        .await;
    assert!(line(&text, "ack 1:").contains("verb: buy"), "{text}");
    let second = line(&text, "ack 2:");
    assert!(second.contains("state: queued") && second.contains("position: 1"), "{second}");
    let buy = number(line(&text, "ack 1:"), "action_id").expect("an action id");
    let eat = number(second, "action_id").expect("an action id");
    // The purchase can end before the answer of `plan` is written. Then
    // its report is in the fresh observation of that answer.
    let bought = match report_of(&text, buy) {
        Some(report) => report,
        None => wait_for(&world, &mut mcp, buy).await.0,
    };
    assert_eq!(number(&bought, "total"), Some(price), "{bought}");
    let (eaten, _) = wait_for(&world, &mut mcp, eat).await;
    assert_eq!(number(&eaten, "nutrition"), catalogue.nutrition(&food), "{eaten}");
    let after = mcp.tool("observe", json!({ "sections": ["self"] })).await;
    assert_eq!(number(block(&after, "self"), "cash"), Some(cash + net - price), "{after}");
    assert!(after.contains("news: none"), "each report came one time:\n{after}");

    // A plan stops at the first action that the server rejects. That is
    // a tool result with `isError`, so the model can correct it.
    let stopped = mcp
        .call(
            "plan",
            json!({ "actions": [
                { "verb": "wait", "args": { "ticks": 1 } },
                { "verb": "fly" },
                { "verb": "wait", "args": { "ticks": 1 } },
            ] }),
        )
        .await;
    assert!(stopped.is_error, "{}", stopped.text);
    assert!(block(&stopped.text, "ack 2").contains("code: unknown_verb"), "{}", stopped.text);
    assert!(
        stopped.text.contains("The server stopped at action 2. It did not send the 1 after it."),
        "{}",
        stopped.text
    );
    let rejected = mcp.call("act", json!({ "verb": "fly", "args": {} })).await;
    assert!(rejected.is_error, "{}", rejected.text);
    let ack = block(&rejected.text, "ack");
    assert!(ack.contains("state: rejected") && ack.contains("code: unknown_verb"), "{ack}");

    // `set_routines`.
    let routine = json!({
        "name": "sleep at night",
        "when": { "type": "clock_between", "from_hour": 22, "to_hour": 6 },
        "then": { "verb": "sleep", "args": {} },
    });
    let set = mcp.tool("set_routines", json!({ "routines": [routine] })).await;
    assert_eq!(set, "routines set: 1\n");
    let full = mcp.tool("observe", json!({ "detail": "full", "sections": ["self"] })).await;
    assert!(full.contains("sleep at night"), "full detail shows the routine:\n{full}");
    let bad = json!({ "name": "fly", "when": { "type": "always" }, "then": { "verb": "fly" } });
    let refused = mcp.call("set_routines", json!({ "routines": [bad] })).await;
    assert!(
        refused.is_error && refused.text.starts_with("rejected: unknown_verb"),
        "{}",
        refused.text
    );
    assert_eq!(mcp.tool("set_routines", json!({ "routines": [] })).await, "routines set: 0\n");

    // An unknown tool is a JSON-RPC error, not a tool result.
    let answer = mcp.rpc("tools/call", json!({ "name": "teleport", "arguments": {} })).await;
    assert_eq!(answer.body["error"]["code"], -32602, "{}", answer.body);

    // The end of the session.
    assert_eq!(mcp.end().await, StatusCode::NO_CONTENT);
    let gone = mcp.rpc("tools/list", json!({})).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.body);
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_version_without_a_session_has_the_same_tools() {
    let world = World::start().await;
    let (_, key) = world.newcomer("Ada").await;
    let mut mcp = Mcp::stateless(&world, &key);

    let found = mcp.result("server/discover", json!({})).await;
    let versions = found["supportedVersions"].as_array().expect("versions");
    assert_eq!(versions[0], STATELESS, "{found}");
    assert!(versions.iter().any(|version| version == SESSION), "{found}");
    assert_eq!(found["resultType"], "complete");

    let tools = mcp.tools().await;
    let names: Vec<&str> =
        tools.iter().map(|tool| tool["name"].as_str().expect("a name")).collect();
    assert_eq!(names, TOOLS);

    // The notice cursor belongs to the resident key: each report comes
    // one time, also without a session.
    let text = mcp.tool("act", json!({ "verb": "wait", "args": { "ticks": 2 } })).await;
    let action = number(line(&text, "ack:"), "action_id").expect("an action id");
    let news = world.until(mcp.tool("wait", json!({ "kinds": ["report"] }))).await;
    let report = report_of(&news, action).expect("the wait ended with the report");
    assert!(report.contains("verb: wait") && report.contains("ok: true"), "{report}");
    let quiet = mcp.tool("observe", json!({ "sections": ["self"] })).await;
    assert!(quiet.contains("news: none"), "{quiet}");

    // A request without the version headers is refused with a message
    // that says what to send.
    let message = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let bare = world
        .request(reqwest::Method::POST, "/mcp", &key)
        .json(&message)
        .send()
        .await
        .expect("the endpoint answers");
    assert_eq!(bare.status(), StatusCode::BAD_REQUEST);
    let body: Value = bare.json().await.expect("JSON");
    assert_eq!(id(&body["id"], "id"), 1);
    assert!(
        body["error"]["message"].as_str().is_some_and(|text| text.contains("MCP-Protocol-Version")),
        "{body}"
    );
    // The endpoint opens no event stream.
    let stream = world.get("/mcp", &key).await;
    assert_eq!(stream.0, StatusCode::METHOD_NOT_ALLOWED, "{}", stream.1);
    world.stop().await;
}
