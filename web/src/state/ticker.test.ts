import { describe, expect, it } from 'vitest';
import { addItems, advance, emptyTicker, recent, score, type TickerItem, type TickerState } from './ticker';

function item(id: string, drama: number, at = 0): Omit<TickerItem, 'shown'> {
  return { id, drama, text: `event ${id}`, at, zone: null, actor: null };
}

function fill(now: number, ...items: Omit<TickerItem, 'shown'>[]): TickerState {
  return addItems(emptyTicker(), items, now);
}

describe('ticker', () => {
  it('shows the event with the highest drama first', () => {
    let state = fill(0, item('a', 5), item('b', 60), item('c', 30));
    state = advance(state, 0);
    expect(state.current?.id).toBe('b');
    // The line holds an item for 5 seconds.
    expect(advance(state, 4_999).current?.id).toBe('b');
    state = advance(state, 5_000);
    expect(state.current?.id).toBe('c');
    state = advance(state, 10_000);
    expect(state.current?.id).toBe('a');
    // Nothing waits: the last item stays, it is not shown again as new.
    const idle = advance(state, 60_000);
    expect(idle).toBe(state);
  });

  it('a headline takes the line at once, and holds it longer', () => {
    let state = advance(fill(0, item('routine', 5)), 0);
    state = addItems(state, [item('heist', 80, 1_000)], 1_000);
    state = advance(state, 1_000);
    expect(state.current?.id).toBe('heist');
    // A second headline waits for the first: 9 seconds.
    state = addItems(state, [item('election', 95, 2_000), item('shift', 6, 2_000)], 2_000);
    expect(advance(state, 9_999).current?.id).toBe('heist');
    state = advance(state, 10_000);
    expect(state.current?.id).toBe('election');
  });

  it('prefers a new event to an old one with a little more drama', () => {
    // 12 points per minute: after 2 minutes a score of 40 is worth 16.
    const old = item('old', 40, 0);
    const fresh = item('fresh', 25, 120_000);
    expect(score({ ...old, shown: false }, 120_000)).toBe(16);
    const state = advance(fill(120_000, old, fresh), 120_000);
    expect(state.current?.id).toBe('fresh');
  });

  it('never shows an event twice and ignores one that it has', () => {
    let state = advance(fill(0, item('a', 50)), 0);
    state = addItems(state, [item('a', 50), item('a', 99)], 100);
    expect(state.items).toHaveLength(1);
    expect(advance(state, 20_000)).toBe(state);
    expect(addItems(state, [{ ...item('empty', 99), text: '' }], 100)).toBe(state);
  });

  it('does not show an event that is too old', () => {
    const state = fill(0, item('stale', 90, 0));
    expect(advance(state, 10 * 60_000 + 1).current).toBeNull();
    expect(advance(state, 10 * 60_000).current?.id).toBe('stale');
  });

  it('a full ticker drops the items with the lowest score', () => {
    const many = Array.from({ length: 170 }, (_, i) => item(`e${i}`, i % 100));
    const state = fill(0, ...many);
    expect(state.items).toHaveLength(160);
    const dramas = state.items.map((entry) => entry.drama);
    // 170 items with the scores 0..99 and 0..69: the ten lowest left.
    expect(Math.min(...dramas)).toBe(5);
    expect(dramas.filter((d) => d === 5)).toHaveLength(2);
    expect(dramas.filter((d) => d === 4)).toHaveLength(0);
    // The order of arrival stays.
    expect(state.items[0]?.id).toBe('e5');
  });

  it('lists the newest notable events without the current one', () => {
    let state = fill(0, item('a', 20, 1), item('b', 5, 2), item('c', 40, 3), item('d', 11, 4), item('e', 90, 5));
    state = advance(state, 5);
    expect(state.current?.id).toBe('e');
    expect(recent(state, 2).map((entry) => entry.id)).toEqual(['d', 'c']);
    expect(recent(state, 9).map((entry) => entry.id)).toEqual(['d', 'c', 'a']);
  });
});
