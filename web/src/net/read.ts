// Small readers for the JSON of the server. Each one returns null for a
// value that does not have the expected shape.

import type { Clock, FeedEvent, Pos, Rect, ResidentView } from '../state/types';

export type Json = Record<string, unknown>;

export function isObject(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function int(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) ? value : null;
}

export function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

export function pos(value: unknown): Pos | null {
  if (!isObject(value)) return null;
  const x = int(value.x);
  const y = int(value.y);
  return x === null || y === null ? null : { x, y };
}

export function rect(value: unknown): Rect | null {
  if (!isObject(value)) return null;
  const x0 = int(value.x0);
  const y0 = int(value.y0);
  const x1 = int(value.x1);
  const y1 = int(value.y1);
  if (x0 === null || y0 === null || x1 === null || y1 === null) return null;
  return { x0, y0, x1, y1 };
}

export function clock(value: unknown): Clock | null {
  if (!isObject(value)) return null;
  const year = int(value.year);
  const day = int(value.day);
  const hour = int(value.hour);
  const minute = int(value.minute);
  if (year === null || day === null || hour === null || minute === null) return null;
  return { year, day, hour, minute };
}

/** Reads a list. One bad entry refuses the whole list. An absent list is empty. */
export function list<T>(value: unknown, read: (entry: unknown) => T | null): T[] | null {
  if (value === undefined) return [];
  if (!Array.isArray(value)) return null;
  const out: T[] = [];
  for (const entry of value) {
    const item = read(entry);
    if (item === null) return null;
    out.push(item);
  }
  return out;
}

export function resident(value: unknown): ResidentView | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const name = text(value.name);
  const at = pos(value.pos);
  if (id === null || name === null || at === null) return null;
  return { id, name, pos: at, activity: text(value.activity), model: text(value.model) };
}

export function pair(value: unknown): [number, number] | null {
  if (!Array.isArray(value)) return null;
  const a = int(value[0]);
  const b = int(value[1]);
  return a === null || b === null ? null : [a, b];
}

export function moved(value: unknown): [number, number, number] | null {
  if (!Array.isArray(value)) return null;
  const id = int(value[0]);
  const x = int(value[1]);
  const y = int(value[2]);
  return id === null || x === null || y === null ? null : [id, x, y];
}

export function activity(value: unknown): [number, string | null] | null {
  if (!Array.isArray(value)) return null;
  const id = int(value[0]);
  return id === null ? null : [id, text(value[1])];
}

export function readEvent(value: unknown): FeedEvent | null {
  if (!isObject(value)) return null;
  const id = text(value.id);
  const frame = int(value.frame);
  const at = clock(value.clock);
  const kind = text(value.kind);
  const drama = int(value.drama);
  if (id === null || frame === null || at === null || kind === null || drama === null) return null;
  const event: FeedEvent = { id, frame, clock: at, kind, drama, data: value.data ?? null };
  const zone = int(value.zone);
  if (zone !== null) event.zone = zone;
  const actor = int(value.actor);
  if (actor !== null) event.actor = actor;
  const actorName = text(value.actor_name);
  if (actorName !== null) event.actor_name = actorName;
  const where = pos(value.pos);
  if (where !== null) event.pos = where;
  return event;
}
