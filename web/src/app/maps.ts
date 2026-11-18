// Loads the maps (tiles and sites) of the districts on the screen, and
// frees the maps that the viewer does not need any more.

import { CONFIG } from '../config';
import type { ZoneMap } from '../state/types';

/** Where painted maps go. The scene is one. */
export interface MapSink {
  hasMap(zone: number): boolean;
  addMap(map: ZoneMap): void;
  keepMaps(zones: ReadonlySet<number>): void;
}

/** Returns a number that changes when the tiles or the sites of a map change. */
export function mapSignature(map: ZoneMap): number {
  let hash = 0x811c9dc5;
  const mix = (byte: number): void => {
    hash = Math.imul(hash ^ byte, 0x01000193) >>> 0;
  };
  for (const tile of map.tiles) mix(tile);
  for (const site of map.sites) {
    const text = `${site.id}|${site.kind}|${site.name}|${site.rect.x0},${site.rect.y0},${site.rect.x1},${site.rect.y1}`;
    for (let i = 0; i < text.length; i++) mix(text.charCodeAt(i) & 0xff);
    mix(0);
  }
  return hash;
}

export class MapLoader {
  private readonly loading = new Set<number>();
  /** Maps that came and wait for their turn to be painted. */
  private readonly waiting: ZoneMap[] = [];
  private readonly signatures = new Map<number, number>();
  /** When each zone was read, wanted and refused, in milliseconds. */
  private readonly readAt = new Map<number, number>();
  private readonly wantedAt = new Map<number, number>();
  private readonly failedAt = new Map<number, number>();
  private wanted: readonly number[] = [];

  constructor(
    private readonly sink: MapSink,
    private readonly fetchMap: (zone: number) => Promise<ZoneMap>,
  ) {}

  /**
   * Says which zones the screen needs, the most important first. The
   * loader reads the maps that it does not have, reads an old map again,
   * and frees the maps that were not needed for the longest time.
   */
  want(zones: readonly number[], now: number): void {
    this.wanted = zones;
    for (const zone of zones) {
      this.wantedAt.set(zone, now);
      if (this.loading.has(zone) || this.loading.size >= CONFIG.maps.parallel) continue;
      const read = this.readAt.get(zone);
      const failed = this.failedAt.get(zone);
      if (failed !== undefined && now - failed < CONFIG.maps.retryMs) continue;
      if (read !== undefined && now - read < CONFIG.maps.refreshMs) continue;
      this.load(zone, now);
    }
    this.free();
  }

  private load(zone: number, now: number): void {
    this.loading.add(zone);
    this.fetchMap(zone).then(
      (map) => {
        this.loading.delete(zone);
        this.failedAt.delete(zone);
        this.readAt.set(zone, now);
        // A map that did not change is not painted again.
        if (this.sink.hasMap(zone) && this.signatures.get(zone) === mapSignature(map)) return;
        this.waiting.push(map);
      },
      () => {
        this.loading.delete(zone);
        this.failedAt.set(zone, now);
      },
    );
  }

  /** Frees the maps above the limit that were not wanted for the longest time. */
  private free(): void {
    const known = [...this.readAt.keys()];
    if (known.length <= CONFIG.view.maxMapZones) return;
    const keep = new Set(
      known
        .sort((a, b) => (this.wantedAt.get(b) ?? 0) - (this.wantedAt.get(a) ?? 0) || a - b)
        .slice(0, CONFIG.view.maxMapZones),
    );
    for (const zone of known) {
      if (keep.has(zone)) continue;
      this.readAt.delete(zone);
      this.wantedAt.delete(zone);
      this.signatures.delete(zone);
    }
    this.sink.keepMaps(keep);
  }

  /** Paints the maps that wait, at most `limit` of them. Call it once in each frame. */
  paint(limit: number = CONFIG.render.paintsPerFrame): void {
    for (let painted = 0; painted < limit; ) {
      const map = this.waiting.shift();
      if (map === undefined) return;
      // A map that was freed while it waited is not painted.
      if (!this.readAt.has(map.zone.id)) continue;
      this.sink.addMap(map);
      this.signatures.set(map.zone.id, mapSignature(map));
      painted += 1;
    }
  }

  /** Returns true while a map that the screen needs is not painted yet. */
  get busy(): boolean {
    return this.waiting.length > 0 || this.wanted.some((zone) => !this.sink.hasMap(zone) && !this.failedAt.has(zone));
  }
}
