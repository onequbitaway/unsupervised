// Tests of `png.mjs`. Run them with `node --test scripts/` in `web/`.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { crc32, deflateSync, inflateSync } from 'node:zlib';

import { decodePng, quantize, toPalette } from './png.mjs';

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function chunk(type, body) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(type, 4, 'latin1');
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), body])), 0);
  return Buffer.concat([head, body, tail]);
}

/**
 * Writes a PNG as Chrome does: 8 bits for each channel, RGB or RGBA. Line
 * `y` uses the filter `filters[y % filters.length]`: 0 (none), 1 (the
 * pixel to the left) or 2 (the pixel above).
 */
function png(width, height, channels, pixelAt, filters) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, channels === 4 ? 6 : 2, 0, 0, 0], 8);
  const stride = width * channels;
  const lines = [];
  for (let y = 0; y < height; y++) {
    const line = Buffer.alloc(stride);
    for (let x = 0; x < width; x++) line.set([...pixelAt(x, y), 255].slice(0, channels), x * channels);
    lines.push(line);
  }
  const raw = Buffer.alloc((stride + 1) * height);
  lines.forEach((line, y) => {
    const filter = filters[y % filters.length];
    raw[y * (stride + 1)] = filter;
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? line[x - channels] : 0;
      const up = y > 0 ? lines[y - 1][x] : 0;
      const predicted = filter === 1 ? left : filter === 2 ? up : 0;
      raw[y * (stride + 1) + 1 + x] = (line[x] - predicted) & 0xff;
    }
  });
  return Buffer.concat([SIGNATURE, chunk('IHDR', header), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

/** Reads the chunks of a PNG file: the type and the body of each. */
function chunks(file) {
  const found = [];
  for (let at = 8; at < file.length; ) {
    const length = file.readUInt32BE(at);
    const body = file.subarray(at + 8, at + 8 + length);
    const type = file.toString('latin1', at + 4, at + 8);
    assert.equal(file.readUInt32BE(at + 8 + length), crc32(Buffer.concat([file.subarray(at + 4, at + 8), body])), `the checksum of ${type}`);
    found.push({ type, body });
    at += 12 + length;
  }
  return found;
}

// Twelve colours that are far from each other.
const COLOURS = Array.from({ length: 12 }, (_unused, index) => [index * 20, 255 - index * 20, (index * 97) % 256]);
const colourAt = (x, y) => COLOURS[(x * 3 + y * 5) % COLOURS.length];

test('a PNG of Chrome is read with each line filter, with and without an alpha channel', () => {
  for (const channels of [3, 4]) {
    const { width, height, rgb } = decodePng(png(16, 9, channels, colourAt, [0, 1, 2]));
    assert.deepEqual([width, height, rgb.length], [16, 9, 16 * 9 * 3]);
    for (let y = 0; y < 9; y++) {
      for (let x = 0; x < 16; x++) {
        assert.deepEqual([...rgb.subarray((y * 16 + x) * 3, (y * 16 + x) * 3 + 3)], colourAt(x, y), `pixel ${x},${y}`);
      }
    }
  }
  assert.throws(() => decodePng(Buffer.from('not a picture')), /not a PNG/);
});

test('a picture with few colours keeps each colour in its palette', () => {
  const file = toPalette(png(16, 9, 4, colourAt, [1, 2]));
  const parts = chunks(file);
  assert.deepEqual(parts.map((part) => part.type), ['IHDR', 'PLTE', 'IDAT', 'IEND']);
  const [header, palette, data] = parts.map((part) => part.body);
  assert.deepEqual([header.readUInt32BE(0), header.readUInt32BE(4)], [16, 9]);
  // 8 bits for each pixel, colour type 3: a palette.
  assert.deepEqual([...header.subarray(8)], [8, 3, 0, 0, 0]);
  assert.equal(palette.length, COLOURS.length * 3);
  const raw = inflateSync(data);
  assert.equal(raw.length, (16 + 1) * 9);
  for (let y = 0; y < 9; y++) {
    assert.equal(raw[y * 17], 0, 'each line has the filter 0');
    for (let x = 0; x < 16; x++) {
      const index = raw[y * 17 + 1 + x];
      assert.deepEqual([...palette.subarray(index * 3, index * 3 + 3)], colourAt(x, y), `pixel ${x},${y}`);
    }
  }
});

test('a picture with many colours gets at most the asked number of colours, near the real ones', () => {
  // A smooth picture with 4,096 different colours.
  const smooth = (x, y) => [x * 4, y * 4, (x + y) * 2];
  const { rgb } = decodePng(png(64, 64, 3, smooth, [0]));
  const { palette, indices } = quantize(rgb, 64);
  assert.equal(palette.length, 64 * 3);
  assert.equal(new Set(indices).size, 64);
  let worst = 0;
  for (let i = 0; i < indices.length; i++) {
    for (let channel = 0; channel < 3; channel++) {
      worst = Math.max(worst, Math.abs(palette[indices[i] * 3 + channel] - rgb[i * 3 + channel]));
    }
  }
  // 64 colours for a cube of 256 x 256 x 256: no channel is far from the real colour.
  assert.ok(worst <= 48, `the worst difference of a channel is ${worst}`);
  // More colours are nearer.
  const fine = quantize(rgb, 256);
  let worstFine = 0;
  for (let i = 0; i < fine.indices.length; i++) {
    for (let channel = 0; channel < 3; channel++) {
      worstFine = Math.max(worstFine, Math.abs(fine.palette[fine.indices[i] * 3 + channel] - rgb[i * 3 + channel]));
    }
  }
  assert.ok(worstFine < worst, `${worstFine} is less than ${worst}`);
});
