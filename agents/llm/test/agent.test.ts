// The loop of the agent against the fake world: observe, think, act, wait.

import assert from "node:assert/strict";
import { test } from "node:test";

import { ApiError } from "@unsupervised/sdk";
import { FakeActionError, type FakeVerb } from "@unsupervised/sdk/testing";

import { LlmAgent, ProviderError } from "../src/index.js";
import { answer, scene } from "./support/scene.js";

const EAT: FakeVerb = {
  info: {
    verb: "eat",
    module: "life",
    summary: "Eat one unit of a food item.",
    free: false,
    scope: "zone",
    args_schema: { type: "object", properties: { item: { type: "string" } }, required: ["item"] },
  },
  begin: () => ({ done: { eaten: 1 } }),
};

test("one turn observes, asks the model once, plans its actions and waits for the report", async () => {
  const { agent, provider, world, resident, paths, requestsTo } = scene({
    answers: [answer({ thought: "go east", actions: [{ verb: "move_to", args: { x: 35, y: 32 } }], wait_seconds: 120 })],
  });
  const result = await agent.turn();

  assert.equal(result.decided, true);
  assert.equal(provider.calls.length, 1);
  assert.equal(agent.decisions, 1);
  assert.equal(result.decision?.thought, "go east");
  assert.deepEqual(
    result.acks.map((ack) => [ack.verb, ack.status]),
    [["move_to", { state: "started", ends_at: 3, ticks: 3 }]],
  );
  assert.equal(result.waitedSeconds, 120);
  assert.deepEqual(paths(), ["GET /v1/catalogue", "GET /v1/observe", "POST /v1/act", "GET /v1/wait"]);
  const plan = requestsTo("/v1/act")[0]?.body as { plan: { verb: string; args: unknown }[] };
  assert.deepEqual(
    plan.plan.map((step) => [step.verb, step.args]),
    [["move_to", { x: 35, y: 32 }]],
  );
  // The report of the walk ended the wait after 3 ticks, not after 120.
  assert.equal(world.tick, 3);
  assert.deepEqual(resident.pos, { x: 35, y: 32 });
});

test("an early wake-up does not ask the model again before the least interval is over", async () => {
  const { agent, provider, world, paths, forget } = scene({
    answers: [answer({ actions: [{ verb: "move_to", args: { x: 35, y: 32 } }], wait_seconds: 120 })],
  });
  await agent.turn();
  forget();

  // Tick 3: the last decision was at tick 0 and the least interval is 30 seconds.
  const held = await agent.turn();
  assert.equal(held.decided, false);
  assert.equal(held.decision, undefined);
  assert.equal(held.waitedSeconds, 27);
  assert.equal(provider.calls.length, 1);
  assert.deepEqual(paths(), ["GET /v1/wait"]);
  assert.equal(world.tick, 30);

  const next = await agent.turn();
  assert.equal(next.decided, true);
  assert.equal(provider.calls.length, 2);
});

test("a plan of several actions costs one decision: a report in the middle does not end the wait", async () => {
  const { agent, provider, world, requestsTo } = scene({
    answers: [
      answer({ actions: [{ verb: "move_to", args: { x: 35, y: 32 } }, { verb: "wait", args: { ticks: 10 } }], wait_seconds: 600 }),
      answer({ wait_seconds: 60 }),
    ],
  });
  await agent.turn();
  // The walk ended at tick 3 and woke the long poll. The agent waited on for the second action.
  assert.equal(world.tick, 13);
  assert.equal(requestsTo("/v1/wait").length, 2);
  assert.equal(provider.calls.length, 1);

  // Tick 13 is too early for a decision. The next decision sees both reports.
  assert.equal((await agent.turn()).decided, false);
  await agent.turn();
  assert.deepEqual(
    provider.data(2)["reports"].map((report: { verb: string; ok: boolean; tick: number }) => [report.verb, report.ok, report.tick]),
    [
      ["move_to", true, 3],
      ["wait", true, 13],
    ],
  );
});

test("an action that fails in the middle of a plan ends the wait at once", async () => {
  const { agent, world } = scene({
    answers: [answer({ actions: [{ verb: "dig" }, { verb: "wait", args: { ticks: 50 } }], wait_seconds: 600 })],
  });
  world.addVerb({
    info: { verb: "dig", module: "economy", summary: "Dig.", free: false, scope: "zone", args_schema: { type: "object" } },
    begin: () => ({
      busy: 2,
      finish: () => {
        throw new FakeActionError("not_here", "There is no ground to dig here.");
      },
    }),
  });
  await agent.turn();
  // The second action runs until tick 52. The agent does not wait for it: the model must know of the failure.
  assert.equal(world.tick, 2);
});

test("a notice that the owner asked for ends the wait in the middle of a plan; other notices do not", async () => {
  const plan = answer({ actions: [{ verb: "listen" }, { verb: "wait", args: { ticks: 50 } }], wait_seconds: 600 });
  const listen = (world: ReturnType<typeof scene>["world"], to: number): FakeVerb => ({
    info: { verb: "listen", module: "society", summary: "Listen.", free: false, scope: "zone", args_schema: { type: "object" } },
    // A message arrives at the moment the action ends.
    begin: () => ({ busy: 2, finish: () => ({ heard: world.tell(to, 2, "Meet me at the shop.", "society.message") }) }),
  });

  const social = scene({ answers: [plan], agent: { wakeKinds: ["society.message", "report"] } });
  social.world.addVerb(listen(social.world, social.resident.id));
  await social.agent.turn();
  assert.equal(social.world.tick, 2);

  const quiet = scene({ answers: [plan] });
  quiet.world.addVerb(listen(quiet.world, quiet.resident.id));
  await quiet.agent.turn();
  assert.equal(quiet.world.tick, 52);
});

test("a turn without a decision waits its whole time, also when a report arrives", async () => {
  const { agent, provider, world, resident } = scene({
    answers: [answer({ actions: [{ verb: "wait", args: { ticks: 15 } }], wait_seconds: 60 })],
    agent: { decisionsPerHour: 1 },
  });
  await agent.turn();
  assert.equal(world.tick, 15);
  // An action ends during the next turn, which has no budget. A routine can do this.
  world.submit(resident, { verb: "wait", args: { ticks: 20 } });
  const held = await agent.turn();
  assert.equal(held.decided, false);
  // The decision of tick 0 is one hour old at tick 3600. The report of tick 35 did not end the turn.
  assert.equal(held.waitedSeconds, 3585);
  assert.equal(world.tick, 3600);
  assert.equal(provider.calls.length, 1);
});

test("an answer of the long poll without news ends the wait, also when the clock stands still", async () => {
  const { client, provider, notes, requestsTo } = scene({ answers: [answer({ wait_seconds: 120 })] });
  // A world that fails after a few long polls, so an agent that asks again and again fails the test.
  let waits = 0;
  const world = {
    catalogue: () => client.catalogue(),
    observe: () => client.observe(),
    plan: (steps: Parameters<typeof client.plan>[0]) => client.plan(steps),
    wait: (options: Parameters<typeof client.wait>[0]) => {
      waits += 1;
      if (waits > 3) throw new Error("the agent asks the server again and again");
      return client.wait(options);
    },
  };
  const agent = new LlmAgent({ world, provider, notes, now: () => 0 });
  const result = await agent.turn();
  assert.equal(result.waitedSeconds, 120);
  assert.equal(waits, 1);
  // The one call of the SDK made two long polls of 60 seconds.
  assert.equal(requestsTo("/v1/wait").length, 2);

  // The same for a turn without a decision: the clock says that no time passed, and the agent still stops.
  const held = await agent.turn();
  assert.equal(held.decided, false);
  assert.equal(held.waitedSeconds, 30);
  assert.equal(waits, 2);
  assert.equal(requestsTo("/v1/wait").length, 3);
});

test("the next prompt shows the report of the action, what the server said, and the new state", async () => {
  const { agent, provider } = scene({
    answers: [answer({ actions: [{ verb: "move_to", args: { x: 35, y: 32 } }], wait_seconds: 120 })],
  });
  await agent.turn();
  await agent.turn();
  await agent.turn();

  const first = provider.data(1);
  assert.equal(first["time"], "year 1 day 1 00:00, tick 0");
  assert.deepEqual(first["observation"].self.pos, { x: 32, y: 32 });
  assert.equal("reports" in first, false);
  assert.equal("last_turn" in first, false);
  assert.equal(first["turns_left_this_hour"], 11);

  const second = provider.data(2);
  assert.equal(second["time"], "year 1 day 1 00:30, tick 30");
  assert.deepEqual(second["observation"].self.pos, { x: 35, y: 32 });
  assert.deepEqual(second["reports"], [{ verb: "move_to", ok: true, tick: 3, result: { x: 35, y: 32 } }]);
  assert.deepEqual(second["last_turn"], { actions: [{ verb: "move_to", state: "started" }] });
  assert.equal(second["turns_left_this_hour"], 10);
  // The section `reports` of the observation is not shown a second time.
  assert.equal("reports" in second["observation"], false);
});

test("the budget bounds the requests to the model in every hour", async () => {
  const { agent, provider, world } = scene({
    answers: [answer({ wait_seconds: 60 })],
    agent: { decisionsPerHour: 3 },
  });
  const callTicks: number[] = [];
  provider.onCall = () => callTicks.push(world.tick);

  const decided: boolean[] = [];
  for (let turn = 0; turn < 12; turn += 1) decided.push((await agent.turn()).decided);

  // Three decisions one minute apart, then nothing until the hour is over.
  assert.deepEqual(callTicks, [0, 60, 120, 3600, 3660, 3720, 7200, 7260, 7320]);
  assert.deepEqual(decided, [true, true, true, false, true, true, true, false, true, true, true, false]);
  for (let i = 0; i + 3 < callTicks.length; i += 1) {
    assert.ok((callTicks[i + 3] as number) - (callTicks[i] as number) >= 3600, `more than 3 decisions in the hour from tick ${callTicks[i]}`);
  }
  // Tick 10800: the decision of tick 7200 is one hour old, so one is free again.
  assert.equal(world.tick, 10_800);
  assert.equal(agent.budgetLeft, 1);
});

test("a turn without budget only blocks in long polls of the server", async () => {
  const { agent, provider, world, gateway, forget } = scene({
    answers: [answer({ wait_seconds: 600 })],
    agent: { decisionsPerHour: 2 },
  });
  await agent.turn();
  await agent.turn();
  assert.equal(world.tick, 1200);
  forget();

  const held = await agent.turn();
  assert.equal(held.decided, false);
  // The first decision is one hour old at tick 3600.
  assert.equal(held.waitedSeconds, 2400);
  assert.equal(world.tick, 3600);
  assert.equal(provider.calls.length, 2);
  // 40 minutes of waiting are 40 long polls and nothing else.
  assert.equal(gateway.requests.length, 40);
  for (const request of gateway.requests) {
    assert.equal(`${request.method} ${request.path}`, "GET /v1/wait");
    assert.equal(request.query["timeout"], "60");
  }

  assert.equal((await agent.turn()).decided, true);
  assert.equal(provider.calls.length, 3);
});

test("the wait of the model is long: one request to the model for hours of game time", async () => {
  const { agent, provider, world, requestsTo } = scene({ answers: [answer({ wait_seconds: 99_999 })] });
  const result = await agent.turn();
  // The greatest wait is one hour.
  assert.equal(result.waitedSeconds, 3600);
  assert.equal(world.tick, 3600);
  assert.equal(provider.calls.length, 1);
  assert.equal(requestsTo("/v1/wait").length, 60);
});

test("only an action report ends a wait early, unless the owner asks for other kinds", async () => {
  const quiet = scene({ answers: [answer({ wait_seconds: 60 })] });
  await quiet.agent.turn();
  assert.deepEqual(
    quiet.requestsTo("/v1/wait").map((request) => request.query["kinds"]),
    ["report"],
  );

  const social = scene({ answers: [answer({ wait_seconds: 60 })], agent: { wakeKinds: ["society.message", "report"] } });
  await social.agent.turn();
  assert.deepEqual(
    social.requestsTo("/v1/wait").map((request) => request.query["kinds"]),
    ["society.message,report"],
  );

  const all = scene({ answers: [answer({ wait_seconds: 60 })], agent: { wakeKinds: [] } });
  await all.agent.turn();
  assert.deepEqual(
    all.requestsTo("/v1/wait").map((request) => "kinds" in request.query),
    [false],
  );
});

test("news is shown to the model once", async () => {
  const { agent, provider, world, resident } = scene({ answers: [answer({ wait_seconds: 60 })] });
  world.tell(resident.id, 2, "The shop sells bread.");
  world.notify(resident.id, "economy.wage_paid", { amount: 600 });
  await agent.turn();
  await agent.turn();

  assert.deepEqual(provider.data(1)["news"], [
    { kind: "society.speech", tick: 0, from: 2, data: { untrusted: true, from: 2, text: "The shop sells bread." } },
    { kind: "economy.wage_paid", tick: 0, data: { amount: 600 } },
  ]);
  assert.equal("news" in provider.data(2), false);
});

test("an answer that is not JSON plans nothing, and the next prompt says so", async () => {
  const { agent, provider, sentVerbs, world } = scene({
    answers: ["I will walk to the shop now.", answer({ actions: [{ verb: "wait", args: { ticks: 5 } }], wait_seconds: 60 })],
  });
  const first = await agent.turn();
  assert.equal(first.decided, true);
  assert.equal(first.decision, undefined);
  assert.deepEqual(first.acks, []);
  assert.deepEqual(sentVerbs(), []);
  // The agent waits the default time, then asks again.
  assert.equal(first.waitedSeconds, 300);
  assert.equal(world.tick, 300);

  await agent.turn();
  assert.deepEqual(provider.data(2)["last_turn"], { error: "Your last answer had no JSON object." });
  assert.deepEqual(sentVerbs(), ["wait"]);
});

test("an error of the model plans nothing and costs one decision of the budget", async () => {
  const { agent, provider, sentVerbs, world } = scene({
    answers: [new ProviderError("The model endpoint answered HTTP 500: down", { status: 500 }), answer({ wait_seconds: 60 })],
    agent: { decisionsPerHour: 5 },
  });
  const first = await agent.turn();
  assert.equal(first.decided, true);
  assert.equal(first.decision, undefined);
  assert.equal(first.waitedSeconds, 300);
  assert.equal(world.tick, 300);
  assert.deepEqual(sentVerbs(), []);
  assert.equal(agent.budgetLeft, 4);

  const second = await agent.turn();
  assert.equal(second.decision?.waitSeconds, 60);
  assert.equal(provider.calls.length, 2);
  // An error of the model is not the fault of the model: the prompt does not blame it.
  assert.equal("last_turn" in provider.data(2), false);
});

test("an error that is not from the model or the server is thrown", async () => {
  const { agent } = scene({ answers: [new TypeError("a bug in the provider")] });
  await assert.rejects(agent.turn(), TypeError);
});

test("a refused action is in the next prompt with its error code, and a result that came at once too", async () => {
  const { agent, provider } = scene({
    answers: [
      answer({ actions: [{ verb: "fly" }, { verb: "wait", args: { ticks: 0 } }, { verb: "cancel" }], wait_seconds: 60 }),
      answer({ wait_seconds: 60 }),
    ],
  });
  const first = await agent.turn();
  assert.deepEqual(
    first.acks.map((ack) => ack.status.state),
    ["rejected", "rejected", "done"],
  );
  await agent.turn();
  assert.deepEqual(provider.data(2)["last_turn"], {
    actions: [
      { verb: "fly", state: "rejected", error: "unknown_verb: No verb is named `fly`." },
      { verb: "wait", state: "rejected", error: "invalid_args: `ticks` must be from 1 to 1440." },
      { verb: "cancel", state: "done", result: { cancelled: null, cleared: 0 } },
    ],
  });
});

test("the model gets at most `maxActions` actions for each turn", async () => {
  const steps = Array.from({ length: 6 }, () => ({ verb: "wait", args: { ticks: 1 } }));
  const { agent, sentVerbs } = scene({ answers: [answer({ actions: steps, wait_seconds: 60 })], agent: { maxActions: 4 } });
  const result = await agent.turn();
  assert.deepEqual(sentVerbs(), ["wait", "wait", "wait", "wait"]);
  assert.deepEqual(
    result.acks.map((ack) => ack.status.state),
    ["started", "queued", "queued", "queued"],
  );
});

test("a note of the model is kept and shown in later prompts", async () => {
  const { agent, provider, notes } = scene({
    answers: [answer({ note: "The shelter is at 7,7.\nIt is free.", wait_seconds: 60 }), answer({ wait_seconds: 60 })],
  });
  await agent.turn();
  // One note is one line, with the game time of the decision.
  assert.deepEqual(notes.notes, ["[year 1 day 1 0:00] The shelter is at 7,7. It is free."]);
  assert.equal("notes" in provider.data(1), false);

  await agent.turn();
  assert.deepEqual(provider.data(2)["notes"], ["[year 1 day 1 0:00] The shelter is at 7,7. It is free."]);
  // An answer without a note adds none.
  assert.equal(notes.notes.length, 1);
});

test("a verb that the model asks about gets its signature in the next prompt", async () => {
  const verbs = Array.from({ length: 30 }, (_unused, index): FakeVerb => {
    const name = `craft_${String(index).padStart(2, "0")}`;
    return {
      info: { verb: name, module: "economy", summary: `Make product ${index}.`, free: false, scope: "zone", args_schema: { type: "object", properties: { qty: { type: "integer" } }, required: ["qty"] } },
      begin: () => ({ done: null }),
    };
  });
  const { agent, provider, world } = scene({
    answers: [answer({ describe: ["craft_29"], wait_seconds: 60 }), answer({ wait_seconds: 60 })],
  });
  for (const verb of verbs) world.addVerb(verb);
  await agent.turn();
  await agent.turn();

  const wanted = "craft_29(qty: int): Make product 29.";
  const first = provider.data(1);
  assert.equal(first["verbs"].length, 20);
  assert.equal(first["verbs"].includes(wanted), false);
  assert.equal(first["other_verbs"].economy.includes("craft_29"), true);

  const second = provider.data(2);
  assert.equal(second["verbs"][0], wanted);
  assert.equal(second["verbs"].length, 20);
  assert.equal(second["other_verbs"].economy.includes("craft_29"), false);
});

test("the agent reads the catalogue again after `catalogueEvery` decisions and uses new verbs", async () => {
  const { agent, provider, world, requestsTo } = scene({
    answers: [answer({ wait_seconds: 60 })],
    agent: { catalogueEvery: 2 },
  });
  await agent.turn();
  world.addVerb(EAT);
  await agent.turn();
  assert.equal(requestsTo("/v1/catalogue").length, 1);
  assert.equal(provider.data(2)["verbs"].includes("eat(item: str): Eat one unit of a food item."), false);

  await agent.turn();
  assert.equal(requestsTo("/v1/catalogue").length, 2);
  assert.equal(provider.data(3)["verbs"].includes("eat(item: str): Eat one unit of a food item."), true);
});

test("`run` makes the asked number of turns", async () => {
  const { agent, provider, world } = scene({ answers: [answer({ wait_seconds: 60 })] });
  await agent.run({ turns: 3 });
  assert.equal(provider.calls.length, 3);
  assert.equal(world.tick, 180);
});

test("an error of the server does not stop `run`: the agent waits and observes again", async () => {
  const { agent, provider, gateway, sleeps, requestsTo } = scene({ answers: [answer({ wait_seconds: 60 })] });
  // The client makes two attempts. Both fail.
  gateway.failNext({ status: 503, times: 2, when: (request) => request.path === "/v1/wait" });
  await agent.run({ turns: 2 });

  assert.deepEqual(sleeps, [30_000]);
  // The first turn asked the model; its wait failed. The second turn observed again and held back.
  assert.equal(provider.calls.length, 1);
  assert.equal(requestsTo("/v1/observe").length, 2);
});

test("a key that the server refuses stops `run` with the error of the server", async () => {
  const { agent, provider, world, resident, sleeps } = scene();
  world.rotateKey(resident);
  await assert.rejects(agent.run({ turns: 3 }), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["unauthorized", 401]);
    return true;
  });
  assert.deepEqual(sleeps, []);
  assert.equal(provider.calls.length, 0);
});

// The time limit turns a run that does not end into a failed test.
test("`run` ends without an error when the caller aborts during the long poll", { timeout: 10_000 }, async () => {
  const { agent, provider, world, gateway } = scene({ world: { time: "manual" }, answers: [answer({ wait_seconds: 600 })] });
  const controller = new AbortController();
  const running = agent.run({ signal: controller.signal });
  await world.parked();
  assert.equal(gateway.requests.at(-1)?.path, "/v1/wait");
  controller.abort();
  await running;
  assert.equal(provider.calls.length, 1);

  // A run with a signal that is aborted already makes no turn.
  await agent.run({ signal: controller.signal });
  assert.equal(provider.calls.length, 1);
});
