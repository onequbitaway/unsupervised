// The key of the population heat map: which colour means how many residents.

import { CONFIG } from '../config';
import { heatColor } from '../render/palette';
import { el, setShown, setText } from './dom';

/** Writes a colour as `#rrggbb`. */
export function cssColor(color: number): string {
  return `#${(color & 0xffffff).toString(16).padStart(6, '0')}`;
}

export class Legend {
  readonly root = el('div', 'legend');
  private readonly bar = el('div', 'legend-bar');
  private readonly most = el('span', 'legend-most');
  private shownMost = -1;

  constructor() {
    const scale = el('div', 'legend-scale');
    scale.append(el('span', '', '0'), this.bar, this.most);
    this.root.append(el('div', 'legend-title', 'Residents per district'), scale);
    this.root.hidden = true;
  }

  /** Shows the key for the fullest zone, at the country level only. */
  update(visible: boolean, populations: ReadonlyMap<number, number>): void {
    setShown(this.root, visible);
    if (!visible) return;
    let most: number = CONFIG.view.heatFloor;
    for (const count of populations.values()) most = Math.max(most, count);
    if (most === this.shownMost) return;
    this.shownMost = most;
    // The same colours as the map, for the same numbers.
    const stops = Array.from({ length: 9 }, (_, i) => cssColor(heatColor(Math.max(1, (most * i * i) / 64), most)));
    this.bar.style.background = `linear-gradient(to right, ${stops.join(', ')})`;
    setText(this.most, most.toLocaleString('en-US'));
  }
}
