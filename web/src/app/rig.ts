// The camera of the viewer: where it is, where it goes, and the moves
// that the controls ask for.

import { CONFIG } from '../config';
import {
  approach,
  type Camera,
  clampScale,
  clampToWorld,
  type Fits,
  fitRect,
  fits,
  type Level,
  levelOf,
  type Size,
  worldBounds,
  zoomAt,
} from '../state/camera';
import type { CityView, Layout, Rect, ZoneView } from '../state/types';
import { centre } from '../state/zones';

export class CameraRig {
  /** Where the camera is now. */
  camera: Camera;
  /** Where the camera goes. */
  target: Camera;
  size: Size;
  private scales: Fits;
  private readonly bounds: Rect;

  constructor(
    private readonly layout: Layout,
    size: Size,
  ) {
    this.size = size;
    this.bounds = worldBounds(layout);
    this.scales = fits(layout, size);
    this.camera = fitRect(this.bounds, size, CONFIG.view.countryFill);
    this.target = { ...this.camera };
  }

  /** The level of what the screen shows now. */
  get level(): Level {
    return levelOf(this.camera.scale, this.scales);
  }

  /** The level of what the screen will show when the camera arrives. */
  get targetLevel(): Level {
    return levelOf(this.target.scale, this.scales);
  }

  /** Returns true when the camera is where it goes. */
  get arrived(): boolean {
    return this.camera.x === this.target.x && this.camera.y === this.target.y && this.camera.scale === this.target.scale;
  }

  /**
   * Returns true when the camera is less than one tile from where it goes.
   * A camera that follows a walking resident is near, but never arrives.
   */
  get near(): boolean {
    const ratio = this.camera.scale / this.target.scale;
    return Math.abs(this.camera.x - this.target.x) < 1 && Math.abs(this.camera.y - this.target.y) < 1 && ratio > 0.98 && ratio < 1.02;
  }

  /** The screen has another size. */
  resize(size: Size): void {
    this.size = size;
    this.scales = fits(this.layout, size);
    this.target = { ...this.target, scale: clampScale(this.target.scale, this.scales) };
  }

  /** Moves the camera towards its target. `dtMs` is the time since the last call. */
  step(dtMs: number): void {
    this.camera = approach(this.camera, this.target, dtMs, CONFIG.view.easeHalfLifeMs);
  }

  /** The camera is at its target at once. */
  arrive(): void {
    this.camera = { ...this.target };
  }

  private aim(next: Camera): void {
    this.target = clampToWorld({ ...next, scale: clampScale(next.scale, this.scales) }, this.bounds);
  }

  /** Moves the picture by pixels, at once: the pointer holds it. */
  drag(dx: number, dy: number): void {
    const scale = this.camera.scale;
    this.aim({ ...this.target, x: this.target.x - dx / scale, y: this.target.y - dy / scale });
    this.camera = { ...this.camera, x: this.target.x, y: this.target.y };
  }

  /** Moves by a part of the screen, for the arrow keys. */
  nudge(dirX: number, dirY: number): void {
    const step = (Math.min(this.size.width, this.size.height) * CONFIG.view.panStep) / this.target.scale;
    this.aim({ ...this.target, x: this.target.x + dirX * step, y: this.target.y + dirY * step });
  }

  /** Zooms around a point of the screen. */
  zoom(factor: number, sx: number, sy: number): void {
    this.aim(zoomAt(this.target, this.size, factor, sx, sy, this.scales));
  }

  /** Zooms around the middle of the screen. */
  zoomCentre(factor: number): void {
    this.zoom(factor, this.size.width / 2, this.size.height / 2);
  }

  /** Looks at a point. The scale stays when `scale` is absent. */
  lookAt(x: number, y: number, scale?: number): void {
    this.aim({ x, y, scale: scale ?? this.target.scale });
  }

  showCountry(): void {
    this.aim(fitRect(this.bounds, this.size, CONFIG.view.countryFill));
  }

  showCity(city: CityView): void {
    const at = centre(city.bounds);
    this.aim({ ...at, scale: this.scales.city });
  }

  showZone(zone: ZoneView): void {
    const at = centre(zone.bounds);
    this.aim({ ...at, scale: this.scales.street });
  }

  /** The scale for following one resident. */
  get followScale(): number {
    return this.scales.street * CONFIG.view.followZoom;
  }
}
