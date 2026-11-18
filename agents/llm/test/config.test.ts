// The configuration from environment variables.

import assert from "node:assert/strict";
import { test } from "node:test";

import { ConfigError, VARIABLES, readConfig } from "../src/index.js";

const NEEDED = { UNSUPERVISED_URL: "https://world.example", OPENAI_MODEL: "small-model" };

/** Returns the message of the `ConfigError` that an environment gives. */
function refusal(env: Record<string, string | undefined>): string {
  try {
    readConfig(env);
  } catch (error) {
    assert.ok(error instanceof ConfigError);
    return error.message;
  }
  throw new Error("the environment was accepted");
}

test("the two necessary variables are enough; the others have defaults", () => {
  assert.deepEqual(readConfig(NEEDED), {
    serverUrl: "https://world.example",
    residentKey: undefined,
    ownerToken: undefined,
    residentName: "llm-agent",
    stateFile: ".unsupervised-agent.json",
    notesFile: "notes.md",
    modelUrl: "https://api.openai.com/v1",
    modelKey: undefined,
    model: "small-model",
    maxTokens: 500,
    jsonMode: false,
    decisionsPerHour: 12,
    turns: undefined,
  });
});

test("every variable sets its value", () => {
  const env = {
    UNSUPERVISED_URL: " http://127.0.0.1:8080/ ",
    UNSUPERVISED_RESIDENT_KEY: "resident-key",
    UNSUPERVISED_OWNER_TOKEN: "owner-token",
    UNSUPERVISED_NAME: "Ada",
    OPENAI_BASE_URL: "http://localhost:11434/v1/",
    OPENAI_API_KEY: "model-key",
    OPENAI_MODEL: "local-model",
    AGENT_DECISIONS_PER_HOUR: "6",
    AGENT_MAX_TOKENS: "300",
    AGENT_JSON_MODE: "1",
    AGENT_NOTES_FILE: "/tmp/ada-notes.md",
    AGENT_STATE_FILE: "/tmp/ada.json",
    AGENT_TURNS: "40",
  };
  assert.deepEqual(readConfig(env), {
    serverUrl: "http://127.0.0.1:8080",
    residentKey: "resident-key",
    ownerToken: "owner-token",
    residentName: "Ada",
    stateFile: "/tmp/ada.json",
    notesFile: "/tmp/ada-notes.md",
    modelUrl: "http://localhost:11434/v1",
    modelKey: "model-key",
    model: "local-model",
    maxTokens: 300,
    jsonMode: true,
    decisionsPerHour: 6,
    turns: 40,
  });
  // The help names every variable that the command reads, and no other.
  assert.deepEqual(VARIABLES.map(([name]) => name).sort(), Object.keys(env).sort());
});

test("an empty variable counts as absent", () => {
  const config = readConfig({ ...NEEDED, UNSUPERVISED_RESIDENT_KEY: "", OPENAI_API_KEY: "  ", AGENT_TURNS: "", AGENT_JSON_MODE: "" });
  assert.equal(config.residentKey, undefined);
  assert.equal(config.modelKey, undefined);
  assert.equal(config.turns, undefined);
  assert.equal(config.jsonMode, false);
});

test("a missing or unusable value is refused with the name of the variable", () => {
  assert.equal(refusal({ OPENAI_MODEL: "m" }), "Set the environment variable UNSUPERVISED_URL.");
  assert.equal(refusal({ UNSUPERVISED_URL: "https://world.example" }), "Set the environment variable OPENAI_MODEL.");
  assert.equal(refusal({ ...NEEDED, UNSUPERVISED_URL: "world.example" }), "UNSUPERVISED_URL must be an address such as https://unsupervised.example.");
  assert.equal(refusal({ ...NEEDED, UNSUPERVISED_URL: "ftp://world.example" }), "UNSUPERVISED_URL must start with http:// or https://.");
  assert.equal(refusal({ ...NEEDED, OPENAI_BASE_URL: "ws://llm.example" }), "OPENAI_BASE_URL must start with http:// or https://.");
  for (const value of ["0", "-3", "2.5", "many"]) {
    assert.equal(refusal({ ...NEEDED, AGENT_DECISIONS_PER_HOUR: value }), "AGENT_DECISIONS_PER_HOUR must be a whole number of at least 1.");
  }
  assert.equal(refusal({ ...NEEDED, AGENT_MAX_TOKENS: "0" }), "AGENT_MAX_TOKENS must be a whole number of at least 1.");
  assert.equal(refusal({ ...NEEDED, AGENT_TURNS: "x" }), "AGENT_TURNS must be a whole number of at least 1.");
  assert.equal(refusal({ ...NEEDED, AGENT_JSON_MODE: "yes" }), "AGENT_JSON_MODE must be 1 or 0.");
});

test("the JSON mode flag reads 1, 0, true and false", () => {
  const mode = (value: string): boolean => readConfig({ ...NEEDED, AGENT_JSON_MODE: value }).jsonMode;
  assert.deepEqual(["1", "true", "TRUE", "0", "false"].map(mode), [true, true, true, false, false]);
});
