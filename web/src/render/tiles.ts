// Paints the tiles of one district into pixels. No drawing interface is
// used: the result is an array of RGBA values, the same on every machine.
// Each tile code has a base colour and a generated pattern.

import { TILE, TILE_COLORS, type Rgb } from './palette';

/** A small hash of two integers: 0 to 255, always the same for the same input. */
export function noise(x: number, y: number, seed: number): number {
  let h = Math.imul(x | 0, 0x27d4eb2d) ^ Math.imul(y | 0, 0x165667b1) ^ Math.imul(seed | 0, 0x9e3779b1);
  h = Math.imul(h ^ (h >>> 15), 0x85ebca6b);
  h = Math.imul(h ^ (h >>> 13), 0xc2b2ae35);
  return (h ^ (h >>> 16)) & 0xff;
}

function codeAt(tiles: Uint8Array, width: number, height: number, x: number, y: number): number {
  if (x < 0 || y < 0 || x >= width || y >= height) return -1;
  return tiles[y * width + x] ?? 0;
}

/**
 * Returns how much lighter or darker one pixel of a tile is than the base
 * colour, as a number to add to each channel. `u` and `v` are the place
 * of the pixel in the tile, from 0 to `px - 1`. `tx` and `ty` are the tile
 * in world coordinates, so that patterns go on across district borders.
 */
function shade(code: number, tx: number, ty: number, u: number, v: number, px: number, seed: number): number {
  const grain = noise(tx * px + u, ty * px + v, seed) / 255 - 0.5;
  const half = px >> 1;
  switch (code) {
    case TILE.grass:
      // Speckles, and a darker tuft here and there.
      return grain * 14 + (noise(tx * px + (u >> 1), ty * px + (v >> 1), seed + 1) > 236 ? -22 : 0);
    case TILE.road:
      // Asphalt grain and a dashed line through the middle of the tile.
      return grain * 8 + ((u === half || v === half) && (u + v) % 4 < 2 ? 10 : 0);
    case TILE.pavement:
      // Slabs: a dark joint on two sides of every tile.
      return u === 0 || v === 0 ? -34 : grain * 8;
    case TILE.floor:
      // Planks: a joint every third row, shifted on every tile.
      return v % 3 === 0 ? -20 : grain * 10 + ((u + tx) % px === 0 ? -14 : 0);
    case TILE.wall:
      // A light top edge so that a wall reads as a block.
      return v < 2 ? 46 : u === 0 || u === px - 1 ? -10 : grain * 8;
    case TILE.water: {
      // Waves: light bands that move sideways from row to row.
      const wave = (u + ((v >> 1) + tx * 3) * 2) % 8;
      return (v + ty) % 4 === 0 && wave < 4 ? 34 : grain * 10;
    }
    case TILE.trees: {
      // A round crown with a light side and a dark rim.
      const dx = u - half + 0.5;
      const dy = v - half + 0.5;
      const r = Math.hypot(dx, dy) / half;
      if (r > 1.0) return 44 + grain * 10;
      return r > 0.82 ? -26 : (dx + dy < 0 ? 22 : -4) + grain * 16;
    }
    case TILE.field:
      // Furrows.
      return (v + ty) % 3 === 0 ? -26 : grain * 12;
    case TILE.rock: {
      // Facets: each quarter of the tile has its own tone.
      const facet = noise(tx * 2 + (u < half ? 0 : 1), ty * 2 + (v < half ? 0 : 1), seed + 2);
      return (facet - 128) / 5 + (u === 0 || v === 0 ? -24 : 0);
    }
    default:
      return grain * 14;
  }
}

/** The colour of the ground under a tree crown: grass. */
const UNDER_TREES: Rgb = TILE_COLORS[TILE.grass] ?? [0, 0, 0];

/**
 * Paints a district: `px` by `px` pixels for each tile, row by row, four
 * bytes per pixel (red, green, blue, alpha). `originX` and `originY` are
 * the world coordinates of the first tile. A tile next to a wall gets a
 * shadow on the side of the wall, which gives buildings depth.
 */
export function paintTiles(
  tiles: Uint8Array,
  width: number,
  height: number,
  px: number,
  originX: number,
  originY: number,
  seed = 7,
): Uint8ClampedArray {
  const out = new Uint8ClampedArray(width * px * height * px * 4);
  const rowBytes = width * px * 4;
  for (let ty = 0; ty < height; ty++) {
    for (let tx = 0; tx < width; tx++) {
      // An unknown code is drawn as grass, with the pattern of grass.
      const raw = codeAt(tiles, width, height, tx, ty);
      const code = TILE_COLORS[raw] === undefined ? TILE.grass : raw;
      const base = TILE_COLORS[code] ?? [0, 0, 0];
      const wallAbove = code !== TILE.wall && codeAt(tiles, width, height, tx, ty - 1) === TILE.wall;
      const wallLeft = code !== TILE.wall && codeAt(tiles, width, height, tx - 1, ty) === TILE.wall;
      for (let v = 0; v < px; v++) {
        let at = (ty * px + v) * rowBytes + tx * px * 4;
        for (let u = 0; u < px; u++) {
          let add = shade(code, originX + tx, originY + ty, u, v, px, seed);
          let color = base;
          if (code === TILE.trees && add > 40) {
            // Outside the crown the ground shows.
            color = UNDER_TREES;
            add -= 44;
          }
          if ((wallAbove && v < px / 3) || (wallLeft && u < px / 4)) add -= 30;
          out[at] = color[0] + add;
          out[at + 1] = color[1] + add;
          out[at + 2] = color[2] + add;
          out[at + 3] = 255;
          at += 4;
        }
      }
    }
  }
  return out;
}
