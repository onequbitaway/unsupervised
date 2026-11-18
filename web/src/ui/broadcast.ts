// The page of a broadcast, over the picture: the lower third with the
// caption and the story, the breaking-news banner, the side panel with one
// card at a time, and the black of a cut. Every text is clean text already
// and goes into the page as text.

import type { Card } from '../state/cards';
import type { LowerThird } from '../state/lowerthird';
import { el, setShown, setText } from './dom';

/** The lower third: what the shot on air is about. */
export class LowerThirdBar {
  readonly root = el('section', 'lower-third');
  private readonly tag = el('span', 'lower-tag');
  private readonly place = el('span', 'lower-place');
  private readonly caption = el('p', 'lower-caption');
  private readonly headline = el('p', 'lower-headline');
  private shown: LowerThird | null = null;

  constructor() {
    const head = el('div', 'lower-head');
    head.append(this.tag, this.place);
    this.root.append(head, this.caption, this.headline);
    this.root.hidden = true;
    this.root.setAttribute('aria-live', 'polite');
  }

  update(model: LowerThird): void {
    if (model === this.shown) return;
    this.shown = model;
    setShown(this.root, model.visible);
    if (!model.visible) return;
    setText(this.tag, model.tag);
    setText(this.place, model.place);
    setShown(this.place, model.place !== '');
    setText(this.caption, model.caption);
    setText(this.headline, model.headline);
    setShown(this.headline, model.headline !== '');
    this.root.classList.toggle('breaking', model.breaking);
  }
}

/** The breaking-news banner at the top. */
export class BreakingBanner {
  readonly root = el('div', 'breaking-banner');
  private readonly line = el('span', 'breaking-line');

  constructor() {
    this.root.append(el('span', 'breaking-tag', 'BREAKING NEWS'), this.line);
    this.root.hidden = true;
    this.root.setAttribute('role', 'status');
  }

  /** Shows the banner with a line, or hides it for an empty line. */
  update(line: string): void {
    setShown(this.root, line !== '');
    if (line !== '') setText(this.line, line);
  }
}

/** The side panel: one card at a time. */
export class SidePanel {
  readonly root = el('aside', 'side-panel');
  private readonly title = el('h2', 'side-title');
  private readonly rows = el('ol', 'side-rows');
  private readonly dots = el('div', 'side-dots');
  private key = '';

  constructor() {
    this.root.append(this.title, this.rows, this.dots);
    this.root.hidden = true;
  }

  /** Shows a card, or hides the panel without one. `at` and `of` say which card of the rotation it is. */
  update(card: Card | null, at: number, of: number): void {
    setShown(this.root, card !== null);
    if (card === null) return;
    // The panel is built again only when its words changed.
    const key = JSON.stringify([card, at, of]);
    if (key === this.key) return;
    const changed = !this.key.startsWith(`[{"id":${JSON.stringify(card.id)},`);
    this.key = key;
    setText(this.title, card.title);
    this.root.dataset.card = card.id.split(':')[0] ?? '';
    this.rows.replaceChildren(
      ...card.rows.map((row) => {
        const item = el('li', 'side-row');
        if (row.rank !== '') item.append(el('span', 'side-rank', row.rank));
        item.append(el('span', 'side-label', row.label));
        if (row.value !== '') item.append(el('span', 'side-value', row.value));
        return item;
      }),
    );
    this.dots.replaceChildren(
      ...Array.from({ length: of > 1 ? of : 0 }, (_, i) => el('span', i === at ? 'side-dot on' : 'side-dot')),
    );
    if (changed) {
      // The card comes in with a short move. The class starts it again.
      this.root.classList.remove('enter');
      void this.root.offsetWidth;
      this.root.classList.add('enter');
    }
  }
}

/** The black of a cut, over the picture and under the words. */
export class Curtain {
  readonly root = el('div', 'curtain');
  private black = -1;

  /** Sets how dark the picture is, 0 to 1. */
  update(black: number): void {
    const next = Math.round(black * 100) / 100;
    if (next === this.black) return;
    this.black = next;
    this.root.style.opacity = String(next);
  }
}
