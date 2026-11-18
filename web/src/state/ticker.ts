// The news ticker as data. It remembers events, shows one at a time in the
// main line, and prefers events with a high drama score.

import { CONFIG } from '../config';

export interface TickerItem {
  /** Id of the event in the feed. */
  id: string;
  drama: number;
  /** The line, already safe to show. */
  text: string;
  /** When the viewer got the event, in milliseconds. */
  at: number;
  /** The zone of the event, if it has a place. */
  zone: number | null;
  /** The resident that caused the event, if any. */
  actor: number | null;
  /** True when the main line showed the item. */
  shown: boolean;
}

export interface TickerState {
  /** The remembered events, oldest first. */
  items: readonly TickerItem[];
  /** The item in the main line. */
  current: TickerItem | null;
  /** When the current item went into the main line. */
  since: number;
}

export function emptyTicker(): TickerState {
  return { items: [], current: null, since: 0 };
}

/**
 * Returns how much an item deserves the main line now: its drama score
 * minus a penalty for its age.
 */
export function score(item: TickerItem, now: number): number {
  const minutes = Math.max(0, now - item.at) / 60_000;
  return item.drama - minutes * CONFIG.ticker.agePenaltyPerMinute;
}

function isHeadline(item: TickerItem): boolean {
  return item.drama >= CONFIG.ticker.headlineDrama;
}

/**
 * Adds events. An event that the ticker already has is ignored. When the
 * ticker is full, the items with the lowest score leave.
 */
export function addItems(state: TickerState, incoming: readonly Omit<TickerItem, 'shown'>[], now: number): TickerState {
  const known = new Set(state.items.map((item) => item.id));
  const fresh: TickerItem[] = [];
  for (const item of incoming) {
    if (known.has(item.id) || item.text === '') continue;
    known.add(item.id);
    fresh.push({ ...item, shown: false });
  }
  if (fresh.length === 0) return state;
  let items = [...state.items, ...fresh];
  const max = CONFIG.ticker.maxItems;
  if (items.length > max) {
    const keep = new Set(
      [...items].sort((a, b) => score(b, now) - score(a, now)).slice(0, max).map((item) => item.id),
    );
    items = items.filter((item) => keep.has(item.id));
  }
  return { ...state, items };
}

/** Returns the best item that the main line did not show yet. */
function best(items: readonly TickerItem[], now: number): TickerItem | null {
  let found: TickerItem | null = null;
  for (const item of items) {
    if (item.shown || now - item.at > CONFIG.ticker.maxAgeMs) continue;
    if (found === null || score(item, now) > score(found, now)) found = item;
  }
  return found;
}

/**
 * Moves the main line on. The current item stays for its time. A headline
 * that waits takes the line at once from an item that is not a headline.
 * When nothing waits, the current item stays.
 */
export function advance(state: TickerState, now: number): TickerState {
  const next = best(state.items, now);
  if (next === null) return state;
  const current = state.current;
  if (current !== null) {
    const hold = isHeadline(current) ? CONFIG.ticker.headlineMs : CONFIG.ticker.showMs;
    const interrupts = isHeadline(next) && !isHeadline(current);
    if (now - state.since < hold && !interrupts) return state;
  }
  const shown = { ...next, shown: true };
  return {
    items: state.items.map((item) => (item.id === next.id ? shown : item)),
    current: shown,
    since: now,
  };
}

/** Returns the newest notable items, newest first, without the current one. */
export function recent(state: TickerState, count: number): TickerItem[] {
  const out: TickerItem[] = [];
  for (let i = state.items.length - 1; i >= 0 && out.length < count; i--) {
    const item = state.items[i];
    if (item === undefined || item.id === state.current?.id) continue;
    if (item.drama >= CONFIG.ticker.notableDrama) out.push(item);
  }
  return out;
}
