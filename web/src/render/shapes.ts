// Shapes as signed distances, and a painter that fills them into pixels.
// The sprites of the viewer are made from these: no drawing interface is
// used, so the result is the same on every machine.

/** The distance from a point to the edge of a shape, in pixels. Negative inside. */
export type Shape = (x: number, y: number) => number;

/** Red, green, blue (0 to 255) and alpha (0 to 1). */
export type Rgba = readonly [number, number, number, number];

/** An image: four bytes per pixel (red, green, blue, alpha), row by row. */
export interface Pixels {
  data: Uint8ClampedArray;
  width: number;
  height: number;
}

export function circle(cx: number, cy: number, radius: number): Shape {
  return (x, y) => Math.hypot(x - cx, y - cy) - radius;
}

/** An ellipse. The distance is exact on the axes and close elsewhere. */
export function ellipse(cx: number, cy: number, rx: number, ry: number): Shape {
  return (x, y) => (Math.hypot((x - cx) / rx, (y - cy) / ry) - 1) * Math.min(rx, ry);
}

/** A rectangle around a centre, with round corners when `round` is above 0. */
export function box(cx: number, cy: number, halfWidth: number, halfHeight: number, round = 0): Shape {
  return (x, y) => {
    const dx = Math.abs(x - cx) - (halfWidth - round);
    const dy = Math.abs(y - cy) - (halfHeight - round);
    return Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - round;
  };
}

/** A line from A to B with round ends. `half` is half of its thickness. */
export function line(ax: number, ay: number, bx: number, by: number, half: number): Shape {
  const abx = bx - ax;
  const aby = by - ay;
  const lengthSquared = abx * abx + aby * aby;
  return (x, y) => {
    const t = lengthSquared === 0 ? 0 : Math.min(1, Math.max(0, ((x - ax) * abx + (y - ay) * aby) / lengthSquared));
    return Math.hypot(x - ax - abx * t, y - ay - aby * t) - half;
  };
}

/** A polygon. The points go around it in any direction. */
export function polygon(points: readonly (readonly [number, number])[]): Shape {
  return (x, y) => {
    let nearest = Infinity;
    let inside = false;
    for (let i = 0, j = points.length - 1; i < points.length; j = i, i++) {
      const [ax, ay] = points[i] ?? [0, 0];
      const [bx, by] = points[j] ?? [0, 0];
      const ex = bx - ax;
      const ey = by - ay;
      const wx = x - ax;
      const wy = y - ay;
      const t = Math.min(1, Math.max(0, (wx * ex + wy * ey) / (ex * ex + ey * ey || 1)));
      nearest = Math.min(nearest, Math.hypot(wx - ex * t, wy - ey * t));
      // The point is inside when a ray from it crosses the outline an odd number of times.
      if (ay > y !== by > y && x < ax + ((y - ay) / (by - ay)) * ex) inside = !inside;
    }
    return inside ? -nearest : nearest;
  };
}

/** Every point that is in at least one of the shapes. */
export function union(...shapes: readonly Shape[]): Shape {
  return (x, y) => {
    let nearest = Infinity;
    for (const shape of shapes) nearest = Math.min(nearest, shape(x, y));
    return nearest;
  };
}

/** The points of `shape` that are not in `hole`. */
export function without(shape: Shape, hole: Shape): Shape {
  return (x, y) => Math.max(shape(x, y), -hole(x, y));
}

/** The shape, larger by `amount` pixels on every side. */
export function grow(shape: Shape, amount: number): Shape {
  return (x, y) => shape(x, y) - amount;
}

/** Makes an image that is fully transparent. */
export function blank(width: number, height: number): Pixels {
  return { data: new Uint8ClampedArray(width * height * 4), width, height };
}

/**
 * Paints a shape over the image, inside the rectangle that starts at
 * (`left`, `top`). The shape uses coordinates from that corner. The edge
 * of the shape is smooth: a pixel that the edge crosses gets a part of the
 * colour.
 */
export function fill(
  image: Pixels,
  left: number,
  top: number,
  width: number,
  height: number,
  shape: Shape,
  color: Rgba,
): void {
  const { data } = image;
  for (let v = 0; v < height; v++) {
    const py = top + v;
    if (py < 0 || py >= image.height) continue;
    for (let u = 0; u < width; u++) {
      const px = left + u;
      if (px < 0 || px >= image.width) continue;
      // The distance at the middle of the pixel gives how much of it is covered.
      const cover = Math.min(1, Math.max(0, 0.5 - shape(u + 0.5, v + 0.5)));
      if (cover <= 0) continue;
      const at = (py * image.width + px) * 4;
      const alpha = color[3] * cover;
      const under = (data[at + 3] ?? 0) / 255;
      const out = alpha + under * (1 - alpha);
      for (let c = 0; c < 3; c++) {
        data[at + c] = ((color[c] ?? 0) * alpha + (data[at + c] ?? 0) * under * (1 - alpha)) / out;
      }
      data[at + 3] = out * 255;
    }
  }
}
