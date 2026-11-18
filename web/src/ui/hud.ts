// The head of the screen: the name of the world, the calendar time, the
// number of residents, the place that the camera shows and the state of
// the connection.

import { clockLabel } from '../state/headline';
import type { Clock } from '../state/types';
import { el, setShown, setText } from './dom';

export interface HudModel {
  clock: Clock | null;
  /** Residents in the whole world. */
  residents: number;
  /** Viewers that are connected to the server. */
  viewers: number;
  /** The place that the camera shows, as clean text. */
  place: string;
  /** What to say about the connection, or an empty text when all is well. */
  notice: string;
}

export class Hud {
  readonly root = el('header', 'hud');
  private readonly clock = el('div', 'hud-clock');
  private readonly counts = el('div', 'hud-counts');
  private readonly place = el('div', 'hud-place');
  private readonly notice = el('div', 'hud-notice');

  constructor() {
    this.root.append(el('div', 'hud-title', 'UNSUPERVISED'), this.clock, this.counts, this.place, this.notice);
    this.notice.hidden = true;
  }

  update(model: HudModel): void {
    setText(this.clock, model.clock === null ? '' : clockLabel(model.clock));
    const residents = `${model.residents.toLocaleString('en-US')} ${model.residents === 1 ? 'resident' : 'residents'}`;
    const viewers = `${model.viewers.toLocaleString('en-US')} watching`;
    setText(this.counts, `${residents} · ${viewers}`);
    setText(this.place, model.place);
    setText(this.notice, model.notice);
    setShown(this.notice, model.notice !== '');
  }
}
