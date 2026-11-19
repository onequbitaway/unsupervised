// The checks of a sponsor mission, for the route `POST /v1/missions` of the
// fake gateway.
//
// The real server does them in this order (`docs/design/protocol.md`,
// "Missions of sponsors"): the switch of the administrator, the form and
// the sizes, the money of one mission, the text filters, the limit of
// missions of one owner. Then the missions module checks the ranges of the
// numbers. This file has the form, the sizes, the money and the ranges.
// The fake does not know the event kinds of the game: it takes each kind.

import type { Value } from "../generated/protocol.js";
import { byteLength } from "./fake-world.js";

/** The limits of the server for sponsor missions: the `[sponsors]` table of its configuration. */
export interface SponsorLimits {
  /** False: the route answers `403 missions_disabled`. */
  enabled: boolean;
  /** Missions that one owner can publish in 24 hours. 0: no limit. */
  missionsPerDay: number;
  /** Greatest sum of new money for one mission, in cents: the reward times the takers. 0: no limit. */
  maxMissionCents: number;
}

/** The defaults of the server. */
export const DEFAULT_SPONSORS: SponsorLimits = { enabled: true, missionsPerDay: 3, maxMissionCents: 500_000 };

/** Why the route refuses a mission: the HTTP status, the error code and one sentence. */
export interface MissionRefusal {
  status: number;
  code: string;
  message: string;
}

/** The defaults of the parameters of the missions module that bound a definition. */
const SIZES = {
  title_max: 80,
  text_max: 1000,
  objectives_max: 4,
  match_max: 4,
  event_kind_max: 64,
  field_name_max: 32,
  field_text_max: 64,
  count_max: 1000,
  reward_min: 100,
  reward_max: 10_000_000,
  takers_max: 50,
  deadline_min: 10,
  deadline_max: 40_320,
};

const MISSION_FIELDS = ["title", "text", "objectives", "mode", "org", "reward", "deadline_in", "takers"];
const OBJECTIVE_FIELDS = ["event", "fields", "count", "actor"];

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Returns true for a whole number from 0 to the greatest value of 32 bits. */
function isCount(value: unknown): boolean {
  return Number.isInteger(value) && (value as number) >= 0 && (value as number) <= 0xffff_ffff;
}

function invalid(message: string): MissionRefusal {
  return { status: 400, code: "invalid_request", message };
}

function badArgs(message: string): MissionRefusal {
  return { status: 400, code: "invalid_args", message };
}

/**
 * Checks the form and the sizes of a definition and the money that it asks
 * for, as the route does before the mission reaches the world. Returns the
 * refusal, or `undefined` for a definition that passes.
 */
export function missionForm(body: unknown, limits: SponsorLimits): MissionRefusal | undefined {
  if (!isObject(body)) return invalid("The body must be an object: the definition of a mission.");
  const unknown = Object.keys(body).find((name) => !MISSION_FIELDS.includes(name));
  if (unknown !== undefined) return invalid(`The body is not a mission: unknown field \`${unknown}\`.`);
  const { title, text, objectives, mode, org, reward, deadline_in: deadline, takers } = body;
  const wrong = (field: string): MissionRefusal => invalid(`The body is not a mission: \`${field}\` is missing or has a wrong type.`);
  if (typeof title !== "string") return wrong("title");
  if (text !== undefined && typeof text !== "string") return wrong("text");
  if (!Array.isArray(objectives)) return wrong("objectives");
  if (mode !== undefined && mode !== "solo" && mode !== "team") return wrong("mode");
  if (org !== undefined && org !== null && !isCount(org)) return wrong("org");
  if (!Number.isInteger(reward)) return wrong("reward");
  if (!isCount(deadline)) return wrong("deadline_in");
  if (takers !== undefined && !isCount(takers)) return wrong("takers");
  for (const [index, objective] of objectives.entries()) {
    const place = `objectives[${index}]`;
    if (!isObject(objective) || Object.keys(objective).some((name) => !OBJECTIVE_FIELDS.includes(name))) return wrong(place);
    if (typeof objective["event"] !== "string") return wrong(`${place}.event`);
    if (objective["fields"] !== undefined && !isObject(objective["fields"])) return wrong(`${place}.fields`);
    if (objective["count"] !== undefined && !isCount(objective["count"])) return wrong(`${place}.count`);
    if (objective["actor"] !== undefined && objective["actor"] !== "assignee" && objective["actor"] !== "team") return wrong(`${place}.actor`);
  }

  if (title.trim() === "") return invalid("`title` must not be empty.");
  if (byteLength(title) > SIZES.title_max) return invalid(`\`title\` has at most ${SIZES.title_max} bytes.`);
  if (typeof text === "string" && byteLength(text) > SIZES.text_max) return invalid(`\`text\` has at most ${SIZES.text_max} bytes.`);
  if (objectives.length < 1 || objectives.length > SIZES.objectives_max) {
    return invalid(`\`objectives\` has from 1 to ${SIZES.objectives_max} objectives.`);
  }
  for (const [index, objective] of (objectives as Record<string, unknown>[]).entries()) {
    const place = `\`objectives[${index}]\``;
    const fields = (objective["fields"] ?? {}) as Record<string, unknown>;
    if (byteLength(objective["event"] as string) > SIZES.event_kind_max) return invalid(`${place}: \`event\` has at most ${SIZES.event_kind_max} bytes.`);
    if (Object.keys(fields).length > SIZES.match_max) return invalid(`${place}: \`fields\` has at most ${SIZES.match_max} data fields.`);
    if (Object.keys(fields).some((name) => byteLength(name) > SIZES.field_name_max)) {
      return invalid(`${place}: the name of a data field has at most ${SIZES.field_name_max} bytes.`);
    }
    if (Object.values(fields).some((value) => typeof value === "string" && byteLength(value) > SIZES.field_text_max)) {
      return invalid(`${place}: a text value of a data field has at most ${SIZES.field_text_max} bytes.`);
    }
  }

  // A reward below zero is no money. The module refuses it.
  const total = Math.max(0, reward as number) * ((takers as number | undefined) ?? 1);
  if (limits.maxMissionCents > 0 && total > limits.maxMissionCents) {
    return invalid(
      `\`reward\` times \`takers\` is more than ${limits.maxMissionCents} cents. That is the greatest sum for one sponsor mission on this server.`,
    );
  }
  return undefined;
}

/**
 * Checks what the missions module checks when the mission reaches the
 * world: the ranges of the numbers, and the fields that only a team
 * mission can have. Returns the refusal, or `undefined`.
 */
export function missionRanges(definition: Record<string, Value>): MissionRefusal | undefined {
  const range = (name: string, value: number, min: number, max: number, unit: string): MissionRefusal | undefined =>
    value < min || value > max ? badArgs(`\`${name}\` must be from ${min} to ${max}${unit}.`) : undefined;
  const team = definition["mode"] === "team";
  if (definition["org"] != null && !team) return badArgs("Only a team mission can name an organization.");
  for (const objective of definition["objectives"] as Record<string, Value>[]) {
    const count = range("count", (objective["count"] as number | undefined) ?? 1, 1, SIZES.count_max, "");
    if (count !== undefined) return count;
    if (objective["actor"] === "team" && !team) return badArgs("Only a team mission can count the events of team members.");
  }
  return (
    range("reward", definition["reward"] as number, SIZES.reward_min, SIZES.reward_max, " cents") ??
    range("takers", (definition["takers"] as number | undefined) ?? 1, 1, SIZES.takers_max, "") ??
    range("deadline_in", definition["deadline_in"] as number, SIZES.deadline_min, SIZES.deadline_max, " game minutes")
  );
}
