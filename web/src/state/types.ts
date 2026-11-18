// Wire types of the viewer feed. See docs/design/protocol.md, "Viewer feed".

export interface Pos {
  x: number;
  y: number;
}

/** Inclusive minimum, exclusive maximum. */
export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface Clock {
  year: number;
  day: number;
  hour: number;
  minute: number;
}

export interface CityView {
  id: number;
  name: string;
  bounds: Rect;
  zones: number[];
}

export interface ZoneView {
  id: number;
  city: number;
  name: string;
  bounds: Rect;
  residents: number;
}

export interface SiteView {
  id: number;
  kind: string;
  name: string;
  rect: Rect;
  tags: string[];
}

export interface ResidentView {
  id: number;
  name: string;
  pos: Pos;
  activity?: string | null;
  model?: string | null;
}

/** The answer of `/v1/world`. */
export interface Layout {
  frame: number;
  clock: Clock;
  cities: CityView[];
  zones: ZoneView[];
}

/** The answer of `/v1/world/zones/{zone}`, without the residents. */
export interface ZoneMap {
  zone: ZoneView;
  width: number;
  height: number;
  tiles: Uint8Array;
  sites: SiteView[];
}

export interface FeedEvent {
  id: string;
  frame: number;
  clock: Clock;
  zone?: number;
  kind: string;
  actor?: number;
  actor_name?: string;
  pos?: Pos;
  drama: number;
  /** The payload. It can hold text that a resident wrote. */
  data: unknown;
}

export type ServerMessage =
  | {
      op: 'world';
      protocol: number;
      frame: number;
      clock: Clock;
      populations: [number, number][];
      dropped: number;
      viewers: number;
    }
  | { op: 'snapshot'; zone: number; frame: number; residents: ResidentView[] }
  | {
      op: 'delta';
      frame: number;
      zone: number;
      tick: number;
      moved: [number, number, number][];
      entered: ResidentView[];
      left: number[];
      activity: [number, string | null][];
      events: FeedEvent[];
    }
  | {
      op: 'frame';
      frame: number;
      clock: Clock;
      populations: [number, number][];
      headlines: FeedEvent[];
      viewers: number;
    }
  | { op: 'director'; shot: Shot }
  | { op: 'stories'; stories: Story[]; reset: boolean }
  | { op: 'unsubscribed'; zones: number[] }
  | { op: 'error'; code: string; message: string }
  | { op: 'pong' };

/** A channel of the show on the feed. */
export type Channel = 'director' | 'stories';

export type ClientMessage =
  | { op: 'subscribe'; zones: number[] }
  | { op: 'unsubscribe'; zones: number[] }
  | { op: 'tune'; channels: Channel[] }
  | { op: 'ping' };

/** The answer of `/v1/feed`. */
export interface FeedPage {
  events: FeedEvent[];
  next: string;
  more: boolean;
  gap: boolean;
}

/** The answer of `/v1/residents/{id}/profile`. */
export interface Profile {
  id: number;
  name: string;
  model?: string;
  alive: boolean;
  zone: number;
  zone_name: string;
  city: number;
  city_name: string;
  pos: Pos;
  activity?: string;
  frame: number;
  cause?: string;
  events: FeedEvent[];
  /** The life story from the chronicle. */
  life: LifeStory;
}

/** How near the camera of the director is. */
export type Zoom = 'country' | 'city' | 'street' | 'close';

/** What the camera of the director looks at. */
export type Focus =
  | { type: 'none' }
  | { type: 'resident'; id: number; name?: string }
  | { type: 'place'; site?: number; name?: string; pos: Pos };

/** The choice of the director: where the camera looks, and why. */
export interface Shot {
  /** It grows by one with each cut. The same number changes the words only. */
  seq: number;
  frame: number;
  clock: Clock;
  zone?: number;
  city?: number;
  pos?: Pos;
  focus: Focus;
  zoom: Zoom;
  /** One line about the shot. It can hold names that residents chose. */
  caption: string;
  breaking: boolean;
  event?: string;
  kind?: string;
  drama: number;
  story?: number;
  label?: string;
  headline?: string;
}

export interface StoryBeat {
  event: string;
  frame: number;
  clock: Clock;
  kind: string;
  drama: number;
  /** One line about the event. Untrusted: it holds names. */
  line: string;
  zone?: number;
  actor?: number;
}

/** A running story: events that belong together. */
export interface Story {
  id: number;
  kind: string;
  label: string;
  headline: string;
  latest: string;
  status: 'open' | 'closed';
  started: number;
  started_clock: Clock;
  updated: number;
  updated_clock: Clock;
  drama: number;
  events: number;
  residents: { id: number; name: string }[];
  zone?: number;
  lead: string;
  timeline: StoryBeat[];
}

/** One event of the chronicle. */
export interface LifeEntry {
  event: string;
  frame: number;
  clock: Clock;
  kind: string;
  drama: number;
  line: string;
  zone?: number;
  site?: number;
}

/** The life story of a resident, by chapter. Each list is oldest first. */
export interface LifeStory {
  born?: LifeEntry;
  jobs: LifeEntry[];
  homes: LifeEntry[];
  deals: LifeEntry[];
  crimes: LifeEntry[];
  offices: LifeEntry[];
  moments: LifeEntry[];
  death?: LifeEntry;
  age?: number;
}

/** One frame of a clip: the lists of a `delta` message. */
export interface ClipFrame {
  frame: number;
  clock: Clock;
  tick: number;
  moved: [number, number, number][];
  entered: ResidentView[];
  left: number[];
  activity: [number, string | null][];
  events: FeedEvent[];
}

/** The answer of `/v1/clips/{event}`: the frames of one zone around one event. */
export interface Clip {
  id: string;
  event: FeedEvent;
  line: string;
  zone: number;
  from: number;
  to: number;
  complete: boolean;
  pinned: boolean;
  residents: ResidentView[];
  frames: ClipFrame[];
  link: string;
}

/** One series of published state. */
export interface Published {
  kind: string;
  key?: string;
  frame: number;
  clock: Clock;
  data: unknown;
}

/** One line of `/v1/published`. */
export interface PublishedRef {
  kind: string;
  key?: string;
  frame: number;
}
