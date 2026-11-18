import { describe, expect, it } from 'vitest';
import type { Camera } from './camera';
import { pickResident } from './pick';
import type { Layout, ZoneView } from './types';
import { initialState, reduce } from './world';
import { busiestZone, centre, contains, intersects, nearestCity, nearestZone, zoneAt, zonesInView, zonesToFollow, zonesToMap } from './zones';

function zone(id: number, city: number, x0: number, y0: number, residents = 0): ZoneView {
  return { id, city, name: `z${id}`, bounds: { x0, y0, x1: x0 + 64, y1: y0 + 64 }, residents };
}

// Two cities side by side with a gap. City 0 has two zones, city 1 has one.
const layout: Layout = {
  frame: 1,
  clock: { year: 1, day: 1, hour: 0, minute: 0 },
  cities: [
    { id: 0, name: 'West', bounds: { x0: 0, y0: 0, x1: 128, y1: 64 }, zones: [0, 1] },
    { id: 1, name: 'East', bounds: { x0: 192, y0: 0, x1: 256, y1: 64 }, zones: [2] },
  ],
  zones: [zone(0, 0, 0, 0, 3), zone(1, 0, 64, 0, 9), zone(2, 1, 192, 0, 5)],
};

const screen = { width: 640, height: 640 };

describe('rectangles', () => {
  it('a rectangle holds its minimum and not its maximum', () => {
    const rect = { x0: 0, y0: 0, x1: 64, y1: 64 };
    expect(contains(rect, 0, 0)).toBe(true);
    expect(contains(rect, 63.9, 63.9)).toBe(true);
    expect(contains(rect, 64, 10)).toBe(false);
    expect(intersects(rect, { x0: 63, y0: 63, x1: 70, y1: 70 })).toBe(true);
    expect(intersects(rect, { x0: 64, y0: 0, x1: 70, y1: 70 })).toBe(false);
    expect(centre(rect)).toEqual({ x: 32, y: 32 });
  });
});

describe('places', () => {
  it('finds the zone at a point, and the nearest zone and city for a point outside', () => {
    expect(zoneAt(layout, 70, 10)?.id).toBe(1);
    expect(zoneAt(layout, 150, 10)).toBeNull();
    expect(nearestZone(layout, 150, 10)?.id).toBe(1);
    expect(nearestZone(layout, 170, 10)?.id).toBe(2);
    expect(nearestCity(layout, 170, 10)?.id).toBe(1);
    expect(nearestCity(layout, -50, 500)?.id).toBe(0);
  });

  it('finds the zone with the most residents, by the newest numbers', () => {
    expect(busiestZone(layout, new Map())?.id).toBe(1);
    expect(busiestZone(layout, new Map([[1, 2], [2, 40]]))?.id).toBe(2);
    expect(busiestZone({ ...layout, zones: [] }, new Map())).toBeNull();
  });
});

describe('zonesInView', () => {
  it('gives the zones on the screen, the nearest to the centre first', () => {
    // 5 pixels for one tile: the screen shows 128 tiles around the point (96, 32).
    const camera: Camera = { x: 96, y: 32, scale: 5 };
    expect(zonesInView(layout, camera, screen).map((z) => z.id)).toEqual([1, 0]);
    // Far from every zone: nothing.
    expect(zonesInView(layout, { x: 1_000, y: 1_000, scale: 5 }, screen)).toEqual([]);
    // The whole country. The point (140, 32) is nearer to zone 2 than to zone 0.
    expect(zonesInView(layout, { x: 140, y: 32, scale: 2 }, screen).map((z) => z.id)).toEqual([1, 2, 0]);
    // Two zones at the same distance: the lower id comes first.
    expect(zonesInView(layout, { x: 128, y: 32, scale: 2 }, screen).map((z) => z.id)).toEqual([1, 0, 2]);
  });

  it('the country level follows no zone; the other levels follow what is on the screen, up to a limit', () => {
    const inView = zonesInView(layout, { x: 140, y: 32, scale: 2 }, screen);
    expect(zonesToFollow('country', inView)).toEqual([]);
    expect(zonesToMap('country', inView)).toEqual([]);
    expect(zonesToFollow('city', inView)).toEqual([1, 2, 0]);
    expect(zonesToMap('street', inView)).toEqual([1, 2, 0]);
    const many = Array.from({ length: 40 }, (_, id) => zone(id, 0, id * 64, 0));
    expect(zonesToFollow('city', many)).toHaveLength(16);
    expect(zonesToFollow('city', many)[15]).toBe(15);
    expect(zonesToMap('city', many)).toHaveLength(25);
  });
});

describe('pickResident', () => {
  const state = reduce(
    initialState(),
    {
      op: 'snapshot',
      zone: 0,
      frame: 1,
      residents: [
        { id: 5, name: 'a', pos: { x: 10, y: 10 } },
        { id: 3, name: 'b', pos: { x: 10, y: 10 } },
        { id: 9, name: 'c', pos: { x: 14, y: 10 } },
      ],
    },
    0,
  );

  it('picks the nearest resident within the radius', () => {
    // A resident stands in the middle of its tile.
    expect(pickResident(state, 14.4, 10.6, 1, 0)).toBe(9);
    expect(pickResident(state, 12.6, 10.5, 2, 0)).toBe(9);
    expect(pickResident(state, 12.4, 10.5, 2, 0)).toBe(3);
    expect(pickResident(state, 30, 30, 2, 0)).toBeNull();
    expect(pickResident(state, 12.5, 10.5, 1, 0)).toBeNull();
  });

  it('of two residents on the same tile the lower id wins', () => {
    expect(pickResident(state, 10.5, 10.5, 1, 0)).toBe(3);
  });

  it('picks a walking resident where it is drawn, not where it goes', () => {
    const moved = reduce(
      state,
      { op: 'delta', frame: 2, zone: 0, tick: 2, moved: [[9, 20, 10]], entered: [], left: [], activity: [], events: [] },
      1_000,
    );
    // The move lasts 1,100 ms. At its start the resident is still at tile 14.
    expect(pickResident(moved, 14.5, 10.5, 0.4, 1_000)).toBe(9);
    expect(pickResident(moved, 20.5, 10.5, 0.4, 1_000)).toBeNull();
    expect(pickResident(moved, 20.5, 10.5, 0.4, 2_100)).toBe(9);
  });
});
