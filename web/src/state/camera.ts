// The camera as numbers: where it looks, how far it is, and what that
// means for the level of detail. No drawing here.

import { CONFIG } from '../config';
import type { Layout, Rect } from './types';

/** The camera looks at a point of the world, with `scale` pixels for one tile. */
export interface Camera {
  x: number;
  y: number;
  scale: number;
}

export interface Size {
  width: number;
  height: number;
}

/** The three zoom levels of the viewer. */
export type Level = 'country' | 'city' | 'street';

/** The scales at which the country, the largest city and one district fill the screen. */
export interface Fits {
  country: number;
  city: number;
  street: number;
}

export function worldToScreen(cam: Camera, view: Size, x: number, y: number): { x: number; y: number } {
  return { x: (x - cam.x) * cam.scale + view.width / 2, y: (y - cam.y) * cam.scale + view.height / 2 };
}

export function screenToWorld(cam: Camera, view: Size, sx: number, sy: number): { x: number; y: number } {
  return { x: (sx - view.width / 2) / cam.scale + cam.x, y: (sy - view.height / 2) / cam.scale + cam.y };
}

/** Returns the part of the world that the screen shows. */
export function visibleRect(cam: Camera, view: Size): Rect {
  const halfW = view.width / 2 / cam.scale;
  const halfH = view.height / 2 / cam.scale;
  return { x0: cam.x - halfW, y0: cam.y - halfH, x1: cam.x + halfW, y1: cam.y + halfH };
}

/** Returns the camera that shows a rectangle on a part `fill` of the screen. */
export function fitRect(rect: Rect, view: Size, fill: number): Camera {
  const width = Math.max(1, rect.x1 - rect.x0);
  const height = Math.max(1, rect.y1 - rect.y0);
  const scale = Math.min(view.width / width, view.height / height) * fill;
  return { x: (rect.x0 + rect.x1) / 2, y: (rect.y0 + rect.y1) / 2, scale };
}

/** Returns the rectangle that holds every city. */
export function worldBounds(layout: Layout): Rect {
  const all = layout.cities.length > 0 ? layout.cities.map((c) => c.bounds) : layout.zones.map((z) => z.bounds);
  if (all.length === 0) return { x0: 0, y0: 0, x1: 64, y1: 64 };
  return {
    x0: Math.min(...all.map((r) => r.x0)),
    y0: Math.min(...all.map((r) => r.y0)),
    x1: Math.max(...all.map((r) => r.x1)),
    y1: Math.max(...all.map((r) => r.y1)),
  };
}

/** Returns the scales of the three levels for a world and a screen. */
export function fits(layout: Layout, view: Size): Fits {
  const country = fitRect(worldBounds(layout), view, CONFIG.view.countryFill).scale;
  const citySide = Math.max(1, ...layout.cities.map((c) => Math.max(c.bounds.x1 - c.bounds.x0, c.bounds.y1 - c.bounds.y0)));
  const zoneSide = Math.max(1, ...layout.zones.map((z) => Math.max(z.bounds.x1 - z.bounds.x0, z.bounds.y1 - z.bounds.y0)));
  const small = Math.min(view.width, view.height);
  const city = Math.max(country, (small / citySide) * CONFIG.view.cityFill);
  const street = Math.max(city, (small / zoneSide) * CONFIG.view.streetFill);
  return { country, city, street };
}

/**
 * Returns the level of a scale. The border between two levels is the
 * geometric mean of their scales, so each button lands in its own level.
 */
export function levelOf(scale: number, f: Fits): Level {
  if (scale < Math.sqrt(f.country * f.city)) return 'country';
  if (scale < Math.sqrt(f.city * f.street)) return 'city';
  return 'street';
}

/** Keeps a scale between the country view and the greatest zoom. */
export function clampScale(scale: number, f: Fits): number {
  return Math.min(Math.max(scale, f.country * 0.6), Math.max(CONFIG.view.maxScale, f.street));
}

/** Zooms by a factor and keeps the point under the pointer where it is. */
export function zoomAt(cam: Camera, view: Size, factor: number, sx: number, sy: number, f: Fits): Camera {
  const scale = clampScale(cam.scale * factor, f);
  const before = screenToWorld(cam, view, sx, sy);
  const next = { ...cam, scale };
  const after = screenToWorld(next, view, sx, sy);
  return { x: cam.x + before.x - after.x, y: cam.y + before.y - after.y, scale };
}

/** Keeps the point that the camera looks at inside the world. */
export function clampToWorld(cam: Camera, bounds: Rect): Camera {
  return {
    ...cam,
    x: Math.min(Math.max(cam.x, bounds.x0), bounds.x1),
    y: Math.min(Math.max(cam.y, bounds.y0), bounds.y1),
  };
}

/**
 * Keeps the picture inside a rectangle: the camera moves so that the screen
 * shows nothing outside of it. On an axis where the rectangle is smaller
 * than the screen, the camera looks at the middle of the rectangle.
 */
export function clampView(cam: Camera, view: Size, rect: Rect): Camera {
  const axis = (at: number, low: number, high: number, pixels: number): number => {
    const half = pixels / 2 / cam.scale;
    if (high - low <= half * 2) return (low + high) / 2;
    return Math.min(Math.max(at, low + half), high - half);
  };
  return {
    ...cam,
    x: axis(cam.x, rect.x0, rect.x1, view.width),
    y: axis(cam.y, rect.y0, rect.y1, view.height),
  };
}

/**
 * Moves a camera towards a target. After `halfLifeMs` half of the way is
 * left, whatever the frame rate. The scale moves in equal ratios.
 */
export function approach(cam: Camera, target: Camera, dtMs: number, halfLifeMs: number): Camera {
  const keep = halfLifeMs <= 0 ? 0 : 0.5 ** (dtMs / halfLifeMs);
  const scale = target.scale * (cam.scale / target.scale) ** keep;
  const next = {
    x: target.x + (cam.x - target.x) * keep,
    y: target.y + (cam.y - target.y) * keep,
    scale,
  };
  const close =
    Math.abs(next.x - target.x) * target.scale < 0.25 &&
    Math.abs(next.y - target.y) * target.scale < 0.25 &&
    Math.abs(next.scale / target.scale - 1) < 0.002;
  return close ? { ...target } : next;
}
