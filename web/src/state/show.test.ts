import { describe, expect, it } from 'vitest';
import { CONFIG } from '../config';
import { electionsToday, emptyShow, reduceShow, sameDay, sortStories } from './show';
import type { ServerMessage, Shot, Story } from './types';

const clock = { year: 1, day: 3, hour: 9, minute: 30 };

export function shot(seq: number, extra: Partial<Shot> = {}): Shot {
  return { seq, frame: seq * 10, clock, focus: { type: 'none' }, zoom: 'country', caption: `shot ${seq}`, breaking: false, drama: 0, ...extra };
}

export function story(id: number, extra: Partial<Story> = {}): Story {
  return {
    id,
    kind: 'crime',
    label: 'Crime',
    headline: `story ${id}`,
    latest: `latest ${id}`,
    status: 'open',
    started: id,
    started_clock: clock,
    updated: id,
    updated_clock: clock,
    drama: 50,
    events: 1,
    residents: [],
    lead: `${id}-0`,
    timeline: [],
    ...extra,
  };
}

const director = (value: Shot): ServerMessage => ({ op: 'director', shot: value });
const stories = (list: Story[], reset = false): ServerMessage => ({ op: 'stories', stories: list, reset });

describe('reduceShow', () => {
  it('starts without a shot and without stories', () => {
    const show = emptyShow();
    expect(show.shot).toBeNull();
    expect(show.stories.size).toBe(0);
  });

  it('takes a new shot and remembers when it came', () => {
    let show = reduceShow(emptyShow(), director(shot(4)), 1_000);
    expect([show.shot?.seq, show.shotAt, show.wordsAt]).toEqual([4, 1_000, 1_000]);
    // The same number: new words, the same start.
    show = reduceShow(show, director(shot(4, { caption: 'new words' })), 3_000);
    expect([show.shot?.caption, show.shotAt, show.wordsAt]).toEqual(['new words', 1_000, 3_000]);
    // Another number: a cut.
    show = reduceShow(show, director(shot(5)), 7_000);
    expect([show.shot?.seq, show.shotAt]).toEqual([5, 7_000]);
  });

  it('adds and changes stories, and forgets the others after a reset', () => {
    let show = reduceShow(emptyShow(), stories([story(1), story(2)]), 0);
    show = reduceShow(show, stories([story(2, { headline: 'changed' }), story(3)]), 0);
    expect([...show.stories.keys()]).toEqual([1, 2, 3]);
    expect(show.stories.get(2)?.headline).toBe('changed');
    show = reduceShow(show, stories([story(9)], true), 0);
    expect([...show.stories.keys()]).toEqual([9]);
  });

  it('does not change for other messages, and never changes its input', () => {
    const before = reduceShow(emptyShow(), stories([story(1)]), 0);
    expect(reduceShow(before, { op: 'pong' }, 5)).toBe(before);
    const after = reduceShow(before, stories([story(2)]), 5);
    expect(before.stories.size).toBe(1);
    expect(after.stories.size).toBe(2);
  });

  it('keeps a bounded number of stories, the open and the new ones', () => {
    const max = CONFIG.broadcast.maxStories;
    const many = Array.from({ length: max + 5 }, (_, i) => story(i + 1, { updated: i + 1 }));
    many[0] = story(1, { updated: 1, status: 'closed' });
    const show = reduceShow(emptyShow(), stories(many), 0);
    expect(show.stories.size).toBe(max);
    // The closed story and the open stories with the oldest events left.
    expect(show.stories.has(1)).toBe(false);
    expect(show.stories.has(2)).toBe(false);
    expect(show.stories.has(max + 5)).toBe(true);
  });
});

describe('stories on the screen', () => {
  it('puts open stories first, then the newest event first', () => {
    const list = [
      story(1, { updated: 50, status: 'closed' }),
      story(2, { updated: 10 }),
      story(3, { updated: 30 }),
      story(4, { updated: 30 }),
    ];
    expect(sortStories(list).map((entry) => entry.id)).toEqual([4, 3, 2, 1]);
  });

  it('finds the elections of the game day', () => {
    const today = { year: 2, day: 7, hour: 20, minute: 0 };
    const list = [
      story(1, { kind: 'election', updated_clock: { year: 2, day: 7, hour: 1, minute: 5 }, updated: 5 }),
      story(2, { kind: 'election', updated_clock: { year: 2, day: 6, hour: 23, minute: 59 } }),
      story(3, { kind: 'crime', updated_clock: today }),
      story(4, { kind: 'election', updated_clock: today, updated: 9 }),
      story(5, { kind: 'election', updated_clock: { year: 1, day: 7, hour: 1, minute: 5 } }),
    ];
    expect(electionsToday(list, today).map((entry) => entry.id)).toEqual([4, 1]);
    expect(electionsToday(list, null)).toEqual([]);
    expect(sameDay(today, { ...today, hour: 0 })).toBe(true);
    expect(sameDay(today, { ...today, day: 8 })).toBe(false);
  });
});
