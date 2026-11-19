// Samples the director and the stories of a running server, and prints how
// often each kind of picture was on air.
//
//   node scripts/director-log.mjs --url http://127.0.0.1:8080 [--seconds 600]
//       [--every 100] [--stories-every 5000] [--out director-log.json]
//
// `--seconds` is the real time of the run; Ctrl-C ends it earlier. `--every`
// is the time between two samples of `GET /v1/director` in milliseconds:
// use the real length of one game minute of the server, so that no shot is
// lost. `--stories-every` is the time between two reads of
// `GET /v1/stories`. `--out` gets the samples as JSON: the tally, each
// shot with its captions, and the stories at the end.
//
// The summary has four kinds of picture: `wide` (no subject: a city or the
// country), `street` and `close` (a resident or a place), and `breaking`
// (breaking news, with every zoom).

import { writeFile } from 'node:fs/promises';
import { clockText, connect, sleep } from './api.mjs';
import { number, readArgs } from './args.mjs';
import { ShotTally } from './shots.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string') {
  console.error('Give the address of the server: --url http://127.0.0.1:8080');
  process.exit(2);
}
const call = connect(args.url);
const seconds = number(args.seconds, 600);
const every = number(args.every, 100);
const storiesEvery = number(args['stories-every'], 5_000);

const tally = new ShotTally();
/** The greatest number of stories that one read showed, and the stories of the last read. */
let storiesPeak = 0;
let stories = [];
let lastSeq = -1;
let stop = false;
process.on('SIGINT', () => (stop = true));
process.on('SIGTERM', () => (stop = true));

const end = Date.now() + seconds * 1000;
let nextStories = 0;
while (!stop && Date.now() < end) {
  const started = Date.now();
  try {
    const state = await call('GET', '/v1/director', null);
    tally.add(state);
    const shot = state.shot;
    if (shot.seq !== lastSeq && shot.seq > 0) {
      lastSeq = shot.seq;
      const subject = shot.focus.type === 'none' ? '' : ` ${shot.focus.type} ${shot.focus.name ?? shot.focus.id ?? ''}`;
      console.log(
        `[${clockText(shot.clock)}] shot ${shot.seq}: ${shot.zoom}${shot.breaking ? ' BREAKING' : ''}${subject} | ` +
          `${shot.caption}${shot.kind ? ` (${shot.kind}, drama ${shot.drama})` : ''}${shot.label ? ` | ${shot.label}: ${shot.headline}` : ''}`,
      );
    }
    if (started >= nextStories) {
      nextStories = started + storiesEvery;
      stories = (await call('GET', '/v1/stories?status=all&limit=128', null)).stories;
      storiesPeak = Math.max(storiesPeak, stories.length);
    }
  } catch (error) {
    // The server stopped, or it is not ready yet.
    console.error(`no sample: ${error instanceof Error ? error.message : error}`);
    await sleep(1_000);
  }
  await sleep(Math.max(0, every - (Date.now() - started)));
}

console.log(tally.summary().join('\n'));
console.log(`  captions by kind of event (frames): ${JSON.stringify(tally.kinds())}`);
console.log(`stories at the end: ${stories.length} (${storiesPeak} at the peak)`);
for (const story of stories) {
  console.log(`  ${story.label} [${story.status}] drama ${story.drama}, ${story.events} events: ${story.headline}`);
}
if (typeof args.out === 'string') {
  const report = {
    frames: tally.frames,
    pictures: tally.pictures,
    zooms: tally.zooms,
    kinds: tally.kinds(),
    shots: tally.list(),
    stories,
  };
  await writeFile(args.out, `${JSON.stringify(report, null, 2)}\n`);
  console.log(`report: ${args.out}`);
}
