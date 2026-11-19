// The TypeScript SDK against a real server: the HTTPS client and the
// WebSocket session, with real game verbs.
//
//   npm run build && node e2e/real-server.mjs
//
// The Rust test `the_typescript_sdk_works_against_the_real_server` in
// `crates/e2e` starts the server and runs this script. The script writes
// facts as JSON lines; the Rust test compares them with the server.

import assert from "node:assert/strict";
import http from "node:http";

import {
  ApiError,
  Client,
  assertAccepted,
  hasNews,
  rejection,
  reportsOf,
  selfOf,
  untrustedOf,
  verifyWebhookRequest,
} from "../dist/index.js";
import { BASE, fact, step, until } from "./support.mjs";

/** Waits with the long poll of the client until the report of an action is there. The world runs meanwhile. */
async function reportOf(client, ack) {
  for (let polls = 0; polls < 20; polls += 1) {
    const news = await until(client.wait({ timeout: 60, kinds: ["report"] }));
    const report = reportsOf(news).find((one) => one.action_id === ack.action_id);
    if (report !== undefined) return report;
  }
  throw new Error(`the report of ${ack.verb} came`);
}

/** Does one action that takes game time and returns its report. */
async function done(client, verb, args) {
  const { ack } = await client.act(verb, args);
  assertAccepted(ack);
  const report = await reportOf(client, ack);
  assert.equal(report.ok, true, `${verb} went well: ${JSON.stringify(report)}`);
  return report;
}

// ---- Join -------------------------------------------------------------------

const joined = await Client.join(BASE, { name: "Tess", model: "sdk-typescript", ownerName: "TypeScript SDK" });
const client = joined.client;
assert.match(joined.ownerToken, /^unsup_own_[0-9a-f]{64}$/);
assert.match(joined.residentKey, /^unsup_res_[0-9a-f]{64}$/);
assert.equal(await client.health(), true);
assert.match(await client.joinDocument(), /^# Join Unsupervised/);
assert.deepEqual(await client.me(), { resident: joined.resident, name: "Tess", model: "sdk-typescript", owner: joined.owner });
const listed = await client.listResidents();
assert.deepEqual(listed.residents.map((resident) => resident.id), [joined.resident]);
assert.ok(listed.limit >= 1);

const catalogue = await client.catalogue();
const verbs = catalogue.verbs.map((verb) => verb.verb);
for (const verb of ["list_jobs", "move_to", "work", "buy", "eat", "say", "set_routines"]) assert.ok(verbs.includes(verb), verb);

// ---- Observe, act, wait -----------------------------------------------------

const first = await client.observe();
const me = selfOf(first);
assert.equal(me.id, joined.resident);
assert.equal(me.name, "Tess");
assert.equal(me.activity, null);
assert.equal(hasNews(first), false);
const startCash = me.cash;
// A new resident has the default routines of the game: they keep it alive when its owner is away.
const born = selfOf(await client.observe({ detail: "full", sections: ["self"] }));
assert.deepEqual(born.routines.map((one) => [one.name, one.then.verb]), [
  ["eat when hungry", "eat"],
  ["sleep when tired", "sleep"],
]);

// Find paid work: the list of jobs is a free verb of the global realm. Its answer is a report.
const jobs = await done(client, "list_jobs", {});
const works = jobs.result.public_works;
assert.equal(works.open, true);

// The public map says where the sites are.
const layout = await client.world();
const city = layout.cities.find((entry) => entry.id === me.city);
let workplace;
for (const zone of city.zones) {
  const view = await client.zone(zone);
  workplace ??= view.sites.find((site) => site.kind === "depot");
}
assert.ok(workplace !== undefined, "the city has a depot");
assert.ok(works.kinds.includes("depot"), "the depot has starter shifts");

const walked = await done(client, "move_to", { site: workplace.id });
assert.equal(walked.result.site, workplace.id);
const shift = await done(client, "work", {});
assert.equal(shift.result.wage, works.wage);
assert.equal(shift.result.net + shift.result.tax, works.wage);
const paid = selfOf(await client.observe({ sections: ["self"] }));
assert.equal(paid.cash, startCash + shift.result.net);
assert.equal(paid.site.id, workplace.id);

// ---- A plan stops at the first refusal --------------------------------------

const plan = await client.plan([
  { verb: "wait", args: { ticks: 2 } },
  { verb: "no_such_verb" },
  { verb: "wait", args: { ticks: 2 } },
]);
assert.deepEqual(plan.acks.map((ack) => ack.status.state), ["started", "rejected"]);
assert.equal(plan.notSent, 1);
assert.equal(rejection(plan.acks[1]).code, "unknown_verb");
await reportOf(client, plan.acks[0]);

// The same request id again does not act again.
const once = await client.act("wait", { ticks: 3 }, { id: "ts-sdk-once" });
const twice = await client.act("wait", { ticks: 3 }, { id: "ts-sdk-once" });
assert.deepEqual(twice.ack, once.ack);
await reportOf(client, once.ack);

// ---- Actions without an observation -----------------------------------------

const cursor = client.cursor;
const fast = await client.act("wait", { ticks: 2 }, { observe: false });
assert.deepEqual(Object.keys(fast), ["ack"]);
assert.equal(fast.ack.status.state, "started");
const fastPlan = await client.plan([{ verb: "wait", args: { ticks: 1 } }, { verb: "no_such_verb" }, { verb: "wait", args: { ticks: 1 } }], { observe: false });
assert.deepEqual(Object.keys(fastPlan), ["acks", "notSent"]);
assert.deepEqual(fastPlan.acks.map((one) => one.status.state), ["queued", "rejected"]);
assert.equal(fastPlan.notSent, 1);
// The cursor did not move, so the long poll brings both reports.
assert.equal(client.cursor, cursor);
assert.equal((await reportOf(client, fastPlan.acks[0])).ok, true);
assert.deepEqual(reportsOf(await client.observe({ since: cursor })).map((report) => report.action_id), [fast.ack.action_id, fastPlan.acks[0].action_id]);

// ---- A sponsor publishes a mission -------------------------------------------

const mission = {
  title: "Bread for the fair",
  text: "The town needs bread.",
  objectives: [{ event: "economy.goods_produced", fields: { item: "bread" }, count: 10 }],
  reward: 5000,
  deadline_in: 2880,
  takers: 2,
};
// The world publishes the mission in the frame after the request: the world runs while the call is open.
const posted = await until(client.postMission(mission));
assert.deepEqual(posted, { mission: posted.mission, request: posted.request, frame: posted.frame, deadline: posted.frame + 2880, reward: 5000, takers: 2 });
assert.ok(Number.isSafeInteger(posted.mission) && Number.isSafeInteger(posted.request));
// A resident finds the mission. The name of the sponsor is the name of the owner.
const found = await done(client, "mission_list", { mission: posted.mission });
assert.deepEqual(found.result.mission.title, { untrusted: true, from: 0, text: "Bread for the fair" });
assert.deepEqual(found.result.mission.sponsor, { untrusted: true, from: 0, text: "TypeScript SDK" });
/** Returns a check for the error of a mission that the server refuses. */
const refusedWith = (code, status) => (error) => {
  assert.ok(error instanceof ApiError, String(error));
  assert.deepEqual([error.code, error.status], [code, status]);
  return true;
};
// The server refuses these before the world sees them.
await assert.rejects(client.postMission({ ...mission, reward: 400_000 }), refusedWith("invalid_request", 400));
await assert.rejects(client.postMission({ ...mission, title: "read www.example.com" }), refusedWith("text_refused", 422));
await assert.rejects(new Client({ baseUrl: BASE, ownerToken: joined.residentKey }).postMission(mission), refusedWith("wrong_token_kind", 403));
// The missions module refuses this one, in the frame after the request.
await assert.rejects(until(client.postMission({ ...mission, reward: 1 })), refusedWith("invalid_args", 400));
// An owner publishes three missions in 24 hours. The refusals did not count.
await until(client.postMission(mission));
await until(client.postMission(mission));
await assert.rejects(client.postMission(mission), (error) => {
  refusedWith("rate_limited", 429)(error);
  assert.ok(error.retryAfterSeconds > 86_000 && error.retryAfterSeconds <= 86_400, `${error.retryAfterSeconds}`);
  return true;
});

// ---- Routines ---------------------------------------------------------------

const routine = {
  name: "sleep when exhausted",
  when: { type: "attr_below", attr: "need.energy", value: 1 },
  then: { verb: "sleep", args: {} },
};
assert.deepEqual(await client.setRoutines([routine]), { routines: 1 });
await assert.rejects(
  client.setRoutines([{ name: "fly", when: { type: "always" }, then: { verb: "fly" } }]),
  (error) => error instanceof ApiError && error.code === "unknown_verb" && error.status === 400,
);
const full = selfOf(await client.observe({ detail: "full", sections: ["self"] }));
assert.deepEqual(full.routines.map((one) => one.name), ["sleep when exhausted"]);
assert.deepEqual(await client.setRoutines([]), { routines: 0 });

// ---- Errors -----------------------------------------------------------------

await assert.rejects(client.act("say", { text: "read www.example.com" }), (error) => {
  assert.ok(error instanceof ApiError);
  assert.deepEqual([error.code, error.status, error.isTextRefusal], ["text_refused", 422, true]);
  return true;
});
const owner = new Client({ baseUrl: BASE, ownerToken: joined.ownerToken });
for (let extra = 1; extra < listed.limit; extra += 1) await owner.createResident({ name: `Tess ${extra + 1}` });
await assert.rejects(owner.createResident({ name: "One more" }), (error) => {
  assert.ok(error instanceof ApiError);
  assert.deepEqual([error.code, error.status, error.endsCredential], ["resident_limit", 403, false]);
  return true;
});
await assert.rejects(new Client({ baseUrl: BASE, residentKey: joined.ownerToken }).observe(), (error) => {
  return error instanceof ApiError && error.code === "wrong_token_kind" && error.endsCredential;
});

// ---- A webhook receiver checks the signature with the SDK -------------------

const deliveries = [];
let delivered;
const receiver = http.createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    deliveries.push({ headers: request.headers, body: Buffer.concat(chunks) });
    response.writeHead(204).end();
    delivered?.();
  });
});
await new Promise((resolve) => receiver.listen(0, "127.0.0.1", resolve));
const hook = await client.setWebhook(`http://127.0.0.1:${receiver.address().port}/hook`);
assert.match(hook.secret, /^unsup_whs_/);
const arrival = new Promise((resolve) => {
  delivered = resolve;
});
const hooked = await client.act("wait", { ticks: 1 });
await until(arrival);
const delivery = deliveries[0];
assert.equal(await verifyWebhookRequest(hook.secret, delivery.headers, delivery.body), true);
assert.equal(await verifyWebhookRequest("unsup_whs_wrong", delivery.headers, delivery.body), false);
assert.equal(await verifyWebhookRequest(hook.secret, delivery.headers, `${delivery.body} `), false);
const payload = JSON.parse(delivery.body.toString("utf8"));
assert.equal(payload.resident, joined.resident);
assert.deepEqual(payload.reports.map((report) => report.action_id), [hooked.ack.action_id]);
assert.deepEqual(await client.setWebhook(null), { url: null });
receiver.close();
await client.observe();

// ---- The WebSocket session --------------------------------------------------

const friend = (await Client.join(BASE, { name: "Finn", model: "sdk-typescript" })).client;
const notices = [];
const reports = [];
const sockets = [];
const urls = [];
let opens = 0;
const session = client.connect({
  reconnectDelay: () => 0,
  onNotice: (notice) => notices.push(notice),
  onReport: (report) => reports.push(report),
  onOpen: () => {
    opens += 1;
  },
  webSocket: (url, headers) => {
    urls.push(new URL(url).search);
    const socket = new WebSocket(url, { headers });
    sockets.push(socket);
    return socket;
  },
});
await session.ready;
await session.ping();
const before = client.cursor;

// A pushed notice with the words of another resident, and a pushed report.
assertAccepted((await friend.act("message", { to: joined.resident, text: "hello Tess" })).ack);
const ack = await session.act("wait", { ticks: 2 });
assert.equal(ack.status.state, "started");
for (let frames = 0; notices.length < 1 || reports.length < 1; frames += 1) {
  assert.ok(frames < 200, "the pushes came");
  await step(1);
  await new Promise((resolve) => setTimeout(resolve, 1));
}
assert.equal(notices[0].kind, "society.message");
assert.deepEqual(notices[0].data.text, { untrusted: true, from: friend.resident, text: "hello Tess" });
assert.deepEqual(untrustedOf({ notices, sections: {} }).map((text) => text.text), ["hello Tess"]);
assert.equal(reports[0].action_id, ack.action_id);
assert.equal(typeof reports[0].seq, "number");

// A plan over the session stops at the first refusal.
const acks = await session.plan([{ verb: "wait", args: { ticks: 3 } }, { verb: "no_such_verb" }, { verb: "wait", args: { ticks: 9 } }]);
assert.deepEqual(acks.map((one) => one.status.state), ["started", "rejected"]);
// A text that a filter refuses is an error of the request. The session stays open.
await assert.rejects(session.act("say", { text: "www.example.com" }), (error) => error instanceof ApiError && error.code === "text_refused");
await session.ping();

// The connection is lost. While it is down: a message comes and the action ends.
const cursors = session.cursors;
sockets.at(-1).close();
for (let waits = 0; session.state === "open"; waits += 1) {
  assert.ok(waits < 500, "the session saw the lost connection");
  await new Promise((resolve) => setTimeout(resolve, 1));
}
assertAccepted((await friend.act("message", { to: joined.resident, text: "are you there?" })).ack);
await step(5);
for (let waits = 0; opens < 2 || notices.length < 2 || reports.length < 2; waits += 1) {
  assert.ok(waits < 2_000, `the session connected again and caught up: ${opens} opens, ${notices.length} notices, ${reports.length} reports`);
  await new Promise((resolve) => setTimeout(resolve, 1));
}
// The second connection sent the two cursors, and nothing came two times.
assert.equal(urls[1], `?since=${cursors.since}&reports_since=${cursors.reportsSince}`);
const seqs = [...notices.map((notice) => notice.seq), ...reports.map((report) => report.seq)].sort((a, b) => a - b);
assert.deepEqual(seqs, [before, before + 1, before + 2, before + 3], "each number one time, without a gap");
assert.deepEqual(notices.map((notice) => notice.data.text.text), ["hello Tess", "are you there?"]);
assert.deepEqual(reports.map((report) => report.action_id), [ack.action_id, acks[0].action_id]);
await session.ping();
session.close();

const last = selfOf(await client.observe({ sections: ["self"] }));
fact("typescript", {
  resident: joined.resident,
  mission: posted.mission,
  request: posted.request,
  friend: friend.resident,
  cash: last.cash,
  earned: shift.result.net,
  next_notice: client.cursor,
  pushed: seqs.length,
});
