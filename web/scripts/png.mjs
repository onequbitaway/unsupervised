// A small PNG reader and writer, for pictures that must be small files.
//
// Chrome writes a screenshot as a PNG with 24 or 32 bits for each pixel.
// A picture of a map has few colours, so a palette of 256 colours shows it
// well and the file is about a third as large. `toPalette` reads a PNG and
// writes it again with a palette. The script uses only the `zlib` of Node:
// no picture library is necessary.

import { crc32, deflateSync, inflateSync } from 'node:zlib';

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
/** Bits of each colour channel that the histogram keeps. */
const BITS = 5;
const SIDE = 1 << BITS;

/**
 * Reads a PNG with 8 bits for each channel, RGB or RGBA, without
 * interlace: the form that Chrome writes. Returns the size and the pixels
 * as RGB bytes.
 */
export function decodePng(file) {
  if (!file.subarray(0, 8).equals(SIGNATURE)) throw new Error('The file is not a PNG.');
  let width = 0;
  let height = 0;
  let channels = 0;
  const data = [];
  for (let at = 8; at < file.length; ) {
    const length = file.readUInt32BE(at);
    const type = file.toString('latin1', at + 4, at + 8);
    const body = file.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const [depth, colour, , , interlace] = body.subarray(8);
      if (depth !== 8 || (colour !== 2 && colour !== 6) || interlace !== 0) {
        throw new Error('The script reads only a PNG with 8 bits for each channel, RGB or RGBA, without interlace.');
      }
      channels = colour === 6 ? 4 : 3;
    } else if (type === 'IDAT') {
      data.push(body);
    }
    at += 12 + length;
  }
  if (channels === 0) throw new Error('The PNG has no header.');
  const raw = inflateSync(Buffer.concat(data));
  const stride = width * channels;
  if (raw.length !== (stride + 1) * height) throw new Error('The PNG has the wrong number of bytes.');
  // Undo the filter of each line. `line` is the line before, without a filter.
  const rgb = Buffer.alloc(width * height * 3);
  let before = Buffer.alloc(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? line[x - channels] : 0;
      const up = before[x];
      const corner = x >= channels ? before[x - channels] : 0;
      let predicted = 0;
      if (filter === 1) predicted = left;
      else if (filter === 2) predicted = up;
      else if (filter === 3) predicted = (left + up) >> 1;
      else if (filter === 4) {
        const estimate = left + up - corner;
        const [dl, du, dc] = [Math.abs(estimate - left), Math.abs(estimate - up), Math.abs(estimate - corner)];
        predicted = dl <= du && dl <= dc ? left : du <= dc ? up : corner;
      } else if (filter !== 0) throw new Error(`The PNG has the unknown filter ${filter}.`);
      line[x] = (line[x] + predicted) & 0xff;
    }
    for (let x = 0; x < width; x++) line.copy(rgb, (y * width + x) * 3, x * channels, x * channels + 3);
    before = line;
  }
  return { width, height, rgb };
}

/** Returns the index of a colour in the histogram. */
function bin(r, g, b) {
  return ((r >> (8 - BITS)) << (2 * BITS)) | ((g >> (8 - BITS)) << BITS) | (b >> (8 - BITS));
}

/**
 * Chooses at most `colours` colours for a picture, by median cut: the box
 * of colours with the most pixels and the longest side is cut in two at
 * its median, again and again. Returns the palette and, for each pixel,
 * the index of its colour.
 */
export function quantize(rgb, colours = 256) {
  const pixels = rgb.length / 3;
  // For each cell of the histogram: the number of pixels and the sum of each channel.
  const count = new Uint32Array(SIDE ** 3);
  const sums = [new Float64Array(SIDE ** 3), new Float64Array(SIDE ** 3), new Float64Array(SIDE ** 3)];
  for (let i = 0; i < pixels; i++) {
    const cell = bin(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]);
    count[cell] += 1;
    for (let channel = 0; channel < 3; channel++) sums[channel][cell] += rgb[i * 3 + channel];
  }
  const used = [];
  for (let cell = 0; cell < count.length; cell++) if (count[cell] > 0) used.push(cell);
  const channelOf = (cell, channel) => (cell >> ((2 - channel) * BITS)) & (SIDE - 1);

  /** Returns a box of cells with its number of pixels and its longest side. */
  const boxOf = (cells) => {
    let total = 0;
    let longest = 0;
    let side = -1;
    for (let channel = 0; channel < 3; channel++) {
      let low = SIDE;
      let high = -1;
      for (const cell of cells) {
        const value = channelOf(cell, channel);
        if (value < low) low = value;
        if (value > high) high = value;
      }
      if (high - low > side) {
        side = high - low;
        longest = channel;
      }
    }
    for (const cell of cells) total += count[cell];
    return { cells, total, longest, side };
  };

  const boxes = [boxOf(used)];
  while (boxes.length < colours) {
    // The box to cut: more than one cell, and the most pixels on its longest side.
    let pick = -1;
    for (let i = 0; i < boxes.length; i++) {
      const box = boxes[i];
      if (box.cells.length < 2) continue;
      if (pick < 0 || box.total * (box.side + 1) > boxes[pick].total * (boxes[pick].side + 1)) pick = i;
    }
    if (pick < 0) break;
    const box = boxes[pick];
    box.cells.sort((a, b) => channelOf(a, box.longest) - channelOf(b, box.longest) || a - b);
    let half = 0;
    let cut = 0;
    while (cut < box.cells.length - 1 && half + count[box.cells[cut]] <= box.total / 2) half += count[box.cells[cut++]];
    cut = Math.max(1, cut);
    boxes.splice(pick, 1, boxOf(box.cells.slice(0, cut)), boxOf(box.cells.slice(cut)));
  }

  // The colour of a box is the mean of its pixels.
  const palette = Buffer.alloc(boxes.length * 3);
  const indexOf = new Uint8Array(SIDE ** 3);
  boxes.forEach((box, index) => {
    for (let channel = 0; channel < 3; channel++) {
      let sum = 0;
      for (const cell of box.cells) sum += sums[channel][cell];
      palette[index * 3 + channel] = Math.round(sum / box.total);
    }
    for (const cell of box.cells) indexOf[cell] = index;
  });
  const indices = Buffer.alloc(pixels);
  for (let i = 0; i < pixels; i++) indices[i] = indexOf[bin(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2])];
  return { palette, indices };
}

/** Returns one chunk of a PNG file. */
function chunk(type, body) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(type, 4, 'latin1');
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc32(body, crc32(head.subarray(4))), 0);
  return Buffer.concat([head, body, tail]);
}

/** Writes a PNG with a palette: one byte for each pixel. */
export function encodePalettePng(width, height, palette, indices) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 3, 0, 0, 0], 8);
  // Each line starts with the filter 0: the bytes of the line as they are.
  const raw = Buffer.alloc((width + 1) * height);
  for (let y = 0; y < height; y++) indices.copy(raw, y * (width + 1) + 1, y * width, (y + 1) * width);
  return Buffer.concat([
    SIGNATURE,
    chunk('IHDR', header),
    chunk('PLTE', palette),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Reads a PNG and returns it again with a palette of at most `colours` colours. */
export function toPalette(file, colours = 256) {
  const { width, height, rgb } = decodePng(file);
  const { palette, indices } = quantize(rgb, colours);
  return encodePalettePng(width, height, palette, indices);
}
