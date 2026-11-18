// The names of residents that the feed does not show: the viewer asks the
// server for their profiles, a few at a time, and remembers the names.

import { CONFIG } from '../config';
import type { Profile } from '../state/types';

export class NameBook {
  private readonly names = new Map<number, string>();
  private readonly asked = new Set<number>();
  private running = 0;

  /**
   * `fetchProfile` reads the profile of a resident. `parallel` is the
   * greatest number of requests at the same time, and `max` the greatest
   * number of names that the book keeps.
   */
  constructor(
    private readonly fetchProfile: (resident: number) => Promise<Profile>,
    private readonly parallel: number = CONFIG.broadcast.nameRequests,
    private readonly max: number = CONFIG.broadcast.namesMax,
  ) {}

  /** Returns the name of a resident, or null while the book does not have it. The name is untrusted. */
  get(resident: number): string | null {
    return this.names.get(resident) ?? null;
  }

  /** Asks for the names of residents that the book does not have yet. */
  want(residents: Iterable<number>): void {
    for (const resident of residents) {
      if (this.running >= this.parallel) return;
      if (this.asked.has(resident)) continue;
      this.asked.add(resident);
      this.running += 1;
      this.fetchProfile(resident).then(
        (profile) => {
          this.running -= 1;
          if (this.names.size >= this.max) {
            // The oldest name leaves. The book asks again when it is needed.
            const oldest = this.names.keys().next().value;
            if (oldest !== undefined) {
              this.names.delete(oldest);
              this.asked.delete(oldest);
            }
          }
          this.names.set(resident, profile.name);
        },
        () => {
          // The server does not know the resident, or did not answer. The
          // book does not ask again: the board shows the id.
          this.running -= 1;
        },
      );
    }
  }
}
