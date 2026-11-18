// Movement between ticks. The server sends one position per tick. The
// viewer shows a resident on the way from where it was drawn to the new
// position, so the motion is smooth at any frame rate.

import { CONFIG } from '../config';

/** A move from one place to another that started at `t0` and lasts `dur`. */
export interface Motion {
  /** Where the resident was drawn when the move started, in tiles. */
  fromX: number;
  fromY: number;
  /** The position that the server gave. */
  x: number;
  y: number;
  /** Start of the move, in milliseconds. */
  t0: number;
  /** Length of the move in milliseconds. 0: the resident is there at once. */
  dur: number;
}

export interface Point {
  x: number;
  y: number;
}

/** Returns how far a move is at time `now`: 0 at the start, 1 at the end. */
export function progress(m: Motion, now: number): number {
  if (m.dur <= 0) return 1;
  const t = (now - m.t0) / m.dur;
  return t <= 0 ? 0 : t >= 1 ? 1 : t;
}

/** Returns where to draw a resident at time `now`. */
export function positionAt(m: Motion, now: number): Point {
  const t = progress(m, now);
  return { x: m.fromX + (m.x - m.fromX) * t, y: m.fromY + (m.y - m.fromY) * t };
}

/** Returns true while the resident is on the way. */
export function isMoving(m: Motion, now: number): boolean {
  return m.dur > 0 && now < m.t0 + m.dur && (m.fromX !== m.x || m.fromY !== m.y);
}

/** Makes a resident that stands at a tile. */
export function standing(x: number, y: number, now: number): Motion {
  return { fromX: x, fromY: y, x, y, t0: now, dur: 0 };
}

/**
 * Starts a move to a new position. The move starts where the resident is
 * drawn now, so a new position in the middle of a move bends the path and
 * does not jump. A move over a long distance is a jump.
 */
export function moveTo(m: Motion, x: number, y: number, now: number, tickMs: number): Motion {
  const from = positionAt(m, now);
  const distance = Math.max(Math.abs(x - from.x), Math.abs(y - from.y));
  if (distance > CONFIG.motion.snapTiles) return standing(x, y, now);
  return { fromX: from.x, fromY: from.y, x, y, t0: now, dur: tickMs * CONFIG.motion.glideTicks };
}

/**
 * Returns the new estimate of the tick length after a frame came `dt`
 * milliseconds after the frame before it. A time outside the limits (a
 * pause, a burst after a slow network) does not change the estimate.
 */
export function measureTick(tickMs: number, dt: number): number {
  const { minTickMs, maxTickMs, tickSmoothing } = CONFIG.motion;
  if (!(dt >= minTickMs && dt <= maxTickMs)) return tickMs;
  return tickMs + (dt - tickMs) * tickSmoothing;
}
