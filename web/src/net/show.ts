// Reads the JSON of the show: the shot of the director, the stories, the
// life story, the clips and the published state. Anything that does not
// have the documented shape is refused. The text inside stays untrusted.

import type {
  Clip,
  ClipFrame,
  Focus,
  LifeEntry,
  LifeStory,
  Published,
  PublishedRef,
  Shot,
  Story,
  StoryBeat,
  Zoom,
} from '../state/types';
import { activity, clock, int, isObject, list, moved, pos, readEvent, resident, text } from './read';

const ZOOMS: readonly Zoom[] = ['country', 'city', 'street', 'close'];

function focus(value: unknown): Focus | null {
  if (!isObject(value)) return null;
  switch (value.type) {
    case 'none':
      return { type: 'none' };
    case 'resident': {
      const id = int(value.id);
      if (id === null) return null;
      const name = text(value.name);
      return name === null ? { type: 'resident', id } : { type: 'resident', id, name };
    }
    case 'place': {
      const at = pos(value.pos);
      if (at === null) return null;
      const place: Focus = { type: 'place', pos: at };
      const site = int(value.site);
      if (site !== null) place.site = site;
      const name = text(value.name);
      if (name !== null) place.name = name;
      return place;
    }
    default:
      return null;
  }
}

/** Reads a shot of the director. */
export function readShot(value: unknown): Shot | null {
  if (!isObject(value)) return null;
  const seq = int(value.seq);
  const frame = int(value.frame);
  const at = clock(value.clock);
  const on = focus(value.focus);
  const caption = text(value.caption);
  const zoom = ZOOMS.find((known) => known === value.zoom);
  if (seq === null || frame === null || at === null || on === null || caption === null || zoom === undefined) {
    return null;
  }
  const shot: Shot = {
    seq,
    frame,
    clock: at,
    focus: on,
    zoom,
    caption,
    breaking: value.breaking === true,
    drama: int(value.drama) ?? 0,
  };
  const zone = int(value.zone);
  if (zone !== null) shot.zone = zone;
  const city = int(value.city);
  if (city !== null) shot.city = city;
  const where = pos(value.pos);
  if (where !== null) shot.pos = where;
  const event = text(value.event);
  if (event !== null) shot.event = event;
  const kind = text(value.kind);
  if (kind !== null) shot.kind = kind;
  const story = int(value.story);
  if (story !== null) shot.story = story;
  const label = text(value.label);
  if (label !== null) shot.label = label;
  const headline = text(value.headline);
  if (headline !== null) shot.headline = headline;
  return shot;
}

/** Reads the answer of `/v1/director`. */
export function readDirector(value: unknown): Shot | null {
  return isObject(value) ? readShot(value.shot) : null;
}

function beat(value: unknown): StoryBeat | null {
  if (!isObject(value)) return null;
  const event = text(value.event);
  const frame = int(value.frame);
  const at = clock(value.clock);
  const line = text(value.line);
  if (event === null || frame === null || at === null || line === null) return null;
  const entry: StoryBeat = { event, frame, clock: at, kind: text(value.kind) ?? '', drama: int(value.drama) ?? 0, line };
  const zone = int(value.zone);
  if (zone !== null) entry.zone = zone;
  const actor = int(value.actor);
  if (actor !== null) entry.actor = actor;
  return entry;
}

function storyResident(value: unknown): { id: number; name: string } | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  return id === null ? null : { id, name: text(value.name) ?? '' };
}

/** Reads a story. */
export function readStory(value: unknown): Story | null {
  if (!isObject(value)) return null;
  const id = int(value.id);
  const kind = text(value.kind);
  const headline = text(value.headline);
  const started = int(value.started);
  const updated = int(value.updated);
  const startedClock = clock(value.started_clock);
  const updatedClock = clock(value.updated_clock);
  const residents = list(value.residents, storyResident);
  const timeline = list(value.timeline, beat);
  if (id === null || kind === null || headline === null || started === null || updated === null) return null;
  if (startedClock === null || updatedClock === null || residents === null || timeline === null) return null;
  const story: Story = {
    id,
    kind,
    label: text(value.label) ?? kind,
    headline,
    latest: text(value.latest) ?? '',
    status: value.status === 'closed' ? 'closed' : 'open',
    started,
    started_clock: startedClock,
    updated,
    updated_clock: updatedClock,
    drama: int(value.drama) ?? 0,
    events: int(value.events) ?? timeline.length,
    residents,
    lead: text(value.lead) ?? '',
    timeline,
  };
  const zone = int(value.zone);
  if (zone !== null) story.zone = zone;
  return story;
}

/** Reads the answer of `/v1/stories`. */
export function readStories(value: unknown): Story[] | null {
  return isObject(value) ? list(value.stories, readStory) : null;
}

function lifeEntry(value: unknown): LifeEntry | null {
  if (!isObject(value)) return null;
  const event = text(value.event);
  const frame = int(value.frame);
  const at = clock(value.clock);
  const line = text(value.line);
  if (event === null || frame === null || at === null || line === null) return null;
  const entry: LifeEntry = { event, frame, clock: at, kind: text(value.kind) ?? '', drama: int(value.drama) ?? 0, line };
  const zone = int(value.zone);
  if (zone !== null) entry.zone = zone;
  const site = int(value.site);
  if (site !== null) entry.site = site;
  return entry;
}

/** Reads a life story. A value that is not one gives an empty life story. */
export function readLife(value: unknown): LifeStory {
  const source = isObject(value) ? value : {};
  // One bad entry does not take the rest of a chapter away.
  const chapter = (entries: unknown): LifeEntry[] =>
    Array.isArray(entries) ? entries.map(lifeEntry).filter((entry): entry is LifeEntry => entry !== null) : [];
  const life: LifeStory = {
    jobs: chapter(source.jobs),
    homes: chapter(source.homes),
    deals: chapter(source.deals),
    crimes: chapter(source.crimes),
    offices: chapter(source.offices),
    moments: chapter(source.moments),
  };
  const born = lifeEntry(source.born);
  if (born !== null) life.born = born;
  const death = lifeEntry(source.death);
  if (death !== null) life.death = death;
  const age = int(source.age);
  if (age !== null) life.age = age;
  return life;
}

function clipFrame(value: unknown): ClipFrame | null {
  if (!isObject(value)) return null;
  const frame = int(value.frame);
  const at = clock(value.clock);
  const lists = {
    moved: list(value.moved, moved),
    entered: list(value.entered, resident),
    left: list(value.left, int),
    activity: list(value.activity, activity),
    events: list(value.events, readEvent),
  };
  if (frame === null || at === null) return null;
  if (!lists.moved || !lists.entered || !lists.left || !lists.activity || !lists.events) return null;
  return { frame, clock: at, tick: int(value.tick) ?? 0, ...lists } as ClipFrame;
}

/** Reads the answer of `/v1/clips/{event}`. */
export function readClip(value: unknown): Clip | null {
  if (!isObject(value)) return null;
  const id = text(value.id);
  const event = readEvent(value.event);
  const zone = int(value.zone);
  const from = int(value.from);
  const to = int(value.to);
  const residents = list(value.residents, resident);
  const frames = list(value.frames, clipFrame);
  if (id === null || event === null || zone === null || from === null || to === null) return null;
  if (residents === null || frames === null) return null;
  return {
    id,
    event,
    line: text(value.line) ?? '',
    zone,
    from,
    to,
    complete: value.complete === true,
    pinned: value.pinned === true,
    residents,
    frames,
    link: text(value.link) ?? '',
  };
}

/** Reads one series of published state, for example a leaderboard. */
export function readPublished(value: unknown): Published | null {
  if (!isObject(value)) return null;
  const kind = text(value.kind);
  const frame = int(value.frame);
  const at = clock(value.clock);
  if (kind === null || frame === null || at === null) return null;
  const published: Published = { kind, frame, clock: at, data: value.data ?? null };
  const key = text(value.key);
  if (key !== null) published.key = key;
  return published;
}

/** Reads the answer of `/v1/stats/economy`: the series of the statistics. */
export function readPublishedKind(value: unknown): Published[] | null {
  return isObject(value) ? list(value.series, readPublished) : null;
}

function publishedRef(value: unknown): PublishedRef | null {
  if (!isObject(value)) return null;
  const kind = text(value.kind);
  const frame = int(value.frame);
  if (kind === null || frame === null) return null;
  const ref: PublishedRef = { kind, frame };
  const key = text(value.key);
  if (key !== null) ref.key = key;
  return ref;
}

/** Reads the answer of `/v1/published`. */
export function readPublishedIndex(value: unknown): PublishedRef[] | null {
  return isObject(value) ? list(value.published, publishedRef) : null;
}
