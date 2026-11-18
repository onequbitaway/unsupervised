// The news ticker on the screen: one main line and a few recent lines.

import { CONFIG } from '../config';
import { recent, type TickerItem, type TickerState } from '../state/ticker';
import { el, setShown, setText } from './dom';

/** Returns the word in front of a line, by its drama score. */
export function tagOf(drama: number): 'HEADLINE' | 'NEWS' | 'LOCAL' {
  if (drama >= CONFIG.ticker.headlineDrama) return 'HEADLINE';
  return drama >= CONFIG.ticker.notableDrama ? 'NEWS' : 'LOCAL';
}

export class TickerBar {
  readonly root = el('footer', 'ticker');
  private readonly main = el('button', 'ticker-main');
  private readonly tag = el('span', 'ticker-tag');
  private readonly line = el('span', 'ticker-line');
  private readonly list = el('ol', 'ticker-recent');
  private readonly rows: { row: HTMLLIElement; item: TickerItem | null }[] = [];
  private shown: TickerState | null = null;
  private current: TickerItem | null = null;

  /** `onPick` gets the item of a line that the viewer pressed. */
  constructor(onPick: (item: TickerItem) => void) {
    this.main.type = 'button';
    this.main.title = 'Go to this event';
    this.main.append(this.tag, this.line);
    this.main.addEventListener('click', () => {
      if (this.current !== null) onPick(this.current);
    });
    for (let i = 0; i < CONFIG.ticker.recentLines; i++) {
      const entry: { row: HTMLLIElement; item: TickerItem | null } = { row: el('li', 'ticker-row'), item: null };
      entry.row.addEventListener('click', () => {
        if (entry.item !== null) onPick(entry.item);
      });
      this.rows.push(entry);
      this.list.append(entry.row);
    }
    this.root.append(this.list, this.main);
    this.root.hidden = true;
  }

  /** Shows the state of the ticker. The lines are clean text already. */
  update(state: TickerState): void {
    if (state === this.shown) return;
    this.shown = state;
    this.current = state.current;
    setShown(this.root, state.current !== null);
    if (state.current === null) return;
    const tag = tagOf(state.current.drama);
    setText(this.tag, tag);
    this.main.dataset.tag = tag.toLowerCase();
    setText(this.line, state.current.text);
    const lines = recent(state, this.rows.length);
    this.rows.forEach((entry, i) => {
      const item = lines[i] ?? null;
      entry.item = item;
      setText(entry.row, item === null ? '' : item.text);
      setShown(entry.row, item !== null);
      if (item !== null) entry.row.dataset.tag = tagOf(item.drama).toLowerCase();
    });
  }
}
