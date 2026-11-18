// The land: one rectangle for each zone, coloured by its number of
// residents. At the country level this is the population heat map. At the
// other levels it is the ground of a district whose map is not loaded yet.

import { Container, Sprite, Texture } from 'pixi.js';
import { CONFIG } from '../config';
import type { Level } from '../state/camera';
import type { Layout } from '../state/types';
import { COLORS, heatColor } from './palette';

export class LandLayer {
  readonly container = new Container();
  private readonly zones = new Map<number, Sprite>();
  private populations: ReadonlyMap<number, number> | null = null;
  private level: Level | null = null;

  constructor(layout: Layout) {
    for (const zone of layout.zones) {
      const { x0, y0, x1, y1 } = zone.bounds;
      const gap = Math.min(x1 - x0, y1 - y0) * CONFIG.view.heatGap;
      const sprite = new Sprite(Texture.WHITE);
      sprite.position.set(x0 + gap, y0 + gap);
      sprite.width = x1 - x0 - gap * 2;
      sprite.height = y1 - y0 - gap * 2;
      sprite.tint = COLORS.land;
      this.zones.set(zone.id, sprite);
      this.container.addChild(sprite);
    }
  }

  /** Colours the zones. It does the work only when something changed. */
  update(level: Level, populations: ReadonlyMap<number, number>): void {
    if (level === this.level && populations === this.populations) return;
    this.level = level;
    this.populations = populations;
    let most: number = CONFIG.view.heatFloor;
    for (const count of populations.values()) most = Math.max(most, count);
    for (const [id, sprite] of this.zones) {
      sprite.tint = level === 'country' ? heatColor(populations.get(id) ?? 0, most) : COLORS.land;
    }
  }
}
