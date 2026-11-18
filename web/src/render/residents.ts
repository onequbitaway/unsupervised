// The residents: a small figure for each one, with a marker of what it
// does. From far a resident is a dot. All of them are particles of one
// image, so thousands of residents are one draw call.

import { Particle, ParticleContainer, Rectangle, Texture } from 'pixi.js';
import { CONFIG } from '../config';
import { activityGroup } from '../state/activity';
import { visibleRect } from '../state/camera';
import type { LabelCandidate } from '../state/labels';
import { positionAt } from '../state/motion';
import type { Rect } from '../state/types';
import { findMover } from '../state/world';
import { intersects } from '../state/zones';
import { COLORS, residentColor } from './palette';
import { CELL, FIGURE_BODY, FIGURE_FEET, paintSprites, type SpriteName } from './sprites';
import { sourceOf, toCanvas } from './textures';
import { drawsDots, figureHeight, type SceneView, showsDetail } from './view';

/** Makes one texture for each sprite. All of them share one image. */
function spriteTextures(): Map<SpriteName, Texture> {
  const sheet = paintSprites();
  const source = sourceOf(toCanvas(sheet.image.data, sheet.image.width, sheet.image.height), false);
  const textures = new Map<SpriteName, Texture>();
  for (const [name, frame] of sheet.frames) {
    textures.set(name, new Texture({ source, frame: new Rectangle(frame.x, frame.y, frame.width, frame.height) }));
  }
  return textures;
}

/** Where a resident is drawn in a frame: the place of its feet, in pixels. */
export interface Drawn {
  sx: number;
  sy: number;
}

export class ResidentLayer {
  readonly container: ParticleContainer<Particle>;
  private readonly textures = spriteTextures();
  /** The colour that each particle has now. */
  private readonly tints: number[] = [];
  private bounds = new Map<number, Rect>();
  // Markers are drawn after all figures. These lists hold them for one frame.
  private markerX: number[] = [];
  private markerY: number[] = [];
  private markerTexture: Texture[] = [];
  /** Residents that the last frame drew. */
  drawn = 0;

  constructor() {
    this.container = new ParticleContainer<Particle>({
      texture: this.texture('stand'),
      dynamicProperties: { position: true, vertex: true, uvs: true, color: true, rotation: false },
    });
  }

  /** Tells the layer where the zones are. */
  setZones(zones: readonly { id: number; bounds: Rect }[]): void {
    this.bounds = new Map(zones.map((zone) => [zone.id, zone.bounds]));
  }

  private texture(name: SpriteName): Texture {
    return this.textures.get(name) ?? Texture.WHITE;
  }

  private put(index: number, texture: Texture, x: number, y: number, scaleX: number, scaleY: number, anchorY: number, tint: number): void {
    const particles = this.container.particleChildren;
    let particle = particles[index];
    if (particle === undefined) {
      particle = new Particle({ texture, anchorX: 0.5, anchorY, tint });
      particles.push(particle);
      this.tints[index] = tint;
    }
    particle.texture = texture;
    particle.x = x;
    particle.y = y;
    particle.scaleX = scaleX;
    particle.scaleY = scaleY;
    particle.anchorY = anchorY;
    if (this.tints[index] !== tint) {
      particle.tint = tint;
      this.tints[index] = tint;
    }
  }

  /**
   * Draws the residents of the zones on the screen. When `candidates` is
   * given, the layer adds every drawn resident to it, for the choice of
   * the names. `speakers` are the residents with a speech bubble.
   */
  draw(view: SceneView, speakers: ReadonlyMap<number, unknown>, candidates: LabelCandidate[] | null): void {
    const { camera, size, state, now, selected } = view;
    const scale = camera.scale;
    const halfWidth = size.width / 2;
    const halfHeight = size.height / 2;
    const dots = drawsDots(view.level, scale);
    const height = figureHeight(scale);
    const figureScale = height / (CELL * FIGURE_BODY);
    const dotScale = Math.min(CONFIG.render.dotMaxPx, Math.max(CONFIG.render.dotMinPx, scale * 1.4)) / (CELL * 0.8);
    const markerPx = Math.min(CONFIG.render.markerMaxPx, Math.max(CONFIG.render.markerMinPx, scale * 1.05));
    const markerScale = markerPx / CELL;
    const withMarkers = showsDetail(view.level, scale);
    // Two tiles more than the screen, so that a figure at the border is whole.
    const seen = visibleRect(camera, size);
    const x0 = seen.x0 - 2;
    const x1 = seen.x1 + 2;
    const y0 = seen.y0 - 2;
    const y1 = seen.y1 + 2;
    const stand = this.texture('stand');
    const walkA = this.texture('walkA');
    const walkB = this.texture('walkB');
    const dot = this.texture('dot');
    const step = Math.floor(now / CONFIG.render.stepMs);

    let count = 0;
    let markers = 0;
    let drawn = 0;

    // The ring of the selected resident is below every figure.
    const chosen = selected === null ? null : findMover(state, selected);
    if (chosen) {
      const at = positionAt(chosen.mover, now);
      const ringScale = (dots ? 16 : Math.max(18, height * 0.75)) / CELL;
      this.put(count++, this.texture('ring'), (at.x + 0.5 - camera.x) * scale + halfWidth, (at.y + 0.5 - camera.y) * scale + halfHeight, ringScale, dots ? ringScale : ringScale * 0.55, 0.5, COLORS.selection);
    }

    for (const zone of state.zones.values()) {
      const bounds = this.bounds.get(zone.id);
      if (bounds && !intersects(bounds, { x0, y0, x1, y1 })) continue;
      for (const mover of zone.residents.values()) {
        // Where the resident is on its way, as `positionAt` gives it.
        let t = mover.dur <= 0 ? 1 : (now - mover.t0) / mover.dur;
        if (t < 0) t = 0;
        const moving = t < 1 && (mover.fromX !== mover.x || mover.fromY !== mover.y);
        if (t > 1) t = 1;
        const x = mover.fromX + (mover.x - mover.fromX) * t;
        const y = mover.fromY + (mover.y - mover.fromY) * t;
        if (x < x0 || x > x1 || y < y0 || y > y1) continue;
        const sx = (x + 0.5 - camera.x) * scale + halfWidth;
        const sy = (y + 0.5 - camera.y) * scale + halfHeight;
        const tint = residentColor(mover.id);
        drawn += 1;
        if (dots) {
          this.put(count++, dot, sx, sy, dotScale, dotScale, 0.5, tint);
        } else {
          // A walking figure changes its pose with every step, and looks where it goes.
          const pose = moving ? ((step + mover.id) % 2 === 0 ? walkA : walkB) : stand;
          const facing = moving && mover.x < mover.fromX ? -1 : 1;
          this.put(count++, pose, sx, sy, figureScale * facing, figureScale, FIGURE_FEET, tint);
        }
        if (withMarkers) {
          const group = moving && mover.activity === null ? 'move' : activityGroup(mover.activity);
          if (group !== 'idle') {
            this.markerX[markers] = sx + height * 0.3;
            this.markerY[markers] = sy - height * 0.98;
            this.markerTexture[markers] = this.texture(`marker.${group}`);
            markers += 1;
          }
        }
        if (candidates !== null) {
          const priority = mover.id === selected ? 3 : speakers.has(mover.id) ? 2 : mover.activity !== null ? 1 : 0;
          candidates.push({ id: mover.id, sx, sy, priority });
        }
      }
    }

    for (let i = 0; i < markers; i++) {
      this.put(count++, this.markerTexture[i] ?? stand, this.markerX[i] ?? 0, this.markerY[i] ?? 0, markerScale, markerScale, 0.5, 0xffffff);
    }

    const particles = this.container.particleChildren;
    if (particles.length !== count) {
      particles.length = count;
      this.tints.length = count;
    }
    // The renderer must count the particles again, and send all of them.
    this.container.update();
    this.drawn = drawn;
  }
}
