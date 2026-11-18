// Reads the profile of the selected resident from time to time. The
// profile says where a resident is when the feed does not show it, so a
// followed resident is found again in any zone.

import { CONFIG } from '../config';
import type { Pos, Profile } from '../state/types';

/** The server answered that it does not know the resident. */
export function isUnknown(error: unknown): boolean {
  return typeof error === 'object' && error !== null && (error as { status?: unknown }).status === 404;
}

export class ProfileWatch {
  /** The resident that the viewer selected, or null. */
  resident: number | null = null;
  /** The last profile of that resident, or null. */
  profile: Profile | null = null;
  /** True when the server knows no resident with this id. */
  unknown = false;
  private asking = false;
  /** When the last request left, in milliseconds. */
  private askedAt = -Infinity;

  constructor(private readonly fetchProfile: (resident: number) => Promise<Profile>) {}

  /** Selects a resident, or none. The profile of the old one is forgotten. */
  select(resident: number | null): void {
    if (resident === this.resident) return;
    this.resident = resident;
    this.profile = null;
    this.unknown = false;
    this.askedAt = -Infinity;
  }

  /** The last place that the server gave for the resident, or null. */
  get lastKnown(): Pos | null {
    return this.profile?.pos ?? null;
  }

  /**
   * Reads the profile when it is time. `searching` is true when the
   * viewer follows the resident and the feed does not show it: then the
   * viewer asks more often.
   */
  tick(now: number, searching: boolean): void {
    const resident = this.resident;
    if (resident === null || this.asking) return;
    const alive = this.profile === null || this.profile.alive;
    // The wait is chosen now, so that a search starts at once when the resident goes out of sight.
    const wait = searching && alive && !this.unknown ? CONFIG.panel.searchMs : CONFIG.panel.refreshMs;
    if (now - this.askedAt < wait) return;
    this.asking = true;
    this.askedAt = now;
    this.fetchProfile(resident).then(
      (profile) => {
        this.asking = false;
        // The viewer selected another resident while the request ran.
        if (this.resident !== resident) return;
        this.profile = profile;
        this.unknown = false;
      },
      (error: unknown) => {
        this.asking = false;
        if (this.resident === resident && isUnknown(error)) this.unknown = true;
      },
    );
  }
}
