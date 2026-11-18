import { describe, expect, it } from 'vitest';
import { positionAt } from './motion';
import type { FeedEvent, ResidentView, ServerMessage } from './types';
import { findMover, initialState, keepZones, reduce, totalPopulation, type ViewerState } from './world';

const clock = { year: 1, day: 1, hour: 0, minute: 0 };

function resident(id: number, x: number, y: number, extra: Partial<ResidentView> = {}): ResidentView {
  return { id, name: `r${id}`, pos: { x, y }, ...extra };
}

function snapshot(zone: number, residents: ResidentView[], frame = 1): ServerMessage {
  return { op: 'snapshot', zone, frame, residents };
}

function delta(zone: number, frame: number, parts: Partial<Extract<ServerMessage, { op: 'delta' }>>): ServerMessage {
  return { op: 'delta', frame, zone, tick: frame, moved: [], entered: [], left: [], activity: [], events: [], ...parts };
}

function frame(n: number, parts: Partial<Extract<ServerMessage, { op: 'frame' }>> = {}): ServerMessage {
  return { op: 'frame', frame: n, clock: { ...clock, minute: n }, populations: [], headlines: [], viewers: 1, ...parts };
}

function said(actor: number, text: unknown): FeedEvent {
  return { id: `1-${actor}`, frame: 1, clock, kind: 'society.said', actor, drama: 3, data: { text } };
}

/** Applies messages that all come at the same time. */
function run(state: ViewerState, now: number, ...messages: ServerMessage[]): ViewerState {
  return messages.reduce((s, m) => reduce(s, m, now), state);
}

function ids(state: ViewerState, zone: number): number[] {
  return [...(state.zones.get(zone)?.residents.keys() ?? [])].sort((a, b) => a - b);
}

describe('reduce', () => {
  it('the world message sets the frame, the clock and every population', () => {
    const state = reduce(
      initialState(),
      { op: 'world', protocol: 1, frame: 7, clock: { ...clock, minute: 7 }, populations: [[0, 3], [1, 0], [2, 5]], dropped: 0, viewers: 4 },
      100,
    );
    expect(state.frame).toBe(7);
    expect(state.clock).toEqual({ year: 1, day: 1, hour: 0, minute: 7 });
    expect([...state.populations]).toEqual([[0, 3], [1, 0], [2, 5]]);
    expect(totalPopulation(state)).toBe(8);
    expect(state.viewers).toBe(4);
  });

  it('a snapshot replaces the residents of its zone only', () => {
    let state = run(initialState(), 0, snapshot(0, [resident(1, 2, 2), resident(2, 3, 3)]), snapshot(1, [resident(9, 70, 2)]));
    state = reduce(state, snapshot(0, [resident(2, 4, 4), resident(3, 5, 5, { model: 'atlas', activity: 'work' })], 5), 10);
    expect(ids(state, 0)).toEqual([2, 3]);
    expect(ids(state, 1)).toEqual([9]);
    expect(state.zones.get(0)?.frame).toBe(5);
    const three = state.zones.get(0)?.residents.get(3);
    expect(three).toMatchObject({ id: 3, name: 'r3', model: 'atlas', activity: 'work', x: 5, y: 5, dur: 0 });
  });

  it('a delta applies left, entered, moved and activity', () => {
    let state = reduce(initialState(), snapshot(0, [resident(1, 2, 2), resident(2, 3, 3)]), 0);
    state = reduce(
      state,
      delta(0, 2, {
        left: [2],
        entered: [resident(3, 9, 9)],
        moved: [[1, 3, 2], [3, 9, 8], [99, 0, 0]],
        activity: [[1, 'work'], [3, null], [99, 'sleep']],
      }),
      1_000,
    );
    expect(ids(state, 0)).toEqual([1, 3]);
    const one = state.zones.get(0)?.residents.get(1);
    expect(one).toMatchObject({ x: 3, y: 2, fromX: 2, fromY: 2, t0: 1_000, dur: 1_100, activity: 'work' });
    // The move of a resident that entered in the same delta is newer than its view.
    expect(state.zones.get(0)?.residents.get(3)).toMatchObject({ x: 9, y: 8, fromX: 9, fromY: 9 });
    expect(state.zones.get(0)?.frame).toBe(2);
  });

  it('never changes the state that it got', () => {
    const before = reduce(initialState(), snapshot(0, [resident(1, 2, 2)]), 0);
    const residents = before.zones.get(0)?.residents;
    const copy = JSON.stringify([...(residents ?? [])]);
    const after = run(before, 500, delta(0, 2, { moved: [[1, 5, 5]], left: [], entered: [resident(2, 1, 1)] }), frame(2));
    expect(after).not.toBe(before);
    expect(before.zones.get(0)?.residents).toBe(residents);
    expect(JSON.stringify([...(residents ?? [])])).toBe(copy);
    expect(before.frame).toBe(0);
    expect(ids(before, 0)).toEqual([1]);
    // A zone that a message does not touch keeps its identity.
    const other = reduce(after, snapshot(1, []), 600);
    expect(other.zones.get(0)).toBe(after.zones.get(0));
  });

  it('a delta for a zone without a snapshot is ignored', () => {
    const state = reduce(initialState(), snapshot(0, [resident(1, 2, 2)]), 0);
    expect(reduce(state, delta(5, 2, { entered: [resident(7, 1, 1)] }), 10)).toBe(state);
  });

  it('a resident that crosses to another zone goes on from where it was drawn', () => {
    const start = run(initialState(), 0, snapshot(0, [resident(1, 63, 5)]), snapshot(1, []));
    // The old zone says `left` before the new zone says `entered`.
    let state = run(start, 1_000, delta(0, 2, { left: [1] }), delta(1, 2, { entered: [resident(1, 64, 5)] }), frame(2));
    expect(ids(state, 0)).toEqual([]);
    let one = state.zones.get(1)?.residents.get(1);
    expect(one).toMatchObject({ fromX: 63, fromY: 5, x: 64, y: 5, dur: 1_100 });
    expect(positionAt(one!, 1_550)).toEqual({ x: 63.5, y: 5 });
    // The other order: `entered` in the new zone comes first.
    state = run(start, 1_000, delta(1, 2, { entered: [resident(1, 64, 5)] }));
    one = state.zones.get(1)?.residents.get(1);
    expect(one).toMatchObject({ fromX: 63, x: 64 });
    expect(findMover(state, 1)?.zone).toBe(0);
    state = run(state, 1_000, delta(0, 2, { left: [1] }));
    expect(findMover(state, 1)?.zone).toBe(1);
  });

  it('the trace of a resident that left ends with the frame', () => {
    let state = run(initialState(), 0, snapshot(0, [resident(1, 10, 10)]), snapshot(1, []));
    state = run(state, 1_000, delta(0, 2, { left: [1] }), frame(2));
    expect(state.traces.size).toBe(1);
    state = run(state, 2_000, frame(3));
    expect(state.traces.size).toBe(0);
    // It comes back much later: it appears at its tile, it does not glide there.
    state = run(state, 9_000, delta(1, 9, { entered: [resident(1, 70, 10)] }));
    expect(state.zones.get(1)?.residents.get(1)).toMatchObject({ fromX: 70, x: 70, dur: 0 });
  });

  it('a new snapshot of a zone moves the residents from where they were drawn', () => {
    let state = reduce(initialState(), snapshot(0, [resident(1, 10, 10)]), 0);
    state = reduce(state, snapshot(0, [resident(1, 12, 10)], 9), 1_000);
    expect(state.zones.get(0)?.residents.get(1)).toMatchObject({ fromX: 10, x: 12, t0: 1_000, dur: 1_100 });
  });

  it('a frame message advances the clock, the populations and the tick estimate', () => {
    let state = reduce(initialState(), { op: 'world', protocol: 1, frame: 1, clock, populations: [[0, 3], [1, 0]], dropped: 0, viewers: 1 }, 0);
    state = reduce(state, frame(2, { populations: [[1, 2]], viewers: 6 }), 1_000);
    expect(state.frame).toBe(2);
    expect(state.clock?.minute).toBe(2);
    expect([...state.populations]).toEqual([[0, 3], [1, 2]]);
    expect(state.viewers).toBe(6);
    // The first frame after a world message measures nothing.
    expect(state.tickMs).toBe(1_000);
    state = reduce(state, frame(3), 1_200);
    expect(state.tickMs).toBe(800);
    state = reduce(state, frame(4), 1_400);
    expect(state.tickMs).toBe(650);
  });

  it('a world message after lost frames counts them and does not measure a tick', () => {
    let state = run(initialState(), 0, frame(1));
    state = reduce(state, frame(2), 200);
    expect(state.tickMs).toBe(800);
    state = reduce(state, { op: 'world', protocol: 1, frame: 40, clock, populations: [], dropped: 12, viewers: 1 }, 9_000);
    expect(state.dropped).toBe(12);
    state = reduce(state, frame(41), 9_100);
    expect(state.tickMs).toBe(800);
  });

  it('speech becomes a bubble for some time', () => {
    let state = reduce(initialState(), snapshot(0, [resident(1, 2, 2)]), 0);
    state = reduce(
      state,
      delta(0, 2, { events: [said(1, { untrusted: true, from: 1, text: '<b>Hello</b>\nall' }), said(2, { nothing: true })] }),
      1_000,
    );
    expect([...state.speech.values()]).toEqual([{ resident: 1, zone: 0, text: '<b>Hello</b> all', until: 7_000 }]);
    // A newer speech of the same resident replaces the bubble.
    state = reduce(state, delta(0, 3, { events: [said(1, 'Again')] }), 2_000);
    expect(state.speech.get(1)).toMatchObject({ text: 'Again', until: 8_000 });
    state = reduce(state, frame(3), 7_999);
    expect(state.speech.size).toBe(1);
    state = reduce(state, frame(4), 8_000);
    expect(state.speech.size).toBe(0);
  });

  it('unsubscribed zones and zones that are not kept are forgotten', () => {
    const state = run(initialState(), 0, snapshot(0, [resident(1, 2, 2)]), snapshot(1, []), snapshot(2, []));
    const after = reduce(state, { op: 'unsubscribed', zones: [1, 7] }, 5);
    expect([...after.zones.keys()]).toEqual([0, 2]);
    expect(reduce(after, { op: 'unsubscribed', zones: [7] }, 5)).toBe(after);
    const kept = keepZones(after, new Set([2, 3]));
    expect([...kept.zones.keys()]).toEqual([2]);
    expect(keepZones(kept, new Set([2]))).toBe(kept);
  });

  it('errors and pongs change nothing', () => {
    const state = initialState();
    expect(reduce(state, { op: 'pong' }, 1)).toBe(state);
    expect(reduce(state, { op: 'error', code: 'unknown_zone', message: 'x' }, 1)).toBe(state);
  });
});
