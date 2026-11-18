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
  rejection,
  reportsOf,
  selfOf,
  signWebhook,
  verifyWebhook,
} from "../src/index.js";
import { FakeGateway } from "../src/testing/index.js";
import { scene } from "./support/setup.js";
import { startServer } from "./support/server.js";

test("a new agent signs up, creates a resident and checks its key", async () => {
  const server = await startServer();
  try {
    const client = new Client({ baseUrl: server.url });
    const signup = await client.signUp({ name: "Grace" });
    assert.deepEqual(signup, { owner: 1, token: "owner-token-1" });
    assert.equal(client.ownerToken, "owner-token-1");

    const created = await client.createResident({ name: "Ada", model: "scripted" });
    assert.equal(created.resident, 1);
    assert.equal(client.residentKey, created.key);
    assert.deepEqual(await client.me(), { resident: 1, name: "Ada" });
    assert.deepEqual(await client.listResidents(), [{ id: 1, name: "Ada", model: "scripted" }]);

    // The server has the name and the model that the client sent.
    assert.equal(server.world.residents.get(1)?.model, "scripted");
    const sent = server.gateway.requests.find((request) => request.path === "/v1/residents");
    assert.deepEqual(sent?.body, { name: "Ada", model: "scripted" });
    assert.equal(sent?.token, "owner-token-1");
  } finally {
    await server.close();
  }
});

test("`Client.join` does the whole sign-up in one call", async () => {
  const server = await startServer();
  try {
    const joined = await Client.join(server.url, { name: "Ada", ownerName: "Grace" });
    assert.equal(joined.owner, 1);
    assert.equal(joined.ownerToken, "owner-token-1");
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
  assert.equal(client.residentKey, rotated.key);
  assert.equal((await client.me()).resident, resident.id);

  const stale = new Client({ baseUrl: server.url, residentKey: oldKey });
  await assert.rejects(stale.me(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.equal(error.code, "unauthorized");
    assert.equal(error.status, 401);
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

test("`plan` queues the steps in order and gives one acknowledgement for each", async (t) => {
  const { client, gateway, world, resident } = await scene(t);
  const { acks, observation } = await client.plan([
    { verb: "move_to", args: { x: 33, y: 32 } },
    { verb: "fly" },
    { verb: "wait", args: { ticks: 4 } },
  ]);
  assert.deepEqual(
    acks.map((ack) => ack.status.state),
    ["started", "rejected", "queued"],
  );
  assert.equal(selfOf(observation)?.queue, 1);
  const body = gateway.requests.at(-1)?.body as { plan: { id: string; verb: string }[] };
  assert.deepEqual(body.plan.map((step) => step.verb), ["move_to", "fly", "wait"]);
  assert.equal(new Set(body.plan.map((step) => step.id)).size, 3);

  // The plan runs: one tick to move, then four ticks of the wait.
  world.step(5);
  assert.deepEqual(resident.pos, { x: 33, y: 32 });
  const after = await client.observe();
  assert.deepEqual(
    reportsOf(after).map((report) => [report.verb, report.ok, report.tick]),
    [
      ["move_to", true, 1],
      ["wait", true, 5],
    ],
  );

  // An empty plan only observes.
  const before = gateway.requests.length;
  const empty = await client.plan([]);
  assert.deepEqual(empty.acks, []);
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
      [1, "society.speech"],
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

  // The same for reports. They have no sequence number, so the client
  // remembers the reports that it returned and drops them from a repeat.
  world.step(1);
  assert.deepEqual(reportsOf(await client.observe()).map((report) => [report.verb, report.tick]), [["wait", 1]]);
  const afterReport = await client.act("wait", { ticks: 1 });
  assert.deepEqual(reportsOf(afterReport.observation), []);
  // A report that the client did not return before stays, also when it comes with an action.
  world.step(1);
  const withNews = await client.act("wait", { ticks: 1 });
  assert.deepEqual(reportsOf(withNews.observation).map((report) => [report.verb, report.tick]), [["wait", 2]]);
  // `observe` has a cursor in the protocol document. An explicit `since` reads old reports again.
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
  assert.deepEqual(observation.notices.map((notice) => notice.data), [{ untrusted: true, from: 7, text: "wake up" }]);
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
  assert.deepEqual(observation.notices.map((notice) => notice.kind), ["society.speech", "economy.rent_due"]);
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
  assert.deepEqual(filtered.notices.map((notice) => notice.kind), ["society.speech"]);
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
  const body = '{"seq":1,"tick":5,"kind":"economy.wage_paid"}';
  const signature = await signWebhook(secret, body);
  assert.match(signature, /^[0-9a-f]{64}$/);
  // A fixed answer, checked with `openssl dgst -sha256 -hmac`. The Python SDK has the same test.
  assert.equal(
    await signWebhook("webhook-secret-1-2", body),
    "99f547faf5157681638e12e4b1c66d90a08e6237a6c7136b2f56f4d059e08d0b",
  );
  assert.equal(await verifyWebhook(secret, body, signature), true);
  assert.equal(await verifyWebhook(secret, body, `sha256=${signature}`), true);
  assert.equal(await verifyWebhook(secret, `${body} `, signature), false);
  assert.equal(await verifyWebhook("another-secret", body, signature), false);
  assert.equal(await verifyWebhook(secret, body, "00"), false);

  assert.deepEqual(await client.setWebhook(null), { url: null });
  assert.equal(resident.webhook, undefined);
});

test("an error answer becomes an `ApiError` with the code of the server", async (t) => {
  const { client, gateway, server } = await scene(t);
  const owner = new Client({ baseUrl: server.url, residentKey: client.ownerToken as string });
  await assert.rejects(owner.observe(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status], ["forbidden", 403]);
    assert.equal(error.message, "forbidden: This route needs a resident key, not an owner token.");
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
