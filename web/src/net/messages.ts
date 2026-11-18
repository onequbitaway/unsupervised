// Reads the JSON of the server into typed values. Anything that does not
// have the documented shape is refused, so the rest of the viewer can trust
// the types. The text inside (names, speech) stays untrusted.

import type {
  CityView,
  Clock,
  FeedEvent,
  FeedPage,
  Layout,
  Pos,
  Profile,
  Rect,
  ResidentView,
  ServerMessage,
  SiteView,
  ZoneMap,
  ZoneView,
} from '../state/types';

type Json = Record<string, unknown>;

function isObject(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function int(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) ? value : null;
}

function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function pos(value: unknown): Pos | null {
  if (!isObject(value)) return null;
  const x = int(value.x);
  const y = int(value.y);
  return x === null || y === null ? null : { x, y };
}

function rect(value: unknown): Rect | null {
  if (!isObject(value)) return null;
  const x0 = int(value.x0);
  const y0 = int(value.y0);
  const x1 = int(value.x1);
  const y1 = int(value.y1);
  if (x0 === null || y0 === null || x1 === null || y1 === null) return null;
  return { x0, y0, x1, y1 };
}

function clock(value: unknown): Clock | null {
  if (!isObject(value)) return null;
  const year = int(value.year);
  const day = int(value.day);
  const hour = int(value.hour);
  const minute = int(value.minute);
  if (year === null || day === null || hour === null || minute === null) return null;
  return { year, day, hour, minute };
}

/** Reads a list. One bad entry refuses the whole list. An absent list is empty. */
function list<T>(value: unknown, read: (entry: unknown) => T | null): T[] | null {
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

function resident(value: unknown): ResidentView | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const name = text(value.name);
  const at = pos(value.pos);
  if (id === null || name === null || at === null) return null;
  return { id, name, pos: at, activity: text(value.activity), model: text(value.model) };
}

function pair(value: unknown): [number, number] | null {
  if (!Array.isArray(value)) return null;
  const a = int(value[0]);
  const b = int(value[1]);
  return a === null || b === null ? null : [a, b];
}

function moved(value: unknown): [number, number, number] | null {
  if (!Array.isArray(value)) return null;
  const id = int(value[0]);
  const x = int(value[1]);
  const y = int(value[2]);
  return id === null || x === null || y === null ? null : [id, x, y];
}

function activity(value: unknown): [number, string | null] | null {
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

/** Reads one message of `/v1/spectate`. Returns null for anything else. */
export function parseServerMessage(raw: string): ServerMessage | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isObject(value)) return null;
  switch (value.op) {
    case 'world': {
      const frame = int(value.frame);
      const at = clock(value.clock);
      const populations = list(value.populations, pair);
      if (frame === null || at === null || populations === null) return null;
      return {
        op: 'world',
        protocol: int(value.protocol) ?? 0,
        frame,
        clock: at,
        populations,
        dropped: int(value.dropped) ?? 0,
        viewers: int(value.viewers) ?? 0,
      };
    }
    case 'snapshot': {
      const zone = int(value.zone);
      const frame = int(value.frame);
      const residents = list(value.residents, resident);
      if (zone === null || frame === null || residents === null) return null;
      return { op: 'snapshot', zone, frame, residents };
    }
    case 'delta': {
      const frame = int(value.frame);
      const zone = int(value.zone);
      const lists = {
        moved: list(value.moved, moved),
        entered: list(value.entered, resident),
        left: list(value.left, int),
        activity: list(value.activity, activity),
        events: list(value.events, readEvent),
      };
      if (frame === null || zone === null) return null;
      if (!lists.moved || !lists.entered || !lists.left || !lists.activity || !lists.events) {
        return null;
      }
      return {
        op: 'delta',
        frame,
        zone,
        tick: int(value.tick) ?? 0,
        moved: lists.moved,
        entered: lists.entered,
        left: lists.left,
        activity: lists.activity,
        events: lists.events,
      };
    }
    case 'frame': {
      const frame = int(value.frame);
      const at = clock(value.clock);
      const populations = list(value.populations, pair);
      const headlines = list(value.headlines, readEvent);
      if (frame === null || at === null || populations === null || headlines === null) return null;
      return { op: 'frame', frame, clock: at, populations, headlines, viewers: int(value.viewers) ?? 0 };
    }
    case 'unsubscribed': {
      const zones = list(value.zones, int);
      return zones === null ? null : { op: 'unsubscribed', zones };
    }
    case 'error': {
      const error = isObject(value.error) ? value.error : {};
      return { op: 'error', code: text(error.code) ?? 'unknown', message: text(error.message) ?? '' };
    }
    case 'pong':
      return { op: 'pong' };
    default:
      return null;
  }
}

function zoneView(value: unknown): ZoneView | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const city = int(value.city);
  const name = text(value.name);
  const bounds = rect(value.bounds);
  if (id === null || city === null || name === null || bounds === null) return null;
  return { id, city, name, bounds, residents: int(value.residents) ?? 0 };
}

function cityView(value: unknown): CityView | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const name = text(value.name);
  const bounds = rect(value.bounds);
  const zones = list(value.zones, int);
  if (id === null || name === null || bounds === null || zones === null) return null;
  return { id, name, bounds, zones };
}

function siteView(value: unknown): SiteView | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const kind = text(value.kind);
  const name = text(value.name);
  const area = rect(value.rect);
  if (id === null || kind === null || name === null || area === null) return null;
  return { id, kind, name, rect: area, tags: list(value.tags, text) ?? [] };
}

/** Reads the answer of `/v1/world`. */
export function readLayout(value: unknown): Layout | null {
  if (!isObject(value) || value.type !== 'layout') return null;
  const frame = int(value.frame);
  const at = clock(value.clock);
  const cities = list(value.cities, cityView);
  const zones = list(value.zones, zoneView);
  if (frame === null || at === null || cities === null || zones === null) return null;
  return { frame, clock: at, cities, zones };
}

/** Reads the answer of `/v1/world/zones/{zone}`. The residents are ignored. */
export function readZoneMap(value: unknown): ZoneMap | null {
  if (!isObject(value) || value.type !== 'zone') return null;
  const zone = zoneView(value.zone);
  const width = int(value.width);
  const height = int(value.height);
  const sites = list(value.sites, siteView);
  if (zone === null || width === null || height === null || sites === null) return null;
  if (width < 0 || height < 0 || width * height > 1_048_576) return null;
  if (!Array.isArray(value.tiles) || value.tiles.length !== width * height) return null;
  const tiles = new Uint8Array(width * height);
  for (let i = 0; i < tiles.length; i++) {
    const code = value.tiles[i];
    tiles[i] = typeof code === 'number' && code >= 0 && code <= 255 ? code : 0;
  }
  return { zone, width, height, tiles, sites };
}

/** Reads the answer of `/v1/feed`. */
export function readFeedPage(value: unknown): FeedPage | null {
  if (!isObject(value)) return null;
  const events = list(value.events, readEvent);
  const next = text(value.next);
  if (events === null || next === null) return null;
  return { events, next, more: value.more === true, gap: value.gap === true };
}

/** Reads the answer of `/v1/residents/{id}/profile`. */
export function readProfile(value: unknown): Profile | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const name = text(value.name);
  const zone = int(value.zone);
  const city = int(value.city);
  const at = pos(value.pos);
  const frame = int(value.frame);
  const events = list(value.events, readEvent);
  if (id === null || name === null || zone === null || city === null) return null;
  if (at === null || frame === null || events === null) return null;
  const profile: Profile = {
    id,
    name,
    alive: value.alive === true,
    zone,
    zone_name: text(value.zone_name) ?? '',
    city,
    city_name: text(value.city_name) ?? '',
    pos: at,
    frame,
    events,
  };
  const model = text(value.model);
  if (model !== null) profile.model = model;
  const doing = text(value.activity);
  if (doing !== null) profile.activity = doing;
  const cause = text(value.cause);
  if (cause !== null) profile.cause = cause;
  return profile;
}
