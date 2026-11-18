// How the camera of a broadcast goes to a shot of the director: where it
// must look, and whether it moves there or cuts through black.

import type { Camera, Fits, Size } from './camera';
import type { Layout, Shot, Zoom } from './types';
import { centre } from './zones';

/** Where the camera of a shot looks. */
export type ShotTarget =
  /** Follow a resident. `x` and `y` are where it was when the shot started, if known. */
  | { kind: 'resident'; id: number; x: number | null; y: number | null; scale: number }
  /** Look at a point. */
  | { kind: 'point'; x: number; y: number; scale: number }
  /** Show one city. */
  | { kind: 'city'; city: number; x: number; y: number; scale: number }
  /** Show the whole country. */
  | { kind: 'country' };

/** Returns the scale of a zoom level of the director, in pixels for one tile. */
export function zoomScale(zoom: Zoom, fits: Fits, tune: { closeZoom: number; streetZoom: number }): number {
  switch (zoom) {
    case 'close':
      return fits.street * tune.closeZoom;
    case 'street':
      return fits.street * tune.streetZoom;
    case 'city':
      return fits.city;
    case 'country':
      return fits.country;
  }
}

/** Returns where the camera must look for a shot. The middle of a tile is half a tile from its corner. */
export function shotTarget(
  shot: Shot,
  layout: Layout,
  fits: Fits,
  tune: { closeZoom: number; streetZoom: number },
): ShotTarget {
  const scale = zoomScale(shot.zoom, fits, tune);
  const at = shot.pos ? { x: shot.pos.x + 0.5, y: shot.pos.y + 0.5 } : null;
  if (shot.focus.type === 'resident') {
    return { kind: 'resident', id: shot.focus.id, x: at?.x ?? null, y: at?.y ?? null, scale };
  }
  if (shot.focus.type === 'place') {
    return { kind: 'point', x: shot.focus.pos.x + 0.5, y: shot.focus.pos.y + 0.5, scale };
  }
  const city = shot.city === undefined ? undefined : layout.cities.find((known) => known.id === shot.city);
  if (city) return { kind: 'city', city: city.id, ...centre(city.bounds), scale: fits.city };
  // A wide view of a district that has no city in the layout.
  const zone = shot.zone === undefined ? undefined : layout.zones.find((known) => known.id === shot.zone);
  if (zone) return { kind: 'point', ...centre(zone.bounds), scale: fits.street };
  return { kind: 'country' };
}

/**
 * Returns true when the camera must cut through black to a new target: the
 * target is more than `screens` screens away, measured at the nearer of
 * the two zoom levels. A shorter way is a smooth move.
 */
export function needsCut(from: Camera, to: Camera, size: Size, screens: number): boolean {
  const scale = Math.max(from.scale, to.scale);
  const dx = (Math.abs(to.x - from.x) * scale) / Math.max(1, size.width);
  const dy = (Math.abs(to.y - from.y) * scale) / Math.max(1, size.height);
  return Math.max(dx, dy) > screens;
}

/**
 * A cut through black: the picture fades out, the camera jumps, the
 * picture stays black until the new place is ready, and fades in.
 */
export interface Fade {
  phase: 'idle' | 'out' | 'hold' | 'in';
  /** When the phase started, in milliseconds. */
  since: number;
}

export const NO_FADE: Fade = { phase: 'idle', since: 0 };

/** Starts a cut. A cut that already fades out goes on. */
export function startFade(fade: Fade, now: number): Fade {
  return fade.phase === 'out' ? fade : { phase: 'out', since: now };
}

export interface FadeTimes {
  /** Time of each half of a cut: to black, and back. */
  fadeMs: number;
  /** Longest time that the picture stays black while the new place loads. */
  holdMaxMs: number;
}

/**
 * Moves a cut on. `black` is how dark the picture is, 0 to 1. `jump` is
 * true one time, when the picture is black: the camera goes to its target
 * then. `ready` says that the new place can be shown; the picture stays
 * black until then, but no longer than `holdMaxMs`.
 */
export function stepFade(
  fade: Fade,
  now: number,
  times: FadeTimes,
  ready: boolean,
): { fade: Fade; black: number; jump: boolean } {
  const part = times.fadeMs <= 0 ? 1 : Math.min(1, Math.max(0, (now - fade.since) / times.fadeMs));
  switch (fade.phase) {
    case 'idle':
      return { fade, black: 0, jump: false };
    case 'out':
      if (part >= 1) return { fade: { phase: 'hold', since: now }, black: 1, jump: true };
      return { fade, black: part, jump: false };
    case 'hold':
      if (ready || now - fade.since >= times.holdMaxMs) return { fade: { phase: 'in', since: now }, black: 1, jump: false };
      return { fade, black: 1, jump: false };
    case 'in':
      if (part >= 1) return { fade: NO_FADE, black: 0, jump: false };
      return { fade, black: 1 - part, jump: false };
  }
}
