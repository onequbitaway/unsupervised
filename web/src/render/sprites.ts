// The sprites of the viewer: resident figures, the dot of a far resident,
// the ring of the selected resident and one marker for each group of
// activities. All of them are painted here from shapes, into one image.
//
// A figure is white and grey on a dark outline. The renderer multiplies it
// with the colour of the resident: white takes the colour, dark stays dark.

import { ACTIVITY_GROUPS, type ActivityGroup } from '../state/activity';
import { blank, box, circle, ellipse, fill, grow, line, type Pixels, polygon, type Rgba, type Shape, union, without } from './shapes';

/** Side of one sprite in the image, in pixels. */
export const CELL = 64;
/** Sprites in one row of the image. */
const COLUMNS = 8;

/** Where the feet of a figure are in its sprite, as a part of the height. */
export const FIGURE_FEET = 57 / CELL;
/** The part of the height of a figure sprite that the figure fills. */
export const FIGURE_BODY = 58 / CELL;

export type FigurePose = 'stand' | 'walkA' | 'walkB';
export type MarkerGroup = Exclude<ActivityGroup, 'idle'>;

/** The groups that have a marker. A resident that does nothing has none. */
export const MARKER_GROUPS: readonly MarkerGroup[] = ACTIVITY_GROUPS.filter(
  (group): group is MarkerGroup => group !== 'idle',
);

export type SpriteName = FigurePose | 'dot' | 'ring' | `marker.${MarkerGroup}`;

/** Every sprite, in the order of its place in the image. */
export const SPRITES: readonly SpriteName[] = [
  'stand',
  'walkA',
  'walkB',
  'dot',
  'ring',
  ...MARKER_GROUPS.map((group): SpriteName => `marker.${group}`),
];

export interface Frame {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface SpriteSheet {
  image: Pixels;
  frames: ReadonlyMap<SpriteName, Frame>;
}

const OUTLINE: Rgba = [18, 20, 28, 1];
const WHITE: Rgba = [255, 255, 255, 1];
const INK: Rgba = [24, 26, 36, 1];

type Layer = readonly [Shape, Rgba];

/** One limb pair: where the left and the right limb end. */
interface Pose {
  feet: readonly [number, number, number, number];
  hands: readonly [number, number, number, number];
}

const POSES: Record<FigurePose, Pose> = {
  stand: { feet: [26, 54, 38, 54], hands: [19, 40, 45, 40] },
  // A step: the legs and the arms are apart, then together.
  walkA: { feet: [21, 53, 43, 52], hands: [15, 37, 49, 37] },
  walkB: { feet: [31, 54, 33, 54], hands: [21, 41, 43, 41] },
};

function figure(pose: FigurePose): Layer[] {
  const { feet, hands } = POSES[pose];
  const legs = union(line(27, 42, feet[0], feet[1], 4), line(37, 42, feet[2], feet[3], 4));
  const arms = union(line(20, 28, hands[0], hands[1], 3.5), line(44, 28, hands[2], hands[3], 3.5));
  const torso = box(32, 33, 10, 11, 5);
  const head = circle(32, 14.5, 9);
  return [
    // A shadow on the ground. It stays dark under every colour.
    [ellipse(32, 57.5, 14, 4.5), [0, 0, 0, 0.35]],
    [grow(union(legs, arms, torso, head), 2.5), OUTLINE],
    [legs, [150, 150, 150, 1]],
    [arms, [205, 205, 205, 1]],
    [torso, [235, 235, 235, 1]],
    // A dark line between the head and the body.
    [grow(head, 2), OUTLINE],
    [grow(head, -0.4), WHITE],
  ];
}

interface MarkerArt {
  color: Rgba;
  ink: Rgba;
  icon: Shape;
}

/** The letter S from straight lines, for the dollar sign. */
function dollar(): Shape {
  const t = 2.8;
  return union(
    line(25, 22, 39, 22, t),
    line(25, 22, 25, 31, t),
    line(25, 31, 39, 31, t),
    line(39, 31, 39, 41, t),
    line(25, 41, 39, 41, t),
    line(32, 15, 32, 48, 2.2),
  );
}

const MARKERS: Record<MarkerGroup, MarkerArt> = {
  // An arrow.
  move: {
    color: [51, 154, 240, 1],
    ink: WHITE,
    icon: polygon([[16, 27], [32, 27], [32, 18], [49, 32], [32, 46], [32, 37], [16, 37]]),
  },
  // A hammer.
  work: { color: [255, 146, 43, 1], ink: WHITE, icon: union(line(21, 47, 37, 27, 3.5), line(30, 18, 46, 30, 6.5)) },
  // The letter Z of sleep.
  rest: {
    color: [116, 143, 252, 1],
    ink: WHITE,
    icon: union(line(22, 20, 42, 20, 3.2), line(42, 20, 22, 44, 3.2), line(22, 44, 42, 44, 3.2)),
  },
  // A bowl with steam.
  eat: {
    color: [55, 178, 77, 1],
    ink: WHITE,
    icon: union(
      without(circle(32, 30, 17), box(32, 17, 20, 13)),
      line(13, 30, 51, 30, 2.5),
      line(26, 13, 26, 21, 2),
      line(38, 13, 38, 21, 2),
    ),
  },
  // A dollar sign.
  trade: { color: [252, 196, 25, 1], ink: INK, icon: dollar() },
  // A speech bubble.
  talk: { color: WHITE, ink: INK, icon: union(box(32, 29, 15, 10, 5), polygon([[24, 36], [24, 47], [33, 37]])) },
  // An exclamation mark.
  crime: { color: [240, 62, 62, 1], ink: WHITE, icon: union(line(32, 17, 32, 35, 4.2), circle(32, 45, 4.6)) },
  // Scales.
  law: {
    color: [54, 79, 199, 1],
    ink: WHITE,
    icon: union(
      line(32, 18, 32, 46, 2.6),
      line(18, 22, 46, 22, 2.6),
      line(24, 46, 40, 46, 2.6),
      polygon([[12, 26], [26, 26], [19, 38]]),
      polygon([[38, 26], [52, 26], [45, 38]]),
    ),
  },
  // A check mark.
  civic: { color: [190, 75, 219, 1], ink: WHITE, icon: union(line(19, 33, 28, 43, 4), line(28, 43, 46, 21, 4)) },
  // Three dots.
  other: { color: [173, 181, 189, 1], ink: INK, icon: union(circle(20, 32, 4.5), circle(32, 32, 4.5), circle(44, 32, 4.5)) },
};

function marker(group: MarkerGroup): Layer[] {
  const art = MARKERS[group];
  return [
    [circle(32, 32, 29), OUTLINE],
    [circle(32, 32, 26), art.color],
    [art.icon, art.ink],
  ];
}

function layersOf(name: SpriteName): Layer[] {
  switch (name) {
    case 'stand':
    case 'walkA':
    case 'walkB':
      return figure(name);
    case 'dot':
      return [
        [circle(32, 32, 26), OUTLINE],
        [circle(32, 32, 20), WHITE],
      ];
    case 'ring': {
      const ring = without(circle(32, 32, 27.5), circle(32, 32, 21));
      return [
        [grow(ring, 2), OUTLINE],
        [ring, WHITE],
      ];
    }
    default:
      return marker(name.slice('marker.'.length) as MarkerGroup);
  }
}

/** Paints every sprite into one image and says where each one is. */
export function paintSprites(): SpriteSheet {
  const rows = Math.ceil(SPRITES.length / COLUMNS);
  const image = blank(COLUMNS * CELL, rows * CELL);
  const frames = new Map<SpriteName, Frame>();
  SPRITES.forEach((name, index) => {
    const x = (index % COLUMNS) * CELL;
    const y = Math.floor(index / COLUMNS) * CELL;
    for (const [shape, color] of layersOf(name)) fill(image, x, y, CELL, CELL, shape, color);
    frames.set(name, { x, y, width: CELL, height: CELL });
  });
  return { image, frames };
}
