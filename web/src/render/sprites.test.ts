import { describe, expect, it } from 'vitest';
import { blank, box, circle, fill, line, polygon, union, without } from './shapes';
import { CELL, FIGURE_FEET, type Frame, MARKER_GROUPS, paintSprites, type SpriteName, SPRITES } from './sprites';

const sheet = paintSprites();

function frame(name: SpriteName): Frame {
  const found = sheet.frames.get(name);
  if (!found) throw new Error(`no sprite ${name}`);
  return found;
}

/** Returns one pixel of a sprite as [red, green, blue, alpha]. */
function pixel(name: SpriteName, u: number, v: number): number[] {
  const f = frame(name);
  const at = ((f.y + v) * sheet.image.width + f.x + u) * 4;
  return Array.from(sheet.image.data.slice(at, at + 4));
}

/** Returns how much two sprites differ: the mean difference of a channel. */
function difference(a: SpriteName, b: SpriteName): number {
  let sum = 0;
  for (let v = 0; v < CELL; v++) {
    for (let u = 0; u < CELL; u++) {
      const pa = pixel(a, u, v);
      const pb = pixel(b, u, v);
      for (let c = 0; c < 4; c++) sum += Math.abs((pa[c] ?? 0) - (pb[c] ?? 0));
    }
  }
  return sum / (CELL * CELL * 4);
}

describe('shapes', () => {
  it('a distance is negative inside, zero on the edge and positive outside', () => {
    expect(circle(10, 10, 4)(10, 10)).toBe(-4);
    expect(circle(10, 10, 4)(14, 10)).toBe(0);
    expect(circle(10, 10, 4)(10, 17)).toBe(3);
    expect(box(10, 10, 4, 2)(10, 10)).toBe(-2);
    expect(box(10, 10, 4, 2)(16, 10)).toBe(2);
    expect(box(10, 10, 4, 2)(17, 14)).toBeCloseTo(Math.hypot(3, 2));
    expect(line(0, 0, 10, 0, 1)(5, 3)).toBe(2);
    expect(line(0, 0, 10, 0, 1)(13, 0)).toBe(2);
    const triangle = polygon([[0, 0], [10, 0], [0, 10]]);
    expect(triangle(2, 2)).toBe(-2);
    expect(triangle(-3, 5)).toBe(3);
    expect(union(circle(0, 0, 1), circle(10, 0, 1))(10, 0)).toBe(-1);
    expect(without(circle(0, 0, 5), circle(0, 0, 2))(0, 0)).toBe(2);
  });

  it('fill paints inside the shape only, with a smooth edge', () => {
    const image = blank(8, 8);
    fill(image, 0, 0, 8, 8, box(4, 4, 2, 2), [200, 100, 50, 1]);
    const alpha = (x: number, y: number): number => image.data[(y * 8 + x) * 4 + 3] ?? -1;
    expect(alpha(3, 3)).toBe(255);
    expect(alpha(0, 0)).toBe(0);
    expect(alpha(6, 3)).toBe(0);
    expect(Array.from(image.data.slice((3 * 8 + 3) * 4, (3 * 8 + 3) * 4 + 3))).toEqual([200, 100, 50]);
    // An edge that crosses the middle of a pixel covers half of it.
    const half = blank(4, 1);
    fill(half, 0, 0, 4, 1, box(0, 0.5, 1.5, 4), [255, 255, 255, 1]);
    expect(Array.from(half.data.filter((_, i) => i % 4 === 3))).toEqual([255, 128, 0, 0]);
  });

  it('fill blends a colour over what is there and stays inside its rectangle', () => {
    const image = blank(4, 2);
    fill(image, 0, 0, 4, 2, box(2, 1, 9, 9), [0, 0, 200, 1]);
    fill(image, 0, 0, 2, 2, box(1, 1, 9, 9), [200, 0, 0, 0.5]);
    expect(Array.from(image.data.slice(0, 4))).toEqual([100, 0, 100, 255]);
    // The second fill had a rectangle of two columns.
    expect(Array.from(image.data.slice(8, 12))).toEqual([0, 0, 200, 255]);
  });
});

describe('paintSprites', () => {
  it('paints the same image every time', () => {
    expect(paintSprites().image.data).toEqual(sheet.image.data);
  });

  it('has one frame for every sprite, and no two frames in the same place', () => {
    expect(SPRITES).toHaveLength(15);
    expect([...sheet.frames.keys()]).toEqual([...SPRITES]);
    const places = new Set(SPRITES.map((name) => `${frame(name).x}:${frame(name).y}`));
    expect(places.size).toBe(SPRITES.length);
    for (const name of SPRITES) {
      const f = frame(name);
      expect(f.x + f.width).toBeLessThanOrEqual(sheet.image.width);
      expect(f.y + f.height).toBeLessThanOrEqual(sheet.image.height);
    }
  });

  it('keeps a clear border around every sprite, so that neighbours do not mix', () => {
    for (const name of SPRITES) {
      let covered = 0;
      for (let i = 0; i < CELL; i++) {
        for (const [u, v] of [[i, 0], [i, CELL - 1], [0, i], [CELL - 1, i]] as const) {
          expect(pixel(name, u, v)[3], `${name} at ${u},${v}`).toBe(0);
        }
        for (let j = 0; j < CELL; j++) covered += (pixel(name, i, j)[3] ?? 0) > 0 ? 1 : 0;
      }
      expect(covered, name).toBeGreaterThan(CELL * CELL * 0.1);
    }
  });

  it('a figure is light on a dark outline, with its feet where the renderer expects them', () => {
    for (const pose of ['stand', 'walkA', 'walkB'] as const) {
      // The middle of the body and of the head take the colour of the resident.
      expect(pixel(pose, 32, 33)).toEqual([235, 235, 235, 255]);
      expect(pixel(pose, 32, 14)).toEqual([255, 255, 255, 255]);
      // The outline above the head stays dark under every colour.
      expect(pixel(pose, 32, 4)).toEqual([18, 20, 28, 255]);
      // Nothing but the shadow is below the feet.
      const below = pixel(pose, 32, Math.round(FIGURE_FEET * CELL) + 4);
      expect(below.slice(0, 3)).toEqual([0, 0, 0]);
      expect(below[3]).toBeLessThan(120);
    }
    expect(difference('stand', 'walkA')).toBeGreaterThan(4);
    expect(difference('walkA', 'walkB')).toBeGreaterThan(4);
  });

  it('gives every group of activities its own marker', () => {
    expect(MARKER_GROUPS).toEqual(['move', 'work', 'rest', 'eat', 'trade', 'talk', 'crime', 'law', 'civic', 'other']);
    for (let a = 0; a < MARKER_GROUPS.length; a++) {
      for (let b = a + 1; b < MARKER_GROUPS.length; b++) {
        const first = MARKER_GROUPS[a] ?? 'other';
        const second = MARKER_GROUPS[b] ?? 'other';
        expect(difference(`marker.${first}`, `marker.${second}`), `${first} and ${second}`).toBeGreaterThan(12);
      }
    }
    // The icon is not the colour of the badge.
    expect(pixel('marker.crime', 32, 25)).toEqual([255, 255, 255, 255]);
    expect(pixel('marker.crime', 14, 32)).toEqual([240, 62, 62, 255]);
  });
});
