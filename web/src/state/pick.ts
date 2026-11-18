// Which resident a click selects.

import { positionAt } from './motion';
import type { ViewerState } from './world';

/**
 * Returns the id of the resident that is drawn nearest to a point of the
 * world, when it is within `radius` tiles. A resident stands in the middle
 * of its tile.
 */
export function pickResident(
  state: ViewerState,
  x: number,
  y: number,
  radius: number,
  now: number,
): number | null {
  let best: number | null = null;
  let bestDistance = radius * radius;
  for (const zone of state.zones.values()) {
    for (const mover of zone.residents.values()) {
      const at = positionAt(mover, now);
      const d = (at.x + 0.5 - x) ** 2 + (at.y + 0.5 - y) ** 2;
      if (d <= bestDistance) {
        // Of two residents on the same spot the lower id wins, always.
        if (d === bestDistance && best !== null && mover.id > best) continue;
        best = mover.id;
        bestDistance = d;
      }
    }
  }
  return best;
}
