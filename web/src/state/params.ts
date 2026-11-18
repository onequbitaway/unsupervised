// The options in the address of the page.
//
//   ?watch=<resident id>   follow one resident, also across zones
//   ?broadcast=1           broadcast mode: the camera follows the director
//   ?resident=<resident id>   the page of one resident, with its life story
//   ?clip=<event id>       the player of one replay clip
//   ?view=country|city|street   the zoom level at the start
//   ?zone=<zone id>        the district to look at at the start

import type { Level } from './camera';

export interface PageOptions {
  /** The resident to follow, or null. */
  watch: number | null;
  /** True in broadcast mode: no controls, the camera follows the director. */
  broadcast: boolean;
  /** The resident whose page to show, or null. */
  resident: number | null;
  /** The clip to play (the id of its event, for example `120-3`), or null. */
  clip: string | null;
  /** The zoom level at the start, or null for the default. */
  view: Level | null;
  /** The zone to look at at the start, or null for the default. */
  zone: number | null;
}

/** Reads an id: digits only, and a number that JavaScript holds exactly. */
function id(value: string | null): number | null {
  if (value === null || !/^\d{1,16}$/.test(value)) return null;
  const number = Number(value);
  return Number.isSafeInteger(number) ? number : null;
}

/** Reads the id of an event: two plain numbers with a dash, for example `120-3`. */
function eventId(value: string | null): string | null {
  return value !== null && /^\d{1,16}-\d{1,9}$/.test(value) ? value : null;
}

function flag(value: string | null): boolean {
  return value !== null && value !== '0' && value.toLowerCase() !== 'false';
}

function level(value: string | null): Level | null {
  return value === 'country' || value === 'city' || value === 'street' ? value : null;
}

/** Reads the options from the query of an address, for example `?watch=7`. */
export function readOptions(search: string): PageOptions {
  const query = new URLSearchParams(search);
  return {
    watch: id(query.get('watch')),
    broadcast: flag(query.get('broadcast')),
    resident: id(query.get('resident')),
    clip: eventId(query.get('clip')),
    view: level(query.get('view')),
    zone: id(query.get('zone')),
  };
}

/**
 * Returns the address of the page with another resident to follow, or
 * with none. Every other option of the address stays. This is the link
 * that a viewer shares.
 */
export function watchLink(href: string, resident: number | null): string {
  const url = new URL(href);
  if (resident === null) url.searchParams.delete('watch');
  else url.searchParams.set('watch', String(resident));
  // A watch link says who to look at, not where.
  url.searchParams.delete('zone');
  url.searchParams.delete('view');
  return url.toString();
}

/** The options that say what the page shows. A link to another view removes them. */
const VIEWS = ['watch', 'broadcast', 'resident', 'clip', 'zone', 'view'] as const;

/** Returns the address of the page with one view and no other. */
function viewLink(href: string, name: (typeof VIEWS)[number] | null, value: string): string {
  const url = new URL(href);
  for (const view of VIEWS) url.searchParams.delete(view);
  if (name !== null) url.searchParams.set(name, value);
  url.hash = '';
  return url.toString();
}

/** Returns the address of the page of a resident. */
export function residentLink(href: string, resident: number): string {
  return viewLink(href, 'resident', String(resident));
}

/** Returns the address of the player of a clip. This is the link that a viewer shares. */
export function clipLink(href: string, event: string): string {
  return viewLink(href, 'clip', event);
}

/** Returns the address of the live map. */
export function liveLink(href: string): string {
  return viewLink(href, null, '');
}
