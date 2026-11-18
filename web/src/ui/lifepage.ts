// The page of one resident: who it is, and its life story by chapter.
// Every text is clean text already and goes into the page as text.

import type { LifePage } from '../state/life';
import { el, setShown, setText } from './dom';

export interface LifeLinks {
  /** The address that follows the resident on the live map. */
  watch: string;
  /** The address of the live map. */
  live: string;
  /** Returns the address of the player for the clip of an event. */
  clip(event: string): string;
}

function link(href: string, text: string, className: string): HTMLAnchorElement {
  const node = el('a', className, text);
  node.setAttribute('href', href);
  return node;
}

export class LifePageView {
  readonly root = el('main', 'life-page');
  private readonly name = el('h1', 'life-name');
  private readonly facts = el('p', 'life-facts');
  private readonly status = el('p', 'life-status');
  private readonly note = el('p', 'life-note');
  private readonly actions = el('nav', 'life-actions');
  private readonly chapters = el('div', 'life-chapters');
  private key = '';

  constructor() {
    const head = el('header', 'life-head');
    head.append(el('div', 'hud-title', 'UNSUPERVISED'), this.name, this.facts, this.status, this.actions);
    this.root.append(head, this.note, this.chapters);
    setText(this.note, 'Reading the life of this resident…');
  }

  /** Shows a sentence in place of the life story, for example that the resident is not known. */
  showNote(text: string): void {
    setText(this.note, text);
    setShown(this.note, text !== '');
  }

  /** Shows a resident. The page is built again only when its words changed. */
  update(page: LifePage, links: LifeLinks): void {
    const key = JSON.stringify(page);
    if (key === this.key) return;
    this.key = key;
    setText(this.name, page.name);
    setText(this.facts, `Resident ${page.id} · model ${page.model}`);
    setText(this.status, page.status);
    this.status.classList.toggle('gone', !page.alive);
    this.actions.replaceChildren(
      ...(page.alive ? [link(links.watch, 'Watch live', 'life-link primary')] : []),
      link(links.live, 'Live map', 'life-link'),
    );
    this.showNote(page.chapters.length === 0 ? 'The chronicle has no event of this resident yet.' : '');
    this.chapters.replaceChildren(
      ...page.chapters.map((chapter) => {
        const section = el('section', 'life-chapter');
        section.dataset.chapter = chapter.id;
        const list = el('ol', 'life-lines');
        for (const line of chapter.lines) {
          const item = el('li', 'life-line');
          const replay = link(links.clip(line.event), 'Replay', 'life-replay');
          replay.title = 'Play the clip of this event, when the server still has it';
          item.append(el('time', '', line.when), el('span', 'life-text', line.line), replay);
          list.append(item);
        }
        section.append(el('h2', 'life-title', chapter.title), list);
        return section;
      }),
    );
  }
}
