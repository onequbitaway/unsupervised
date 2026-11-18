import { describe, expect, it } from 'vitest';
import {
  approach,
  clampToWorld,
  fitRect,
  fits,
  levelOf,
  screenToWorld,
  visibleRect,
  worldBounds,
  worldToScreen,
  zoomAt,
} from './camera';
import type { Layout, ZoneView } from './types';
import { busiestZone, nearestCity, nearestZone, zoneAt, zonesInView, zonesToFollow, zonesToMap } from './zones';

const clock = { year: 1, day: 1, hour: 0, minute: 0 };
const view = { width: 1920, height: 1080 };

/** Cities of `side` by `side` zones of 64 tiles, in a row with a gap of one zone. */
function layout(cities: number, side: number): Layout {
  const out: Layout = { frame: 0, clock, cities: [], zones: [] };
  for (let c = 0; c < cities; c++) {
    const x0 = c * (side + 1) * 64;
    const zones: number[] = [];
    for (let row = 0; row < side; row++) {
      for (let col = 0; col < side; col++) {
        const id = out.zones.length;
        zones.push(id);
        out.zones.push({
          id,
          city: c,
          name: `C${c} ${col}-${row}`,
          bounds: { x0: x0 + col * 64, y0: row * 64, x1: x0 + col * 64 + 64, y1: row * 64 + 64 },
          residents: id,
        });
      }
    }
    out.cities.push({ id: c, name: `City ${c}`, bounds: { x0, y0: 0, x1: x0 + side * 64, y1: side * 64 }, zones });
  }
  return out;
}

describe('camera', () => {
  it('maps between the world and the screen', () => {
    const cam = { x: 100, y: 50, scale: 4 };
    expect(worldToScreen(cam, view, 100, 50)).toEqual({ x: 960, y: 540 });
    expect(worldToScreen(cam, view, 110, 40)).toEqual({ x: 1000, y: 500 });
    expect(screenToWorld(cam, view, 1000, 500)).toEqual({ x: 110, y: 40 });
    expect(visibleRect(cam, view)).toEqual({ x0: -140, y0: -85, x1: 340, y1: 185 });
  });

  it('fits a rectangle on a part of the screen', () => {
    expect(fitRect({ x0: 0, y0: 0, x1: 192, y1: 192 }, view, 1)).toEqual({ x: 96, y: 96, scale: 5.625 });
    expect(fitRect({ x0: 0, y0: 0, x1: 960, y1: 10 }, view, 0.5).scale).toBe(1);
    expect(worldBounds(layout(2, 3))).toEqual({ x0: 0, y0: 0, x1: 448, y1: 192 });
  });

  it('each level has its own range of scales', () => {
    const f = fits(layout(2, 3), view);
    expect(f.country).toBeCloseTo((1920 / 448) * 0.72, 6);
    expect(f.city).toBeCloseTo((1080 / 192) * 0.92, 6);
    expect(f.street).toBeCloseTo((1080 / 64) * 1.05, 6);
    expect(levelOf(f.country, f)).toBe('country');
    expect(levelOf(f.city, f)).toBe('city');
    expect(levelOf(f.street, f)).toBe('street');
    expect(levelOf(f.street * 3, f)).toBe('street');
    expect(levelOf(f.country * 0.5, f)).toBe('country');
  });

  it('a world of one city still has a country level', () => {
    const f = fits(layout(1, 3), view);
    expect(f.country).toBeLessThan(f.city);
    expect(levelOf(f.country, f)).toBe('country');
    expect(levelOf(f.city, f)).toBe('city');
    // A world of one district: the city and the street are the same view.
    const tiny = fits(layout(1, 1), view);
    expect(levelOf(tiny.country, tiny)).toBe('country');
    expect(levelOf(tiny.street, tiny)).toBe('street');
  });

  it('zooms around the pointer', () => {
    const f = fits(layout(2, 3), view);
    const cam = { x: 100, y: 100, scale: 4 };
    const zoomed = zoomAt(cam, view, 2, 1160, 540, f);
    expect(zoomed.scale).toBe(8);
    // The point under the pointer (150, 100) stays under the pointer.
    expect(screenToWorld(zoomed, view, 1160, 540)).toEqual({ x: 150, y: 100 });
    // The scale stays between the limits.
    expect(zoomAt(cam, view, 1000, 960, 540, f).scale).toBe(56);
    expect(zoomAt(cam, view, 0.0001, 960, 540, f).scale).toBeCloseTo(f.country * 0.6, 6);
    expect(clampToWorld({ x: -50, y: 900, scale: 2 }, { x0: 0, y0: 0, x1: 448, y1: 192 })).toEqual({ x: 0, y: 192, scale: 2 });
  });

  it('approaches a target by half in each half-life, at any frame rate', () => {
    const from = { x: 0, y: 0, scale: 1 };
    const to = { x: 100, y: 0, scale: 16 };
    const once = approach(from, to, 140, 140);
    expect(once.x).toBeCloseTo(50, 6);
    expect(once.scale).toBeCloseTo(4, 6);
    let stepped = from;
    for (let i = 0; i < 10; i++) stepped = approach(stepped, to, 14, 140);
    expect(stepped.x).toBeCloseTo(50, 6);
    expect(stepped.scale).toBeCloseTo(4, 6);
    // Near the target the camera stops exactly on it.
    expect(approach({ x: 99.99, y: 0, scale: 16 }, to, 16, 140)).toEqual(to);
  });
});

describe('zones', () => {
  const world = layout(2, 3);
  const id = (zone: ZoneView | null): number | undefined => zone?.id;

  it('finds the zone and the city of a point', () => {
    expect(id(zoneAt(world, 70, 10))).toBe(1);
    expect(id(zoneAt(world, 64, 64))).toBe(4);
    expect(zoneAt(world, 200, 10)).toBeNull();
    expect(id(nearestZone(world, 200, 10))).toBe(2);
    expect(id(nearestZone(world, 250, 10))).toBe(9);
    expect(nearestCity(world, 250, 500)?.id).toBe(1);
  });

  it('lists the zones on the screen, the nearest first', () => {
    // 16 pixels for a tile: the screen shows 120 by 67.5 tiles.
    const cam = { x: 96, y: 96, scale: 16 };
    expect(zonesInView(world, cam, view).map((zone) => zone.id)).toEqual([4, 1, 3, 5, 7, 0, 2, 6, 8]);
    const close = { x: 32, y: 32, scale: 32 };
    expect(zonesInView(world, close, view).map((zone) => zone.id)).toEqual([0]);
  });

  it('follows no zone at the country level and at most 16 at the others', () => {
    const big = layout(1, 6);
    const inView = zonesInView(big, { x: 192, y: 192, scale: 2 }, view);
    expect(inView).toHaveLength(36);
    expect(zonesToFollow('country', inView)).toEqual([]);
    expect(zonesToMap('country', inView)).toEqual([]);
    const followed = zonesToFollow('city', inView);
    expect(followed).toHaveLength(16);
    // The four zones around the centre come first.
    expect(followed.slice(0, 4).sort((a, b) => a - b)).toEqual([14, 15, 20, 21]);
    expect(zonesToMap('city', inView)).toHaveLength(25);
    expect(zonesToFollow('street', inView.slice(0, 2))).toEqual(inView.slice(0, 2).map((z) => z.id));
  });

  it('finds the zone with the most residents', () => {
    expect(id(busiestZone(world, new Map()))).toBe(17);
    expect(id(busiestZone(world, new Map([[3, 500]])))).toBe(3);
  });
});
