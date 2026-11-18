// The command as a process: the environment, real HTTP to a local fake
// server and a local fake chat endpoint, and the stop signal.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { fileURLToPath } from "node:url";

import { FakeGateway, FakeWorld } from "@unsupervised/sdk/testing";

import { answer, dataOf } from "./support/scene.js";
import { serve } from "./support/http.js";

/** The compiled entry point, next to the compiled tests. */
const MAIN = fileURLToPath(new URL("../src/main.js", import.meta.url));

interface Stage {
  world: FakeWorld;
  /** The bodies of the requests that the chat endpoint got, with their authorization header. */
  chats: { authorization: string | undefined; path: string; body: Record<string, any> }[];
  directory: string;
  env: Record<string, string>;
}

/** Starts a fake world and a fake chat endpoint on local ports. `reply` is the text of every model answer. */
async function stage(t: TestContext, time: "auto" | "manual", reply: string): Promise<Stage> {
  const directory = await mkdtemp(join(tmpdir(), "unsupervised-command-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const world = new FakeWorld({ time });
  const gateway = new FakeGateway(world);
  const game = await serve((request) => gateway.handle(request));
  const chats: Stage["chats"] = [];
  const model = await serve((request) => {
    chats.push({ authorization: request.headers["authorization"], path: request.url, body: JSON.parse(request.body) as Record<string, any> });
    const body = JSON.stringify({ choices: [{ message: { content: reply } }] });
    return { status: 200, headers: { "content-type": "application/json" }, body };
  });
  t.after(async () => {
    world.expireWaits();
    await game.close();
    await model.close();
  });
  return {
    world,
    chats,
    directory,
    env: {
      PATH: process.env["PATH"] ?? "",
      UNSUPERVISED_URL: game.url,
      UNSUPERVISED_NAME: "Ada",
      OPENAI_BASE_URL: `${model.url}/v1`,
      OPENAI_API_KEY: "model-key",
      OPENAI_MODEL: "small-model",
      AGENT_STATE_FILE: join(directory, "agent.json"),
      AGENT_NOTES_FILE: join(directory, "notes.md"),
    },
  };
}

interface Running {
  /** Sends a signal to the process. */
  kill(signal: NodeJS.Signals): void;
  /** Resolves when the process ended, with its exit status and its output. */
  ended: Promise<{ status: number | null; signal: NodeJS.Signals | null; stdout: string; stderr: string }>;
}

/**
 * Starts the command. Both output pipes are read while it runs. A process
 * that still runs when the test ends is killed, so a failed test leaves
 * nothing behind.
 */
function start(t: TestContext, env: Record<string, string>): Running {
  const child = spawn(process.execPath, [MAIN], { env, stdio: ["ignore", "pipe", "pipe"] });
  t.after(() => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => (stdout += chunk));
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => (stderr += chunk));
  const ended = new Promise<Awaited<Running["ended"]>>((resolve, reject) => {
    child.on("error", reject);
    child.on("close", (status, signal) => resolve({ status, signal, stdout, stderr }));
  });
  return { kill: (signal) => child.kill(signal), ended };
}

test("the process joins from the environment, asks the chat endpoint and acts", { timeout: 20_000 }, async (t) => {
  const reply = answer({ thought: "walk", actions: [{ verb: "move_to", args: { x: 34, y: 32 } }], note: "I can walk.", wait_seconds: 60 });
  const { world, chats, directory, env } = await stage(t, "auto", reply);
  const { status, stdout, stderr } = await start(t, { ...env, AGENT_TURNS: "1", AGENT_DECISIONS_PER_HOUR: "7", AGENT_MAX_TOKENS: "321" }).ended;

  assert.equal(stderr, "");
  assert.equal(status, 0, stdout);
  // The resident exists with the name and the model of the environment, and it walked.
  const resident = world.residents.get(1);
  assert.equal(resident?.name, "Ada");
  assert.equal(resident?.model, "small-model");
  assert.deepEqual(resident?.pos, { x: 34, y: 32 });

  // One request went to the chat endpoint, with the key, the model and the two messages.
  assert.equal(chats.length, 1);
  assert.equal(chats[0]?.path, "/v1/chat/completions");
  assert.equal(chats[0]?.authorization, "Bearer model-key");
  assert.equal(chats[0]?.body["model"], "small-model");
  assert.equal(chats[0]?.body["max_tokens"], 321);
  assert.equal(dataOf(chats[0]?.body["messages"])["turns_left_this_hour"], 6);

  // The key is in the state file and the note is in the notes file. Neither secret is in the output.
  const state = JSON.parse(await readFile(join(directory, "agent.json"), "utf8")) as { servers: Record<string, { resident_key: string }> };
  assert.equal(state.servers[env["UNSUPERVISED_URL"] as string]?.resident_key, resident?.key);
  assert.equal(await readFile(join(directory, "notes.md"), "utf8"), "[year 1 day 1 0:00] I can walk.\n");
  assert.equal(stdout.includes(resident?.key as string), false);
  assert.equal(stdout.includes("model-key"), false);
  assert.match(stdout, /the agent stopped after 1 decision\(s\)\n$/);
});

test("the process stops cleanly on SIGINT while it blocks in the long poll", { timeout: 20_000 }, async (t) => {
  const { world, chats, env } = await stage(t, "manual", answer({ wait_seconds: 600 }));
  const running = start(t, env);
  // The world parks the long poll of the agent: the decision is made and the agent waits.
  await world.parked();
  assert.equal(chats.length, 1);
  running.kill("SIGINT");
  const { status, signal, stdout, stderr } = await running.ended;

  assert.equal(stderr, "");
  assert.equal(signal, null);
  assert.equal(status, 0, stdout);
  assert.match(stdout, /the agent stopped after 1 decision\(s\)\n$/);
});

test("the process exits with 2 and names the missing variable", { timeout: 20_000 }, async (t) => {
  const { chats, env } = await stage(t, "auto", answer({}));
  const { OPENAI_MODEL: _model, ...withoutModel } = env;
  const { status, stdout } = await start(t, withoutModel).ended;
  assert.equal(status, 2);
  assert.equal(stdout.startsWith("Set the environment variable OPENAI_MODEL.\n"), true);
  assert.equal(chats.length, 0);
});
