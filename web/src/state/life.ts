// The words of the page of a resident: who it is and its life story, by
// chapter. Everything that came from the world is cleaned and cut here.

import { CONFIG } from '../config';
import { activityLabel } from './activity';
import { clockLabel } from './headline';
import { cleanText } from './text';
import type { LifeEntry, Profile } from './types';

export interface LifeLine {
  /** Id of the event. The player of clips can replay it. */
  event: string;
  when: string;
  line: string;
}

export interface LifeChapter {
  /** A stable name, for example `jobs`. */
  id: string;
  title: string;
  lines: LifeLine[];
}

export interface LifePage {
  name: string;
  id: string;
  model: string;
  /** One sentence about where the resident is and what it does, or how it ended. */
  status: string;
  alive: boolean;
  /** The chapters that have something to say, in the order of a life. */
  chapters: LifeChapter[];
}

function lineOf(entry: LifeEntry): LifeLine {
  return { event: entry.event, when: clockLabel(entry.clock), line: cleanText(entry.line, CONFIG.page.lineMax) };
}

function chapter(id: string, title: string, entries: readonly LifeEntry[]): LifeChapter | null {
  const lines = entries.map(lineOf).filter((line) => line.line !== '');
  return lines.length === 0 ? null : { id, title, lines };
}

/** Returns the status sentence of a resident. */
function statusOf(profile: Profile): string {
  const city = cleanText(profile.city_name, CONFIG.text.nameMax);
  const zone = cleanText(profile.zone_name, CONFIG.text.nameMax);
  const place = [city, zone].filter((part) => part !== '').join(' · ') || `zone ${profile.zone}`;
  if (profile.alive) return `In ${place} · ${activityLabel(profile.activity)}`;
  const cause = cleanText(profile.cause?.replace(/_/g, ' '), CONFIG.page.causeMax);
  const age = profile.life.age === undefined ? '' : ` at the age of ${Math.floor(profile.life.age)}`;
  const why = cause === '' ? '' : ` (${cause})`;
  return `Left the world${age}${why} · last seen in ${place}`;
}

/** Returns the words of the page of a resident. */
export function lifePage(profile: Profile): LifePage {
  const { life } = profile;
  const chapters = [
    chapter('born', 'Arrival', life.born ? [life.born] : []),
    chapter('jobs', 'Work', life.jobs),
    chapter('homes', 'Homes', life.homes),
    chapter('deals', 'Deals', life.deals),
    chapter('crimes', 'Crime and law', life.crimes),
    chapter('offices', 'Public office', life.offices),
    chapter('moments', 'Other moments', life.moments),
    chapter('death', 'The end', life.death ? [life.death] : []),
  ];
  return {
    name: cleanText(profile.name, CONFIG.page.nameMax) || `Resident ${profile.id}`,
    id: `#${profile.id}`,
    model: cleanText(profile.model, CONFIG.text.modelMax) || 'not given',
    status: statusOf(profile),
    alive: profile.alive,
    chapters: chapters.filter((entry): entry is LifeChapter => entry !== null),
  };
}
