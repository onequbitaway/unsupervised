// The desk of a broadcast: it reads what the side panel shows from the
// server from time to time. The boards and the numbers of the economy
// are absent until a module publishes them; the desk then has nothing for
// them, and the panel shows its other cards.

import { CONFIG } from '../config';
import type { Published, PublishedRef } from '../state/types';

/** The kind of the published state that holds the boards. */
export const BOARD_KIND = 'missions.leaderboard_published';

export interface DeskApi {
  /** Reads the list of the published state. */
  index(): Promise<PublishedRef[]>;
  /** Reads one board, or null when the server does not have it. */
  board(name: string): Promise<Published | null>;
  /** Reads the statistics of the economy, or null when the server does not have them. */
  economy(): Promise<Published | null>;
}

export class BroadcastDesk {
  /** The boards that the server has, in the order of their names. */
  boards: Published[] = [];
  /** The statistics of the economy, or null. */
  economy: Published | null = null;
  private names: string[] = [];
  private indexAt = -Infinity;
  private pollAt = -Infinity;
  private busy = 0;

  constructor(private readonly api: DeskApi) {}

  /** Reads from the server when it is time. Call it in each frame. */
  tick(now: number): void {
    if (this.busy > 0) return;
    if (now - this.indexAt >= CONFIG.broadcast.indexMs) {
      this.indexAt = now;
      this.run(this.api.index(), (refs) => {
        this.names = refs
          .filter((ref) => ref.kind === BOARD_KIND && ref.key !== undefined)
          .map((ref) => ref.key as string)
          .sort()
          .slice(0, CONFIG.broadcast.maxBoards);
        // A board that the server has no more leaves the panel.
        this.boards = this.boards.filter((board) => board.key !== undefined && this.names.includes(board.key));
        // The new list is read at once.
        this.pollAt = -Infinity;
      });
      return;
    }
    if (now - this.pollAt < CONFIG.broadcast.pollMs) return;
    this.pollAt = now;
    this.run(this.api.economy(), (economy) => {
      this.economy = economy;
    });
    for (const name of this.names) {
      this.run(this.api.board(name), (board) => {
        const others = this.boards.filter((known) => known.key !== name);
        this.boards = (board === null ? others : [...others, board]).sort((a, b) =>
          (a.key ?? '').localeCompare(b.key ?? ''),
        );
      });
    }
  }

  private run<T>(request: Promise<T>, done: (value: T) => void): void {
    this.busy += 1;
    request.then(
      (value) => {
        this.busy -= 1;
        done(value);
      },
      () => {
        // The server did not answer. What the desk has stays; the next
        // read tries again.
        this.busy -= 1;
      },
    );
  }
}
