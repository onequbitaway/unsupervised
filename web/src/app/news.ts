// Feeds the news ticker: events of the subscribed zones and headlines
// come with the feed; notable events of the whole world come from
// `/v1/feed`, which the viewer asks from time to time.

import { CONFIG } from '../config';
import { headline, type NameOf } from '../state/headline';
import { addItems, advance, emptyTicker, type TickerState } from '../state/ticker';
import type { FeedEvent, FeedPage } from '../state/types';

export class NewsDesk {
  /** The ticker as data. A new object each time it changes. */
  state: TickerState = emptyTicker();
  private cursor: string | null = null;
  private asking = false;
  private askAt = 0;

  /**
   * `nameOf` finds the name of a resident that the viewer knows.
   * `fetchPage` reads `/v1/feed` after a cursor, with a lowest drama score.
   */
  constructor(
    private readonly nameOf: NameOf,
    private readonly fetchPage: (after: string | null, minDrama: number) => Promise<FeedPage>,
  ) {}

  /** Adds events. An event that the ticker has already is ignored. */
  add(events: readonly FeedEvent[], now: number): void {
    if (events.length === 0) return;
    const items = events.map((event) => ({
      id: event.id,
      drama: event.drama,
      // The line is cleaned and cut here. The ticker shows it as text.
      text: headline(event, this.nameOf),
      at: now,
      zone: event.zone ?? null,
      actor: event.actor ?? null,
    }));
    this.state = addItems(this.state, items, now);
  }

  /** Moves the main line on, and asks the server for news when it is time. */
  tick(now: number): void {
    this.state = advance(this.state, now);
    if (this.asking || now < this.askAt) return;
    this.asking = true;
    this.askAt = now + CONFIG.ticker.pollMs;
    this.fetchPage(this.cursor, CONFIG.ticker.notableDrama).then(
      (page) => {
        this.asking = false;
        this.cursor = page.next;
        this.add(page.events, now);
        // More events wait: ask again at the next tick.
        if (page.more) this.askAt = 0;
      },
      () => {
        // The server did not answer. The next request tries again.
        this.asking = false;
      },
    );
  }
}
