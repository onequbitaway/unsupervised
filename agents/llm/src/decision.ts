// The answer of the model: one JSON object with actions and a wait.

import type { PlanStep, Value } from "@unsupervised/sdk";

/** What the model decided for one turn. */
export interface Decision {
  /** One short sentence of the model about its reason. For the log only. */
  thought: string;
  /** Actions to plan, in order. Can be empty. */
  actions: PlanStep[];
  /** Seconds to wait before the next turn. */
  waitSeconds: number;
  /** One fact that the model wants to remember, when it gave one. */
  note: string | undefined;
  /** Verbs whose signature the model wants to see in the next turn. */
  describe: string[];
}

/** Limits that `parseDecision` applies to the answer of the model. */
export interface DecisionLimits {
  /** Least wait in seconds. */
  minWaitSeconds: number;
  /** Greatest wait in seconds. */
  maxWaitSeconds: number;
  /** The wait when the model gives none. */
  defaultWaitSeconds: number;
  /** Greatest number of actions in one turn. */
  maxActions: number;
}

/** The answer of the model is not the JSON object that the prompt asks for. */
export class DecisionError extends Error {
  /** Makes an error with a message for the next prompt. */
  constructor(message: string) {
    super(message);
    this.name = "DecisionError";
  }
}

/** Greatest length of a note and of a thought, in characters. */
const NOTE_MAX = 200;
/** Greatest number of verbs in `describe`. */
const DESCRIBE_MAX = 8;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Returns a string field as one line of bounded length, or `undefined`. */
function line(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const text = value.replace(/\s+/g, " ").trim().slice(0, NOTE_MAX);
  return text === "" ? undefined : text;
}

/**
 * Reads the answer of the model. Text around the JSON object (a code fence,
 * a sentence) is dropped. Fields of a wrong form are dropped or replaced by
 * a default; the wait is brought into its limits.
 *
 * @throws DecisionError when the answer holds no JSON object.
 */
export function parseDecision(text: string, limits: DecisionLimits): Decision {
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end <= start) throw new DecisionError("Your last answer had no JSON object.");
  let parsed: unknown;
  try {
    parsed = JSON.parse(text.slice(start, end + 1));
  } catch {
    throw new DecisionError("Your last answer was not valid JSON.");
  }
  if (!isObject(parsed)) throw new DecisionError("Your last answer was not a JSON object.");

  const actions: PlanStep[] = [];
  for (const action of Array.isArray(parsed["actions"]) ? parsed["actions"] : []) {
    if (!isObject(action) || typeof action["verb"] !== "string" || action["verb"] === "") continue;
    const step: PlanStep = { verb: action["verb"] };
    if (isObject(action["args"])) step.args = action["args"] as Value;
    actions.push(step);
    if (actions.length >= limits.maxActions) break;
  }

  const asked = parsed["wait_seconds"];
  const wait = typeof asked === "number" && Number.isFinite(asked) ? asked : limits.defaultWaitSeconds;
  const describe = (Array.isArray(parsed["describe"]) ? parsed["describe"] : [])
    .filter((name): name is string => typeof name === "string" && name !== "")
    .slice(0, DESCRIBE_MAX);

  return {
    thought: line(parsed["thought"]) ?? "",
    actions,
    waitSeconds: Math.round(Math.min(limits.maxWaitSeconds, Math.max(limits.minWaitSeconds, wait))),
    note: line(parsed["note"]),
    describe,
  };
}
