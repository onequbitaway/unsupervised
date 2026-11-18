// The ground of the districts: one painted image for each district whose
// map (tiles) the viewer has.

import { Container, Sprite, Texture } from 'pixi.js';
import { CONFIG } from '../config';
import type { ZoneMap } from '../state/types';
import { sourceOf, toCanvas } from './textures';
import { paintTiles } from './tiles';

export class GroundLayer {
  readonly container = new Container();
  private readonly districts = new Map<number, Sprite>();

  has(zone: number): boolean {
    return this.districts.has(zone);
  }

  /** Paints the tiles of a district and shows them. */
  add(map: ZoneMap): void {
    this.remove(map.zone.id);
    if (map.width <= 0 || map.height <= 0) return;
    const px = CONFIG.render.tilePx;
    const { x0, y0 } = map.zone.bounds;
    const pixels = paintTiles(map.tiles, map.width, map.height, px, x0, y0);
    const canvas = toCanvas(pixels, map.width * px, map.height * px);
    const sprite = new Sprite(new Texture({ source: sourceOf(canvas, true) }));
    sprite.position.set(x0, y0);
    // One tile is one unit of the world.
    sprite.scale.set(1 / px);
    this.districts.set(map.zone.id, sprite);
    this.container.addChild(sprite);
  }

  /** Forgets the districts that are not in `zones`, and frees their images. */
  keep(zones: ReadonlySet<number>): void {
    for (const zone of [...this.districts.keys()]) {
      if (!zones.has(zone)) this.remove(zone);
    }
  }

  private remove(zone: number): void {
    const sprite = this.districts.get(zone);
    if (sprite === undefined) return;
    this.districts.delete(zone);
    this.container.removeChild(sprite);
    sprite.destroy({ texture: true, textureSource: true });
  }
}
