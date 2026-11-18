// The words that the viewer shows for a site.

import { CONFIG } from '../config';
import { cleanText } from './text';
import type { SiteView } from './types';

/** Returns a site kind as words: `police_station` gives `police station`. */
export function kindLabel(kind: string): string {
  return cleanText(kind.replace(/_/g, ' '), CONFIG.text.nameMax);
}

/**
 * Returns the label of a site: its name, or its kind when it has no name.
 * An owner can choose the name, so it is cleaned and cut like all text
 * from residents.
 */
export function siteLabel(site: Pick<SiteView, 'name' | 'kind'>): string {
  return cleanText(site.name, CONFIG.text.nameMax) || kindLabel(site.kind) || 'site';
}
