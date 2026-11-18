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
  | { op: 'unsubscribed'; zones: number[] }
  | { op: 'error'; code: string; message: string }
  | { op: 'pong' };

export type ClientMessage =
  | { op: 'subscribe'; zones: number[] }
  | { op: 'unsubscribe'; zones: number[] }
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
}
