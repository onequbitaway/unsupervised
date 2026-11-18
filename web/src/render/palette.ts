// Colours. All art of the viewer is made from these numbers.

/** Tile codes of docs/design/game-design.md, section 2. */
export const TILE = {
  grass: 0,
  road: 1,
  pavement: 2,
  floor: 3,
  wall: 4,
  water: 5,
  trees: 6,
  field: 7,
  rock: 8,
} as const;

export type Rgb = readonly [number, number, number];

/** The base colour of each tile code. An unknown code is drawn as grass. */
export const TILE_COLORS: readonly Rgb[] = [
  [62, 110, 58], // grass
  [58, 60, 68], // road
  [150, 150, 156], // pavement
  [196, 170, 128], // floor
  [44, 38, 40], // wall
  [44, 104, 176], // water
  [34, 82, 44], // trees
  [150, 136, 62], // field
  [104, 100, 96], // rock
];

export const COLORS = {
  background: 0x0b0e14,
  land: 0x151a23,
  cityOutline: 0xf2f4f8,
  zoneLine: 0x0b0e14,
  selection: 0xffe14d,
  bubble: 0xffffff,
  bubbleText: 0x10131a,
  labelText: 0xffffff,
  labelStroke: 0x0b0e14,
  siteText: 0xfff3c4,
  placeText: 0xffffff,
  districtLine: 0x0b0e14,
} as const;

/** The colour of a site kind, as a tint over its tiles. */
const SITE_COLORS: Record<string, number> = {
  home: 0x6fb1ff,
  apartment: 0x6fb1ff,
  shelter: 0x8fd0c8,
  shop: 0xffb347,
  restaurant: 0xff8c5a,
  office: 0xb9a6ff,
  factory: 0xc98f6b,
  workshop: 0xc98f6b,
  warehouse: 0xa58f7a,
  depot: 0xa58f7a,
  farm: 0xd8c65a,
  mine: 0x9aa0a8,
  forest: 0x4fae62,
  park: 0x6edc7a,
  bank: 0xffd84d,
  exchange: 0xffd84d,
  city_hall: 0xff6f91,
  courthouse: 0xff6f91,
  police_station: 0x5f8cff,
  jail: 0x7f8aa3,
  hospital: 0xff5a5a,
  school: 0x57d3ff,
  station: 0xe0e0e0,
  newsroom: 0xf0a0ff,
};

export function siteColor(kind: string): number {
  return Object.hasOwn(SITE_COLORS, kind) ? (SITE_COLORS[kind] ?? 0xd0d0d0) : 0xd0d0d0;
}

/** Bright colours for residents. They stay readable on every tile. */
const RESIDENT_COLORS: readonly number[] = [
  0xff6b6b, 0xffa94d, 0xffd43b, 0xa9e34b, 0x51cf66, 0x38d9a9, 0x3bc9db, 0x4dabf7, 0x748ffc,
  0xb197fc, 0xe599f7, 0xf783ac, 0xffffff, 0xffc9c9, 0xd8f5a2, 0x99e9f2,
];

/** Returns the colour of a resident. The same id always has the same colour. */
export function residentColor(id: number): number {
  // A multiplicative hash spreads neighbour ids over the palette.
  const mixed = Math.imul(id >>> 0, 0x9e3779b1) >>> 0;
  return RESIDENT_COLORS[mixed % RESIDENT_COLORS.length] ?? 0xffffff;
}

/**
 * The colours of the population heat map, from empty to full. The red
 * channel never falls from one colour to the next, so a fuller zone never
 * looks colder.
 */
const HEAT: readonly Rgb[] = [
  [24, 30, 44],
  [30, 62, 124],
  [34, 128, 160],
  [72, 180, 110],
  [228, 200, 60],
  [230, 110, 40],
  [230, 40, 50],
];

/**
 * Returns the heat colour of a zone with `count` residents when the
 * fullest zone has `max`. The scale is the square root, so a few residents
 * already show. An empty zone has the first colour.
 */
export function heatColor(count: number, max: number): number {
  if (!(count > 0) || !(max > 0)) return rgb(HEAT[0] ?? [0, 0, 0]);
  const t = Math.min(1, Math.sqrt(count / max));
  // The first colour is for empty zones only.
  const scaled = 1 + t * (HEAT.length - 2);
  const low = Math.min(HEAT.length - 2, Math.floor(scaled));
  const a = HEAT[low] ?? [0, 0, 0];
  const b = HEAT[low + 1] ?? a;
  const f = scaled - low;
  return rgb([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]);
}

/** Packs a colour into one number: 0xRRGGBB. */
export function rgb(color: Rgb): number {
  const clamp = (v: number): number => Math.min(255, Math.max(0, Math.round(v)));
  return (clamp(color[0]) << 16) | (clamp(color[1]) << 8) | clamp(color[2]);
}
