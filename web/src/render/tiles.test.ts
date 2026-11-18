import { describe, expect, it } from 'vitest';
import { heatColor, residentColor, rgb, siteColor, TILE, TILE_COLORS } from './palette';
import { noise, paintTiles } from './tiles';

/** Returns the mean colour of one tile of a painted district. */
function mean(pixels: Uint8ClampedArray, width: number, px: number, tx: number, ty: number): number[] {
  const sum = [0, 0, 0];
  for (let v = 0; v < px; v++) {
    for (let u = 0; u < px; u++) {
      const at = ((ty * px + v) * width * px + tx * px + u) * 4;
      for (let c = 0; c < 3; c++) sum[c] = (sum[c] ?? 0) + (pixels[at + c] ?? 0);
    }
  }
  return sum.map((s) => s / (px * px));
}

describe('paintTiles', () => {
  it('paints the size that it was asked for, fully opaque', () => {
    const pixels = paintTiles(new Uint8Array(6), 3, 2, 8, 0, 0);
    expect(pixels).toHaveLength(3 * 8 * 2 * 8 * 4);
    for (let at = 3; at < pixels.length; at += 4) expect(pixels[at]).toBe(255);
  });

  it('gives every tile code its own colour', () => {
    const codes = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    // One row of tiles with a gap of grass between them, away from walls.
    const tiles = new Uint8Array(codes.length * 2);
    codes.forEach((code, i) => {
      tiles[i * 2] = code;
    });
    const pixels = paintTiles(tiles, codes.length * 2, 1, 12, 0, 0);
    const means = codes.map((_, i) => mean(pixels, codes.length * 2, 12, i * 2, 0));
    means.forEach((colour, i) => {
      const base = TILE_COLORS[codes[i] ?? 0] ?? [0, 0, 0];
      // The pattern changes the mean colour of a tile only a little.
      // Trees show grass around the crown, so they are between the two.
      const limit = codes[i] === TILE.trees ? 40 : 16;
      for (let c = 0; c < 3; c++) expect(Math.abs((colour[c] ?? 0) - (base[c] ?? 0))).toBeLessThan(limit);
    });
    // No two codes look the same.
    for (let a = 0; a < means.length; a++) {
      for (let b = a + 1; b < means.length; b++) {
        const d = Math.hypot(...(means[a] ?? []).map((v, c) => v - ((means[b] ?? [])[c] ?? 0)));
        expect(d, `codes ${a} and ${b}`).toBeGreaterThan(18);
      }
    }
  });

  it('gives tiles a pattern, not one flat colour', () => {
    for (const code of [0, 1, 2, 3, 4, 5, 6, 7, 8]) {
      const pixels = paintTiles(new Uint8Array([code]), 1, 1, 12, 5, 9);
      const reds = new Set<number>();
      for (let at = 0; at < pixels.length; at += 4) reds.add(pixels[at] ?? 0);
      expect(reds.size, `code ${code}`).toBeGreaterThan(3);
    }
  });

  it('paints the same pixels every time, and other pixels for another place', () => {
    const tiles = new Uint8Array([0, 5, 6, 1]);
    const first = paintTiles(tiles, 2, 2, 8, 10, 20);
    expect(paintTiles(tiles, 2, 2, 8, 10, 20)).toEqual(first);
    expect(paintTiles(tiles, 2, 2, 8, 11, 20)).not.toEqual(first);
  });

  it('draws an unknown code as grass and a wall with a shadow below it', () => {
    const unknown = mean(paintTiles(new Uint8Array([200]), 1, 1, 12, 0, 0), 1, 12, 0, 0);
    const grass = mean(paintTiles(new Uint8Array([0]), 1, 1, 12, 0, 0), 1, 12, 0, 0);
    expect(unknown).toEqual(grass);
    // Pavement below a wall is darker than pavement below pavement.
    const shaded = mean(paintTiles(new Uint8Array([4, 2]), 1, 2, 12, 0, 0), 1, 12, 0, 1);
    const open = mean(paintTiles(new Uint8Array([2, 2]), 1, 2, 12, 0, 0), 1, 12, 0, 1);
    expect((open[0] ?? 0) - (shaded[0] ?? 0)).toBeGreaterThan(5);
  });

  it('the noise is spread over its whole range', () => {
    const seen = new Set<number>();
    for (let i = 0; i < 4_000; i++) seen.add(noise(i, i * 7, 3));
    expect(seen.size).toBeGreaterThan(250);
    expect(noise(12, 34, 5)).toBe(noise(12, 34, 5));
  });
});

describe('palette', () => {
  it('packs a colour and keeps each channel in range', () => {
    expect(rgb([255, 128, 0])).toBe(0xff8000);
    expect(rgb([300, -5, 15.6])).toBe(0xff0010);
  });

  it('heat goes from the colour of an empty zone to red', () => {
    expect(heatColor(0, 100)).toBe(0x181e2c);
    expect(heatColor(5, 0)).toBe(0x181e2c);
    expect(heatColor(100, 100)).toBe(0xe62832);
    expect(heatColor(500, 100)).toBe(0xe62832);
    // One resident already differs from none.
    expect(heatColor(1, 100)).not.toBe(heatColor(0, 100));
    // More residents never look colder: red grows, then green falls.
    const reds = [1, 4, 16, 36, 64, 100].map((n) => heatColor(n, 100) >> 16);
    expect([...reds].sort((a, b) => a - b)).toEqual(reds);
  });

  it('a resident keeps its colour and neighbours differ', () => {
    expect(residentColor(42)).toBe(residentColor(42));
    const colours = new Set(Array.from({ length: 64 }, (_, id) => residentColor(id)));
    expect(colours.size).toBe(16);
    expect(residentColor(1)).not.toBe(residentColor(2));
  });

  it('a site kind has a colour, also an unknown one', () => {
    expect(siteColor('bank')).toBe(0xffd84d);
    expect(siteColor('spaceport')).toBe(0xd0d0d0);
    expect(siteColor('constructor')).toBe(0xd0d0d0);
  });
});
