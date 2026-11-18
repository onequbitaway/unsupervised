// Measures how smooth the viewer is: the time between two frames of the
// screen, and the time that the viewer works for one frame.
//
//   npm run perf -- --url http://127.0.0.1:8090 [--query "view=street&zone=0"]
//       [--seconds 10] [--width 1920] [--height 1080] [--throttle 4] [--out shot.png]
//
// `--throttle 4` makes the processor of the page four times slower, to
// get an idea of a weaker machine.

import { resolve } from 'node:path';
import { number, readArgs } from './args.mjs';
import { openViewer, viewerUrl } from './browser.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string') {
  console.error('Give the address of the server: --url http://127.0.0.1:8090');
  process.exit(2);
}
const seconds = number(args.seconds, 10);
const url = viewerUrl(args.url, typeof args.query === 'string' ? args.query : '');

try {
  const { page, close } = await openViewer(url, {
    width: number(args.width, 1920),
    height: number(args.height, 1080),
    timeoutMs: number(args.timeout, 60_000),
  });
  try {
    const gpu = await page.evaluate(() => {
      const gl = document.createElement('canvas').getContext('webgl2');
      const info = gl?.getExtension('WEBGL_debug_renderer_info');
      return gl && info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : 'unknown';
    });
    const throttle = number(args.throttle, 1);
    if (throttle > 1) {
      const session = await page.context().newCDPSession(page);
      await session.send('Emulation.setCPUThrottlingRate', { rate: throttle });
    }
    await page.waitForTimeout(1_000);
    await page.evaluate(() => window.__unsup?.resetStats());
    await page.waitForTimeout(seconds * 1_000);
    const stats = await page.evaluate(() => window.__unsup?.stats());
    if (typeof args.out === 'string') await page.screenshot({ path: resolve(args.out), type: 'png' });
    const ms = (value) => value.toFixed(2);
    console.log(`renderer: ${gpu}${throttle > 1 ? `, processor ${throttle} times slower` : ''}`);
    console.log(`level ${stats.level}, ${stats.scale.toFixed(1)} pixels for one tile, ${stats.zones} zones subscribed`);
    console.log(
      `residents: ${stats.residents.drawn} drawn, ${stats.residents.known} known, ${stats.residents.world} in the world; ` +
        `${stats.labels.names} names, ${stats.labels.sites} site labels, ${stats.labels.bubbles} speech bubbles`,
    );
    console.log(
      `time between frames over ${stats.frameMs.count} frames: mean ${ms(stats.frameMs.mean)} ms ` +
        `(${(1000 / stats.frameMs.mean).toFixed(1)} frames per second), 95% below ${ms(stats.frameMs.p95)} ms, worst ${ms(stats.frameMs.max)} ms`,
    );
    console.log(
      `work for one frame: mean ${ms(stats.workMs.mean)} ms, 95% below ${ms(stats.workMs.p95)} ms, worst ${ms(stats.workMs.max)} ms`,
    );
  } finally {
    await close();
  }
} catch (error) {
  console.error(`No measurement: ${error instanceof Error ? error.message : error}`);
  process.exit(1);
}
