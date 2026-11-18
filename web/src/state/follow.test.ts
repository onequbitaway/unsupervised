import { describe, expect, it } from 'vitest';
import { followTarget } from './follow';
import type { ServerMessage } from './types';
import { initialState, reduce, type ViewerState } from './world';

function run(state: ViewerState, now: number, ...messages: ServerMessage[]): ViewerState {
  return messages.reduce((s, m) => reduce(s, m, now), state);
}

const empty = { moved: [], entered: [], left: [], activity: [], events: [] };

describe('followTarget', () => {
  it('is the place where the resident is drawn, also in the middle of a move', () => {
    let state = run(initialState(), 0, {
      op: 'snapshot',
      zone: 0,
      frame: 1,
      residents: [{ id: 7, name: 'Ada', pos: { x: 10, y: 20 } }],
    });
    expect(followTarget(state, 7, null, 0)).toEqual({ x: 10.5, y: 20.5, live: true });
    state = reduce(state, { op: 'delta', frame: 2, zone: 0, tick: 2, ...empty, moved: [[7, 12, 20]] }, 1_000);
    // The move lasts 1,100 ms. After half of it the resident is half of the way.
    expect(followTarget(state, 7, null, 1_550)).toEqual({ x: 11.5, y: 20.5, live: true });
    expect(followTarget(state, 7, { x: 99, y: 99 }, 5_000)).toEqual({ x: 12.5, y: 20.5, live: true });
  });

  it('goes with the resident into another zone', () => {
    let state = run(
      initialState(),
      0,
      { op: 'snapshot', zone: 0, frame: 1, residents: [{ id: 7, name: 'Ada', pos: { x: 63, y: 5 } }] },
      { op: 'snapshot', zone: 1, frame: 1, residents: [] },
    );
    state = run(
      state,
      1_000,
      { op: 'delta', frame: 2, zone: 0, tick: 2, ...empty, left: [7] },
      { op: 'delta', frame: 2, zone: 1, tick: 2, ...empty, entered: [{ id: 7, name: 'Ada', pos: { x: 64, y: 5 } }] },
    );
    expect(followTarget(state, 7, null, 10_000)).toEqual({ x: 64.5, y: 5.5, live: true });
  });

  it('is the last known place when the feed does not show the resident', () => {
    const state = initialState();
    expect(followTarget(state, 7, { x: 300, y: 40 }, 0)).toEqual({ x: 300.5, y: 40.5, live: false });
    expect(followTarget(state, 7, null, 0)).toBeNull();
  });
});
