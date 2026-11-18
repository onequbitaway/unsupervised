import { describe, expect, it } from 'vitest';
import { type LabelCandidate, pickLabels } from './labels';
import { pickResident } from './pick';
import { initialState, reduce } from './world';

const options = { budget: 3, cx: 500, cy: 500, cellWidth: 60, cellHeight: 20 };

function at(id: number, sx: number, sy: number, priority = 0): LabelCandidate {
  return { id, sx, sy, priority };
}

describe('pickLabels', () => {
  it('prefers residents near the centre of the screen', () => {
    const picked = pickLabels([at(1, 100, 100), at(2, 510, 500), at(3, 700, 520), at(4, 900, 900)], options);
    expect(picked).toEqual([2, 3, 1]);
  });

  it('a priority wins over the distance', () => {
    const picked = pickLabels([at(1, 500, 500), at(2, 30, 30, 5), at(3, 800, 800, 9)], { ...options, budget: 2 });
    expect(picked).toEqual([3, 2]);
  });

  it('never puts two names in the same place', () => {
    // Residents 1, 2 and 3 stand in one crowd; 4 stands away from it.
    const crowd = [at(1, 500, 500), at(2, 505, 503), at(3, 530, 505), at(4, 500, 560)];
    expect(pickLabels(crowd, { ...options, budget: 10 })).toEqual([1, 4]);
    // One row lower is another place.
    expect(pickLabels([at(1, 500, 500), at(2, 500, 521)], options)).toEqual([1, 2]);
  });

  it('the same input gives the same names', () => {
    const tie = [at(9, 520, 500), at(3, 480, 500)];
    expect(pickLabels(tie, { ...options, budget: 1 })).toEqual([3]);
    expect(pickLabels([...tie].reverse(), { ...options, budget: 1 })).toEqual([3]);
    expect(pickLabels([], options)).toEqual([]);
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
        { id: 1, name: 'a', pos: { x: 10, y: 10 } },
        { id: 2, name: 'b', pos: { x: 12, y: 10 } },
        { id: 3, name: 'c', pos: { x: 12, y: 10 } },
      ],
    },
    0,
  );

  it('selects the nearest resident within the radius', () => {
    expect(pickResident(state, 10.5, 10.5, 1, 0)).toBe(1);
    expect(pickResident(state, 11.4, 10.5, 1, 0)).toBe(1);
    expect(pickResident(state, 11.6, 10.5, 1, 0)).toBe(2);
    expect(pickResident(state, 20, 20, 1, 0)).toBeNull();
    expect(pickResident(state, 13.4, 10.5, 1, 0)).toBe(2);
    expect(pickResident(state, 13.6, 10.5, 1, 0)).toBeNull();
  });

  it('follows a resident while it moves', () => {
    const moved = reduce(
      state,
      { op: 'delta', frame: 2, zone: 0, tick: 2, moved: [[1, 20, 10]], entered: [], left: [], activity: [], events: [] },
      1_000,
    );
    // Half of the way after half of the 1,100 milliseconds.
    expect(pickResident(moved, 15.5, 10.5, 0.2, 1_550)).toBe(1);
    expect(pickResident(moved, 15.5, 10.5, 0.2, 1_000)).toBeNull();
  });
});
