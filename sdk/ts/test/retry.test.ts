// Retry: which requests go out again, how long the client waits, and that a
// repeat with the same request id does not act twice.

import assert from "node:assert/strict";
import { test } from "node:test";

import { ApiError, Client, TransportError, defaultRetryDelay } from "../src/index.js";
import { scene } from "./support/setup.js";

test("a request that the server refused for load goes out again", async (t) => {
  const { client, gateway, sleeps } = await scene(t);
  gateway.failNext({ status: 503, code: "overloaded", times: 2 });
  const observation = await client.observe();
  assert.equal(observation.resident, 1);
  assert.equal(gateway.requests.length, 3);
  // The test delay is the attempt number in milliseconds.
  assert.deepEqual(sleeps, [1, 2]);
});

test("the client waits as long as `Retry-After` says", async (t) => {
  const { client, gateway, sleeps } = await scene(t);
  gateway.failNext({ status: 429, code: "rate_limited", headers: { "retry-after": "7" } });
  await client.observe();
  assert.equal(gateway.requests.length, 2);
  assert.deepEqual(sleeps, [7000]);
});

test("the client gives up after `maxRetries` new attempts and throws the last error", async (t) => {
  const { client, gateway, sleeps } = await scene(t, {}, { maxRetries: 2 });
  gateway.failNext({ status: 503, code: "overloaded", message: "The simulation is overloaded.", times: 10 });
  await assert.rejects(client.observe(), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.code, error.status, error.isSimError, error.isTransient], ["overloaded", 503, true, true]);
    return true;
  });
  assert.equal(gateway.requests.length, 3);
  assert.deepEqual(sleeps, [1, 2]);
});

test("an error that a repeat cannot cure is thrown at once", async (t) => {
  const { client, gateway, sleeps } = await scene(t);
  gateway.failNext({ status: 400, code: "invalid", times: 5 });
  await assert.rejects(client.observe(), (error: unknown) => error instanceof ApiError && error.code === "invalid");
  assert.equal(gateway.requests.length, 1);
  assert.deepEqual(sleeps, []);
});

test("an action whose answer was lost is sent again with the same id and runs once", async (t) => {
  const { client, gateway, server, resident, sleeps } = await scene(t);
  server.loseResponses(1);
  const { ack } = await client.act("move_to", { x: 40, y: 32 });

  const sent = gateway.requests.filter((request) => request.path === "/v1/act");
  assert.equal(sent.length, 2);
  const ids = sent.map((request) => (request.body as { id: string }).id);
  assert.equal(ids[0], ids[1]);
  assert.equal(ack.request_id, ids[0]);
  assert.deepEqual(sleeps, [1]);

  // The server acted on the first request. The repeat got the first answer.
  assert.deepEqual(ack.status, { state: "started", ends_at: 8, ticks: 8 });
  assert.equal(resident.current?.action_id, ack.action_id);
  assert.equal(resident.queue.length, 0);
});

test("a plan whose answer was lost does not queue its steps twice", async (t) => {
  const { client, server, resident } = await scene(t);
  server.loseResponses(1);
  const { acks } = await client.plan([
    { verb: "wait", args: { ticks: 5 } },
    { verb: "wait", args: { ticks: 5 } },
  ]);
  assert.deepEqual(
    acks.map((ack) => ack.status.state),
    ["started", "queued"],
  );
  assert.equal(resident.queue.length, 1);
});

test("a sign-up without an answer is not sent again, because it has no idempotency key", async (t) => {
  const { server, gateway, world, sleeps } = await scene(t);
  const client = new Client({
    baseUrl: server.url,
    sleep: async (milliseconds) => {
      sleeps.push(milliseconds);
    },
  });
  server.loseResponses(1);
  await assert.rejects(client.signUp({ name: "Grace" }), TransportError);
  assert.equal(gateway.requests.filter((request) => request.path === "/v1/owners").length, 1);
  assert.deepEqual(sleeps, []);

  // A refusal for load is safe to repeat: the server did nothing.
  gateway.failNext({ status: 503, code: "overloaded" });
  const signup = await client.signUp({ name: "Grace" });
  assert.equal(gateway.requests.filter((request) => request.path === "/v1/owners").length, 3);
  // The lost sign-up made owner 2 on the server; this one is owner 3.
  assert.equal(signup.owner.id, 3);
  assert.equal(world.ownerOf(signup.token), 3);
});

test("a read without an answer goes out again, and fails with `TransportError` when no attempt gets through", async (t) => {
  const { client, gateway, server, sleeps } = await scene(t, {}, { maxRetries: 1 });
  server.refuseRequests(1);
  assert.equal((await client.me()).name, "Ada");
  assert.deepEqual(sleeps, [1]);

  server.refuseRequests(2);
  const before = gateway.requests.length;
  await assert.rejects(client.me(), (error: unknown) => {
    assert.ok(error instanceof TransportError);
    assert.equal(error.message, "GET /v1/me got no answer");
    return true;
  });
  // The server cut both attempts before the gateway saw them.
  assert.equal(gateway.requests.length, before);
  assert.deepEqual(sleeps, [1, 1]);
});

test("a caller that gives up stops the long poll", async (t) => {
  const { client, world, sleeps } = await scene(t);
  const controller = new AbortController();
  const waiting = client.wait({ timeout: 60, signal: controller.signal });
  await world.parked();
  controller.abort(new Error("the caller gave up"));
  await assert.rejects(waiting, /the caller gave up/);
  assert.deepEqual(sleeps, []);
});

test("the default delay doubles, stays below its ceiling and follows `Retry-After`", () => {
  for (let attempt = 1; attempt <= 12; attempt += 1) {
    const ceiling = Math.min(8000, 250 * 2 ** (attempt - 1));
    for (let i = 0; i < 20; i += 1) {
      const delay = defaultRetryDelay(attempt, undefined);
      assert.ok(delay >= ceiling / 2 && delay <= ceiling, `attempt ${attempt}: ${delay}`);
    }
  }
  assert.equal(defaultRetryDelay(1, 3), 3000);
  assert.equal(defaultRetryDelay(9, 0), 0);
});
