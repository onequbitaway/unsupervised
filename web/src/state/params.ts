// The options in the address of the page.
//
//   ?watch=<resident id>   follow one resident, also across zones
//   ?broadcast=1           hide the controls, for a video stream
//   ?view=country|city|street   the zoom level at the start
//   ?zone=<zone id>        the district to look at at the start

import type { Level } from './camera';

export interface PageOptions {
  /** The resident to follow, or null. */
  watch: number | null;
  /** True when the controls are hidden. */
  broadcast: boolean;
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
