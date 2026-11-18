// The page of one resident (`?resident=<id>`): it reads the profile from
// time to time and shows the life story.

import { CONFIG } from '../config';
import { fetchProfile } from '../net/api';
import { lifePage } from '../state/life';
import { clipLink, liveLink, watchLink } from '../state/params';
import type { Profile } from '../state/types';
import { LifePageView } from '../ui/lifepage';
import { isUnknown } from './watch';

/** What the page shows, for scripts that check it. */
export interface PageStats {
  settled: boolean;
  page: 'resident';
  resident: number;
  chapters: number;
}

export class ResidentPage {
  private readonly view = new LifePageView();
  private profile: Profile | null = null;
  private answered = false;

  constructor(
    root: HTMLElement,
    private readonly resident: number,
    private readonly read: (resident: number) => Promise<Profile> = fetchProfile,
  ) {
    root.classList.add('reading');
    root.append(this.view.root);
  }

  /** Reads the profile now, and again from time to time. */
  start(): void {
    void this.refresh();
    globalThis.setInterval(() => void this.refresh(), CONFIG.page.refreshMs);
  }

  /** Reads the profile one time and shows it. */
  async refresh(): Promise<void> {
    try {
      this.profile = await this.read(this.resident);
      this.answered = true;
      const here = globalThis.location.href;
      this.view.update(lifePage(this.profile), {
        watch: watchLink(liveLink(here), this.resident),
        live: liveLink(here),
        clip: (event) => clipLink(here, event),
      });
    } catch (error) {
      this.answered = true;
      // The life story that the page has stays when the server does not answer.
      if (isUnknown(error)) this.view.showNote(`No resident with the id ${this.resident} is known.`);
      else if (this.profile === null) this.view.showNote('The server does not answer. The page tries again.');
    }
  }

  stats(): PageStats {
    return {
      settled: this.answered,
      page: 'resident',
      resident: this.resident,
      chapters: this.profile === null ? 0 : lifePage(this.profile).chapters.length,
    };
  }
}
