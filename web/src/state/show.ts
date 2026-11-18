// The show as data: the shot of the director and the stories. A pure
// reducer applies the messages of the feed.

import { CONFIG } from '../config';
import type { Clock, ServerMessage, Shot, Story } from './types';

export interface ShowState {
  /** The shot on air, or null before the first `director` message. */
  shot: Shot | null;
  /** When the shot on air started for this viewer, in milliseconds. */
  shotAt: number;
  /** When the words of the shot last changed, in milliseconds. */
  wordsAt: number;
  /** The stories that the viewer knows, by id. */
  stories: ReadonlyMap<number, Story>;
}

export function emptyShow(): ShowState {
  return { shot: null, shotAt: 0, wordsAt: 0, stories: new Map() };
}

/** Keeps the newest stories when there are too many: open ones first. */
function bounded(stories: Map<number, Story>): Map<number, Story> {
  const max = CONFIG.broadcast.maxStories;
  if (stories.size <= max) return stories;
  const keep = sortStories([...stories.values()]).slice(0, max);
  return new Map(keep.map((story) => [story.id, story]));
}

/**
 * Applies one message of the feed to the show. Other messages change
 * nothing. `now` is the time at which the message came.
 */
export function reduceShow(state: ShowState, message: ServerMessage, now: number): ShowState {
  switch (message.op) {
    case 'director': {
      // The same number is the same shot with new words: the camera stays.
      const cut = state.shot === null || state.shot.seq !== message.shot.seq;
      return { ...state, shot: message.shot, shotAt: cut ? now : state.shotAt, wordsAt: now };
    }
    case 'stories': {
      const stories = message.reset ? new Map<number, Story>() : new Map(state.stories);
      for (const story of message.stories) stories.set(story.id, story);
      return { ...state, stories: bounded(stories) };
    }
    default:
      return state;
  }
}

/** Orders stories for the screen: open ones first, then the newest event first. */
export function sortStories(stories: readonly Story[]): Story[] {
  const rank = (story: Story): number => (story.status === 'open' ? 0 : 1);
  return [...stories].sort((a, b) => rank(a) - rank(b) || b.updated - a.updated || b.id - a.id);
}

/** Returns true when two calendar times are in the same game day. */
export function sameDay(a: Clock, b: Clock): boolean {
  return a.year === b.year && a.day === b.day;
}

/**
 * Returns the election stories of today: the stories of the kind
 * `election` whose newest event is in the game day of `clock`. An election
 * day is a day with such a story.
 */
export function electionsToday(stories: Iterable<Story>, clock: Clock | null): Story[] {
  if (clock === null) return [];
  return sortStories([...stories].filter((story) => story.kind === 'election' && sameDay(story.updated_clock, clock)));
}
