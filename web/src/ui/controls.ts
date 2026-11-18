// The controls: zoom, and the three levels. `?broadcast=1` hides them.

import { CONFIG } from '../config';
import type { Level } from '../state/camera';
import { button, el } from './dom';

export interface ControlHandlers {
  /** Zoom by a factor around the middle of the screen. */
  zoom(factor: number): void;
  /** Go to a level. */
  level(level: Level): void;
}

const LEVELS: readonly { level: Level; label: string; hint: string }[] = [
  { level: 'country', label: 'Country', hint: 'Show the whole country (key 1)' },
  { level: 'city', label: 'City', hint: 'Show one city (key 2)' },
  { level: 'street', label: 'Street', hint: 'Show one district at street level (key 3)' },
];

export class Controls {
  readonly root = el('nav', 'controls');
  private readonly levels = new Map<Level, HTMLButtonElement>();

  constructor(handlers: ControlHandlers) {
    const levels = el('div', 'controls-levels');
    for (const { level, label, hint } of LEVELS) {
      const node = button(label, hint, () => handlers.level(level));
      this.levels.set(level, node);
      levels.append(node);
    }
    const zoom = el('div', 'controls-zoom');
    zoom.append(
      button('+', 'Zoom in (key +)', () => handlers.zoom(CONFIG.view.zoomStep)),
      button('−', 'Zoom out (key -)', () => handlers.zoom(1 / CONFIG.view.zoomStep)),
    );
    const hint = el('p', 'controls-hint', 'Drag to move. Scroll to zoom. Click a resident to open its panel.');
    this.root.append(levels, zoom, hint);
  }

  /** Marks the level that the camera shows. */
  setLevel(level: Level): void {
    for (const [key, node] of this.levels) {
      const active = key === level;
      if (node.classList.contains('active') !== active) node.classList.toggle('active', active);
      node.setAttribute('aria-pressed', String(active));
    }
  }
}
