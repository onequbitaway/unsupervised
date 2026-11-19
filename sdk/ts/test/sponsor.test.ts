// Missions of a sponsor: `postMission` with an owner token.

import assert from "node:assert/strict";
import { test } from "node:test";

import { ApiError, MissingCredentialError, TransportError, type MissionDefinition } from "../src/index.js";
import { scene } from "./support/setup.js";

const BREAD: MissionDefinition = {
  title: "Bread for the fair",
  text: "The town needs bread.",
  objectives: [{ event: "economy.goods_produced", fields: { item: "bread" }, count: 10 }],
  reward: 5000,
  deadline_in: 2880,
  takers: 2,
};

/** Returns the error of a mission that the server must refuse. */
async function refusal(posting: Promise<unknown>): Promise<ApiError> {
  const error: unknown = await posting.then(
    () => undefined,
    (thrown: unknown) => thrown,
  );
  assert.ok(error instanceof ApiError, `the server refused the mission: ${String(error)}`);
  return error;
}

test("an owner publishes a mission with its token, and the answer names the mission", async (t) => {
  const { client, gateway, world } = await scene(t);
  world.step(7);
  const posted = await client.postMission(BREAD);
  assert.deepEqual(posted, { mission: posted.mission, request: 1, frame: 7, deadline: 7 + 2880, reward: 5000, takers: 2 });
  assert.equal(world.missions.length, 1);
  assert.deepEqual([world.missions[0]?.mission, world.missions[0]?.owner], [posted.mission, 1]);

  const sent = gateway.requests.at(-1);
  assert.deepEqual([sent?.method, sent?.path, sent?.query], ["POST", "/v1/missions", {}]);
  assert.equal(world.ownerOf(sent?.token), 1);
  assert.deepEqual(sent?.body, BREAD);

  // A mission without the optional fields: one taker.
  const small = await client.postMission({ title: "One loaf", objectives: [{ event: "economy.goods_produced" }], reward: 100, deadline_in: 10 });
  assert.deepEqual([small.request, small.takers, small.deadline], [2, 1, 17]);
});

test("a mission needs an owner token: a resident key is refused, and no token sends nothing", async (t) => {
  const { client, gateway, resident, server } = await scene(t);
  const { Client } = await import("../src/index.js");
  const before = gateway.requests.length;
  await assert.rejects(new Client({ baseUrl: server.url, residentKey: resident.key }).postMission(BREAD), MissingCredentialError);
  assert.equal(gateway.requests.length, before);

  const wrong = await refusal(new Client({ baseUrl: server.url, ownerToken: resident.key }).postMission(BREAD));
  assert.deepEqual([wrong.code, wrong.status], ["wrong_token_kind", 403]);
  assert.equal((await client.postMission(BREAD)).request, 1);
});

test("the limit of one owner answers with `Retry-After`, and the client does not wait for it", async (t) => {
  const { client, gateway, sleeps } = await scene(t);
  let now = 1_000_000;
  gateway.now = () => now;
  for (let sent = 0; sent < 3; sent += 1) {
    await client.postMission(BREAD);
    now += 60_000;
  }
  const before = gateway.requests.length;
  const limited = await refusal(client.postMission(BREAD));
  assert.deepEqual([limited.code, limited.status, limited.isTransient], ["rate_limited", 429, true]);
  // The first mission counts for 24 hours. Three minutes of them are over.
  assert.equal(limited.retryAfterSeconds, 86_400 - 180);
  // One request went out: the client did not sleep for a day and did not try again.
  assert.equal(gateway.requests.length, before + 1);
  assert.deepEqual(sleeps, []);

  // The limit counts for each owner, and a mission is free again after its 24 hours.
  const other = (await client.forResident("unused").signUp({ name: "Bob" })).token;
  const { Client } = await import("../src/index.js");
  assert.equal((await new Client({ baseUrl: client.baseUrl, ownerToken: other }).postMission(BREAD)).request, 4);
  now = 1_000_000 + 86_400_000;
  assert.equal((await client.postMission(BREAD)).request, 5);
});

test("the server refuses a mission with the code of the check that failed, and nothing is published", async (t) => {
  const { client, gateway, world } = await scene(t);
  const cases: [Record<string, unknown>, string, number][] = [
    [{ sponsor: "Acme" }, "invalid_request", 400],
    [{ title: " " }, "invalid_request", 400],
    [{ objectives: [] }, "invalid_request", 400],
    // 500,001 cents of new money: more than the server gives to one mission.
    [{ reward: 166_667, takers: 3 }, "invalid_request", 400],
    [{ title: "read www.example.com" }, "text_refused", 422],
    [{ objectives: [{ event: "see http://example.com" }] }, "text_refused", 422],
    // The missions module refuses these: the numbers are out of their ranges.
    [{ reward: 99 }, "invalid_args", 400],
    [{ deadline_in: 9 }, "invalid_args", 400],
    [{ takers: 51, reward: 100 }, "invalid_args", 400],
    [{ org: 4 }, "invalid_args", 400],
  ];
  assert.equal(cases.length, 10);
  for (const [change, code, status] of cases) {
    const error = await refusal(client.postMission({ ...BREAD, ...change } as MissionDefinition));
    assert.deepEqual([error.code, error.status], [code, status], JSON.stringify(change));
  }
  gateway.sponsors.enabled = false;
  const off = await refusal(client.postMission(BREAD));
  assert.deepEqual([off.code, off.status, off.endsCredential], ["missions_disabled", 403, false]);
  assert.deepEqual(world.missions, []);

  // A mission that was refused does not count for the limit of the owner.
  gateway.sponsors.enabled = true;
  gateway.sponsors.missionsPerDay = 1;
  assert.equal((await client.postMission(BREAD)).request, 1);
  assert.equal((await refusal(client.postMission(BREAD))).code, "rate_limited");
});

test("a mission without an answer is not sent again: the world can have it", async (t) => {
  const { client, gateway, server, world, sleeps } = await scene(t);
  // The world took the mission and did not answer in time.
  gateway.failNext({ status: 503, code: "answer_timeout", message: "Look in `/v1/feed` for the request 41." });
  const late = await refusal(client.postMission(BREAD));
  assert.deepEqual([late.code, late.status], ["answer_timeout", 503]);
  assert.match(late.message, /request 41/);
  // A refusal for load is not repeated either: only the caller knows when to send a mission again.
  gateway.failNext({ status: 503, code: "overloaded" });
  assert.equal((await refusal(client.postMission(BREAD))).code, "overloaded");
  assert.equal(gateway.requests.filter((request) => request.path === "/v1/missions").length, 2);

  // The answer is lost on the way: the server published the mission.
  server.loseResponses(1);
  await assert.rejects(client.postMission(BREAD), TransportError);
  assert.equal(world.missions.length, 1);
  assert.equal(gateway.requests.filter((request) => request.path === "/v1/missions").length, 3);
  assert.deepEqual(sleeps, []);
});
