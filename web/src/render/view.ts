// What the layers need to draw one frame.

import { CONFIG } from '../config';
import type { Camera, Level, Size } from '../state/camera';
import type { Layout } from '../state/types';
import type { ViewerState } from '../state/world';

export interface SceneView {
  camera: Camera;
  size: Size;
  level: Level;
  /** The time of this frame, in milliseconds on the clock of the feed state. */
  now: number;
  layout: Layout;
  state: ViewerState;
  /** The selected resident, or null. */
  selected: number | null;
}

/** Returns the factor on the label sizes for a screen: larger on a large screen. */
export function labelScale(size: Size): number {
  const { labelBaseHeight, labelScaleMin, labelScaleMax } = CONFIG.render;
  return Math.min(labelScaleMax, Math.max(labelScaleMin, size.height / labelBaseHeight));
}

/** Returns the height of a resident figure on the screen, in pixels. */
export function figureHeight(scale: number): number {
  return Math.max(CONFIG.render.minFigurePx, CONFIG.render.figureTiles * scale);
}

/**
 * Returns true when a resident is drawn as a dot: at the city level, and
 * at street level when a tile is too small for a figure.
 */
export function drawsDots(level: Level, scale: number): boolean {
  return level !== 'street' || scale < CONFIG.render.dotBelowPx;
}

/**
 * Returns true when the picture shows the detail of the street: names,
 * activity markers, speech bubbles and site labels.
 */
export function showsDetail(level: Level, scale: number): boolean {
  return level === 'street' && scale >= CONFIG.render.labelsBelowPx;
}
