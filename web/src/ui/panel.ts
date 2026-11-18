// The panel of the selected resident: who it is, where it is, what it
// does and what it did. It also gives the watch link.

import type { PanelFacts } from '../state/profile';
import { button, el, setShown, setText } from './dom';

export interface PanelHandlers {
  /** The viewer closed the panel. */
  close(): void;
  /** The viewer wants to follow the resident, or to stop. */
  follow(on: boolean): void;
}

export interface PanelView {
  facts: PanelFacts;
  /** The colour of the resident on the map, as `#rrggbb`. */
  color: string;
  /** True while the camera follows the resident. */
  following: boolean;
  /** The address that follows this resident. */
  link: string;
}

function fact(list: HTMLElement, label: string): HTMLElement {
  const value = el('dd');
  list.append(el('dt', '', label), value);
  return value;
}

export class ResidentPanel {
  readonly root = el('aside', 'panel');
  private readonly swatch = el('span', 'panel-swatch');
  private readonly name = el('h2', 'panel-name');
  private readonly id: HTMLElement;
  private readonly model: HTMLElement;
  private readonly place: HTMLElement;
  private readonly doing: HTMLElement;
  private readonly note = el('p', 'panel-note');
  private readonly follow: HTMLButtonElement;
  private readonly link = el('a', 'panel-link', 'Watch link');
  private readonly copy: HTMLButtonElement;
  private readonly events = el('ol', 'panel-events');
  private readonly none = el('p', 'panel-none', 'No public events yet.');
  private following = false;
  private eventKey = '';

  constructor(handlers: PanelHandlers) {
    const head = el('div', 'panel-head');
    head.append(this.swatch, this.name, button('×', 'Close the panel (key Escape)', () => handlers.close(), 'panel-close'));
    const facts = el('dl', 'panel-facts');
    this.id = fact(facts, 'Resident');
    this.model = fact(facts, 'Model');
    this.place = fact(facts, 'Place');
    this.doing = fact(facts, 'Doing');
    this.follow = button('Follow', 'Keep the camera on this resident', () => handlers.follow(!this.following), 'panel-follow');
    this.copy = button('Copy link', 'Copy the watch link', () => void this.copyLink());
    this.link.title = 'A link that follows this resident';
    const actions = el('div', 'panel-actions');
    actions.append(this.follow, this.link, this.copy);
    this.root.append(head, facts, this.note, actions, el('h3', 'panel-title', 'Recent events'), this.events, this.none);
    this.root.hidden = true;
  }

  /** Shows a resident. Every text of `view.facts` is clean text already. */
  show(view: PanelView): void {
    const { facts } = view;
    setShown(this.root, true);
    this.swatch.style.background = view.color;
    setText(this.name, facts.name);
    setText(this.id, facts.id);
    setText(this.model, facts.model);
    setText(this.place, facts.place);
    setText(this.doing, facts.doing);
    setText(this.note, facts.note);
    setShown(this.note, facts.note !== '');
    this.following = view.following;
    setText(this.follow, view.following ? 'Stop following' : 'Follow');
    this.follow.classList.toggle('active', view.following);
    if (this.link.getAttribute('href') !== view.link) {
      this.link.setAttribute('href', view.link);
      setText(this.copy, 'Copy link');
    }
    // The list is built again only when its events changed.
    const key = facts.events.map((event) => event.id).join(' ');
    if (key !== this.eventKey) {
      this.eventKey = key;
      this.events.replaceChildren(
        ...facts.events.map((event) => {
          const row = el('li');
          row.append(el('time', '', event.when), el('span', '', event.line));
          return row;
        }),
      );
    }
    setShown(this.none, facts.events.length === 0);
  }

  hide(): void {
    setShown(this.root, false);
  }

  private async copyLink(): Promise<void> {
    const href = this.link.href;
    try {
      await navigator.clipboard.writeText(href);
      setText(this.copy, 'Copied');
    } catch {
      // The page has no access to the clipboard. The link itself can be copied.
      setText(this.copy, 'Use the link');
    }
  }
}
