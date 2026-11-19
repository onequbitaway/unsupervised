// Sends the probes of `sdk/conformance/probes.json` to the fake gateway of
// this SDK and to a real server, and compares the answers: the status, the
// error code and the form of the body. See `sdk/conformance/README.md`.
//
//   npm run build && node e2e/conformance.mjs
//
// The Rust test `fakes_answer_like_the_real_server` in `crates/e2e` starts
// the server and runs this script.

import { readFile } from "node:fs/promises";

import { FakeGateway } from "../dist/testing/index.js";
import { ADMIN, BASE, admin, fact, step, until } from "./support.mjs";

/** Game minutes of one day, and the greatest number of days that a resident without food lives in a probe. */
const DAY = 1440;
const DAYS_MAX = 30;

const probes = JSON.parse(await readFile(new URL("../../conformance/probes.json", import.meta.url), "utf8"));

/** Returns the form of a JSON value: the names of its fields and the type of each value. */
function shapeOf(value, path, probe) {
  if ((probe.ignore ?? []).includes(path)) return "(not compared)";
  if (value === null) return "null";
  if (Array.isArray(value)) {
    const entries = probe.lists === "first" ? value.slice(0, 1) : value;
    return entries.map((entry) => shapeOf(entry, path, probe));
  }
  if (typeof value === "object") {
    const out = {};
    for (const name of Object.keys(value).sort()) {
      const inner = path === "" ? name : `${path}.${name}`;
      if (!(probe.ignore ?? []).includes(inner)) out[name] = shapeOf(value[name], inner, probe);
    }
    return out;
  }
  return typeof value;
}

/** Reads the value at a path such as `resident.id`. */
function at(value, path) {
  return path.split(".").reduce((inner, name) => inner?.[name], value);
}

/** One side of the comparison: the fake or the real server. */
class Side {
  constructor(name, send) {
    this.name = name;
    this.send = send;
    this.saved = {};
  }

  /** Sends a request. Returns the status and the parsed body. */
  async request(method, path, token, body, raw) {
    const headers = {};
    if (token !== undefined) headers.authorization = `Bearer ${token}`;
    let text;
    if (raw !== undefined) {
      text = raw.text;
      headers["content-type"] = raw.contentType;
    } else if (body !== undefined) {
      text = JSON.stringify(body);
      headers["content-type"] = "application/json";
    }
    const answer = await this.send(method, path, headers, text);
    let parsed = null;
    try {
      parsed = answer.body === "" ? null : JSON.parse(answer.body);
    } catch {
      parsed = { "(not JSON)": answer.body.slice(0, 80) };
    }
    return { status: answer.status, body: parsed };
  }

  token(auth) {
    if (auth === "owner") return this.saved.owner_token;
    if (auth === "resident") return this.saved.resident_key;
    if (auth === "other") return this.saved.other_key;
    if (auth === "unknown") return "unsup_res_00000000000000000000000000000000000000000000000000000000000000ff";
    return undefined;
  }

  /** Makes the second owner with its resident, for the probes that need another party. */
  async setUp() {
    const owner = await this.request("POST", "/v1/owners", undefined, { name: "Other owner" });
    const resident = await this.request("POST", "/v1/residents", owner.body.token, { name: "Bo" });
    this.saved.other = resident.body.resident.id;
    this.saved.other_key = resident.body.key;
  }

  /** Runs one probe that is a request. */
  async probe(probe) {
    const path = probe.path.replace("{resident}", this.saved.resident).replace("{other}", this.saved.other);
    const raw = probe.raw === undefined ? undefined : { text: probe.raw, contentType: probe.content_type };
    const sent = this.request(probe.method, path, this.token(probe.auth), probe.body, raw);
    // The answer of the real server to some requests needs a frame of the world.
    const answer = await (probe.runs === true ? this.during(sent) : sent);
    for (const [name, where] of Object.entries(probe.save ?? {})) this.saved[name] = at(answer.body, where);
    const seen = { status: answer.status, code: answer.body?.error?.code ?? null };
    if (probe.shape === true) seen.shape = shapeOf(answer.body, "", probe);
    if (probe.count !== undefined) seen.count = at(answer.body, probe.count)?.length ?? "(no list)";
    return seen;
  }
}

// ---- The fake ---------------------------------------------------------------

const gateway = new FakeGateway();
const fake = new Side("fake", async (method, url, headers, body) => {
  const answer = gateway.handle({ method, url, headers, body });
  if (url.startsWith("/v1/wait")) {
    // The fake world moves only when a test steps it. A long poll that
    // parks ends here as its timeout does on the real server.
    await Promise.race([answer, gateway.world.parked()]);
    gateway.world.expireWaits();
  }
  return answer;
});
fake.advance = async (ticks) => gateway.world.step(ticks);
// The fake answers at once: its world does not have to run.
fake.during = (request) => request;
fake.starve = async () => {
  gateway.world.die(gateway.world.residents.get(fake.saved.other));
};
fake.tell = async (text) => {
  gateway.world.tell(fake.saved.resident, fake.saved.other, text, "society.message");
};
fake.sanction = async (kind) => {
  const resident = gateway.world.residents.get(fake.saved.resident);
  if (kind === "mute") gateway.muted.add(resident.id);
  else if (kind === "unmute") gateway.muted.delete(resident.id);
  else if (kind === "ban") gateway.banned.add(resident.owner);
  else if (kind === "unban") gateway.banned.delete(resident.owner);
  else if (kind === "despawn") gateway.world.despawn(resident);
  else throw new Error(`the sanction ${kind} is not known`);
};

// ---- The real server --------------------------------------------------------

const real = new Side("real", async (method, path, headers, body) => {
  const response = await fetch(`${BASE}${path}`, { method, headers, ...(body === undefined ? {} : { body }) });
  return { status: response.status, body: await response.text() };
});
real.advance = async (ticks) => {
  await step(ticks);
};
real.during = (request) => until(request);
real.starve = async () => {
  // The public profile says when the resident is dead. No call with its key is made: the first one is a probe.
  for (let days = 0; ; days += 1) {
    const profile = await (await fetch(`${BASE}/v1/residents/${real.saved.other}/profile`)).json();
    if (profile.alive === false) return;
    if (days >= DAYS_MAX) throw new Error(`a resident without food died in ${DAYS_MAX} game days`);
    await step(DAY);
  }
};
real.tell = async (text) => {
  const action = { verb: "message", args: { to: real.saved.resident, text } };
  const sent = await real.request("POST", "/v1/act", real.saved.other_key, action);
  if (sent.status !== 200 || sent.body.acks[0].status.state !== "done") {
    throw new Error(`the other resident cannot send a message: ${JSON.stringify(sent.body)}`);
  }
  // A message to another district arrives in the next frame.
  await step(2);
};
real.sanction = async (kind) => {
  if (ADMIN === "") throw new Error("Set E2E_ADMIN_TOKEN for the sanctions.");
  const me = await real.request("GET", "/v1/residents", real.saved.owner_token);
  if (kind === "mute" || kind === "unmute" || kind === "despawn") {
    await admin(`/admin/residents/${real.saved.resident}/${kind}`);
  } else {
    // The owner id is in the answer of `/v1/me` of one of its residents.
    const owner = real.saved.owner ?? (await real.request("GET", "/v1/me", real.saved.resident_key)).body.owner;
    real.saved.owner = owner;
    await admin(`/admin/owners/${owner}/${kind}`);
  }
  return me;
};

// ---- Compare ----------------------------------------------------------------

/** Runs one probe on one side. A side that does not answer in ten seconds fails the run. */
function inTime(side, probe) {
  let timer;
  const late = new Promise((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`the ${side.name} side did not answer the probe "${probe.name}"`)), 10_000);
  });
  return Promise.race([side.probe(probe), late]).finally(() => clearTimeout(timer));
}

const mismatches = [];
let requests = 0;
await fake.setUp();
await real.setUp();
for (const probe of probes) {
  if (probe.advance !== undefined) {
    await fake.advance(probe.advance);
    await real.advance(probe.advance);
  } else if (probe.tell !== undefined) {
    await fake.tell(probe.tell);
    await real.tell(probe.tell);
  } else if (probe.starve !== undefined) {
    if (probe.starve !== "other") throw new Error(`the probes starve only the other resident, not ${probe.starve}`);
    await fake.starve();
    await real.starve();
  } else if (probe.sanction !== undefined) {
    // The owner id of the real side is read while the key still works.
    if (probe.sanction === "ban") real.saved.owner = (await real.request("GET", "/v1/me", real.saved.resident_key)).body.owner;
    await fake.sanction(probe.sanction);
    await real.sanction(probe.sanction);
  } else {
    requests += 1;
    const [mine, theirs] = [await inTime(fake, probe), await inTime(real, probe)];
    const [a, b] = [JSON.stringify(mine), JSON.stringify(theirs)];
    if (a !== b) mismatches.push({ probe: probe.name, fake: mine, real: theirs });
    // The probe says which answer it is about. Another answer of both
    // sides would compare well and show nothing.
    const expected = `${probe.status} ${probe.code ?? ""}`.trim();
    const got = `${theirs.status} ${theirs.code ?? ""}`.trim();
    if (expected !== got) mismatches.push({ probe: `${probe.name} (the probe expects ${expected})`, fake: mine, real: theirs });
    if (process.env.E2E_PRINT === "1") console.error(`${got}\t${probe.name}`);
  }
}
for (const mismatch of mismatches) {
  console.error(`MISMATCH: ${mismatch.probe}\n  fake: ${JSON.stringify(mismatch.fake)}\n  real: ${JSON.stringify(mismatch.real)}`);
}
fact("conformance", { sdk: "typescript", probes: requests, mismatches: mismatches.length });
process.exit(mismatches.length === 0 ? 0 : 1);
