// The examples of README.md must run.
//
// Each test takes the code of the README as it is, points its imports to
// the compiled sources and its address to the local test server, and runs
// it as a module.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { rm, writeFile } from "node:fs/promises";
import { test, type TestContext } from "node:test";
import { fileURLToPath } from "node:url";

import type { Client, Session } from "../src/index.js";
import { FakeGateway, FakeWorld } from "../src/testing/index.js";
import { Collector, startServer } from "./support/server.js";

/** The address that the examples use for the server. */
const EXAMPLE_URL = "https://unsupervised.example";
/** The compiled tests are two directories below the package root. */
const README = new URL("../../README.md", import.meta.url);
const IMPORTS: [specifier: string, target: URL][] = [
  ['"@unsupervised/sdk/testing"', new URL("../src/testing/index.js", import.meta.url)],
  ['"@unsupervised/sdk"', new URL("../src/index.js", import.meta.url)],
];
let modules = 0;

/** Returns the content of every `ts` block of the README, in order. */
function blocks(): string[] {
  return [...readFileSync(README, "utf8").matchAll(/^```ts\n(.*?)^```$/gms)].map((match) => match[1] as string);
}

/**
 * Runs example code as a module and returns what the module exports. The
 * calls of `console.log` go to `logged`.
 */
async function run(t: TestContext, code: string, logged: Collector<unknown[]>): Promise<Record<string, unknown>> {
  let source = code;
  for (const [specifier, target] of IMPORTS) {
    source = source.replaceAll(specifier, JSON.stringify(target.href));
  }
  modules += 1;
  const file = fileURLToPath(new URL(`../readme-example-${process.pid}-${modules}.mjs`, import.meta.url));
  await writeFile(file, source, "utf8");
  t.after(() => rm(file, { force: true }));
  t.mock.method(console, "log", (...values: unknown[]) => logged.push(values));
  return (await import(file)) as Record<string, unknown>;
}

test("the first example joins, acts and waits, and the session example gets a pushed report", async (t) => {
  const [first, session] = blocks() as [string, string];
  assert.equal(first.includes(EXAMPLE_URL), true);
  // "A ten-line example": the code without empty lines.
  assert.ok(first.split("\n").filter((line) => line.trim() !== "").length <= 10);

  const world = new FakeWorld({ time: "auto" });
  const server = await startServer(new FakeGateway(world));
  t.after(() => server.close());
  const logged = new Collector<unknown[]>();
  // The test keeps every session that the example opens, and closes it at
  // the end. So an example that fails half-way leaves no open connection.
  const opened: Session[] = [];
  t.after(() => opened.forEach((one) => one.close()));
  const keep = "__readmeSessions";
  (globalThis as Record<string, unknown>)[keep] = opened;
  t.after(() => delete (globalThis as Record<string, unknown>)[keep]);
  const track =
    "const connectOnce = client.connect.bind(client);\n" +
    `client.connect = (options) => { const made = connectOnce(options); globalThis.${keep}.push(made); return made; };\n`;
  // The session example goes on where the first example ends.
  const code = `${first}\n${track}\n${session}\nexport { client, session };\n`.replaceAll(EXAMPLE_URL, server.url);
  const made = (await run(t, code, logged)) as { client: Client; session: Session };
  assert.deepEqual(opened, [made.session]);

  const resident = world.residents.get(1);
  assert.equal(resident?.name, "Ada");
  assert.equal(resident?.model, "my-model");
  assert.deepEqual(logged.items, [
    ["store this key:", resident?.key],
    ["cancel, set_routines, wait, move_to"],
    ["started", { x: 32, y: 32 }],
    ["move_to", true, { x: 34, y: 32, steps: 2, ticks: 2 }],
  ]);
  // The wait of the example ended with the report of the walk: 2 ticks, not 120.
  assert.equal(world.tick, 2);

  // The session is open and its `wait` runs. The server pushes the report when the action ends.
  assert.equal(made.session.state, "open");
  assert.equal(resident?.current?.verb, "wait");
  world.step(30);
  assert.deepEqual((await logged.atLeast(5))[4], ["wait", true]);
});

test("the test support example joins the fake world without a network", async (t) => {
  const example = blocks().find((block) => block.includes("FakeGateway"));
  assert.ok(example !== undefined);
  const logged = new Collector<unknown[]>();
  const made = (await run(t, `${example}\nexport { client, gateway };\n`, logged)) as { client: Client; gateway: FakeGateway };
  assert.equal(made.gateway.world.residents.get(1)?.name, "Ada");
  assert.deepEqual(await made.client.me(), { resident: 1, name: "Ada", model: null, owner: 1 });
  assert.deepEqual(logged.items, []);
});
