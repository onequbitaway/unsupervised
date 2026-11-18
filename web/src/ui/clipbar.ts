// The bar of the clip player: what the clip is about, how far it is, and
// the buttons. Every text is clean text already and goes into the page as
// text.

import { button, el, setShown, setText } from './dom';

export interface ClipBarHandlers {
  /** The viewer wants to stop the clip, or to go on. */
  toggle(): void;
  /** The viewer wants the clip from its start. */
  restart(): void;
  /** The viewer wants the server to keep the clip. */
  pin(): void;
}

export interface ClipBarModel {
  /** One line about the event of the clip. */
  line: string;
  /** The calendar time of the picture. */
  when: string;
  /** The place of the clip. */
  place: string;
  /** How far the clip is, 0 to 1. */
  progress: number;
  playing: boolean;
  /** `pinned`: the server keeps the clip. `open`: it can be pinned. `wait`: a pin was asked for. `off`: the server pins for nobody. */
  pin: 'pinned' | 'open' | 'wait' | 'off';
  /** A sentence about the state of the clip, or an empty text. */
  note: string;
}

export class ClipBar {
  readonly root = el('section', 'clip-bar');
  private readonly line = el('p', 'clip-line');
  private readonly when = el('span', 'clip-when');
  private readonly place = el('span', 'clip-place');
  private readonly note = el('span', 'clip-note');
  private readonly fill = el('div', 'clip-fill');
  private readonly play: HTMLButtonElement;
  private readonly pin: HTMLButtonElement;
  private readonly copy: HTMLButtonElement;
  private readonly live = el('a', 'clip-live', 'Live map');

  /** `link` is the address of the clip, `live` the address of the live map. */
  constructor(
    handlers: ClipBarHandlers,
    private readonly link: string,
    live: string,
  ) {
    this.play = button('Pause', 'Stop the clip, or go on', () => handlers.toggle());
    this.pin = button('Keep this clip', 'Ask the server to keep this clip', () => handlers.pin());
    this.copy = button('Copy link', 'Copy the link of this clip', () => void this.copyLink());
    this.live.setAttribute('href', live);
    const head = el('div', 'clip-head');
    head.append(el('span', 'clip-tag', 'REPLAY'), this.when, this.place, this.note);
    const track = el('div', 'clip-track');
    track.append(this.fill);
    const actions = el('div', 'clip-actions');
    actions.append(
      this.play,
      button('Restart', 'Play the clip from its start', () => handlers.restart()),
      this.pin,
      this.copy,
      this.live,
    );
    this.root.append(head, this.line, track, actions);
  }

  update(model: ClipBarModel): void {
    setText(this.line, model.line);
    setText(this.when, model.when);
    setText(this.place, model.place);
    setText(this.note, model.note);
    setShown(this.note, model.note !== '');
    this.fill.style.width = `${Math.round(Math.min(1, Math.max(0, model.progress)) * 1000) / 10}%`;
    setText(this.play, model.playing ? 'Pause' : 'Play');
    setShown(this.pin, model.pin !== 'off');
    this.pin.disabled = model.pin !== 'open';
    setText(this.pin, model.pin === 'pinned' ? 'Kept' : model.pin === 'wait' ? 'Will be kept' : 'Keep this clip');
  }

  private async copyLink(): Promise<void> {
    try {
      await navigator.clipboard.writeText(this.link);
      setText(this.copy, 'Copied');
    } catch {
      // The page has no access to the clipboard. The address of the page is the link.
      setText(this.copy, 'Use the address');
    }
  }
}
