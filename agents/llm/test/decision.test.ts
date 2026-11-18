// Reading the answer of the model.

import assert from "node:assert/strict";
import { test } from "node:test";

import { DecisionError, parseDecision, type DecisionLimits } from "../src/index.js";

const LIMITS: DecisionLimits = { minWaitSeconds: 60, maxWaitSeconds: 3600, defaultWaitSeconds: 300, maxActions: 3 };

test("a full answer is read as it is", () => {
  const decision = parseDecision(
    JSON.stringify({
      thought: "I am hungry.",
      actions: [{ verb: "move_to", args: { x: 4, y: 5 } }, { verb: "eat", args: { item: "bread" } }],
      wait_seconds: 240,
      note: "The shop is at 4,5.",
      describe: ["buy"],
    }),
    LIMITS,
  );
  assert.deepEqual(decision, {
    thought: "I am hungry.",
    actions: [
      { verb: "move_to", args: { x: 4, y: 5 } },
      { verb: "eat", args: { item: "bread" } },
    ],
    waitSeconds: 240,
    note: "The shop is at 4,5.",
    describe: ["buy"],
  });
});

test("text around the JSON object is dropped", () => {
  const fenced = 'Here is my answer:\n```json\n{"thought":"rest","actions":[],"wait_seconds":600}\n```\nGood luck!';
  assert.deepEqual(parseDecision(fenced, LIMITS), { thought: "rest", actions: [], waitSeconds: 600, note: undefined, describe: [] });
});

test("an answer without a JSON object is an error with a message for the next prompt", () => {
  assert.throws(() => parseDecision("I will go to the shop.", LIMITS), (error: unknown) => {
    assert.ok(error instanceof DecisionError);
    assert.equal(error.message, "Your last answer had no JSON object.");
    return true;
  });
  assert.throws(() => parseDecision('{"actions": [}', LIMITS), (error: unknown) => {
    assert.ok(error instanceof DecisionError);
    assert.equal(error.message, "Your last answer was not valid JSON.");
    return true;
  });
  assert.throws(() => parseDecision("}{", LIMITS), DecisionError);
});

test("the wait is brought into its limits, and the default is used when it is absent", () => {
  const wait = (value: unknown): number => parseDecision(JSON.stringify({ wait_seconds: value }), LIMITS).waitSeconds;
  assert.equal(wait(5), 60);
  assert.equal(wait(-100), 60);
  assert.equal(wait(100_000), 3600);
  assert.equal(wait(90.6), 91);
  assert.equal(wait("600"), 300);
  assert.equal(wait(null), 300);
  assert.equal(parseDecision("{}", LIMITS).waitSeconds, 300);
});

test("actions of a wrong form are dropped, and the rest is cut at the greatest number", () => {
  const decision = parseDecision(
    JSON.stringify({
      actions: [
        "eat",
        { args: { item: "bread" } },
        { verb: "" },
        { verb: 7 },
        { verb: "wait", args: [5] },
        { verb: "eat", args: { item: "bread" } },
        { verb: "sleep" },
        { verb: "work" },
      ],
    }),
    LIMITS,
  );
  // Arguments that are not an object are left out; the server then says what is missing.
  assert.deepEqual(decision.actions, [{ verb: "wait" }, { verb: "eat", args: { item: "bread" } }, { verb: "sleep" }]);
  assert.deepEqual(parseDecision('{"actions":"eat"}', LIMITS).actions, []);
});

test("a thought and a note are one line of bounded length", () => {
  const decision = parseDecision(JSON.stringify({ thought: "  a\n\nb\t c  ", note: `${"n".repeat(300)}` }), LIMITS);
  assert.equal(decision.thought, "a b c");
  assert.equal(decision.note, "n".repeat(200));
  const empty = parseDecision(JSON.stringify({ thought: 5, note: "   " }), LIMITS);
  assert.equal(empty.thought, "");
  assert.equal(empty.note, undefined);
});

test("`describe` keeps names only, at most eight", () => {
  const names = Array.from({ length: 12 }, (_unused, index) => `verb_${index}`);
  const decision = parseDecision(JSON.stringify({ describe: [3, "", null, ...names] }), LIMITS);
  assert.deepEqual(decision.describe, names.slice(0, 8));
  assert.deepEqual(parseDecision('{"describe":"buy"}', LIMITS).describe, []);
});
