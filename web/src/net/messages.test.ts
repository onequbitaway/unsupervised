import { describe, expect, it } from 'vitest';
import { parseServerMessage, readEvent, readFeedPage, readLayout, readProfile, readZoneMap } from './messages';

const clock = { year: 1, day: 2, hour: 3, minute: 4 };

function parse(value: unknown): ReturnType<typeof parseServerMessage> {
  return parseServerMessage(JSON.stringify(value));
}

describe('parseServerMessage', () => {
  it('reads the world message', () => {
    expect(parse({ op: 'world', protocol: 1, frame: 9, clock, populations: [[0, 3], [1, 0]], dropped: 2, viewers: 5 })).toEqual({
      op: 'world',
      protocol: 1,
      frame: 9,
      clock,
      populations: [[0, 3], [1, 0]],
      dropped: 2,
      viewers: 5,
    });
  });

  it('reads a snapshot, with residents that have no activity and no model', () => {
    expect(
      parse({
        op: 'snapshot',
        zone: 2,
        frame: 9,
        residents: [
          { id: 7, name: 'Ada', pos: { x: 1, y: -2 }, activity: 'work', model: 'atlas' },
          { id: 8, name: 'Bo', pos: { x: 3, y: 4 } },
        ],
      }),
    ).toEqual({
      op: 'snapshot',
      zone: 2,
      frame: 9,
      residents: [
        { id: 7, name: 'Ada', pos: { x: 1, y: -2 }, activity: 'work', model: 'atlas' },
        { id: 8, name: 'Bo', pos: { x: 3, y: 4 }, activity: null, model: null },
      ],
    });
  });

  it('reads a delta, and takes an absent list as empty', () => {
    expect(parse({ op: 'delta', frame: 9, zone: 2, tick: 8, moved: [[7, 5, 6]], activity: [[7, null], [8, 'eat']] })).toEqual({
      op: 'delta',
      frame: 9,
      zone: 2,
      tick: 8,
      moved: [[7, 5, 6]],
      entered: [],
      left: [],
      activity: [[7, null], [8, 'eat']],
      events: [],
    });
  });

  it('reads the frame message with its headlines', () => {
    const event = { id: '9-0', frame: 9, clock, tick: 9, realm: 'global', kind: 'politics.election_decided', drama: 85, data: { city: 1 } };
    expect(parse({ op: 'frame', frame: 9, clock, populations: [[1, 4]], headlines: [event], viewers: 2 })).toEqual({
      op: 'frame',
      frame: 9,
      clock,
      populations: [[1, 4]],
      headlines: [{ id: '9-0', frame: 9, clock, kind: 'politics.election_decided', drama: 85, data: { city: 1 } }],
      viewers: 2,
    });
  });

  it('reads the small messages', () => {
    expect(parse({ op: 'unsubscribed', zones: [1, 2] })).toEqual({ op: 'unsubscribed', zones: [1, 2] });
    expect(parse({ op: 'pong' })).toEqual({ op: 'pong' });
    expect(parse({ op: 'error', error: { code: 'unknown_zone', message: 'Zone 99 does not exist.' } })).toEqual({
      op: 'error',
      code: 'unknown_zone',
      message: 'Zone 99 does not exist.',
    });
  });

  it('refuses what is not a message of the feed', () => {
    expect(parseServerMessage('not json')).toBeNull();
    expect(parseServerMessage('[1, 2]')).toBeNull();
    expect(parse({ op: 'act' })).toBeNull();
    expect(parse({ frame: 1 })).toBeNull();
  });

  it('refuses a message with a field of the wrong type', () => {
    expect(parse({ op: 'world', frame: '9', clock, populations: [] })).toBeNull();
    expect(parse({ op: 'world', frame: 9, clock: { year: 1 }, populations: [] })).toBeNull();
    expect(parse({ op: 'snapshot', zone: 2, frame: 9, residents: [{ id: 7, name: 'Ada' }] })).toBeNull();
    expect(parse({ op: 'snapshot', zone: 2, frame: 9, residents: [{ id: 7.5, name: 'Ada', pos: { x: 1, y: 1 } }] })).toBeNull();
    expect(parse({ op: 'delta', frame: 9, zone: 2, moved: [[7, 5]] })).toBeNull();
    expect(parse({ op: 'delta', frame: 9, zone: 2, left: ['7'] })).toBeNull();
    expect(parse({ op: 'delta', frame: 9, zone: 2, moved: 'all' })).toBeNull();
    expect(parse({ op: 'frame', frame: 9, clock, headlines: [{ id: 5 }] })).toBeNull();
    expect(parse({ op: 'unsubscribed', zones: 'all' })).toBeNull();
  });
});

describe('readEvent', () => {
  it('keeps the optional fields only when they are there', () => {
    const base = { id: '3-1', frame: 3, clock, kind: 'society.said', drama: 3 };
    expect(readEvent(base)).toEqual({ ...base, data: null });
    expect(readEvent({ ...base, zone: 4, actor: 7, actor_name: 'Ada', pos: { x: 1, y: 2 }, data: { text: 'hi' } })).toEqual({
      ...base,
      zone: 4,
      actor: 7,
      actor_name: 'Ada',
      pos: { x: 1, y: 2 },
      data: { text: 'hi' },
    });
    expect(readEvent({ ...base, drama: 'high' })).toBeNull();
    expect(readEvent('event')).toBeNull();
  });
});

describe('the HTTP answers', () => {
  const zone = { id: 3, city: 1, name: 'Town 1-0', bounds: { x0: 64, y0: 0, x1: 66, y1: 2 }, residents: 5 };

  it('reads the layout', () => {
    const layout = { type: 'layout', frame: 9, clock, cities: [{ id: 1, name: 'Town', bounds: zone.bounds, zones: [3] }], zones: [zone] };
    expect(readLayout(layout)).toEqual({ frame: 9, clock, cities: layout.cities, zones: [zone] });
    expect(readLayout({ ...layout, type: 'zone' })).toBeNull();
    expect(readLayout({ ...layout, zones: [{ ...zone, bounds: { x0: 0 } }] })).toBeNull();
  });

  it('reads the map of a zone and ignores its residents', () => {
    const site = { id: 40, kind: 'shop', name: 'Shop', rect: { x0: 64, y0: 0, x1: 65, y1: 1 }, tags: ['public'] };
    const view = { type: 'zone', zone, width: 2, height: 2, tiles: [0, 1, 4, 300], sites: [site], residents: [{ id: 1 }] };
    const map = readZoneMap(view);
    expect(map).toEqual({ zone, width: 2, height: 2, tiles: new Uint8Array([0, 1, 4, 0]), sites: [site] });
    // The number of tiles must be the width times the height.
    expect(readZoneMap({ ...view, tiles: [0, 1, 4] })).toBeNull();
    expect(readZoneMap({ ...view, width: 100_000, height: 100_000 })).toBeNull();
    expect(readZoneMap({ ...view, sites: [{ ...site, rect: null }] })).toBeNull();
  });

  it('reads a page of the feed', () => {
    const event = { id: '3-1', frame: 3, clock, kind: 'a.b', drama: 20 };
    expect(readFeedPage({ events: [event], next: '3-1', more: true, gap: false })).toEqual({
      events: [{ ...event, data: null }],
      next: '3-1',
      more: true,
      gap: false,
    });
    expect(readFeedPage({ events: [event] })).toBeNull();
    expect(readFeedPage({ events: [{ id: 1 }], next: '0-0' })).toBeNull();
  });

  it('reads a profile', () => {
    const profile = {
      id: 7,
      name: 'Ada',
      model: 'atlas',
      alive: true,
      zone: 3,
      zone_name: 'Town 1-0',
      city: 1,
      city_name: 'Town',
      pos: { x: 64, y: 1 },
      activity: 'work',
      frame: 9,
      events: [],
    };
    // A server without a chronicle sends no life story: the profile has an empty one.
    const noLife = { jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] };
    expect(readProfile(profile)).toEqual({ ...profile, life: noLife });
    const born = { event: '1-0', frame: 1, clock, kind: 'kernel.spawned', drama: 10, line: 'Ada arrived in Plaza', zone: 0 };
    expect(readProfile({ ...profile, life: { born, jobs: [born] } })?.life).toEqual({ ...noLife, born, jobs: [born] });
    const gone = readProfile({ ...profile, model: undefined, activity: undefined, alive: false, cause: 'old_age' });
    expect(gone).toMatchObject({ alive: false, cause: 'old_age' });
    expect(gone).not.toHaveProperty('model');
    expect(gone).not.toHaveProperty('activity');
    expect(readProfile({ ...profile, pos: [64, 1] })).toBeNull();
    expect(readProfile({ ...profile, id: 'seven' })).toBeNull();
  });
});
