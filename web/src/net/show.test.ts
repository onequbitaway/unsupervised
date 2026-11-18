import { describe, expect, it } from 'vitest';
import { parseServerMessage } from './messages';
import { readClip, readDirector, readPublished, readPublishedIndex, readPublishedKind, readShot, readStories, readStory } from './show';

const clock = { year: 1, day: 2, hour: 3, minute: 4 };

const shot = {
  seq: 3,
  frame: 40,
  clock,
  zone: 2,
  city: 0,
  pos: { x: 10, y: 20 },
  focus: { type: 'resident', id: 7, name: 'Ada' },
  zoom: 'close',
  caption: 'Bo stole $45.00 from Ada',
  breaking: true,
  event: '40-1',
  kind: 'crime.theft_committed',
  drama: 80,
  story: 4,
  label: 'Crime',
  headline: 'The bank job',
};

const story = {
  id: 4,
  kind: 'crime',
  label: 'Crime',
  headline: 'The bank job',
  latest: 'Ada was arrested',
  status: 'open',
  started: 40,
  started_clock: clock,
  updated: 55,
  updated_clock: clock,
  drama: 80,
  events: 2,
  residents: [{ id: 7, name: 'Ada' }],
  zone: 2,
  lead: '40-1',
  timeline: [{ event: '40-1', frame: 40, clock, kind: 'crime.theft_committed', drama: 80, line: 'Bo stole', zone: 2, actor: 8 }],
};

describe('readShot', () => {
  it('reads a shot of a resident with every field', () => {
    expect(readShot(shot)).toEqual(shot);
  });

  it('reads the three kinds of focus', () => {
    expect(readShot({ ...shot, focus: { type: 'none' } })?.focus).toEqual({ type: 'none' });
    expect(readShot({ ...shot, focus: { type: 'place', site: 9, name: 'First Bank', pos: { x: 1, y: 2 } } })?.focus).toEqual({
      type: 'place',
      site: 9,
      name: 'First Bank',
      pos: { x: 1, y: 2 },
    });
    expect(readShot({ ...shot, focus: { type: 'place', pos: { x: 1, y: 2 } } })?.focus).toEqual({
      type: 'place',
      pos: { x: 1, y: 2 },
    });
    expect(readShot({ ...shot, focus: { type: 'resident', id: 7 } })?.focus).toEqual({ type: 'resident', id: 7 });
  });

  it('leaves out what a wide view does not have', () => {
    const wide = readShot({ seq: 1, frame: 1, clock, focus: { type: 'none' }, zoom: 'country', caption: 'The whole country', breaking: false, drama: 0 });
    expect(wide).toEqual({ seq: 1, frame: 1, clock, focus: { type: 'none' }, zoom: 'country', caption: 'The whole country', breaking: false, drama: 0 });
  });

  it('refuses a shot that is not valid', () => {
    expect(readShot({ ...shot, zoom: 'orbit' })).toBeNull();
    expect(readShot({ ...shot, focus: { type: 'moon' } })).toBeNull();
    expect(readShot({ ...shot, focus: { type: 'resident', id: 'seven' } })).toBeNull();
    expect(readShot({ ...shot, focus: { type: 'place' } })).toBeNull();
    expect(readShot({ ...shot, seq: '3' })).toBeNull();
    expect(readShot({ ...shot, caption: 5 })).toBeNull();
    expect(readShot(null)).toBeNull();
  });

  it('reads the answer of the director route', () => {
    expect(readDirector({ frame: 41, shot })).toEqual(shot);
    expect(readDirector({ frame: 41 })).toBeNull();
  });
});

describe('readStory', () => {
  it('reads a story with its residents and its timeline', () => {
    expect(readStory(story)).toEqual(story);
    expect(readStories({ frame: 60, stories: [story] })).toEqual([story]);
  });

  it('reads a closed story, and an open one for any other status', () => {
    expect(readStory({ ...story, status: 'closed' })?.status).toBe('closed');
    expect(readStory({ ...story, status: 'paused' })?.status).toBe('open');
  });

  it('refuses a story that is not valid, and a list with one', () => {
    expect(readStory({ ...story, id: 'four' })).toBeNull();
    expect(readStory({ ...story, timeline: [{ frame: 1 }] })).toBeNull();
    expect(readStory({ ...story, residents: 'many' })).toBeNull();
    expect(readStories({ stories: [story, { id: 5 }] })).toBeNull();
    expect(readStories('stories')).toBeNull();
  });
});

describe('the show on the feed', () => {
  const parse = (value: unknown) => parseServerMessage(JSON.stringify(value));

  it('reads a director message', () => {
    expect(parse({ op: 'director', shot })).toEqual({ op: 'director', shot });
    expect(parse({ op: 'director', shot: { ...shot, zoom: 'orbit' } })).toBeNull();
    expect(parse({ op: 'director' })).toBeNull();
  });

  it('reads a stories message, with and without a reset', () => {
    expect(parse({ op: 'stories', stories: [story] })).toEqual({ op: 'stories', stories: [story], reset: false });
    expect(parse({ op: 'stories', stories: [], reset: true })).toEqual({ op: 'stories', stories: [], reset: true });
    expect(parse({ op: 'stories', stories: [{ id: 1 }] })).toBeNull();
  });
});

describe('readClip', () => {
  const event = { id: '5-0', frame: 5, clock, kind: 'crime.theft_committed', drama: 55, zone: 2, actor: 8, data: null };
  const clip = {
    id: '5-0',
    event,
    line: 'Bo stole $45.00 from Ada',
    zone: 2,
    from: 3,
    to: 8,
    complete: true,
    pinned: false,
    residents: [{ id: 7, name: 'Ada', pos: { x: 10, y: 4 }, activity: null, model: null }],
    frames: [
      { frame: 4, clock, tick: 4, moved: [[7, 11, 4]], entered: [], left: [], activity: [[7, 'walk']], events: [] },
      { frame: 5, clock, tick: 5, moved: [], entered: [], left: [9], activity: [[7, null]], events: [event] },
    ],
    link: '/?clip=5-0',
  };

  it('reads a clip with its residents and its frames', () => {
    expect(readClip(clip)).toEqual(clip);
  });

  it('fills the lists that a frame leaves out', () => {
    const bare = readClip({ ...clip, frames: [{ frame: 6, clock, tick: 6 }] });
    expect(bare?.frames).toEqual([{ frame: 6, clock, tick: 6, moved: [], entered: [], left: [], activity: [], events: [] }]);
  });

  it('refuses a clip that is not valid', () => {
    expect(readClip({ ...clip, zone: 'two' })).toBeNull();
    expect(readClip({ ...clip, event: { id: 5 } })).toBeNull();
    expect(readClip({ ...clip, frames: [{ clock }] })).toBeNull();
    expect(readClip({ ...clip, frames: [{ frame: 4, clock, moved: [['a', 1, 2]] }] })).toBeNull();
    expect(readClip({ ...clip, residents: [{ id: 7 }] })).toBeNull();
    expect(readClip(undefined)).toBeNull();
  });
});

describe('published state', () => {
  it('reads one series, with any data', () => {
    expect(readPublished({ kind: 'missions.leaderboard_published', key: 'wealth', frame: 9, clock, data: { rows: [] } })).toEqual({
      kind: 'missions.leaderboard_published',
      key: 'wealth',
      frame: 9,
      clock,
      data: { rows: [] },
    });
    expect(readPublished({ kind: 'finance.stats_published', frame: 9, clock })).toEqual({
      kind: 'finance.stats_published',
      frame: 9,
      clock,
      data: null,
    });
    expect(readPublished({ kind: 'x_published', clock })).toBeNull();
  });

  it('reads the series of a kind and the list of all series', () => {
    const series = { kind: 'finance.stats_published', frame: 9, clock, data: 1 };
    expect(readPublishedKind({ kind: 'finance.stats_published', series: [series] })).toEqual([series]);
    expect(readPublishedKind({ series: [{ frame: 9 }] })).toBeNull();
    expect(readPublishedIndex({ published: [{ kind: 'a_published', frame: 3 }, { kind: 'b_published', key: 'k', frame: 4 }] })).toEqual([
      { kind: 'a_published', frame: 3 },
      { kind: 'b_published', key: 'k', frame: 4 },
    ]);
    expect(readPublishedIndex({ published: [{ key: 'k' }] })).toBeNull();
  });
});
