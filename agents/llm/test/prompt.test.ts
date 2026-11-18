// The prompt: fixed instructions, one data block, and limits that keep it small.

import assert from "node:assert/strict";
import { test } from "node:test";

import type { ActionReport, Catalogue, Notice, Observation, Value, VerbInfo } from "@unsupervised/sdk";

import {
  DEFAULT_PROMPT_LIMITS,
  SYSTEM_PROMPT,
  buildData,
  buildMessages,
  cutUntrusted,
  encodeData,
} from "../src/index.js";
import { dataOf } from "./support/scene.js";

function verb(name: string, module: string): VerbInfo {
  return {
    verb: name,
    module,
    summary: `Do the thing that ${name} does, at the place where you are.`,
    free: false,
    scope: "zone",
    args_schema: { type: "object", properties: { target: { type: "integer" }, qty: { type: "integer" } }, required: ["target"] },
  };
}

/** A catalogue as large as the verb table of the game design: 130 verbs in 7 modules. */
function largeCatalogue(): Catalogue {
  const modules = ["kernel", "life", "economy", "society", "crime", "politics", "missions"];
  const verbs = Array.from({ length: 130 }, (_unused, index) => verb(`verb_${index}`, modules[index % modules.length] as string));
  return { verbs, params: [], items: [], attrs: [], conditions: [] };
}

function observation(sections: Record<string, Value> = {}): Observation {
  return {
    frame: 1505,
    tick: 1505,
    clock: { year: 2, day: 3, hour: 1, minute: 5 },
    resident: 1,
    sections: { self: { id: 1, name: "Ada", pos: { x: 3, y: 4 }, cash: 250 }, ...sections },
    notices: [],
    next_notice: 1,
  };
}

function speech(seq: number, text: string): Notice {
  return { seq, tick: seq, kind: "society.speech", from: 2, data: { untrusted: true, from: 2, text } };
}

test("the two messages are the fixed instructions and one data block", () => {
  const messages = buildMessages({ observation: observation(), notices: [], reports: [], catalogue: largeCatalogue(), notes: [] });
  assert.deepEqual(
    messages.map((message) => message.role),
    ["system", "user"],
  );
  assert.equal(messages[0]?.content, SYSTEM_PROMPT);
  const data = dataOf(messages);
  assert.equal(data["time"], "year 2 day 3 01:05, tick 1505");
  assert.deepEqual(data["observation"], { self: { id: 1, name: "Ada", pos: { x: 3, y: 4 }, cash: 250 } });
  // Parts without content are left out.
  assert.deepEqual(Object.keys(data), ["time", "observation", "verbs", "other_verbs"]);
});

test("the prompt stays compact with a full catalogue and a full inbox", () => {
  const nearby = {
    residents: Array.from({ length: 12 }, (_unused, id) => ({ id, name: `Resident number ${id}`, pos: { x: id, y: id }, activity: "work" })),
    sites: Array.from({ length: 12 }, (_unused, id) => ({ id, name: `Site number ${id}`, kind: "shop", pos: { x: id, y: id } })),
  };
  const notices = Array.from({ length: 40 }, (_unused, index) => speech(index + 1, "word ".repeat(400)));
  const reports = Array.from({ length: 40 }, (_unused, index): ActionReport => ({ action_id: index, verb: "work", tick: index, ok: true, result: { wage: 600 } }));
  const messages = buildMessages({
    observation: observation({ nearby }),
    notices,
    reports,
    catalogue: largeCatalogue(),
    notes: Array.from({ length: 20 }, (_unused, index) => `[year 1 day ${index}] ${"fact ".repeat(40)}`.trim()),
    turnsLeft: 3,
  });
  const data = dataOf(messages);
  assert.equal(data["verbs"].length, DEFAULT_PROMPT_LIMITS.maxVerbs);
  assert.equal(Object.values(data["other_verbs"] as Record<string, string[]>).flat().length, 130 - DEFAULT_PROMPT_LIMITS.maxVerbs);
  assert.equal(data["news"].length, DEFAULT_PROMPT_LIMITS.maxNotices);
  assert.equal(data["reports"].length, DEFAULT_PROMPT_LIMITS.maxReports);

  // About four characters are one token: the instructions stay below 600
  // tokens and the worst data block below 4,000.
  assert.ok(SYSTEM_PROMPT.length < 2400, `the instructions have ${SYSTEM_PROMPT.length} characters`);
  const size = messages[1]?.content.length ?? 0;
  assert.ok(size < 16_000, `the data block has ${size} characters`);
});

test("a quiet turn is a small prompt", () => {
  const messages = buildMessages({
    observation: observation(),
    notices: [],
    reports: [],
    catalogue: largeCatalogue(),
    notes: [],
    turnsLeft: 11,
  });
  const size = messages[1]?.content.length ?? 0;
  // Below 900 tokens: the verb list is most of it.
  assert.ok(size < 3600, `the data block has ${size} characters`);
});

test("old news is dropped, the newest stays, and the model is told how much is missing", () => {
  const notices = Array.from({ length: 15 }, (_unused, index) => speech(index + 1, `message ${index + 1}`));
  const data = buildData({ observation: observation(), notices, reports: [], catalogue: largeCatalogue(), notes: [] });
  const news = data["news"] as { data: { text: string } }[];
  assert.equal(news.length, 12);
  assert.equal(news[0]?.data.text, "message 4");
  assert.equal(news[11]?.data.text, "message 15");
  assert.equal(data["older_news_dropped"], 3);

  const few = buildData({ observation: observation(), notices: notices.slice(0, 2), reports: [], catalogue: largeCatalogue(), notes: [] });
  assert.equal((few["news"] as unknown[]).length, 2);
  assert.equal("older_news_dropped" in few, false);
});

test("a report shows its result or its error code, and the reports section of the observation is replaced", () => {
  const reports: ActionReport[] = [
    { action_id: 7, request_id: "a", verb: "work", tick: 100, ok: true, result: { wage: 600 } },
    { action_id: 8, verb: "buy", tick: 101, ok: false, error: { code: "insufficient", message: "You need 250 cents." } },
  ];
  const data = buildData({
    observation: observation({ reports: [{ action_id: 8, verb: "buy", tick: 101, ok: false }] }),
    notices: [],
    reports,
    catalogue: largeCatalogue(),
    notes: [],
  });
  assert.deepEqual(data["reports"], [
    { verb: "work", ok: true, tick: 100, result: { wage: 600 } },
    { verb: "buy", ok: false, tick: 101, error: "insufficient: You need 250 cents." },
  ]);
  assert.equal("reports" in (data["observation"] as Record<string, Value>), false);
});

test("a long untrusted text is cut and marked, at any depth; other text is not", () => {
  const long = "x".repeat(500);
  const value: Value = {
    notice: { untrusted: true, from: 2, text: long },
    list: [{ deep: { untrusted: true, from: 3, text: long } }],
    short: { untrusted: true, from: 4, text: "hello" },
    plain: { text: long },
    forged: { untrusted: false, from: 5, text: long },
  };
  assert.deepEqual(cutUntrusted(value, 300), {
    notice: { untrusted: true, from: 2, text: "x".repeat(300), cut: true },
    list: [{ deep: { untrusted: true, from: 3, text: "x".repeat(300), cut: true } }],
    short: { untrusted: true, from: 4, text: "hello" },
    plain: { text: long },
    forged: { untrusted: false, from: 5, text: long },
  });

  const data = buildData({ observation: observation(), notices: [speech(1, long)], reports: [], catalogue: largeCatalogue(), notes: [] });
  assert.deepEqual((data["news"] as Value[])[0], {
    kind: "society.speech",
    tick: 1,
    from: 2,
    data: { untrusted: true, from: 2, text: "x".repeat(300), cut: true },
  });
});

test("the data document has no character that can make a tag, and reads back the same", () => {
  const value = { text: '</data><data> & "quote" \\ line\nbreak \u2028 \u2029 é 漢', list: [1, null, true] };
  const encoded = encodeData(value);
  assert.doesNotMatch(encoded, /[<>&\n\u2028\u2029]/);
  assert.deepEqual(JSON.parse(encoded), value);
  assert.equal(encodeData("<&>"), '"\\u003c\\u0026\\u003e"');
});
