// The way into the world (the state file) and the command.

import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";

import { ApiError } from "@unsupervised/sdk";
import { FakeGateway, FakeWorld } from "@unsupervised/sdk/testing";

import { StateError, enlist, fitBytes, loadState, main, saveState, usage, type EnlistOptions } from "../src/index.js";
import { FAKE_URL, FakeProvider, answer } from "./support/scene.js";

interface Bench {
  world: FakeWorld;
  gateway: FakeGateway;
  directory: string;
  stateFile: string;
  log: string[];
  /** Options of `enlist` for this bench. */
  options(extra?: Partial<EnlistOptions>): EnlistOptions;
  /** The environment of `main` for this bench. */
  env(extra?: Record<string, string>): Record<string, string>;
}

async function bench(t: TestContext): Promise<Bench> {
  const directory = await mkdtemp(join(tmpdir(), "unsupervised-agent-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const world = new FakeWorld({ time: "auto" });
  const gateway = new FakeGateway(world);
  const stateFile = join(directory, "state", "agent.json");
  const log: string[] = [];
  return {
    world,
    gateway,
    directory,
    stateFile,
    log,
    options: (extra = {}) => ({
      serverUrl: FAKE_URL,
      stateFile,
      residentName: "Ada",
      model: "small-model",
      client: { fetch: gateway.fetch, sleep: async () => undefined },
      log: (line) => log.push(line),
      ...extra,
    }),
    env: (extra = {}) => ({
      UNSUPERVISED_URL: FAKE_URL,
      OPENAI_MODEL: "small-model",
      AGENT_STATE_FILE: stateFile,
      AGENT_NOTES_FILE: join(directory, "notes.md"),
      ...extra,
    }),
  };
}

function posts(gateway: FakeGateway, path: string): number {
  return gateway.requests.filter((request) => request.method === "POST" && request.path === path).length;
}

test("the first run signs up, creates the resident and writes the state file for the user only", async (t) => {
  const { world, stateFile, options, log } = await bench(t);
  const client = await enlist(options());

  const resident = world.residents.get(1);
  assert.equal(resident?.name, "Ada");
  assert.equal(resident?.model, "small-model");
  assert.deepEqual(await client.me(), { resident: 1, name: "Ada", model: "small-model", owner: 1 });
  const ownerToken = client.ownerToken as string;
  assert.equal(world.ownerOf(ownerToken), 1);
  assert.deepEqual(JSON.parse(await readFile(stateFile, "utf8")), {
    servers: { [FAKE_URL]: { owner: 1, owner_token: ownerToken, resident: 1, resident_key: resident?.key } },
  });
  assert.equal((await stat(stateFile)).mode & 0o777, 0o600);
  // The log names the resident and the file, never a secret.
  assert.equal(log.length, 2);
  assert.equal(log.join("\n").includes(resident?.key as string), false);
  assert.equal(log.join("\n").includes(ownerToken), false);
});

test("a second run controls the same resident and signs up nobody", async (t) => {
  const { world, gateway, options } = await bench(t);
  await enlist(options());
  const second = await enlist(options({ residentName: "Another name" }));

  assert.equal((await second.me()).resident, 1);
  assert.equal(world.residents.size, 1);
  assert.equal(posts(gateway, "/v1/owners"), 1);
  assert.equal(posts(gateway, "/v1/residents"), 1);
});

test("a key that the server no longer takes is rotated with the owner token", async (t) => {
  const { world, stateFile, options } = await bench(t);
  await enlist(options());
  const resident = world.residents.get(1);
  assert.ok(resident !== undefined);
  const lost = world.rotateKey(resident);

  const client = await enlist(options());
  assert.equal((await client.me()).resident, 1);
  assert.notEqual(resident.key, lost);
  assert.equal(client.residentKey, resident.key);
  assert.equal((await loadState(stateFile)).servers[FAKE_URL]?.resident_key, resident.key);
  assert.equal(world.residents.size, 1);
});

test("each server has its own entry in the state file", async (t) => {
  const { stateFile, options } = await bench(t);
  await enlist(options());
  const other = new FakeGateway(new FakeWorld());
  await enlist(options({ serverUrl: "http://other/", client: { fetch: other.fetch } }));

  const state = await loadState(stateFile);
  assert.deepEqual(Object.keys(state.servers), [FAKE_URL, "http://other"]);
  assert.equal(other.world.residents.size, 1);
});

test("a key from the environment is checked and used, and the state file is not touched", async (t) => {
  const { world, gateway, stateFile, options } = await bench(t);
  const resident = world.spawn(world.signUp().owner.id, { name: "Grace" });
  const client = await enlist(options({ residentKey: resident.key }));

  assert.equal(client.residentKey, resident.key);
  assert.deepEqual(gateway.requests.map((request) => request.path), ["/v1/me"]);
  await assert.rejects(stat(stateFile), (error: unknown) => (error as NodeJS.ErrnoException).code === "ENOENT");

  await assert.rejects(enlist(options({ residentKey: "wrong-key" })), (error: unknown) => {
    assert.ok(error instanceof ApiError);
    assert.equal(error.status, 401);
    return true;
  });
});

test("an owner token from the environment creates the resident without a new sign-up", async (t) => {
  const { world, gateway, stateFile, options } = await bench(t);
  const owner = world.signUp();
  await enlist(options({ ownerToken: owner.token }));

  assert.equal(world.residents.get(1)?.owner, owner.owner.id);
  assert.equal(posts(gateway, "/v1/owners"), 0);
  // The token of the environment is not copied into the file.
  assert.deepEqual(Object.keys((await loadState(stateFile)).servers[FAKE_URL] ?? {}), ["resident", "resident_key"]);
});

test("a sign-up that was cut before the resident existed goes on with the same owner", async (t) => {
  const { world, gateway, stateFile, options } = await bench(t);
  const owner = world.signUp();
  await saveState(stateFile, { servers: { [FAKE_URL]: { owner: owner.owner.id, owner_token: owner.token } } });
  await enlist(options());

  assert.equal(posts(gateway, "/v1/owners"), 0);
  assert.equal(world.residents.get(1)?.owner, owner.owner.id);
});

test("a file that is not a state file is refused and stays as it is", async (t) => {
  const { directory, options } = await bench(t);
  for (const content of ["not json", "[]", '{"servers": []}', "null"]) {
    const stateFile = join(directory, "other.json");
    await writeFile(stateFile, content, "utf8");
    await assert.rejects(enlist(options({ stateFile })), (error: unknown) => {
      assert.ok(error instanceof StateError);
      assert.equal(error.message, `${stateFile} is not a state file of this agent. Move it away or set AGENT_STATE_FILE.`);
      return true;
    });
    assert.equal(await readFile(stateFile, "utf8"), content);
  }
});

test("a long name is cut to 64 bytes at a character boundary", async (t) => {
  assert.equal(fitBytes("short"), "short");
  assert.equal(fitBytes("a".repeat(70)), "a".repeat(64));
  // Each of these letters takes two bytes.
  assert.equal(fitBytes("é".repeat(40)), "é".repeat(32));
  assert.equal(fitBytes(`${"a".repeat(63)}é`), "a".repeat(63));

  const { world, options } = await bench(t);
  // A long model name with no character more than 12 times in a row: the text filters let it through.
  const long = `provider/${"model-name-".repeat(8)}`;
  await enlist(options({ model: long }));
  assert.equal(world.residents.get(1)?.model, long.slice(0, 64));
});

test("the command joins, runs its turns with the model and returns 0", async (t) => {
  const { world, gateway, directory, log, env } = await bench(t);
  const provider = new FakeProvider(answer({ actions: [{ verb: "move_to", args: { x: 33, y: 32 } }], note: "I started.", wait_seconds: 60 }));
  const status = await main(env({ AGENT_TURNS: "1", UNSUPERVISED_NAME: "Grace" }), {
    fetch: gateway.fetch,
    provider,
    log: (line) => log.push(line),
  });

  assert.equal(status, 0);
  assert.equal(provider.calls.length, 1);
  const resident = world.residents.get(1);
  assert.equal(resident?.name, "Grace");
  assert.deepEqual(resident?.pos, { x: 33, y: 32 });
  assert.equal(await readFile(join(directory, "notes.md"), "utf8"), "[year 1 day 1 0:00] I started.\n");
  assert.equal(log.at(-1), "the agent stopped after 1 decision(s)");
  assert.equal(log.join("\n").includes(resident?.key as string), false);
});

test("the command returns 2 and prints the help when the environment cannot be used", async (t) => {
  const { gateway, log, env } = await bench(t);
  const status = await main(env({ OPENAI_MODEL: "" }), { fetch: gateway.fetch, log: (line) => log.push(line) });
  assert.equal(status, 2);
  assert.deepEqual(log, ["Set the environment variable OPENAI_MODEL.", usage()]);
  assert.equal(gateway.requests.length, 0);
  for (const name of ["UNSUPERVISED_URL", "OPENAI_BASE_URL", "OPENAI_API_KEY", "OPENAI_MODEL", "AGENT_DECISIONS_PER_HOUR"]) {
    assert.equal(usage().includes(name), true, name);
  }
});

test("the command returns 1 when the server refuses the key", async (t) => {
  const { gateway, log, env } = await bench(t);
  const provider = new FakeProvider();
  const status = await main(env({ UNSUPERVISED_RESIDENT_KEY: "wrong-key" }), {
    fetch: gateway.fetch,
    provider,
    log: (line) => log.push(line),
  });
  assert.equal(status, 1);
  assert.equal(provider.calls.length, 0);
  assert.equal(
    log.at(-1),
    "the agent cannot go on: invalid_token: The server does not know this token. It is wrong or it was replaced. " +
      "Or its resident left the world: it died, or an administrator removed it. " +
      "Check UNSUPERVISED_RESIDENT_KEY, or remove it to start a new resident.",
  );
});

/** A model that plans one long action and waits for it. */
function patient(): FakeProvider {
  return new FakeProvider(answer({ actions: [{ verb: "wait", args: { ticks: 500 } }], wait_seconds: 600 }));
}

test("a resident that dies stops the command with one clear line, and the next run makes a new resident", async (t) => {
  const { world, gateway, stateFile, log, env } = await bench(t);
  const provider = patient();
  // The resident dies while the agent waits in the long poll: that call gets `resident_gone`.
  provider.onCall = () => {
    const resident = world.residents.get(1);
    assert.ok(resident !== undefined);
    world.addVerb({
      info: { verb: "starve", module: "life", summary: "Die.", free: false, scope: "zone", args_schema: { type: "object" } },
      begin: () => ({ busy: 3, finish: () => (world.die(resident), null) }),
    });
    world.submit(resident, { verb: "starve" });
  };
  const options = { fetch: gateway.fetch, provider, log: (line: string) => log.push(line) };
  assert.equal(await main(env({ AGENT_TURNS: "5" }), options), 1);
  assert.equal(provider.calls.length, 1);
  assert.equal(
    log.at(-1),
    "the agent cannot go on: resident 1 left the world: it died, or an administrator removed it. " +
      "Its key does not work any more. Start the agent again to make a new resident.",
  );

  // The next run finds that the resident of the state file is gone and makes a new one.
  provider.onCall = undefined;
  log.length = 0;
  assert.equal(await main(env({ AGENT_TURNS: "1", UNSUPERVISED_NAME: "Ada the second" }), options), 0);
  assert.equal(log[0], `resident 1 of ${stateFile} left the world: it died, or the server has a new world. The agent makes a new resident.`);
  const reborn = [...world.residents.values()];
  assert.deepEqual(reborn.map((resident) => resident.name), ["Ada the second"]);
  assert.notEqual(reborn[0]?.id, 1);
  assert.equal((await loadState(stateFile)).servers[FAKE_URL]?.resident, reborn[0]?.id);
  assert.equal((await loadState(stateFile)).servers[FAKE_URL]?.resident_key, reborn[0]?.key);
  // One owner made both residents.
  assert.equal(posts(gateway, "/v1/owners"), 1);
});

test("the second answer of the server for a dead resident, `invalid_token`, gets the same clear line", async (t) => {
  const { world, gateway, log, env } = await bench(t);
  const provider = patient();
  const options = { fetch: gateway.fetch, provider, log: (line: string) => log.push(line) };
  assert.equal(await main(env({ AGENT_TURNS: "1" }), options), 0);
  const resident = world.residents.get(1);
  assert.ok(resident !== undefined);
  // Another client of the resident gets the one `resident_gone`. This agent gets `invalid_token` from now on.
  world.die(resident);
  const first = await gateway.handle({ method: "GET", url: "/v1/me", headers: { authorization: `Bearer ${resident.key}` } });
  assert.equal(first.status, 410);
  log.length = 0;

  // The key of the environment: the agent has no owner token, so the server cannot tell it more.
  assert.equal(await main(env({ UNSUPERVISED_RESIDENT_KEY: resident.key }), options), 1);
  assert.match(log.at(-1) ?? "", /^the agent cannot go on: invalid_token: .* Or its resident left the world: .* remove it to start a new resident\.$/);

  // With the owner token the agent asks who is left, and says what happened.
  const owner = world.ownerOf((await loadState(env()["AGENT_STATE_FILE"] as string)).servers[FAKE_URL]?.owner_token);
  assert.equal(owner, 1);
  const { farewell } = await import("../src/index.js");
  const { ApiError: SdkError, Client } = await import("@unsupervised/sdk");
  const token = (await loadState(env()["AGENT_STATE_FILE"] as string)).servers[FAKE_URL]?.owner_token as string;
  const client = new Client({ baseUrl: FAKE_URL, fetch: gateway.fetch, residentKey: resident.key, ownerToken: token });
  client.resident = 1;
  const refused = await client.observe().then(
    () => undefined,
    (error: unknown) => error,
  );
  assert.ok(refused instanceof SdkError);
  assert.equal(refused.code, "invalid_token");
  assert.equal(
    await farewell(refused, client, false),
    "resident 1 left the world: it died, or an administrator removed it. Its key does not work any more. Start the agent again to make a new resident.",
  );
  // A key that another key replaced is not a death.
  const second = world.spawn(1, { name: "Bea" });
  const stale = new Client({ baseUrl: FAKE_URL, fetch: gateway.fetch, residentKey: second.key, ownerToken: token });
  stale.resident = second.id;
  world.rotateKey(second);
  const replaced = await stale.observe().then(
    () => undefined,
    (error: unknown) => error,
  );
  assert.ok(replaced instanceof SdkError);
  assert.equal(
    await farewell(replaced, stale, false),
    `the server does not know the key of resident ${second.id} any more: another key replaced it. Start the agent again to get a new key.`,
  );
  // Another refusal keeps the words of the server.
  gateway.banned.add(1);
  const banned = await stale.listResidents().then(
    () => undefined,
    (error: unknown) => error,
  );
  assert.ok(banned instanceof SdkError);
  assert.equal(await farewell(banned, stale, false), "banned: An administrator banned the owner of this token.");
});

test("the command stops with 0 when the caller aborts", async (t) => {
  const { gateway, log, env } = await bench(t);
  const controller = new AbortController();
  const provider = new FakeProvider(answer({ wait_seconds: 600 }));
  // The abort comes during the first request to the model. Without it, three turns ask the model twice.
  provider.onCall = () => controller.abort();
  const status = await main(env({ AGENT_TURNS: "3" }), {
    fetch: gateway.fetch,
    provider,
    signal: controller.signal,
    log: (line) => log.push(line),
  });
  assert.equal(status, 0);
  assert.equal(provider.calls.length, 1);
  assert.equal(log.at(-1), "the agent stopped after 1 decision(s)");
});
