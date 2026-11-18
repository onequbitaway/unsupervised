// The HTTP reads of the viewer. Every answer is checked before use.

import type { Clip, FeedPage, Layout, Profile, Published, PublishedRef, Shot, Story, ZoneMap } from '../state/types';
import { readFeedPage, readLayout, readProfile, readZoneMap } from './messages';
import { readClip, readDirector, readPublished, readPublishedIndex, readPublishedKind, readStories } from './show';

/** The server answered with an error, or with something else than expected. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

async function getJson<T>(path: string, read: (value: unknown) => T | null, method = 'GET'): Promise<T> {
  const response = await fetch(path, { method, headers: { accept: 'application/json' } });
  if (!response.ok) throw new ApiError(response.status, `${path} answered ${response.status}`);
  const value = read(await response.json());
  if (value === null) throw new ApiError(response.status, `${path} gave an answer that is not valid`);
  return value;
}

export function fetchLayout(): Promise<Layout> {
  return getJson('/v1/world', readLayout);
}

/** Reads the tiles and sites of a zone. The feed gives the residents. */
export function fetchZoneMap(zone: number): Promise<ZoneMap> {
  return getJson(`/v1/world/zones/${zone}?residents=false`, readZoneMap);
}

/** Reads notable events of the whole world after a cursor. */
export function fetchFeed(after: string | null, minDrama: number): Promise<FeedPage> {
  const cursor = after === null ? '' : `&after=${encodeURIComponent(after)}`;
  return getJson(`/v1/feed?min_drama=${minDrama}&limit=40${cursor}`, readFeedPage);
}

export function fetchProfile(resident: number): Promise<Profile> {
  return getJson(`/v1/residents/${resident}/profile`, readProfile);
}

/** Returns `fallback` when the server answers 404: a module did not publish the data yet. */
async function orAbsent<T>(request: Promise<T>, fallback: T): Promise<T> {
  try {
    return await request;
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return fallback;
    throw error;
  }
}

/** Reads the shot on air. */
export function fetchDirector(): Promise<Shot> {
  return getJson('/v1/director', readDirector);
}

/** Reads the running stories. */
export function fetchStories(): Promise<Story[]> {
  return getJson('/v1/stories?limit=40', readStories);
}

/** Reads a replay clip. */
export function fetchClip(id: string): Promise<Clip> {
  return getJson(`/v1/clips/${encodeURIComponent(id)}`, readClip);
}

/** Asks the server to keep a clip. Returns true when the clip is pinned now. */
export function pinClip(id: string): Promise<boolean> {
  return getJson(`/v1/clips/${encodeURIComponent(id)}/pin`, (value) => {
    const pinned = (value as { pinned?: unknown } | null)?.pinned;
    return typeof pinned === 'boolean' ? pinned : null;
  }, 'POST');
}

/** Reads the list of the published state. */
export function fetchPublishedIndex(): Promise<PublishedRef[]> {
  return getJson('/v1/published', readPublishedIndex);
}

/** Reads one leaderboard, or null until a module publishes it. */
export function fetchLeaderboard(board: string): Promise<Published | null> {
  return orAbsent(getJson<Published | null>(`/v1/leaderboards/${encodeURIComponent(board)}`, readPublished), null);
}

/** Reads the newest statistics of the economy, or null until a module publishes them. */
export function fetchEconomy(): Promise<Published | null> {
  return orAbsent(
    getJson('/v1/stats/economy', readPublishedKind).then((series) => series[series.length - 1] ?? null),
    null,
  );
}
