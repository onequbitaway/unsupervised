// Which verbs the prompt shows, and how.
//
// The catalogue can hold more than a hundred verbs. The prompt shows the
// signature of a few verbs that matter now, and only the names of the others.
// The model asks for more signatures with the `describe` field of its answer.

import type { Catalogue, Value, VerbInfo } from "@unsupervised/sdk";

/** Verbs that every resident needs, in the order in which they get a place in the prompt. */
export const CORE_VERBS: readonly string[] = [
  "wait",
  "set_routines",
  "cancel",
  "move_to",
  "eat",
  "sleep",
  "work",
  "buy",
  "list_jobs",
  "apply_job",
  "say",
  "message",
  "inspect",
  "gather",
  "rest",
  "sell",
  "travel",
];

/** Greatest number of verbs with a signature. */
export const DEFAULT_VERBS_MAX = 20;

/** The verbs of one prompt. */
export interface VerbList {
  /** One line for each verb that matters: name, arguments and summary. */
  signatures: string[];
  /** The names of the other verbs, by module. */
  others: Record<string, string[]>;
}

const TYPE_NAMES: Record<string, string> = {
  integer: "int",
  number: "num",
  string: "str",
  boolean: "bool",
  array: "list",
  object: "obj",
};

interface Schema {
  type?: string | string[];
  $ref?: string;
  properties?: Record<string, Schema>;
  required?: string[];
  items?: Schema;
}

/** Returns a short name for the type of an argument. */
function typeName(schema: Schema | undefined): string {
  if (schema === undefined) return "any";
  if (typeof schema.$ref === "string") return schema.$ref.slice(schema.$ref.lastIndexOf("/") + 1);
  const types = (Array.isArray(schema.type) ? schema.type : [schema.type]).filter(
    (type): type is string => typeof type === "string" && type !== "null",
  );
  const type = types[0];
  if (type === undefined) return "any";
  if (type === "array" && schema.items !== undefined) return `${typeName(schema.items)}[]`;
  return TYPE_NAMES[type] ?? type;
}

/** Returns one line for a verb: `name(arg: type, optional?: type) [free]: summary`. */
export function signature(verb: VerbInfo): string {
  const schema = (verb.args_schema ?? {}) as Schema;
  const required = schema.required ?? [];
  const names = Object.keys(schema.properties ?? {});
  // Required arguments first, in the order of the schema.
  const ordered = [...required.filter((name) => names.includes(name)), ...names.filter((name) => !required.includes(name))];
  const args = ordered.map((name) => `${name}${required.includes(name) ? "" : "?"}: ${typeName(schema.properties?.[name])}`);
  return `${verb.verb}(${args.join(", ")})${verb.free ? " [free]" : ""}: ${verb.summary}`;
}

/**
 * Chooses the verbs of a prompt. `focus` names verbs that matter now: the
 * ones that the model asked about and the ones in recent reports. They come
 * first, then the core verbs, up to `max`. The other verbs are listed by
 * name only.
 */
export function selectVerbs(catalogue: Catalogue, focus: readonly string[] = [], max = DEFAULT_VERBS_MAX): VerbList {
  const byName = new Map(catalogue.verbs.map((verb) => [verb.verb, verb]));
  const chosen: VerbInfo[] = [];
  for (const name of [...focus, ...CORE_VERBS]) {
    const verb = byName.get(name);
    if (verb !== undefined && !chosen.includes(verb) && chosen.length < max) chosen.push(verb);
  }
  // A small catalogue fits as a whole.
  for (const verb of catalogue.verbs) {
    if (!chosen.includes(verb) && chosen.length < max) chosen.push(verb);
  }
  const others: Record<string, string[]> = {};
  for (const verb of catalogue.verbs) {
    if (!chosen.includes(verb)) (others[verb.module] ??= []).push(verb.verb);
  }
  return { signatures: chosen.map(signature), others };
}

/** Returns the JSON form of the verb list, for the data block of the prompt. */
export function verbData(list: VerbList): Record<string, Value> {
  const data: Record<string, Value> = { verbs: list.signatures };
  if (Object.keys(list.others).length > 0) data["other_verbs"] = list.others;
  return data;
}
