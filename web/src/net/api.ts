// The HTTP reads of the viewer. Every answer is checked before use.

import type { FeedPage, Layout, Profile, ZoneMap } from '../state/types';
import { readFeedPage, readLayout, readProfile, readZoneMap } from './messages';

/** The server answered with an error, or with something else than expected. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

async function getJson<T>(path: string, read: (value: unknown) => T | null): Promise<T> {
  const response = await fetch(path, { headers: { accept: 'application/json' } });
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
