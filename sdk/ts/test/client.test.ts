// The HTTPS client against the fake server: sign-up, actions, the notice
// cursor, the long poll, routines and the webhook.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  ActionRejectedError,
  ApiError,
  Client,
  MissingCredentialError,
  assertAccepted,
  hasNews,
  PLAN_MAX,
  rejection,
  reportsOf,
  selfOf,
  WEBHOOK_SIGNATURE_HEADER,
  WEBHOOK_TIMESTAMP_HEADER,
  signWebhook,
  verifyWebhook,
  untrustedOf,
  verifyWebhookRequest,
  type ActionAck,
} from "../src/index.js";
import { FakeGateway } from "../src/testing/index.js";
import { scene } from "./support/setup.js";
import { startServer } from "./support/server.js";

test("a new agent signs up, creates a resident and checks its key", async () => {
  const server = await startServer();
  try {
    const client = new Client({ baseUrl: server.url });
    const signup = await client.signUp({ name: "Grace" });
    assert.deepEqual(signup.owner, { id: 1, name: "Grace", created_at: 1_800_000_000 });
    assert.match(signup.token, /^unsup_own_[0-9a-f]{64}$/);
    assert.equal(client.ownerToken, signup.token);

    const created = await client.createResident({ name: "Ada", model: "scripted" });
    const record = { id: 1, name: "Ada", model: "scripted", created_at: 1_800_000_000, key_hint: `${created.key.slice(0, 16)}...` };
    assert.deepEqual(created.resident, record);
    assert.match(created.key, /^unsup_res_[0-9a-f]{64}$/);
    assert.equal(client.residentKey, created.key);
    assert.equal(client.resident, 1);
    assert.deepEqual(await client.me(), { resident: 1, name: "Ada", model: "scripted", owner: 1 });
    assert.deepEqual(await client.listResidents(), { residents: [record], limit: 3 });

    // The server has the name and the model that the client sent.
    assert.equal(server.world.residents.get(1)?.model, "scripted");
    const sent = server.gateway.requests.find((request) => request.path === "/v1/residents");
    assert.deepEqual(sent?.body, { name: "Ada", model: "scripted" });
    assert.equal(sent?.token, signup.token);

    // An owner has a limit of residents in the world.
    await client.createResident({ name: "Bo" });
    await client.createResident({ name: "Cy" });
    await assert.rejects(client.createResident({ name: "Di" }), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.deepEqual([error.code, error.status], ["resident_limit", 403]);
      assert.equal(error.endsCredential, false);
      return true;
    });
    assert.equal(server.world.residents.size, 3);
    // A name that the text filters refuse makes no resident.
    server.world.limits.residents_per_owner = 0;
    await assert.rejects(client.createResident({ name: "visit www.example.com" }), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.deepEqual([error.code, error.status, error.isTextRefusal], ["text_refused", 422, true]);
      return true;
    });
    assert.equal(server.world.residents.size, 3);
  } finally {
    await server.close();
  }
});

test("`Client.join` does the whole sign-up in one call", async () => {
  const server = await startServer();
  try {
    const joined = await Client.join(server.url, { name: "Ada", ownerName: "Grace" });
    assert.equal(joined.owner, 1);
    assert.match(joined.ownerToken, /^unsup_own_/);
    assert.deepEqual(server.gateway.requests[0]?.body, { name: "Grace" });
    assert.equal(joined.resident, 1);
    assert.equal(joined.residentKey, server.world.residents.get(1)?.key);
    assert.equal((await joined.client.me()).name, "Ada");

    // With an owner token, no new owner signs up.
    const second = await Client.join(server.url, { name: "Bob", ownerToken: joined.ownerToken });
    assert.equal(second.owner, undefined);
    assert.equal(second.resident, 2);
    assert.equal(server.world.residents.get(2)?.owner, 1);
    assert.equal(server.gateway.requests.filter((request) => request.path === "/v1/owners").length, 1);
  } finally {
    await server.close();
  }
});

test("a rotated key replaces the old key", async (t) => {
  const { client, resident, server } = await scene(t);
  const oldKey = resident.key;
  await client.me();
  const rotated = await client.rotateKey(resident.id);
  assert.notEqual(rotated.key, oldKey);
  assert.deepEqual(rotated, { resident: resident.id, key: resident.key, key_hint: `${resident.key.slice(0, 16)}...` });
  assert.equal(client.residentKey, rotated.key);
  assert.equal((await client.me()).resident, resident.id);

  const stale = new Client({ baseUrl: server.url, residentKey: oldKey });
  await assert.rejects(stale.me(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.equal(error.code, "invalid_token");
    assert.equal(error.status, 401);
    assert.equal(error.endsCredential, true);
    return true;
  });
  // The resident of another owner looks the same as no resident.
  const stranger = await Client.join(server.url, { name: "Eve" });
  await assert.rejects(stranger.client.rotateKey(resident.id), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["not_found", 404]);
    return true;
  });
});

test("a call without the token that it needs sends nothing", async () => {
  const server = await startServer();
  try {
    const client = new Client({ baseUrl: server.url });
    await assert.rejects(client.observe(), MissingCredentialError);
    await assert.rejects(client.act("wait", { ticks: 1 }), MissingCredentialError);
    await assert.rejects(client.setRoutines([]), MissingCredentialError);
    await assert.rejects(client.createResident({ name: "Ada" }), MissingCredentialError);
    assert.equal(server.gateway.requests.length, 0);
  } finally {
    await server.close();
  }
});

test("the public routes need no token", async (t) => {
  const { server } = await scene(t);
  server.gateway.joinDocument = "# How to join\n";
  const client = new Client({ baseUrl: `${server.url}/` });
  assert.equal(await client.health(), true);
  assert.equal(await client.joinDocument(), "# How to join\n");
  const catalogue = await client.catalogue();
  assert.deepEqual(
    catalogue.verbs.map((verb) => `${verb.module}.${verb.verb}`),
    ["kernel.cancel", "kernel.set_routines", "kernel.wait", "life.move_to"],
  );
  const layout = await client.world();
  assert.equal(layout.type, "layout");
  assert.deepEqual(layout.zones.map((zone) => zone.id), [0]);
  const zone = await client.zone(0);
  assert.equal(zone.type, "zone");
  assert.deepEqual(zone.residents.map((resident) => resident.name), ["Ada"]);
  assert.ok(server.gateway.requests.every((request) => request.token === undefined));
});

test("`act` returns the acknowledgement and the observation, and sends an idempotency key", async (t) => {
  const { client, gateway, resident } = await scene(t);
  const { ack, observation } = await client.act("move_to", { x: 35, y: 30 });
  assert.equal(ack.verb, "move_to");
  assert.deepEqual(ack.status, { state: "started", ends_at: 3, ticks: 3 });
  assert.equal(selfOf(observation)?.activity?.verb, "move_to");
  assert.equal(observation.resident, resident.id);

  const body = gateway.requests.at(-1)?.body as { id: string; verb: string; args: unknown };
  assert.match(body.id, /^[0-9a-f-]{36}$/);
  assert.equal(ack.request_id, body.id);
  assert.deepEqual({ verb: body.verb, args: body.args }, { verb: "move_to", args: { x: 35, y: 30 } });

  // A key of the caller goes out as it is.
  const second = await client.act("wait", { ticks: 2 }, { id: "my-key" });
  assert.equal(second.ack.request_id, "my-key");
  assert.deepEqual(second.ack.status, { state: "queued", position: 1 });
});

test("a refused action comes back as data with its error code", async (t) => {
  const { client, resident } = await scene(t);
  const unknown = await client.act("fly");
  assert.equal(unknown.ack.status.state, "rejected");
  assert.equal(rejection(unknown.ack)?.code, "unknown_verb");
  assert.throws(
    () => assertAccepted(unknown.ack),
    (error: unknown) => {
      assert.ok(error instanceof ActionRejectedError);
      assert.equal(error.code, "unknown_verb");
      assert.equal(error.ack, unknown.ack);
      return true;
    },
  );

  const invalid = await client.act("wait", { ticks: 0 });
  assert.equal(rejection(invalid.ack)?.code, "invalid_args");

  resident.conditions["frozen"] = null;
  const blocked = await client.act("wait", { ticks: 1 });
  const error = rejection(blocked.ack);
  assert.equal(error?.code, "blocked");
  assert.deepEqual(error?.details, { condition: "frozen" });

  delete resident.conditions["frozen"];
  const accepted = await client.act("wait", { ticks: 1 });
  assert.equal(rejection(accepted.ack), undefined);
  assert.equal(assertAccepted(accepted.ack), accepted.ack);
});

test("`plan` queues the steps in order, and the server stops at the first refusal", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  const { acks, notSent, observation } = await client.plan([
    { verb: "move_to", args: { x: 33, y: 32 } },
    { verb: "wait", args: { ticks: 4 } },
  ]);
  assert.deepEqual(
    acks.map((ack) => ack.status.state),
    ["started", "queued"],
  );
  assert.equal(notSent, 0);
  assert.equal(selfOf(observation)?.queue, 1);
  const body = gateway.requests.at(-1)?.body as { plan: { id: string; verb: string }[] };
  assert.deepEqual(body.plan.map((step) => step.verb), ["move_to", "wait"]);
  assert.equal(new Set(body.plan.map((step) => step.id)).size, 2);

  // The plan runs: one tick to move, then four ticks of the wait.
  world.step(5);
  assert.deepEqual(resident.pos, { x: 33, y: 32 });
  const after = await client.observe();
  assert.deepEqual(
    reportsOf(after).map((report) => [report.verb, report.ok, report.tick, report.seq]),
    [
      ["move_to", true, 1, 1],
      ["wait", true, 5, 2],
    ],
  );

  // The server stops after the first step that the simulation rejects.
  // The steps after it were not sent: they have no acknowledgement.
  const stopped = await client.plan([
    { verb: "wait", args: { ticks: 2 } },
    { verb: "fly" },
    { verb: "wait", args: { ticks: 4 } },
    { verb: "wait", args: { ticks: 5 } },
  ]);
  assert.deepEqual(
    stopped.acks.map((ack) => ack.status.state),
    ["started", "rejected"],
  );
  assert.equal(stopped.notSent, 2);
  assert.equal(rejection(stopped.acks[1] as ActionAck)?.code, "unknown_verb");
  assert.equal(selfOf(stopped.observation)?.queue, 0);

  // A plan has at most 16 steps. The server refuses a longer one as a whole.
  const long = Array.from({ length: PLAN_MAX + 1 }, () => ({ verb: "wait", args: { ticks: 1 } }));
  await assert.rejects(client.plan(long), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["invalid_request", 400]);
    return true;
  });

  // An empty plan only observes.
  const before = gateway.requests.length;
  const empty = await client.plan([]);
  assert.deepEqual(empty.acks, []);
  assert.equal(empty.notSent, 0);
  assert.equal(gateway.requests.length, before + 1);
  assert.equal(gateway.requests.at(-1)?.path, "/v1/observe");
});

test("`observe` keeps the notice cursor", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  world.tell(resident.id, 7, "hello");
  world.notify(resident.id, "economy.wage_paid", { amount: 500 });

  const first = await client.observe();
  assert.deepEqual(
    first.notices.map((notice) => [notice.seq, notice.kind]),
    [
      [1, "society.said"],
      [2, "economy.wage_paid"],
    ],
  );
  assert.equal(client.cursor, 3);
  assert.equal(gateway.requests.at(-1)?.query["since"], undefined);

  // Nothing new: the next observation has no notice, and the request carried the cursor.
  const second = await client.observe();
  assert.deepEqual(second.notices, []);
  assert.equal(gateway.requests.at(-1)?.query["since"], "3");
  assert.equal(hasNews(second), false);

  world.notify(resident.id, "economy.rent_due");
  const third = await client.observe({ detail: "full", sections: ["self"] });
  assert.deepEqual(third.notices.map((notice) => notice.seq), [3]);
  assert.deepEqual(Object.keys(third.sections), ["self"]);
  assert.equal(selfOf(third)?.["owner"], 1);
  assert.deepEqual(gateway.requests.at(-1)?.query, { detail: "full", since: "3", sections: "self" });

  // An explicit `since` reads older notices again.
  const again = await client.observe({ since: 2 });
  assert.deepEqual(again.notices.map((notice) => notice.seq), [2, 3]);
});

test("`act` moves the cursor, and the client drops notices that a server repeats", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  world.tell(resident.id, 7, "one");
  await client.observe();
  world.tell(resident.id, 7, "two");

  const acted = await client.act("wait", { ticks: 1 });
  assert.equal(gateway.requests.at(-1)?.query["since"], "2");
  assert.deepEqual(acted.observation.notices.map((notice) => notice.seq), [2]);
  assert.equal(client.cursor, 3);

  // A server that does not read `since` on this route sends the whole inbox.
  gateway.actIgnoresSince = true;
  world.tell(resident.id, 7, "three");
  const repeated = await client.act("wait", { ticks: 1 });
  assert.deepEqual(repeated.observation.notices.map((notice) => notice.seq), [3]);
  assert.equal(client.cursor, 4);

  // The same for reports: each has a `seq` of the same sequence, so the
  // client drops the reports before its cursor.
  world.step(1);
  assert.deepEqual(reportsOf(await client.observe()).map((report) => [report.verb, report.tick]), [["wait", 1]]);
  const afterReport = await client.act("wait", { ticks: 1 });
  assert.deepEqual(reportsOf(afterReport.observation), []);
  // A report that the client did not return before stays, also when it comes with an action.
  world.step(1);
  const withNews = await client.act("wait", { ticks: 1 });
  assert.deepEqual(reportsOf(withNews.observation).map((report) => [report.verb, report.tick]), [["wait", 2]]);
  // An explicit `since` reads old reports again.
  assert.deepEqual(reportsOf(await client.observe({ since: 1 })).map((report) => report.tick), [1, 2]);
});

test("`wait` blocks until a notice arrives", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  await client.observe();
  const waiting = client.wait({ timeout: 30 });
  await world.parked();
  assert.deepEqual(gateway.requests.at(-1)?.query, { timeout: "30", since: "1" });

  world.tell(resident.id, 7, "wake up");
  const observation = await waiting;
  assert.deepEqual(observation.notices.map((notice) => notice.data), [
    { text: { untrusted: true, from: 7, text: "wake up" } },
  ]);
  assert.deepEqual(untrustedOf(observation), [{ untrusted: true, from: 7, text: "wake up" }]);
  assert.equal(client.cursor, 2);
});

test("`wait` returns the report of an action that ended", async (t) => {
  const { client, world } = await scene(t);
  await client.act("wait", { ticks: 3 });
  const waiting = client.wait({ timeout: 30, kinds: ["report"] });
  await world.parked();
  world.step(3);
  const observation = await waiting;
  const reports = reportsOf(observation);
  assert.equal(reports.length, 1);
  assert.deepEqual([reports[0]?.verb, reports[0]?.ok, reports[0]?.result], ["wait", true, { waited: 3 }]);
});

test("`wait` with kinds sleeps through other notices", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  const waiting = client.wait({ timeout: 30, kinds: ["economy.rent_due", "report"] });
  await world.parked();
  assert.equal(gateway.requests.at(-1)?.query["kinds"], "economy.rent_due,report");
  world.tell(resident.id, 7, "noise");
  world.notify(resident.id, "economy.rent_due");
  const observation = await waiting;
  // The observation holds everything since the cursor, also the notice that did not wake the call.
  assert.deepEqual(observation.notices.map((notice) => notice.kind), ["society.said", "economy.rent_due"]);
});

test("a long `wait` makes several long polls and stops at the first news", async (t) => {
  const { client, gateway, world, resident } = await scene(t, { time: "auto" });
  const waits = (): string[] =>
    gateway.requests.filter((request) => request.path === "/v1/wait").map((request) => request.query["timeout"] ?? "");

  // No news: 150 seconds are three polls, and game time passed.
  const quiet = await client.wait({ timeout: 150 });
  assert.equal(hasNews(quiet), false);
  assert.deepEqual(waits(), ["60", "60", "30"]);
  assert.equal(world.tick, 150);

  // News in the second poll: the third poll is not made.
  await client.act("wait", { ticks: 90 });
  gateway.requests.length = 0;
  const busy = await client.wait({ timeout: 600 });
  assert.deepEqual(waits(), ["60", "60"]);
  assert.deepEqual(reportsOf(busy).map((report) => report.verb), ["wait"]);
  assert.equal(world.tick, 240);
  assert.equal(resident.current, undefined);

  // With kinds, a notice of another kind does not end the long wait.
  gateway.requests.length = 0;
  world.tell(resident.id, 7, "noise");
  const filtered = await client.wait({ timeout: 120, kinds: ["report"] });
  assert.deepEqual(waits(), ["60", "60"]);
  assert.equal(hasNews(filtered, ["report"]), false);
  // The answer still holds the notice that came during the wait.
  assert.deepEqual(filtered.notices.map((notice) => notice.kind), ["society.said"]);
  assert.deepEqual((await client.observe()).notices, []);

  // A timeout of zero only observes.
  gateway.requests.length = 0;
  await client.wait({ timeout: 0 });
  assert.deepEqual(gateway.requests.map((request) => request.path), ["/v1/observe"]);
});

test("`setRoutines` replaces the routines and reports a routine that is not valid", async (t) => {
  const { client, gateway, resident } = await scene(t, {
    attrs: [{ name: "need.energy", min: 0, max: 1000, default: 1000, summary: "0 means exhausted." }],
  });
  const routines = [
    {
      name: "rest when tired",
      when: { type: "attr_below" as const, attr: "need.energy", value: 300 },
      then: { verb: "wait", args: { ticks: 60 } },
    },
  ];
  assert.deepEqual(await client.setRoutines(routines), { routines: 1 });
  assert.deepEqual(gateway.requests.at(-1)?.body, { routines });
  assert.equal(gateway.requests.at(-1)?.method, "PUT");
  assert.deepEqual(resident.routines, routines);
  const full = await client.observe({ detail: "full" });
  assert.deepEqual(selfOf(full)?.["routines"], [{ ...routines[0], runs: 0, failures: 0 }]);

  await assert.rejects(
    client.setRoutines([{ name: "fly", when: { type: "always" }, then: { verb: "fly" } }]),
    (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.equal(error.code, "unknown_verb");
      assert.equal(error.status, 400);
      assert.equal(error.isActionError, true);
      return true;
    },
  );
  // The refused list changed nothing.
  assert.deepEqual(resident.routines, routines);
  assert.deepEqual(await client.setRoutines([]), { routines: 0 });
  assert.deepEqual(resident.routines, []);
});

test("`setWebhook` sets the address and the secret signs deliveries", async (t) => {
  const { client, resident } = await scene(t);
  const info = await client.setWebhook("https://agent.example/hook");
  assert.equal(info.url, "https://agent.example/hook");
  assert.equal(info.secret, resident.webhook?.secret);
  assert.equal(typeof info.secret, "string");

  const secret = info.secret as string;
  assert.match(secret, /^unsup_whs_[0-9a-f]{64}$/);
  const body = '{"resident":7,"notices":[],"reports":[],"next_notice":3}';
  const timestamp = "1800000000";
  const signature = await signWebhook(secret, timestamp, body);
  assert.match(signature, /^v1=[0-9a-f]{64}$/);
  // A fixed answer, checked with `openssl dgst -sha256 -hmac` over
  // `v1:1800000000:<body>`. The Python SDK has the same test.
  assert.equal(
    await signWebhook("unsup_whs_0123456789abcdef", timestamp, body),
    "v1=01a9e07c2db22745c9c76477a6830e5ad54b65815ab99955b16ec7651a0d6089",
  );
  const now = { now: 1_800_000_010 };
  assert.equal(await verifyWebhook(secret, { timestamp, signature, body }, now), true);
  assert.equal(await verifyWebhook(secret, { timestamp, signature, body: new TextEncoder().encode(body) }, now), true);
  // A changed body, another secret, another time and a wrong form do not pass.
  assert.equal(await verifyWebhook(secret, { timestamp, signature, body: `${body} ` }, now), false);
  assert.equal(await verifyWebhook("another-secret", { timestamp, signature, body }, now), false);
  assert.equal(await verifyWebhook(secret, { timestamp: "1800000001", signature, body }, now), false);
  assert.equal(await verifyWebhook(secret, { timestamp, signature: signature.slice(3), body }, now), false);
  assert.equal(await verifyWebhook(secret, { timestamp, signature: "v1=00", body }, now), false);
  assert.equal(await verifyWebhook(secret, { timestamp: undefined, signature, body }, now), false);
  assert.equal(await verifyWebhook(secret, { timestamp, signature: null, body }, now), false);
  // An old delivery is refused, so that nobody can send it again.
  assert.equal(await verifyWebhook(secret, { timestamp, signature, body }, { now: 1_800_000_301 }), false);
  assert.equal(await verifyWebhook(secret, { timestamp, signature, body }, { now: 1_800_000_300 }), true);
  assert.equal(await verifyWebhook(secret, { timestamp, signature, body }, { now: 1_900_000_000, toleranceSeconds: Infinity }), true);
  // The headers of a delivery, as `node:http` and as `fetch` give them.
  const headers = { [WEBHOOK_TIMESTAMP_HEADER]: timestamp, [WEBHOOK_SIGNATURE_HEADER]: signature };
  assert.equal(await verifyWebhookRequest(secret, headers, body, now), true);
  assert.equal(await verifyWebhookRequest(secret, new Headers(headers), body, now), true);
  assert.equal(await verifyWebhookRequest(secret, {}, body, now), false);

  // Each call makes a new secret.
  const second = await client.setWebhook("https://agent.example/hook");
  assert.notEqual(second.secret, secret);
  assert.deepEqual(await client.setWebhook(null), { url: null });
  assert.equal(resident.webhook, undefined);
});

test("an error answer becomes an `ApiError` with the code of the server", async (t) => {
  const { client, gateway, server } = await scene(t);
  const owner = new Client({ baseUrl: server.url, residentKey: client.ownerToken as string });
  await assert.rejects(owner.observe(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["wrong_token_kind", 403]);
    assert.equal(error.message, "wrong_token_kind: This route needs a resident key. You sent an owner token.");
    assert.equal(error.endsCredential, true);
    return true;
  });

  // An answer without the documented body keeps its status.
  gateway.failNext({ status: 400, body: "Failed to parse the request body" });
  await assert.rejects(client.observe(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["http_error", 400]);
    assert.equal(error.message, "http_error: HTTP 400: Failed to parse the request body");
    return true;
  });
});

test("a client works with the `fetch` of the fake gateway, without a network", async () => {
  const gateway = new FakeGateway();
  const joined = await Client.join("http://fake", { name: "Ada", fetch: gateway.fetch });
  const { ack } = await joined.client.act("wait", { ticks: 2 });
  assert.equal(ack.status.state, "started");
  gateway.world.step(2);
  assert.deepEqual(reportsOf(await joined.client.observe()).map((report) => report.verb), ["wait"]);
});

test("refused text, a mute and a ban are errors with their codes, and nothing reaches the world", async (t) => {
  const { client, gateway, resident, world } = await scene(t);
  // A text that a filter refuses: 422, and the action did not start.
  await assert.rejects(client.act("say", { text: "read https://example.com" }), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status, error.isTextRefusal, error.endsCredential], ["text_refused", 422, true, false]);
    assert.match(error.message, /args\.text/);
    assert.doesNotMatch(error.message, /example/);
    return true;
  });
  // One refused text stops a whole plan: no step was sent.
  const plan = client.plan([{ verb: "wait", args: { ticks: 3 } }, { verb: "say", args: { text: "www.example.com" } }]);
  await assert.rejects(plan, (error: unknown) => error instanceof ApiError && error.code === "text_refused");
  assert.equal(resident.current, undefined);
  // The text of a routine passes the same filters.
  const routine = { name: "greet", when: { type: "always" as const }, then: { verb: "wait", args: { note: "www.example.com" } } };
  await assert.rejects(client.setRoutines([routine]), (error: unknown) => error instanceof ApiError && error.status === 422);
  assert.deepEqual(resident.routines, []);

  // A muted resident cannot send speech. Every other action works.
  gateway.muted.add(resident.id);
  await assert.rejects(client.act("say", { text: "hello" }), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status, error.isTextRefusal, error.endsCredential], ["muted", 403, true, false]);
    return true;
  });
  assert.equal((await client.act("wait", { ticks: 1 })).ack.status.state, "started");

  // A ban stops the key and the owner token.
  gateway.banned.add(resident.owner);
  await assert.rejects(client.observe(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status, error.endsCredential], ["banned", 403, true]);
    return true;
  });
  await assert.rejects(client.listResidents(), (error: unknown) => error instanceof ApiError && error.code === "banned");
  gateway.banned.clear();

  // A resident that an administrator removed: its key ends at once.
  world.despawn(resident);
  await assert.rejects(client.observe(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status, error.endsCredential], ["invalid_token", 401, true]);
    return true;
  });
});

test("the first call for a resident that died gets `resident_gone`, and each later call gets `invalid_token`", async (t) => {
  const { client, world, resident } = await scene(t);
  const second = world.spawn(resident.owner, { name: "Bea" });
  world.die(resident);
  const refused: [string, number | undefined, boolean][] = [];
  for (const call of [() => client.observe(), () => client.act("wait", { ticks: 1 }), () => client.me()]) {
    await assert.rejects(call(), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      refused.push([error.code, error.status, error.endsCredential]);
      return true;
    });
  }
  assert.deepEqual(refused, [
    ["resident_gone", 410, true],
    ["invalid_token", 401, true],
    ["invalid_token", 401, true],
  ]);
  // The owner sees who is left, and the other resident lives on.
  assert.deepEqual((await client.listResidents()).residents.map((one) => one.id), [second.id]);
  assert.equal((await client.forResident(second.key).me()).name, "Bea");
});

test("a long poll that is open when the resident dies gets `resident_gone`", async (t) => {
  const { client, world, resident } = await scene(t);
  const waiting = client.wait({ timeout: 60 });
  await world.parked();
  world.die(resident);
  await assert.rejects(waiting, (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["resident_gone", 410]);
    return true;
  });
  // That call told the gateway of the death. The key is unknown now.
  await assert.rejects(client.observe(), (error: unknown) => error instanceof ApiError && error.code === "invalid_token");
});

test("`observe: false` returns the acknowledgements alone and leaves the cursor where it is", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  world.tell(resident.id, 2, "Good day.");
  const sent = await client.act("wait", { ticks: 2 }, { observe: false, id: "fast-1" });
  assert.deepEqual(sent, { ack: { action_id: sent.ack.action_id, request_id: "fast-1", verb: "wait", status: { state: "started", ends_at: 2, ticks: 2 } } });
  assert.equal("observation" in sent, false);
  const request = gateway.requests.at(-1);
  assert.deepEqual([request?.path, request?.query], ["/v1/act", { observe: "false" }]);

  const planned = await client.plan([{ verb: "wait", args: { ticks: 1 } }, { verb: "fly" }, { verb: "wait", args: { ticks: 1 } }], { observe: false });
  assert.deepEqual(planned.acks.map((ack) => ack.status.state), ["queued", "rejected"]);
  assert.equal(planned.notSent, 1);
  assert.equal("observation" in planned, false);
  // An empty plan without an observation sends nothing.
  const before = gateway.requests.length;
  assert.deepEqual(await client.plan([], { observe: false }), { acks: [], notSent: 0 });
  assert.equal(gateway.requests.length, before);

  // The cursor did not move: the next observation has the notice and both reports.
  assert.equal(client.cursor, 0);
  world.step(3);
  const news = await client.observe();
  assert.deepEqual(news.notices.map((notice) => notice.kind), ["society.said"]);
  assert.deepEqual(reportsOf(news).map((report) => report.verb), ["wait", "wait"]);
  assert.equal(client.cursor, 4);
});

test("the fake gateway refuses what the real gateway refuses", async (t) => {
  const { gateway, resident } = await scene(t);
  const ask = async (method: string, url: string, body?: unknown, token: string | null = resident.key) => {
    const headers: Record<string, string> = token === null ? {} : { authorization: `Bearer ${token}` };
    if (body !== undefined) headers["content-type"] = "application/json";
    const answer = await gateway.handle({ method, url, headers, body: body === undefined ? undefined : JSON.stringify(body) });
    const parsed = JSON.parse(answer.body) as { error?: { code: string } };
    return [answer.status, parsed.error?.code];
  };
  assert.deepEqual(await ask("GET", "/v1/observe", undefined, null), [401, "unauthorized"]);
  assert.deepEqual(await ask("GET", "/v1/observe", undefined, "unsup_res_wrong"), [401, "invalid_token"]);
  assert.deepEqual(await ask("GET", "/v1/residents"), [403, "wrong_token_kind"]);
  assert.deepEqual(await ask("GET", "/v1/observe?verbose=1"), [400, "invalid_request"]);
  assert.deepEqual(await ask("GET", "/v1/observe?detail=huge"), [400, "invalid_request"]);
  assert.deepEqual(await ask("GET", "/v1/wait?timeout=61"), [400, "invalid_request"]);
  assert.deepEqual(await ask("POST", "/v1/act", { verb: "wait", args: { ticks: 1 }, priority: 9 }), [400, "invalid_request"]);
  assert.deepEqual(await ask("POST", "/v1/act", { plan: [] }), [400, "invalid_request"]);
  assert.deepEqual(await ask("PUT", "/v1/webhook", {}), [400, "invalid_request"]);
  assert.deepEqual(await ask("PUT", "/v1/webhook", { url: "ftp://example.org/hook" }), [400, "invalid_request"]);
  assert.deepEqual(await ask("PUT", "/v1/routines", { routines: [1] }), [400, "invalid_request"]);
  assert.deepEqual(await ask("GET", "/v1/nothing"), [404, "not_found"]);
  assert.deepEqual(await ask("DELETE", "/v1/act"), [405, "method_not_allowed"]);
  const text = await gateway.handle({ method: "POST", url: "/v1/act", headers: { authorization: `Bearer ${resident.key}` }, body: "{}" });
  assert.equal(text.status, 415);
  // An answer with the status 401 has the header `WWW-Authenticate`.
  const bare = await gateway.handle({ method: "GET", url: "/v1/me" });
  assert.equal(bare.headers["www-authenticate"], "Bearer");
});
