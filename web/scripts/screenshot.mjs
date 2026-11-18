// Saves a picture of the viewer as a PNG file.
//
//   npm run screenshot -- --url http://127.0.0.1:8080 [--query "view=country"]
//       [--out shot.png] [--width 1920] [--height 1080] [--settle 1500]
//
// `--query` adds options to the address, for example `view=country`,
// `zone=3`, `watch=1048577` or `broadcast=1`. `--settle` is the time in
// milliseconds between the moment the picture is complete and the shot.

import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { number, readArgs } from './args.mjs';
import { openViewer, viewerUrl } from './browser.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string') {
  console.error('Give the address of the server: --url http://127.0.0.1:8080');
  process.exit(2);
}
const out = resolve(typeof args.out === 'string' ? args.out : 'screenshot.png');
const url = viewerUrl(args.url, typeof args.query === 'string' ? args.query : '');

try {
  const { page, close } = await openViewer(url, {
    width: number(args.width, 1920),
    height: number(args.height, 1080),
    timeoutMs: number(args.timeout, 30_000),
  });
  try {
    await page.waitForTimeout(number(args.settle, 1_500));
    await mkdir(dirname(out), { recursive: true });
    await page.screenshot({ path: out, type: 'png' });
    const stats = await page.evaluate(() => window.__unsup?.stats());
    console.log(`${out}`);
    console.log(
      `level ${stats.level}, frame ${stats.frame}, ${stats.residents.drawn} residents drawn of ${stats.residents.world} in the world, ` +
        `${stats.labels.names} names, ${stats.labels.sites} site labels, ${stats.labels.bubbles} speech bubbles`,
    );
  } finally {
    await close();
  }
} catch (error) {
  console.error(`No picture: ${error instanceof Error ? error.message : error}`);
  process.exit(1);
}
