// Tests of the tally of the director: `node --test scripts/`.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { pictureOf, ShotTally } from './shots.mjs';

/** A sample of `GET /v1/director`. */
function sample(frame, seq, more = {}) {
  const shot = {
    seq,
    frame: more.start ?? frame,
    zoom: 'city',
    focus: { type: 'none' },
    caption: 'Latent City · 68 residents',
    breaking: false,
    drama: 0,
    ...more.shot,
  };
  return { frame, shot };
}

const arrest = {
  zoom: 'close',
  focus: { type: 'resident', id: 7, name: 'Otto' },
  caption: 'Otto arrested Sly',
  kind: 'crime.arrest_made',
  drama: 55,
  label: 'Crime',
  headline: 'A theft took $42.50 in Old Town',
};

test('a shot is breaking, wide, close or street', () => {
  assert.equal(pictureOf(sample(1, 1).shot), 'wide');
  assert.equal(pictureOf(sample(1, 1, { shot: { zoom: 'country' } }).shot), 'wide');
  assert.equal(pictureOf(sample(1, 2, { shot: arrest }).shot), 'close');
  assert.equal(pictureOf(sample(1, 2, { shot: { ...arrest, zoom: 'street' } }).shot), 'street');
  // Breaking news wins, also without a subject.
  assert.equal(pictureOf(sample(1, 3, { shot: { breaking: true } }).shot), 'breaking');
  assert.equal(pictureOf(sample(1, 3, { shot: { ...arrest, breaking: true } }).shot), 'breaking');
});

test('a sample counts for the frames since the sample before it', () => {
  const tally = new ShotTally();
  tally.add(sample(100, 1));
  // The same frame again adds nothing.
  tally.add(sample(100, 1));
  tally.add(sample(110, 1));
  tally.add(sample(130, 2, { start: 125, shot: arrest }));
  tally.add(sample(140, 2, { start: 125, shot: arrest }));
  tally.add(sample(150, 3, { start: 148 }));
  assert.equal(tally.frames, 51);
  assert.deepEqual(tally.pictures, { wide: 21, street: 0, close: 30, breaking: 0 });
  assert.deepEqual(tally.zooms, { city: 21, close: 30 });
  assert.equal(tally.share('close'), 30 / 51);
  assert.deepEqual(
    tally.list().map((shot) => [shot.seq, shot.frame, shot.picture, shot.frames]),
    [
      [1, 100, 'wide', 11],
      [2, 125, 'close', 30],
      [3, 148, 'wide', 10],
    ],
  );
  assert.deepEqual(tally.kinds(), { '(wide view)': 21, 'crime.arrest_made': 30 });
  assert.equal(tally.summary()[0], '51 frames sampled, 3 shots');
});

test('the words of a shot that change are kept one time each', () => {
  const tally = new ShotTally();
  tally.add(sample(10, 4, { shot: arrest }));
  tally.add(sample(11, 4, { start: 10, shot: arrest }));
  tally.add(sample(12, 4, { start: 10, shot: { ...arrest, caption: 'Sly was charged with theft', kind: 'crime.suspect_charged', drama: 45 } }));
  const [shot] = tally.list();
  assert.deepEqual(
    shot.captions.map((words) => [words.caption, words.kind, words.drama, words.label]),
    [
      ['Otto arrested Sly', 'crime.arrest_made', 55, 'Crime'],
      ['Sly was charged with theft', 'crime.suspect_charged', 45, 'Crime'],
    ],
  );
  assert.equal(shot.frames, 3);
});

test('the empty shot before the first frame is not counted', () => {
  const tally = new ShotTally();
  tally.add(sample(0, 0, { shot: { caption: '', zoom: 'country' } }));
  tally.add(sample(0, 0, { shot: { caption: '', zoom: 'country' } }));
  assert.equal(tally.frames, 0);
  assert.equal(tally.share('wide'), 0);
  assert.deepEqual(tally.list(), []);
  tally.add(sample(3, 1));
  assert.equal(tally.frames, 3);
});
