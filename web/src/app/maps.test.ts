import { describe, expect, it } from 'vitest';
import type { ZoneMap } from '../state/types';
import { MapLoader, mapSignature, type MapSink } from './maps';

function map(zone: number, tile = 0, siteName = 'Shop'): ZoneMap {
  return {
    zone: { id: zone, city: 0, name: `z${zone}`, bounds: { x0: zone * 2, y0: 0, x1: zone * 2 + 2, y1: 2 }, residents: 0 },
    width: 2,
    height: 2,
    tiles: new Uint8Array([tile, 1, 2, 3]),
    sites: [{ id: zone * 10, kind: 'shop', name: siteName, rect: { x0: 0, y0: 0, x1: 1, y1: 1 }, tags: [] }],
  };
}

class FakeSink implements MapSink {
  painted: number[] = [];
  maps = new Set<number>();
  hasMap(zone: number): boolean {
    return this.maps.has(zone);
  }
  addMap(added: ZoneMap): void {
    this.maps.add(added.zone.id);
    this.painted.push(added.zone.id);
  }
  keepMaps(zones: ReadonlySet<number>): void {
    for (const zone of [...this.maps]) if (!zones.has(zone)) this.maps.delete(zone);
  }
}

/** A server whose answers the test gives by hand. */
class FakeServer {
  asked: number[] = [];
  private waiting = new Map<number, { resolve: (map: ZoneMap) => void; reject: (error: Error) => void }>();
  fetch = (zone: number): Promise<ZoneMap> => {
    this.asked.push(zone);
    return new Promise((resolve, reject) => this.waiting.set(zone, { resolve, reject }));
  };
  async answer(zone: number, answer: ZoneMap | Error): Promise<void> {
    const call = this.waiting.get(zone);
    if (!call) throw new Error(`zone ${zone} was not asked`);
    this.waiting.delete(zone);
    if (answer instanceof Error) call.reject(answer);
    else call.resolve(answer);
    // The loader sees the answer in the next turn of the event loop.
    await Promise.resolve();
    await Promise.resolve();
  }
}

describe('mapSignature', () => {
  it('changes with a tile and with a site, and with nothing else', () => {
    expect(mapSignature(map(1))).toBe(mapSignature(map(1)));
    expect(mapSignature(map(1, 5))).not.toBe(mapSignature(map(1)));
    expect(mapSignature(map(1, 0, 'Bakery'))).not.toBe(mapSignature(map(1)));
  });
});

describe('MapLoader', () => {
  it('reads the maps that the screen needs, a few at a time, and paints one in each frame', async () => {
    const sink = new FakeSink();
    const server = new FakeServer();
    const loader = new MapLoader(sink, server.fetch);
    loader.want([3, 1, 4, 5, 9, 2], 0);
    // Four at a time, in the order of the wish.
    expect(server.asked).toEqual([3, 1, 4, 5]);
    expect(loader.busy).toBe(true);
    await server.answer(1, map(1));
    await server.answer(3, map(3));
    loader.paint();
    expect(sink.painted).toEqual([1]);
    loader.paint();
    expect(sink.painted).toEqual([1, 3]);
    // The next wish reads the rest. Nothing is asked twice.
    loader.want([3, 1, 4, 5, 9, 2], 200);
    expect(server.asked).toEqual([3, 1, 4, 5, 9, 2]);
    for (const zone of [4, 5, 9, 2]) await server.answer(zone, map(zone));
    loader.paint(10);
    expect(sink.painted).toEqual([1, 3, 4, 5, 9, 2]);
    expect(loader.busy).toBe(false);
  });

  it('asks again after a refusal, but not at once', async () => {
    const sink = new FakeSink();
    const server = new FakeServer();
    const loader = new MapLoader(sink, server.fetch);
    loader.want([7], 0);
    await server.answer(7, new Error('503'));
    loader.want([7], 1_000);
    expect(server.asked).toEqual([7]);
    // A refused map does not hold the viewer back.
    expect(loader.busy).toBe(false);
    loader.want([7], 3_000);
    expect(server.asked).toEqual([7, 7]);
    await server.answer(7, map(7));
    loader.paint();
    expect(sink.painted).toEqual([7]);
  });

  it('reads an old map again and paints it only when it changed', async () => {
    const sink = new FakeSink();
    const server = new FakeServer();
    const loader = new MapLoader(sink, server.fetch);
    loader.want([7], 0);
    await server.answer(7, map(7));
    loader.paint();
    loader.want([7], 59_999);
    expect(server.asked).toEqual([7]);
    loader.want([7], 60_000);
    expect(server.asked).toEqual([7, 7]);
    await server.answer(7, map(7));
    loader.paint();
    expect(sink.painted).toEqual([7]);
    loader.want([7], 120_000);
    await server.answer(7, map(7, 0, 'New name'));
    loader.paint();
    expect(sink.painted).toEqual([7, 7]);
  });

  it('frees the maps that were not needed for the longest time', async () => {
    const sink = new FakeSink();
    const server = new FakeServer();
    const loader = new MapLoader(sink, server.fetch);
    // 26 zones, one more than the limit. Zone 0 is the one that was needed first.
    for (let zone = 0; zone < 26; zone++) {
      loader.want([zone], zone);
      await server.answer(zone, map(zone));
      loader.paint();
    }
    expect(sink.maps.size).toBe(26);
    loader.want([25], 100);
    expect(sink.maps.size).toBe(25);
    expect(sink.maps.has(0)).toBe(false);
    expect(sink.maps.has(1)).toBe(true);
    // A freed map is read again when the screen needs it.
    loader.want([0], 200);
    expect(server.asked.filter((zone) => zone === 0)).toHaveLength(2);
  });
});
