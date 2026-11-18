// Saves a picture of the viewer as a PNG file.
//
//   npm run screenshot -- --url http://127.0.0.1:8080 [--query "view=country"]
//       [--out shot.png] [--width 1920] [--height 1080] [--settle 1500]
//       [--count 1] [--every 4000] [--max-bytes 600000] [--press Escape]
//
// `--query` adds options to the address, for example `view=country`,
// `zone=3`, `watch=1048577`, `broadcast=1`, `resident=7` or `clip=120-3`.
// `--settle` is the time in milliseconds between the moment the picture is
// complete and the first shot. `--count` saves more than one picture, with
// `--every` milliseconds between two: `shot.png` becomes `shot-1.png`,
// `shot-2.png` and so on. Use it for a broadcast, which changes by itself.
// `--press` presses one key before the first shot, for example `Escape`
// to close the panel of the resident that `watch=<id>` follows.
// `--max-bytes` is the greatest size of one file. A picture that is larger
// is written again with a palette of 256 colours, which is about a third of
// the size. The script fails when the file is still too large.

import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { number, readArgs } from './args.mjs';
import { openViewer, viewerUrl } from './browser.mjs';
import { toPalette } from './png.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string') {
  console.error('Give the address of the server: --url http://127.0.0.1:8080');
  process.exit(2);
}
const out = resolve(typeof args.out === 'string' ? args.out : 'screenshot.png');
const url = viewerUrl(args.url, typeof args.query === 'string' ? args.query : '');
const count = Math.max(1, number(args.count, 1));
const every = number(args.every, 4_000);
const maxBytes = number(args['max-bytes'], Infinity);

/** Returns the file of one shot of a series. */
function fileOf(shot) {
  return count === 1 ? out : out.replace(/(\.png)?$/i, `-${shot}.png`);
}

/** Says what the page showed at a shot. */
function describe(stats) {
  if (stats.page === 'resident') return `page of resident ${stats.resident}, ${stats.chapters} chapters`;
  if (stats.page === 'clip') {
    return `clip ${stats.clip}, frame ${stats.frame} of ${stats.from} to ${stats.to}, ${stats.residents} residents in the zone`;
  }
  const picture =
    `level ${stats.level}, frame ${stats.frame}, ${stats.residents.drawn} residents drawn of ${stats.residents.world} in the world, ` +
    `${stats.labels.names} names, ${stats.labels.sites} site labels, ${stats.labels.bubbles} speech bubbles`;
  if (stats.broadcast === null) return picture;
  const show = stats.broadcast;
  return (
    `${picture}\n  broadcast: shot ${show.shot}${show.breaking ? ' (breaking news)' : ''}, caption "${show.caption}", ` +
    `card ${show.card} of [${show.cards.join(', ')}], ${show.stories} stories`
  );
}

try {
  const { page, close } = await openViewer(url, {
    width: number(args.width, 1920),
    height: number(args.height, 1080),
    timeoutMs: number(args.timeout, 30_000),
  });
  try {
    await page.waitForTimeout(number(args.settle, 1_500));
    if (typeof args.press === 'string') {
      await page.keyboard.press(args.press);
      // The page draws the change in its next frames.
      await page.waitForTimeout(300);
    }
    await mkdir(dirname(out), { recursive: true });
    for (let shot = 1; shot <= count; shot++) {
      if (shot > 1) await page.waitForTimeout(every);
      const file = fileOf(shot);
      let picture = await page.screenshot({ type: 'png' });
      const stats = await page.evaluate(() => window.__unsup?.stats());
      let form = 'full colour';
      if (picture.length > maxBytes) {
        picture = toPalette(picture);
        form = '256 colours';
      }
      await writeFile(file, picture);
      console.log(`${file} (${picture.length} bytes, ${form})`);
      console.log(describe(stats));
      if (picture.length > maxBytes) throw new Error(`${file} has ${picture.length} bytes: more than --max-bytes ${maxBytes}.`);
    }
  } finally {
    await close();
  }
} catch (error) {
  console.error(`No picture: ${error instanceof Error ? error.message : error}`);
  process.exit(1);
}
