// Which residents get a name on the screen. A crowd has more residents
// than the screen has room for names, so the viewer gives a limited number
// of names to the most important residents, and never two names in the
// same place.

export interface LabelCandidate {
  id: number;
  /** Where the name would be, in pixels. */
  sx: number;
  sy: number;
  /** A higher number wins. The selected resident and speakers have one. */
  priority: number;
}

export interface LabelOptions {
  /** Names at most. */
  budget: number;
  /** The centre of the screen, in pixels: nearer residents win. */
  cx: number;
  cy: number;
  /** Room that one name needs, in pixels. */
  cellWidth: number;
  cellHeight: number;
}

/**
 * Returns the ids of the residents that get a name: by priority, then by
 * distance to the centre of the screen, then by id. A resident whose name
 * would sit in the same cell as a name that was chosen before gets none.
 */
export function pickLabels(candidates: readonly LabelCandidate[], options: LabelOptions): number[] {
  const ranked = candidates
    .map((c) => ({ c, distance: (c.sx - options.cx) ** 2 + (c.sy - options.cy) ** 2 }))
    .sort((a, b) => b.c.priority - a.c.priority || a.distance - b.distance || a.c.id - b.c.id);
  const taken = new Set<string>();
  const out: number[] = [];
  for (const { c } of ranked) {
    if (out.length >= options.budget) break;
    const col = Math.floor(c.sx / options.cellWidth);
    const row = Math.floor(c.sy / options.cellHeight);
    const cell = `${col}:${row}`;
    // A name is about as wide as two cells: it also needs the cell beside it.
    const beside = `${c.sx / options.cellWidth - col < 0.5 ? col - 1 : col + 1}:${row}`;
    if (taken.has(cell) || taken.has(beside)) continue;
    taken.add(cell);
    out.push(c.id);
  }
  return out;
}
