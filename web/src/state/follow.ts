// Following one resident: where the camera must look.

import { positionAt } from './motion';
import type { Pos } from './types';
import { findMover, type ViewerState } from './world';

/** Where to look for a followed resident. */
export interface FollowTarget {
  /** The middle of the tile of the resident, in tiles. */
  x: number;
  y: number;
  /** True when the feed shows the resident now. */
  live: boolean;
}

/**
 * Returns where the camera must look to follow a resident. While the feed
 * shows the resident, that is the place where it is drawn, also in the
 * middle of a move and in any zone. Else it is the last place that the
 * profile gave: the camera goes there, the viewer subscribes to the zones
 * around it, and the feed shows the resident again. Without a known place
 * the answer is null.
 */
export function followTarget(
  state: ViewerState,
  resident: number,
  lastKnown: Pos | null,
  now: number,
): FollowTarget | null {
  const found = findMover(state, resident);
  if (found) {
    const at = positionAt(found.mover, now);
    return { x: at.x + 0.5, y: at.y + 0.5, live: true };
  }
  if (lastKnown) return { x: lastKnown.x + 0.5, y: lastKnown.y + 0.5, live: false };
  return null;
}
