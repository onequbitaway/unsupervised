import { describe, expect, it } from 'vitest';
import { needsCut, NO_FADE, shotTarget, startFade, stepFade, zoomScale } from './cut';
import type { Layout, Shot } from './types';

const clock = { year: 1, day: 1, hour: 0, minute: 0 };
const layout: Layout = {
  frame: 0,
  clock,
  cities: [{ id: 0, name: 'A', bounds: { x0: 0, y0: 0, x1: 128, y1: 128 }, zones: [0] }],
  zones: [{ id: 0, city: 0, name: 'A 0-0', bounds: { x0: 0, y0: 0, x1: 64, y1: 64 }, residents: 0 }],
};
const fits = { country: 2, city: 6, street: 12 };
const tune = { closeZoom: 2, streetZoom: 1.5 };

function shot(extra: Partial<Shot>): Shot {
  return { seq: 1, frame: 1, clock, focus: { type: 'none' }, zoom: 'country', caption: '', breaking: false, drama: 0, ...extra };
}

describe('shotTarget', () => {
  it('gives each zoom level of the director a scale', () => {
    expect(['country', 'city', 'street', 'close'].map((zoom) => zoomScale(zoom as Shot['zoom'], fits, tune))).toEqual([
      2, 6, 18, 24,
    ]);
  });

  it('follows the resident of a shot, from the place where it was', () => {
    const target = shotTarget(shot({ focus: { type: 'resident', id: 7 }, zoom: 'close', pos: { x: 10, y: 20 } }), layout, fits, tune);
    expect(target).toEqual({ kind: 'resident', id: 7, x: 10.5, y: 20.5, scale: 24 });
    const unknown = shotTarget(shot({ focus: { type: 'resident', id: 7 }, zoom: 'street' }), layout, fits, tune);
    expect(unknown).toEqual({ kind: 'resident', id: 7, x: null, y: null, scale: 18 });
  });

  it('looks at the middle of the tile of a place', () => {
    const target = shotTarget(shot({ focus: { type: 'place', pos: { x: 4, y: 9 } }, zoom: 'street' }), layout, fits, tune);
    expect(target).toEqual({ kind: 'point', x: 4.5, y: 9.5, scale: 18 });
  });

  it('shows the city of a wide view, or the country', () => {
    expect(shotTarget(shot({ city: 0, zoom: 'city' }), layout, fits, tune)).toEqual({
      kind: 'city',
      city: 0,
      x: 64,
      y: 64,
      scale: 6,
    });
    // A city that the layout does not have: the district, then the country.
    expect(shotTarget(shot({ city: 9, zone: 0 }), layout, fits, tune)).toEqual({ kind: 'point', x: 32, y: 32, scale: 12 });
    expect(shotTarget(shot({ city: 9 }), layout, fits, tune)).toEqual({ kind: 'country' });
    expect(shotTarget(shot({}), layout, fits, tune)).toEqual({ kind: 'country' });
  });
});

describe('needsCut', () => {
  const size = { width: 1000, height: 500 };

  it('moves to a near target and cuts to a far one', () => {
    const from = { x: 100, y: 100, scale: 10 };
    // 100 tiles at 10 pixels: one screen wide.
    expect(needsCut(from, { x: 200, y: 100, scale: 10 }, size, 1.5)).toBe(false);
    expect(needsCut(from, { x: 260, y: 100, scale: 10 }, size, 1.5)).toBe(true);
    // The height of the screen is smaller: 80 tiles are 1.6 screens.
    expect(needsCut(from, { x: 100, y: 180, scale: 10 }, size, 1.5)).toBe(true);
  });

  it('measures the way at the nearer zoom level', () => {
    const wide = { x: 100, y: 100, scale: 1 };
    // 60 tiles are nothing for the country view, and much for the close view.
    expect(needsCut(wide, { x: 160, y: 100, scale: 1 }, size, 1.5)).toBe(false);
    expect(needsCut(wide, { x: 160, y: 100, scale: 40 }, size, 1.5)).toBe(true);
    expect(needsCut({ x: 160, y: 100, scale: 40 }, wide, size, 1.5)).toBe(true);
    // The same place at another zoom level is a move.
    expect(needsCut(wide, { x: 100, y: 100, scale: 40 }, size, 1.5)).toBe(false);
  });
});

describe('a cut through black', () => {
  const times = { fadeMs: 400, holdMaxMs: 1_000 };

  it('fades out, jumps one time, waits for the new place, and fades in', () => {
    let fade = startFade(NO_FADE, 1_000);
    expect(fade).toEqual({ phase: 'out', since: 1_000 });
    let step = stepFade(fade, 1_100, times, false);
    expect([step.black, step.jump, step.fade.phase]).toEqual([0.25, false, 'out']);
    step = stepFade(fade, 1_400, times, false);
    expect([step.black, step.jump, step.fade]).toEqual([1, true, { phase: 'hold', since: 1_400 }]);
    fade = step.fade;
    // The new place is not ready: the picture stays black, without a second jump.
    step = stepFade(fade, 1_700, times, false);
    expect([step.black, step.jump, step.fade.phase]).toEqual([1, false, 'hold']);
    step = stepFade(fade, 1_800, times, true);
    expect([step.black, step.jump, step.fade]).toEqual([1, false, { phase: 'in', since: 1_800 }]);
    fade = step.fade;
    step = stepFade(fade, 1_900, times, true);
    expect([step.black, step.jump, step.fade.phase]).toEqual([0.75, false, 'in']);
    step = stepFade(fade, 2_200, times, true);
    expect([step.black, step.jump, step.fade]).toEqual([0, false, NO_FADE]);
  });

  it('does not stay black for ever when the new place never gets ready', () => {
    const hold = { phase: 'hold' as const, since: 5_000 };
    expect(stepFade(hold, 5_999, times, false).fade.phase).toBe('hold');
    expect(stepFade(hold, 6_000, times, false).fade).toEqual({ phase: 'in', since: 6_000 });
  });

  it('does nothing without a cut, and keeps a cut that already fades out', () => {
    expect(stepFade(NO_FADE, 5_000, times, true)).toEqual({ fade: NO_FADE, black: 0, jump: false });
    const running = { phase: 'out' as const, since: 1_000 };
    expect(startFade(running, 1_200)).toBe(running);
    // A new cut while the picture fades in starts to fade out again.
    expect(startFade({ phase: 'in', since: 1_000 }, 1_200)).toEqual({ phase: 'out', since: 1_200 });
    // Without a time for the fade the jump is at once.
    expect(stepFade({ phase: 'out', since: 0 }, 0, { fadeMs: 0, holdMaxMs: 0 }, true).jump).toBe(true);
  });
});
