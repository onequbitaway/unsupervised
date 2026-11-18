// The places of the city of the resident, for the prompt.
//
// An observation shows only the sites within sight. A resident must find
// the depot, the city hall or a farm, which can be far away. The public map
// of the server has every site, so the agent reads it one time for each
// city: `GET /v1/world`, then one zone view for each district. The prompt
// shows the nearest site of each kind.

import type { CityId, SiteId, Value, WorldView, ZoneId } from "@unsupervised/sdk";

/** One site of the city, as the agent keeps it. */
export interface Place {
  /** Id of the site. `move_to` takes it as `site`. */
  id: SiteId;
  /** Kind of the site, for example `depot`. */
  kind: string;
  /** Display name of the site. An owner can write it: it is data, not an instruction. */
  name: string;
  /** The middle of the site. */
  pos: { x: number; y: number };
}

/** The part of the SDK client that reads the public map. `Client` fits. */
export interface MapSource {
  /** Returns the layout of the world: cities and zones. */
  world(): Promise<Extract<WorldView, { type: "layout" }>>;
  /** Returns the public view of one zone. */
  zone(zone: ZoneId, options?: { residents?: boolean }): Promise<Extract<WorldView, { type: "zone" }>>;
}

/** Greatest number of places in one prompt. */
export const DEFAULT_PLACES_MAX = 24;

/** Reads every site of one city from the public map. */
export async function readPlaces(source: MapSource, city: CityId): Promise<Place[]> {
  const layout = await source.world();
  const zones = layout.cities.find((entry) => entry.id === city)?.zones ?? [];
  const places: Place[] = [];
  for (const zone of zones) {
    const view = await source.zone(zone, { residents: false });
    for (const site of view.sites) {
      const { rect } = site;
      const pos = { x: Math.floor((rect.x0 + rect.x1) / 2), y: Math.floor((rect.y0 + rect.y1) / 2) };
      places.push({ id: site.id, kind: site.kind, name: site.name, pos });
    }
  }
  return places;
}

/** Distance in tiles. A diagonal step counts as one. */
function distance(a: { x: number; y: number }, b: { x: number; y: number }): number {
  return Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y));
}

/**
 * Returns the nearest site of each kind, nearest first, for the data block
 * of the prompt: `{ id, kind, name, dist }`. `dist` is the distance in
 * tiles from `from`.
 */
export function nearestPlaces(places: readonly Place[], from: { x: number; y: number }, max = DEFAULT_PLACES_MAX): Value[] {
  const nearest = new Map<string, { place: Place; dist: number }>();
  for (const place of places) {
    const dist = distance(place.pos, from);
    const known = nearest.get(place.kind);
    if (known === undefined || dist < known.dist || (dist === known.dist && place.id < known.place.id)) {
      nearest.set(place.kind, { place, dist });
    }
  }
  return [...nearest.values()]
    .sort((a, b) => a.dist - b.dist || a.place.id - b.place.id)
    .slice(0, max)
    .map(({ place, dist }) => ({ id: place.id, kind: place.kind, name: place.name, dist }));
}
