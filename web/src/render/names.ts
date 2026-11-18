// The names of the residents on the screen. A crowd has more residents
// than room for names, so a limited number of residents get one.

import { CONFIG } from '../config';
import { type LabelCandidate, pickLabels } from '../state/labels';
import { positionAt } from '../state/motion';
import { cleanText } from '../state/text';
import { findMover } from '../state/world';
import { LabelPool, outlined } from './labels';
import { COLORS } from './palette';
import { labelScale, type SceneView, showsDetail } from './view';

export class NameLayer {
  readonly labels = new LabelPool(outlined(CONFIG.render.nameFontPx, COLORS.labelText, COLORS.labelStroke), 0.5, 0);
  private chosen: number[] = [];
  private chosenAt = -Infinity;

  /** Returns true when it is time to choose again which residents get a name. */
  due(view: SceneView): boolean {
    return showsDetail(view.level, view.camera.scale) && view.now - this.chosenAt >= CONFIG.render.nameEveryMs;
  }

  /** Chooses the residents that get a name, from those that the frame drew. */
  choose(candidates: readonly LabelCandidate[], view: SceneView): void {
    const fontSize = CONFIG.render.nameFontPx * labelScale(view.size);
    this.chosen = pickLabels(candidates, {
      budget: CONFIG.render.nameBudget,
      cx: view.size.width / 2,
      cy: view.size.height / 2,
      cellWidth: fontSize * 5,
      cellHeight: fontSize * 1.5,
    });
    this.chosenAt = view.now;
  }

  draw(view: SceneView): void {
    const { camera, size, state, now } = view;
    this.labels.setFontSize(Math.round(CONFIG.render.nameFontPx * labelScale(size)));
    this.labels.begin();
    if (showsDetail(view.level, camera.scale)) {
      for (const id of this.chosen) {
        const found = findMover(state, id);
        if (!found) continue;
        const at = positionAt(found.mover, now);
        const sx = (at.x + 0.5 - camera.x) * camera.scale + size.width / 2;
        const sy = (at.y + 0.5 - camera.y) * camera.scale + size.height / 2;
        // The name of a resident is untrusted. It is cleaned, cut and drawn as text.
        const name = cleanText(found.mover.name, CONFIG.text.nameMax) || `#${id}`;
        this.labels.place(id, name, sx, sy + 3);
      }
    }
    this.labels.end();
  }
}
