import { describe, expect, it } from 'vitest';
import { isMoving, measureTick, moveTo, positionAt, progress, standing } from './motion';

describe('motion', () => {
  it('a resident that stands is at its tile', () => {
    const m = standing(4, 9, 1_000);
    expect(positionAt(m, 0)).toEqual({ x: 4, y: 9 });
    expect(positionAt(m, 5_000)).toEqual({ x: 4, y: 9 });
    expect(isMoving(m, 1_000)).toBe(false);
    expect(progress(m, 1_000)).toBe(1);
  });

  it('a move goes in a straight line and lasts a little more than one tick', () => {
    const m = moveTo(standing(0, 0, 0), 2, 4, 1_000, 1_000);
    expect(m.dur).toBe(1_100);
    expect(positionAt(m, 1_000)).toEqual({ x: 0, y: 0 });
    expect(positionAt(m, 1_550)).toEqual({ x: 1, y: 2 });
    expect(positionAt(m, 2_100)).toEqual({ x: 2, y: 4 });
    expect(positionAt(m, 9_000)).toEqual({ x: 2, y: 4 });
    expect(isMoving(m, 1_550)).toBe(true);
    expect(isMoving(m, 2_100)).toBe(false);
  });

  it('a new position in the middle of a move starts where the resident is drawn', () => {
    const first = moveTo(standing(0, 0, 0), 10, 0, 0, 1_000);
    // After one tick the resident is drawn at 10 / 1.1 tiles.
    const second = moveTo(first, 10, 10, 1_000, 1_000);
    expect(second.fromX).toBeCloseTo(9.0909, 3);
    expect(second.fromY).toBe(0);
    expect(positionAt(second, 1_000).x).toBeCloseTo(9.0909, 3);
    expect(positionAt(second, 2_100)).toEqual({ x: 10, y: 10 });
  });

  it('a walk of one tile per tick never stops between tiles', () => {
    let m = standing(0, 0, 0);
    let last = 0;
    for (let tick = 1; tick <= 6; tick++) {
      m = moveTo(m, tick, 0, tick * 500, 500);
      // Just before the next position comes, the resident still moves.
      const before = positionAt(m, tick * 500 + 499).x;
      expect(before).toBeGreaterThan(last);
      expect(isMoving(m, tick * 500 + 499)).toBe(true);
      last = before;
    }
    expect(last).toBeGreaterThan(5);
    expect(last).toBeLessThan(6);
  });

  it('a long distance is a jump', () => {
    const m = moveTo(standing(0, 0, 0), 41, 0, 100, 1_000);
    expect(m.dur).toBe(0);
    expect(positionAt(m, 100)).toEqual({ x: 41, y: 0 });
    expect(moveTo(standing(0, 0, 0), 40, 40, 100, 1_000).dur).toBe(1_100);
  });

  it('the tick length follows the time between frames', () => {
    expect(measureTick(1_000, 200)).toBe(800);
    expect(measureTick(800, 200)).toBe(650);
    // A pause or a burst does not change the estimate.
    expect(measureTick(1_000, 30_000)).toBe(1_000);
    expect(measureTick(1_000, 2)).toBe(1_000);
    expect(measureTick(1_000, Number.NaN)).toBe(1_000);
  });
});
