// The words of the resident panel. Everything that came from the world is
// cleaned and cut here. The result is plain text.

import { CONFIG } from '../config';
import { activityLabel } from './activity';
import { clockLabel, headline, type NameOf } from './headline';
import { cleanText } from './text';
import type { Profile } from './types';

/** What the feed shows of a resident now. */
export interface LiveResident {
  name: string;
  model: string | null;
  activity: string | null;
}

export interface PanelInput {
  id: number;
  /** What the feed shows now, or null when the resident is in no subscribed zone. */
  live: LiveResident | null;
  /** The last profile from the server, or null while there is none. */
  profile: Profile | null;
  /** True when the server knows no resident with this id. */
  unknown: boolean;
}

export interface PanelFacts {
  name: string;
  id: string;
  model: string;
  place: string;
  doing: string;
  /** A sentence about the state of the resident, or an empty text. */
  note: string;
  /** The recent public events, the newest first. */
  events: { id: string; when: string; line: string }[];
}

/** Longest name in the panel. The panel has more room than a label on the map. */
const PANEL_NAME_MAX = 40;

/** Returns the words of the panel for a resident. The feed is newer than the profile. */
export function panelFacts(input: PanelInput, nameOf: NameOf = () => null): PanelFacts {
  const { id, live, profile } = input;
  const name = cleanText(live?.name ?? profile?.name, PANEL_NAME_MAX) || `Resident ${id}`;
  const model = cleanText(live?.model ?? profile?.model, CONFIG.text.modelMax) || 'not given';
  const city = cleanText(profile?.city_name, CONFIG.text.nameMax);
  const zone = cleanText(profile?.zone_name, CONFIG.text.nameMax);
  const place = [city, zone].filter((part) => part !== '').join(' · ') || (profile ? `Zone ${profile.zone}` : '…');
  const gone = profile !== null && !profile.alive;
  const doing = gone ? '—' : activityLabel(live ? live.activity : profile?.activity);
  let note = '';
  if (input.unknown) note = 'No resident with this id is in the world.';
  else if (gone) {
    const cause = cleanText(profile.cause?.replace(/_/g, ' '), 32);
    note = cause === '' ? 'This resident left the world.' : `This resident left the world (${cause}).`;
  }
  const events = (profile?.events ?? [])
    .map((event) => ({ id: event.id, when: clockLabel(event.clock), line: headline(event, nameOf) }))
    .reverse();
  return { name, id: `#${id}`, model, place, doing, note, events };
}
