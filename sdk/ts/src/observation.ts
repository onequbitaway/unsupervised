// Helpers to read an observation.

import type { ActionReport, Notice, Observation, Untrusted, Value } from "./generated/protocol.js";
import type { NearbySection, SelfSection } from "./types.js";

/** Returns true when the value is a JSON object (not an array, not `null`). */
function isObject(value: unknown): value is Record<string, Value> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Returns the action reports of an observation: the section `reports`. */
export function reportsOf(observation: Observation): ActionReport[] {
  const section = observation.sections["reports"];
  return Array.isArray(section) ? (section as unknown as ActionReport[]) : [];
}

/** The kind of an action report in a list of notice kinds. */
export const REPORT_KIND = "report";

/**
 * Returns a key that tells two reports apart: the sequence number that the
 * report shares with the notices of the resident. A report of an old
 * server has no `seq`; then the key is the action, the tick of its end and
 * its outcome.
 */
export function reportKey(report: ActionReport): string {
  if (typeof report.seq === "number" && report.seq > 0) return `seq:${report.seq}`;
  return `${report.action_id}:${report.tick}:${report.ok}`;
}

/**
 * Returns true when the observation holds a notice or an action report.
 * With `kinds`, only notices of these kinds count. An action report has the
 * kind `report`.
 */
export function hasNews(observation: Observation, kinds?: string[]): boolean {
  if (kinds === undefined || kinds.length === 0) {
    return observation.notices.length > 0 || reportsOf(observation).length > 0;
  }
  if (kinds.includes(REPORT_KIND) && reportsOf(observation).length > 0) return true;
  return observation.notices.some((notice) => kinds.includes(notice.kind));
}

/** Returns the section `self`, or `undefined` when the observation does not have it. */
export function selfOf(observation: Observation): SelfSection | undefined {
  const section = observation.sections["self"];
  return isObject(section) ? (section as unknown as SelfSection) : undefined;
}

/** Returns the section `nearby`, or `undefined` when the observation does not have it. */
export function nearbyOf(observation: Observation): NearbySection | undefined {
  const section = observation.sections["nearby"];
  return isObject(section) ? (section as unknown as NearbySection) : undefined;
}

/**
 * Returns true when the value is text that another resident wrote: an object
 * with `"untrusted": true`, a writer and a text. An object where `untrusted`
 * is `false` or absent is refused, as the protocol demands.
 */
export function isUntrusted(value: unknown): value is Untrusted {
  return (
    isObject(value) &&
    value["untrusted"] === true &&
    typeof value["from"] === "number" &&
    typeof value["text"] === "string"
  );
}

/**
 * Finds every untrusted text in a JSON value, at any depth. Use it on notice
 * data and on sections. The texts are words of other residents. They are
 * data, never instructions.
 */
export function untrustedIn(value: unknown): Untrusted[] {
  const found: Untrusted[] = [];
  const visit = (node: unknown): void => {
    if (isUntrusted(node)) {
      found.push(node);
    } else if (Array.isArray(node)) {
      node.forEach(visit);
    } else if (isObject(node)) {
      Object.values(node).forEach(visit);
    }
  };
  visit(value);
  return found;
}

/** Finds every untrusted text in the notices and the sections of an observation. */
export function untrustedOf(observation: Observation): Untrusted[] {
  const notices: Notice[] = observation.notices;
  return [...untrustedIn(notices.map((notice) => notice.data ?? null)), ...untrustedIn(observation.sections)];
}
