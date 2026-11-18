// The library example of README.md must run.
//
// The test takes the code of the README as it is, points its addresses to a
// local fake server and a local fake chat endpoint, and runs it as a module.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { FakeGateway, FakeWorld } from "@unsupervised/sdk/testing";

import { VARIABLES } from "../src/index.js";
import { serve } from "./support/http.js";
import { answer, dataOf } from "./support/scene.js";

/** The compiled tests are two directories below the package root. */
const README = readFileSync(new URL("../../README.md", import.meta.url), "utf8");

test("the library example joins and runs its turns with the model", { timeout: 60_000 }, async (t) => {
  const example = [...README.matchAll(/^```ts\n(.*?)^```$/gms)].map((match) => match[1] as string)[0];
  assert.ok(example !== undefined);
  // "A ten-line example": the code without empty lines.
  assert.ok(example.split("\n").filter((line) => line.trim() !== "").length <= 10);

  const directory = await mkdtemp(join(tmpdir(), "unsupervised-readme-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const world = new FakeWorld({ time: "auto" });
  const gateway = new FakeGateway(world);
  // When the test is over, the server refuses every key. An example that
  // still runs then stops at once: a refused key ends `run`.
  let over = false;
  const refusal = JSON.stringify({ error: { code: "unauthorized", message: "The test is over." } });
  const game = await serve((request) =>
    over ? { status: 401, headers: { "content-type": "application/json" }, body: refusal } : gateway.handle(request),
  );
  const chats: { authorization: string | undefined; body: Record<string, any> }[] = [];
  const reply = answer({ actions: [{ verb: "move_to", args: { x: 33, y: 33 } }], note: "I am Ada.", wait_seconds: 900 });
  const model = await serve((request) => {
    chats.push({ authorization: request.headers["authorization"], body: JSON.parse(request.body) as Record<string, any> });
    return { status: 200, headers: { "content-type": "application/json" }, body: JSON.stringify({ choices: [{ message: { content: reply } }] }) };
  });
  let exampleRun: Promise<unknown> = Promise.resolve();
  t.after(async () => {
    over = true;
    await exampleRun.catch(() => undefined);
    await game.close();
    await model.close();
  });

  const notes = join(directory, "notes.md");
  const replacements: [string, string][] = [
    ['"@unsupervised/agent-llm"', JSON.stringify(new URL("../src/index.js", import.meta.url).href)],
    ['"https://unsupervised.example"', JSON.stringify(game.url)],
    ['"https://api.openai.com/v1"', JSON.stringify(`${model.url}/v1`)],
    ['"notes.md"', JSON.stringify(notes)],
  ];
  let code = example;
  for (const [from, to] of replacements) {
    assert.equal(code.split(from).length, 2, `the example has ${from} one time`);
    code = code.replace(from, to);
  }
  // The module is next to the compiled tests, so `@unsupervised/sdk` resolves as in a project.
  const file = fileURLToPath(new URL(`../readme-example-${process.pid}.mjs`, import.meta.url));
  await writeFile(file, code, "utf8");
  t.after(() => rm(file, { force: true }));
  const before = process.env["OPENAI_API_KEY"];
  process.env["OPENAI_API_KEY"] = "example-key";
  t.after(() => {
    if (before === undefined) delete process.env["OPENAI_API_KEY"];
    else process.env["OPENAI_API_KEY"] = before;
  });

  exampleRun = import(file);
  await exampleRun;

  const resident = world.residents.get(1);
  assert.equal(resident?.name, "Ada");
  assert.equal(resident?.model, "my-model");
  assert.deepEqual(resident?.pos, { x: 33, y: 33 });
  // The example uses the real clock, and the fake world answers a long poll
  // at once. So all 20 turns pass in less than the least interval between
  // two decisions (30 seconds): the first turn asks the model, and the 19
  // others each wait 30 seconds of game time.
  assert.equal(chats.length, 1);
  assert.equal(world.tick, 1 + 19 * 30);
  assert.equal(chats[0]?.authorization, "Bearer example-key");
  assert.equal(chats[0]?.body["model"], "my-model");
  assert.equal(dataOf(chats[0]?.body["messages"])["turns_left_this_hour"], 5);
  assert.equal((await readFile(notes, "utf8")).startsWith("[year 1 day 1 0:00] I am Ada.\n"), true);
});

test("the table of the README names every variable that the command reads, and no other", () => {
  const rows = [...README.matchAll(/^\| `([A-Z_]+)` \|/gm)].map((match) => match[1] as string);
  assert.deepEqual(rows, VARIABLES.map(([name]) => name));
});
