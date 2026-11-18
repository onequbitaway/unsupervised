// Broadcast mode: the camera follows the director, and the page shows the
// caption, the story, breaking news and the cards of the side panel. It
// needs no input from a person.

import { CONFIG } from '../config';
import { fetchEconomy, fetchLeaderboard, fetchProfile, fetchPublishedIndex } from '../net/api';
import type { Camera, Fits, Size } from '../state/camera';
import { boardResidents, buildCards, type Card, cardAt } from '../state/cards';
import { type Fade, needsCut, NO_FADE, shotTarget, type ShotTarget, startFade, stepFade } from '../state/cut';
import type { NameOf } from '../state/headline';
import { breakingBanner, lowerThird, type LowerThird } from '../state/lowerthird';
import { emptyShow, reduceShow, type ShowState } from '../state/show';
import type { Clock, Layout, ServerMessage, Shot } from '../state/types';
import { BreakingBanner, Curtain, LowerThirdBar, SidePanel } from '../ui/broadcast';
import { BroadcastDesk } from './desk';
import { NameBook } from './names';

/** What the broadcast needs from the viewer. */
export interface Stagehand {
  /** Where the camera is, and the size of the screen. */
  camera(): Camera;
  size(): Size;
  fits(): Fits;
  /** The camera that shows the whole country. */
  countryCamera(): Camera;
  /** Points the camera at a target. With `jump` the camera is there at once. */
  direct(target: ShotTarget, jump: boolean): void;
  /** True when the picture can show the place that the camera looks at. */
  ready(): boolean;
  /** Finds the name of a resident that the feed shows now. */
  nameOf: NameOf;
}

export interface BroadcastFrame {
  now: number;
  clock: Clock | null;
  populations: ReadonlyMap<number, number>;
}

/** What the broadcast shows, for scripts that check it. */
export interface BroadcastStats {
  shot: number | null;
  caption: string;
  breaking: boolean;
  card: string | null;
  cards: string[];
  stories: number;
  fading: boolean;
}

export class Broadcast {
  readonly lower = new LowerThirdBar();
  readonly banner = new BreakingBanner();
  readonly side = new SidePanel();
  readonly curtain = new Curtain();
  private show: ShowState = emptyShow();
  private readonly desk = new BroadcastDesk({
    index: fetchPublishedIndex,
    board: fetchLeaderboard,
    economy: fetchEconomy,
  });
  private readonly names = new NameBook(fetchProfile);
  private fade: Fade = NO_FADE;
  /** The number of the shot that the camera has, or null before the first one. */
  private applied: number | null = null;
  /** The target that the camera gets when the picture is black. */
  private pending: ShotTarget | null = null;
  private third: LowerThird | null = null;
  private thirdOf: Shot | null = null;
  private cards: Card[] = [];
  private card: Card | null = null;
  private cardsAt = -Infinity;

  constructor(
    private readonly layout: Layout,
    private readonly hand: Stagehand,
  ) {}

  /** Applies one message of the feed to the show. */
  onMessage(message: ServerMessage, now: number): void {
    this.show = reduceShow(this.show, message, now);
  }

  /** Returns where the camera will be for a target, when the target says it. */
  private cameraOf(target: ShotTarget): Camera | null {
    switch (target.kind) {
      case 'country':
        return this.hand.countryCamera();
      case 'resident':
        return target.x === null || target.y === null ? null : { x: target.x, y: target.y, scale: target.scale };
      default:
        return { x: target.x, y: target.y, scale: target.scale };
    }
  }

  /** Gives the camera the shot on air: a move to a near place, a cut through black to a far one. */
  private follow(now: number): void {
    const shot = this.show.shot;
    if (shot !== null && shot.seq !== this.applied) {
      const first = this.applied === null;
      this.applied = shot.seq;
      const target = shotTarget(shot, this.layout, this.hand.fits(), CONFIG.broadcast);
      const to = this.cameraOf(target);
      const far = to !== null && needsCut(this.hand.camera(), to, this.hand.size(), CONFIG.broadcast.cutScreens);
      if (first) {
        // The broadcast starts on the shot: no move across the country.
        this.hand.direct(target, true);
      } else if (far || this.pending !== null) {
        this.pending = target;
        this.fade = startFade(this.fade, now);
      } else {
        this.hand.direct(target, false);
      }
    }
    const step = stepFade(this.fade, now, CONFIG.broadcast, this.hand.ready());
    this.fade = step.fade;
    if (step.jump && this.pending !== null) {
      this.hand.direct(this.pending, true);
      this.pending = null;
    }
    this.curtain.update(step.black);
  }

  /** Chooses the card of the side panel. The cards are made again a few times in a second. */
  private rotate(frame: BroadcastFrame): void {
    const { now } = frame;
    if (now - this.cardsAt >= CONFIG.ui.everyMs) {
      this.cardsAt = now;
      const nameOf: NameOf = (id) => this.hand.nameOf(id) ?? this.names.get(id);
      this.names.want(this.desk.boards.flatMap(boardResidents).filter((id) => nameOf(id) === null));
      this.cards = buildCards({
        layout: this.layout,
        populations: frame.populations,
        clock: frame.clock,
        stories: this.show.stories.values(),
        boards: this.desk.boards,
        economy: this.desk.economy,
        nameOf,
      });
    }
    this.card = cardAt(this.cards, now, CONFIG.broadcast.cardMs);
    const at = this.card === null ? 0 : this.cards.indexOf(this.card);
    this.side.update(this.card, at, this.cards.length);
  }

  /** Runs the broadcast for one frame of the screen. */
  frame(frame: BroadcastFrame): void {
    this.desk.tick(frame.now);
    this.follow(frame.now);
    const shot = this.show.shot;
    if (shot !== this.thirdOf) {
      this.thirdOf = shot;
      this.third = lowerThird(shot, this.layout);
    }
    if (this.third !== null) this.lower.update(this.third);
    this.banner.update(breakingBanner(this.show, frame.now));
    this.rotate(frame);
  }

  /** Returns what the broadcast shows, for scripts that check it. */
  stats(): BroadcastStats {
    return {
      shot: this.show.shot?.seq ?? null,
      caption: this.third?.caption ?? '',
      breaking: this.show.shot?.breaking ?? false,
      card: this.card?.id ?? null,
      cards: this.cards.map((card) => card.id),
      stories: this.show.stories.size,
      fading: this.fade.phase !== 'idle',
    };
  }
}
