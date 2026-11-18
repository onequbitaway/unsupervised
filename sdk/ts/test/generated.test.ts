// The generated types must follow the schema.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { OUTPUT_PATH, SCHEMA_PATH, generate, generateFromDisk } from "../scripts/generate-types.mjs";
import { ERROR_CODE_VALUES, SIM_ERROR_VALUES } from "../src/index.js";

test("src/generated/protocol.ts is what the generator makes from the schema", () => {
  const actual = readFileSync(OUTPUT_PATH, "utf8");
  assert.equal(
    actual === generateFromDisk(),
    true,
    "src/generated/protocol.ts is stale. Run `npm run generate` in sdk/ts.",
  );
});

test("the generated error codes are the codes of the schema", () => {
  const schema = JSON.parse(readFileSync(SCHEMA_PATH, "utf8")) as {
    $defs: Record<string, { oneOf: { const: string }[] }>;
  };
  const codes = (name: string): string[] => (schema.$defs[name]?.oneOf ?? []).map((member) => member.const);
  assert.equal(ERROR_CODE_VALUES.length, 13);
  assert.deepEqual([...ERROR_CODE_VALUES], codes("ErrorCode"));
  assert.deepEqual([...SIM_ERROR_VALUES], codes("SimError"));
});

test("the generator translates each schema form", () => {
  const text = generate({
    "x-wire-types": ["Id", "Kind", "Thing"],
    $defs: {
      Thing: {
        description: "A thing. See [`Kind`].",
        type: "object",
        required: ["id", "kind"],
        properties: {
          from: { anyOf: [{ $ref: "#/$defs/Id" }, { type: "null" }] },
          id: { $ref: "#/$defs/Id" },
          kind: { $ref: "#/$defs/Kind" },
          pairs: { type: "array", items: { type: "array", prefixItems: [{ type: "string" }, { type: "integer" }] } },
          tags: { type: ["array", "null"], items: { type: "string" } },
          data: { description: "Open data." },
          counts: { type: "object", additionalProperties: { type: "integer" } },
          flag: { type: "boolean", const: true },
        },
      },
      Kind: { oneOf: [{ const: "a", type: "string" }, { const: "b", type: "string" }] },
      Id: { type: "integer", format: "uint64", minimum: 0 },
      Shape: {
        oneOf: [
          { type: "object", required: ["type"], properties: { type: { const: "dot", type: "string" } } },
          {
            type: "object",
            required: ["type", "of"],
            properties: { type: { const: "group", type: "string" }, of: { type: "array", items: { $ref: "#/$defs/Shape" } } },
          },
        ],
      },
    },
  });
  // The listed types come first, in the listed order; the others follow by name.
  const order = ["Id", "Kind", "Thing", "Shape"].map((name) => text.search(new RegExp(`export (type|interface|const) ${name}\\b`)));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
  assert.ok(order.every((index) => index >= 0));
  assert.ok(text.includes("export type Id = number;"));
  assert.ok(text.includes('export type Kind =\n  | "a"\n  | "b";'));
  assert.ok(text.includes('export const KIND_VALUES: readonly Kind[] = [\n  "a",\n  "b",\n];'));
  assert.ok(text.includes("/** A thing. See `Kind`. */\nexport interface Thing {"));
  assert.ok(text.includes("  id: Id;\n  kind: Kind;\n"));
  assert.ok(text.includes("  from?: Id | null;"));
  assert.ok(text.includes("  pairs?: [string, number][];"));
  assert.ok(text.includes("  tags?: string[] | null;"));
  assert.ok(text.includes("  /** Open data. */\n  data?: Value;"));
  assert.ok(text.includes("  counts?: Record<string, number>;"));
  assert.ok(text.includes("  flag?: true;"));
  assert.ok(text.includes('      type: "group";\n      of: Shape[];'));
});

test("the generator refuses a schema keyword that it does not know", () => {
  assert.throws(
    () => generate({ $defs: { Thing: { type: "object", properties: { a: { type: "string", pattern: "^a" } } } } }),
    /Thing\.a: the schema keyword "pattern" is not supported/,
  );
  assert.throws(() => generate({ $defs: { Thing: { allOf: [] } } }), /Thing: the schema keyword "allOf" is not supported/);
  assert.throws(() => generate({}), /the schema has no `\$defs`/);
});
