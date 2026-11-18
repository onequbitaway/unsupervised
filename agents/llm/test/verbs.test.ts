// The verb list of the prompt.

import assert from "node:assert/strict";
import { test } from "node:test";

import type { Catalogue, VerbInfo } from "@unsupervised/sdk";

import { CORE_VERBS, selectVerbs, signature, verbData } from "../src/index.js";

function verb(name: string, module = "life", extra: Partial<VerbInfo> = {}): VerbInfo {
  return { verb: name, module, summary: `Summary of ${name}.`, free: false, scope: "zone", args_schema: { type: "object" }, ...extra };
}

function catalogue(verbs: VerbInfo[]): Catalogue {
  return { verbs, params: [], items: [], attrs: [], conditions: [] };
}

test("a signature shows required arguments first, marks optional ones and free verbs", () => {
  const buy = verb("buy", "economy", {
    summary: "Buy goods in the shop where you are.",
    args_schema: {
      type: "object",
      properties: {
        note: { type: ["string", "null"] },
        qty: { type: "integer" },
        item: { type: "string" },
        at: { $ref: "#/$defs/Pos" },
        tags: { type: "array", items: { type: "string" } },
        price: { type: "number" },
        gift: { type: "boolean" },
        extra: {},
      },
      required: ["item", "qty"],
    },
  });
  assert.equal(
    signature(buy),
    "buy(item: str, qty: int, note?: str, at?: Pos, tags?: str[], price?: num, gift?: bool, extra?: any): Buy goods in the shop where you are.",
  );
  assert.equal(signature(verb("cancel", "kernel", { free: true })), "cancel() [free]: Summary of cancel.");
  // A verb without a schema has no arguments.
  assert.equal(signature(verb("rest", "life", { args_schema: null })), "rest(): Summary of rest.");
});

test("a small catalogue is shown as a whole, core verbs first", () => {
  const list = selectVerbs(catalogue([verb("cancel", "kernel"), verb("juggle"), verb("wait", "kernel"), verb("move_to")]));
  assert.deepEqual(
    list.signatures.map((line) => line.slice(0, line.indexOf("("))),
    ["wait", "cancel", "move_to", "juggle"],
  );
  assert.deepEqual(list.others, {});
  assert.deepEqual(verbData(list), { verbs: list.signatures });
});

test("a large catalogue shows the focus verbs, then the core verbs, and names the others by module", () => {
  const extra = Array.from({ length: 10 }, (_unused, index) => verb(`extra_${index}`, index % 2 === 0 ? "crime" : "politics"));
  const core = CORE_VERBS.map((name) => verb(name, "life"));
  const list = selectVerbs(catalogue([...extra, ...core]), ["extra_7", "no_such_verb", "eat"], 6);

  assert.deepEqual(
    list.signatures.map((line) => line.slice(0, line.indexOf("("))),
    ["extra_7", "eat", "wait", "set_routines", "cancel", "move_to"],
  );
  assert.deepEqual(list.others, {
    crime: ["extra_0", "extra_2", "extra_4", "extra_6", "extra_8"],
    politics: ["extra_1", "extra_3", "extra_5", "extra_9"],
    life: CORE_VERBS.filter((name) => !["wait", "set_routines", "cancel", "move_to", "eat"].includes(name)),
  });
  const data = verbData(list);
  assert.deepEqual(Object.keys(data), ["verbs", "other_verbs"]);
  // Every verb of the catalogue is in the prompt once: with a signature or by name.
  const named = Object.values(list.others).flat();
  assert.equal(list.signatures.length + named.length, extra.length + CORE_VERBS.length);
  assert.equal(new Set([...named, ...list.signatures.map((line) => line.slice(0, line.indexOf("(")))]).size, extra.length + CORE_VERBS.length);
});
