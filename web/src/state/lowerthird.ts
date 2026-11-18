// The words of the broadcast for the shot on air: the lower third and the
// breaking-news banner. Every piece of text that came from the world is
// cleaned and cut here. The result is plain text.

import { CONFIG } from '../config';
import type { ShowState } from './show';
import { cleanText } from './text';
import type { Layout, Shot } from './types';

export interface LowerThird {
  /** False when there is nothing to say. */
  visible: boolean;
  /** The word in front: the kind of the story, `LIVE` or `OVERVIEW`. */
  tag: string;
  /** One line about the shot. */
  caption: string;
  /** The headline of the story of the shot, when it says more than the caption. */
  headline: string;
  /** The city and the district of the shot. */
  place: string;
  breaking: boolean;
}

const NOTHING: LowerThird = { visible: false, tag: '', caption: '', headline: '', place: '', breaking: false };

/** Returns the city and the district of a shot, as clean text. */
export function shotPlace(shot: Shot, layout: Layout): string {
  const zone = shot.zone === undefined ? undefined : layout.zones.find((known) => known.id === shot.zone);
  const cityId = shot.city ?? zone?.city;
  const city = cityId === undefined ? undefined : layout.cities.find((known) => known.id === cityId);
  const names = [cleanText(city?.name, CONFIG.text.nameMax), cleanText(zone?.name, CONFIG.text.nameMax)];
  return names.filter((name) => name !== '').join(' · ');
}

/** Returns the lower third for the shot on air. */
export function lowerThird(shot: Shot | null, layout: Layout): LowerThird {
  if (shot === null) return NOTHING;
  const caption = cleanText(shot.caption, CONFIG.broadcast.captionMax);
  if (caption === '') return NOTHING;
  const headline = cleanText(shot.headline, CONFIG.broadcast.captionMax);
  const label = cleanText(shot.label, CONFIG.broadcast.tagMax).toUpperCase();
  const wide = shot.focus.type === 'none';
  return {
    visible: true,
    tag: label !== '' ? label : wide ? 'OVERVIEW' : 'LIVE',
    caption,
    // A headline that repeats the caption says nothing more.
    headline: headline === caption ? '' : headline,
    // The caption of a wide view names its place already.
    place: wide && !shot.breaking ? '' : shotPlace(shot, layout),
    breaking: shot.breaking,
  };
}

/**
 * Returns the text of the breaking-news banner, or an empty text when no
 * banner shows: the shot on air is breaking news and is younger than the
 * time of the banner.
 */
export function breakingBanner(show: ShowState, now: number): string {
  const shot = show.shot;
  if (shot === null || !shot.breaking) return '';
  if (now - show.shotAt >= CONFIG.broadcast.breakingMs) return '';
  return cleanText(shot.caption, CONFIG.broadcast.captionMax);
}
