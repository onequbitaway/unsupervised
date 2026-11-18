#!/usr/bin/env node
// Writes src/generated/protocol.ts from schema/protocol.schema.json.
//
//   node scripts/generate-types.mjs           write the file
//   node scripts/generate-types.mjs --check   fail when the file is stale
//
// The schema comes from the Rust wire types (`make schema`). This script has
// no dependencies. It stops with an error when the schema uses a keyword that
// it does not know, so a change in the schema format cannot pass unseen.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Returns the directory of the package: the nearest directory above this
 * file that has a `package.json`. The tests run a compiled copy of this
 * file from another directory, so a fixed relative path is not enough.
 */
function packageRoot() {
  let directory = dirname(fileURLToPath(import.meta.url));
  while (!existsSync(join(directory, "package.json"))) {
    const parent = dirname(directory);
    if (parent === directory) throw new Error("no package.json above the generator script");
    directory = parent;
  }
  return directory;
}

const PACKAGE_ROOT = packageRoot();

/** Path of the schema that the Rust server exports. */
export const SCHEMA_PATH = resolve(PACKAGE_ROOT, "../../schema/protocol.schema.json");

/** Path of the generated TypeScript file. */
export const OUTPUT_PATH = resolve(PACKAGE_ROOT, "src/generated/protocol.ts");

const KNOWN_KEYWORDS = new Set([
  "$ref",
  "additionalProperties",
  "anyOf",
  "const",
  "default",
  "description",
  "enum",
  "format",
  "items",
  "maxItems",
  "maximum",
  "minItems",
  "minimum",
  "oneOf",
  "prefixItems",
  "properties",
  "required",
  "title",
  "type",
]);

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

/**
 * @typedef {{ [key: string]: any }} Schema
 */

/**
 * Stops when a schema node has a keyword that this script cannot translate.
 * @param {Schema} node
 * @param {string} where
 */
function checkKeywords(node, where) {
  for (const key of Object.keys(node)) {
    if (!KNOWN_KEYWORDS.has(key)) {
      throw new Error(`${where}: the schema keyword "${key}" is not supported`);
    }
  }
}

/**
 * Turns Rust documentation links such as [`Name`] into `Name`.
 * @param {string} text
 */
function cleanDoc(text) {
  return text.replace(/\[(`[^`]+`)\]/g, "$1").replace(/\*\//g, "* /");
}

/**
 * Returns a JSDoc block, or an empty string when there is nothing to say.
 * @param {Schema} node
 * @param {string} indent
 */
function doc(node, indent) {
  /** @type {string[]} */
  const lines = [];
  if (typeof node.description === "string") {
    lines.push(...cleanDoc(node.description).split("\n"));
  }
  if (node.default !== undefined) {
    if (lines.length > 0) lines.push("");
    lines.push(`@default ${JSON.stringify(node.default)}`);
  }
  if (lines.length === 0) return "";
  if (lines.length === 1) return `${indent}/** ${lines[0]} */\n`;
  const body = lines.map((line) => `${indent} *${line === "" ? "" : ` ${line}`}`).join("\n");
  return `${indent}/**\n${body}\n${indent} */\n`;
}

/**
 * Returns true when the node is one fixed string.
 * @param {Schema} node
 */
function isStringConst(node) {
  return typeof node.const === "string";
}

/**
 * Returns true when a type expression is a union at its top level, so that
 * it needs parentheses before `[]`.
 * @param {string} type
 */
function isUnion(type) {
  let depth = 0;
  for (const char of type) {
    if ("{[(<".includes(char)) depth += 1;
    else if ("}])>".includes(char)) depth -= 1;
    else if (char === "|" && depth === 0) return true;
  }
  return false;
}

/**
 * Joins the members of a union and removes repeats.
 * @param {string[]} members
 */
function union(members) {
  return [...new Set(members)].join(" | ");
}

/**
 * Returns the inline object type of a node with `properties`.
 * @param {Schema} node
 * @param {string} where
 * @param {string} indent
 */
function objectType(node, where, indent) {
  const inner = `${indent}  `;
  /** @type {Schema} */
  const properties = node.properties ?? {};
  /** @type {string[]} */
  const required = node.required ?? [];
  for (const name of required) {
    if (!(name in properties)) {
      throw new Error(`${where}: the required property "${name}" has no schema`);
    }
  }
  // Declaration order for required properties, then the optional ones.
  const names = [...required, ...Object.keys(properties).filter((name) => !required.includes(name))];
  let out = "{\n";
  for (const name of names) {
    const property = properties[name];
    const key = IDENTIFIER.test(name) ? name : JSON.stringify(name);
    const optional = required.includes(name) ? "" : "?";
    out += doc(property, inner);
    out += `${inner}${key}${optional}: ${typeOf(property, `${where}.${name}`, inner)};\n`;
  }
  if (node.additionalProperties !== undefined && node.additionalProperties !== false) {
    const value =
      node.additionalProperties === true
        ? "Value"
        : typeOf(node.additionalProperties, `${where}.*`, inner);
    out += `${inner}[key: string]: ${value};\n`;
  }
  return `${out}${indent}}`;
}

/**
 * Returns the TypeScript type expression of a schema node.
 * @param {Schema | boolean} node
 * @param {string} where
 * @param {string} indent
 * @returns {string}
 */
function typeOf(node, where, indent) {
  if (node === true) return "Value";
  if (typeof node !== "object" || node === null) {
    throw new Error(`${where}: the schema \`${String(node)}\` is not supported`);
  }
  checkKeywords(node, where);
  if (typeof node.$ref === "string") {
    const prefix = "#/$defs/";
    if (!node.$ref.startsWith(prefix)) {
      throw new Error(`${where}: the reference "${node.$ref}" is not supported`);
    }
    return node.$ref.slice(prefix.length);
  }
  if (node.const !== undefined) return JSON.stringify(node.const);
  if (Array.isArray(node.enum)) return union(node.enum.map((value) => JSON.stringify(value)));
  const alternatives = node.oneOf ?? node.anyOf;
  if (Array.isArray(alternatives)) {
    return union(alternatives.map((member, i) => typeOf(member, `${where}[${i}]`, indent)));
  }
  if (Array.isArray(node.type)) {
    return union(node.type.map((type) => typeOf({ ...node, type }, where, indent)));
  }
  switch (node.type) {
    case undefined:
      return "Value";
    case "null":
      return "null";
    case "boolean":
      return "boolean";
    case "string":
      return "string";
    case "integer":
    case "number":
      return "number";
    case "array": {
      if (Array.isArray(node.prefixItems)) {
        const items = node.prefixItems.map((item, i) => typeOf(item, `${where}[${i}]`, indent));
        return `[${items.join(", ")}]`;
      }
      const item = node.items === undefined ? "Value" : typeOf(node.items, `${where}[]`, indent);
      return isUnion(item) ? `(${item})[]` : `${item}[]`;
    }
    case "object": {
      if (node.properties !== undefined) return objectType(node, where, indent);
      if (node.additionalProperties === undefined || node.additionalProperties === true) {
        return "Record<string, Value>";
      }
      if (node.additionalProperties === false) return "Record<string, never>";
      return `Record<string, ${typeOf(node.additionalProperties, `${where}.*`, indent)}>`;
    }
    default:
      throw new Error(`${where}: the type "${node.type}" is not supported`);
  }
}

/**
 * Returns a copy of a schema node without the named keywords.
 * @param {Schema} node
 * @param {string[]} keys
 * @returns {Schema}
 */
function without(node, keys) {
  return Object.fromEntries(Object.entries(node).filter(([key]) => !keys.includes(key)));
}

/**
 * Returns the name of the constant that lists the values of a string union.
 * @param {string} name
 */
function valuesName(name) {
  return `${name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toUpperCase()}_VALUES`;
}

/**
 * Returns the declaration of one named wire type.
 * @param {string} name
 * @param {Schema} node
 */
function declaration(name, node) {
  if (!IDENTIFIER.test(name)) throw new Error(`the type name "${name}" is not an identifier`);
  checkKeywords(node, name);
  const alternatives = node.oneOf ?? node.anyOf;
  if (Array.isArray(alternatives) && alternatives.every(isStringConst)) {
    // An enum without data: a union of strings and a list of its values.
    let out = doc(node, "");
    out += `export type ${name} =\n`;
    for (const member of alternatives) {
      checkKeywords(member, name);
      out += doc(member, "  ");
      out += `  | ${JSON.stringify(member.const)}\n`;
    }
    out = `${out.trimEnd()};\n\n`;
    out += `/** Every value of \`${name}\`, in the order of the protocol document. */\n`;
    out += `export const ${valuesName(name)}: readonly ${name}[] = [\n`;
    for (const member of alternatives) {
      out += `  ${JSON.stringify(member.const)},\n`;
    }
    out += "];\n";
    return out;
  }
  if (Array.isArray(alternatives)) {
    // An enum with data: a union with one member for each variant.
    let out = doc(node, "");
    out += `export type ${name} =\n`;
    alternatives.forEach((member, i) => {
      out += doc(member, "  ");
      out += `  | ${typeOf(without(member, ["description", "default"]), `${name}[${i}]`, "    ")}\n`;
    });
    return `${out.trimEnd()};\n`;
  }
  if (node.type === "object" && node.properties !== undefined) {
    return `${doc(node, "")}export interface ${name} ${objectType(node, name, "")}\n`;
  }
  return `${doc(node, "")}export type ${name} = ${typeOf(without(node, ["description"]), name, "")};\n`;
}

/**
 * Returns the text of the generated file.
 * @param {Schema} schema The content of `protocol.schema.json`.
 * @returns {string}
 */
export function generate(schema) {
  const defs = schema.$defs;
  if (typeof defs !== "object" || defs === null) {
    throw new Error("the schema has no `$defs`");
  }
  /** @type {string[]} */
  const listed = Array.isArray(schema["x-wire-types"]) ? schema["x-wire-types"] : [];
  const order = [
    ...listed.filter((name) => name in defs),
    ...Object.keys(defs)
      .filter((name) => !listed.includes(name))
      .sort(),
  ];
  const parts = [
    [
      "// Generated by scripts/generate-types.mjs from schema/protocol.schema.json.",
      "// Do not edit. Run `npm run generate` after the schema changes.",
      "",
      "/** A JSON value. Open payloads use it: arguments, results, notice data, sections. */",
      "export type Value = null | boolean | number | string | Value[] | { [key: string]: Value };",
      "",
    ].join("\n"),
  ];
  for (const name of order) {
    parts.push(declaration(name, defs[name]));
  }
  return parts.join("\n");
}

/** Reads the schema file and returns the text that the output file must have. */
export function generateFromDisk() {
  return generate(JSON.parse(readFileSync(SCHEMA_PATH, "utf8")));
}

function main() {
  const expected = generateFromDisk();
  if (process.argv.includes("--check")) {
    let actual = "";
    try {
      actual = readFileSync(OUTPUT_PATH, "utf8");
    } catch {
      // A missing file is stale.
    }
    if (actual !== expected) {
      console.error(`${OUTPUT_PATH} is stale. Run \`npm run generate\`.`);
      process.exit(1);
    }
    console.log(`${OUTPUT_PATH} is current.`);
    return;
  }
  writeFileSync(OUTPUT_PATH, expected);
  console.log(`wrote ${OUTPUT_PATH}`);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
