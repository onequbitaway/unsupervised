// Questions about the layout: which zone is where, and which zones the
// viewer needs for what the camera shows.

import { CONFIG } from '../config';
import { type Camera, type Level, type Size, visibleRect } from './camera';
import type { CityView, Layout, Rect, ZoneView } from './types';

export function contains(rect: Rect, x: number, y: number): boolean {
  return x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1;
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
}

export function centre(rect: Rect): { x: number; y: number } {
  return { x: (rect.x0 + rect.x1) / 2, y: (rect.y0 + rect.y1) / 2 };
}

/** Returns the distance from a point to a rectangle. It is 0 inside. */
function distance(rect: Rect, x: number, y: number): number {
  const dx = Math.max(rect.x0 - x, 0, x - rect.x1);
  const dy = Math.max(rect.y0 - y, 0, y - rect.y1);
  return Math.hypot(dx, dy);
}

export function zoneAt(layout: Layout, x: number, y: number): ZoneView | null {
  return layout.zones.find((zone) => contains(zone.bounds, x, y)) ?? null;
}

/** Returns the zone that is nearest to a point. */
export function nearestZone(layout: Layout, x: number, y: number): ZoneView | null {
  let best: ZoneView | null = null;
  let bestDistance = Infinity;
  for (const zone of layout.zones) {
    const d = distance(zone.bounds, x, y);
    if (d < bestDistance) {
      best = zone;
      bestDistance = d;
    }
  }
  return best;
}

/** Returns the city that is nearest to a point. */
export function nearestCity(layout: Layout, x: number, y: number): CityView | null {
  let best: CityView | null = null;
  let bestDistance = Infinity;
  for (const city of layout.cities) {
    const d = distance(city.bounds, x, y);
    if (d < bestDistance) {
      best = city;
      bestDistance = d;
    }
  }
  return best;
}

/** Returns the zones that the screen shows, the nearest to the centre first. */
export function zonesInView(layout: Layout, cam: Camera, view: Size): ZoneView[] {
  const rect = visibleRect(cam, view);
  return layout.zones
    .filter((zone) => intersects(zone.bounds, rect))
    .sort((a, b) => distance(a.bounds, cam.x, cam.y) - distance(b.bounds, cam.x, cam.y) || a.id - b.id);
}

/**
 * Returns the zones whose residents the viewer needs. The country level
 * needs none: it shows numbers of residents only. The other levels need
 * the zones on the screen, the nearest first, up to the limit.
 */
export function zonesToFollow(level: Level, inView: readonly ZoneView[]): number[] {
  if (level === 'country') return [];
  return inView.slice(0, CONFIG.feed.maxZones).map((zone) => zone.id);
}

/** Returns the zones whose map (tiles and sites) the viewer draws. */
export function zonesToMap(level: Level, inView: readonly ZoneView[]): number[] {
  if (level === 'country') return [];
  return inView.slice(0, CONFIG.view.maxMapZones).map((zone) => zone.id);
}

/** Returns the zone with the most residents. */
export function busiestZone(layout: Layout, populations: ReadonlyMap<number, number>): ZoneView | null {
  let best: ZoneView | null = null;
  let most = -1;
  for (const zone of layout.zones) {
    const count = populations.get(zone.id) ?? zone.residents;
    if (count > most) {
      best = zone;
      most = count;
    }
  }
  return best;
}
