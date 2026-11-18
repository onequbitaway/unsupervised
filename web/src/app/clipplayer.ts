// The player of one replay clip (`?clip=<event id>`). It gives the frames
// of the clip to the reducer of the live feed, one frame at a time, so the
// clip looks like the live view. The clip starts again at its end.

import { UPDATE_PRIORITY } from 'pixi.js';
import { CONFIG } from '../config';
import { ApiError, fetchClip, fetchZoneMap, pinClip } from '../net/api';
import { Scene } from '../render/scene';
import { sizeOf, type Stage } from '../render/stage';
import { clipFrame, clipStart, playhead } from '../state/clipplay';
import { followTarget } from '../state/follow';
import { clockLabel } from '../state/headline';
import { clipLink, liveLink } from '../state/params';
import { cleanText } from '../state/text';
import type { Clip, Layout } from '../state/types';
import { initialState, reduce, type ViewerState } from '../state/world';
import { centre, zonesInView, zonesToMap } from '../state/zones';
import { ClipBar, type ClipBarModel } from '../ui/clipbar';
import { MapLoader } from './maps';
import { CameraRig } from './rig';

/** What the player shows, for scripts that check it. */
export interface ClipStats {
  settled: boolean;
  page: 'clip';
  clip: string;
  frame: number;
  from: number;
  to: number;
  complete: boolean;
  residents: number;
}

export class ClipPlayer {
  private state: ViewerState;
  private readonly rig: CameraRig;
  private readonly scene: Scene;
  private readonly maps: MapLoader;
  private readonly bar: ClipBar;
  /** The time that the clip played, in milliseconds. It stands while the clip is stopped. */
  private elapsed = 0;
  private playing = true;
  private frameAt: number | null = null;
  /** The last frame of the clip that the state holds. */
  private applied: number;
  private loop = 0;
  private pin: ClipBarModel['pin'];
  private askAt = 0;
  private asking = false;
  private slowAt = -Infinity;
  private placed = false;

  constructor(
    root: HTMLElement,
    private readonly stage: Stage,
    private readonly layout: Layout,
    private clip: Clip,
  ) {
    const size = sizeOf(stage);
    this.rig = new CameraRig(layout, size);
    this.scene = new Scene(stage, layout);
    this.maps = new MapLoader(this.scene, fetchZoneMap);
    this.pin = clip.pinned ? 'pinned' : 'open';
    this.state = this.startState(performance.now());
    this.applied = clip.from - 1;
    const here = globalThis.location.href;
    this.bar = new ClipBar(
      { toggle: () => this.toggle(), restart: () => this.restart(), pin: () => void this.keep() },
      clipLink(here, clip.id),
      liveLink(here),
    );
    root.classList.add('replay');
    root.append(this.bar.root);
    stage.app.ticker.add(() => this.frame(), undefined, UPDATE_PRIORITY.HIGH);
  }

  /** Returns the state before the first frame of the clip. */
  private startState(now: number): ViewerState {
    const base: ViewerState = {
      ...initialState(),
      clock: this.clip.frames[0]?.clock ?? this.clip.event.clock,
      populations: new Map(this.layout.zones.map((zone) => [zone.id, zone.residents])),
      tickMs: CONFIG.clip.frameMs,
    };
    return reduce(base, clipStart(this.clip), now);
  }

  private toggle(): void {
    this.playing = !this.playing;
  }

  private restart(): void {
    this.elapsed = 0;
    this.playing = true;
    this.rewind(performance.now());
  }

  private rewind(now: number): void {
    this.state = this.startState(now);
    this.applied = this.clip.from - 1;
  }

  /** Asks the server to keep the clip. */
  private async keep(): Promise<void> {
    if (this.pin !== 'open') return;
    this.pin = 'wait';
    try {
      this.pin = (await pinClip(this.clip.id)) ? 'pinned' : 'wait';
    } catch (error) {
      // 403: this server pins for nobody. Else the viewer can try again.
      this.pin = error instanceof ApiError && error.status === 403 ? 'off' : 'open';
    }
  }

  /** Reads the clip again while frames after its event are still to come. */
  private poll(now: number): void {
    if (this.clip.complete || this.asking || now < this.askAt) return;
    this.asking = true;
    this.askAt = now + CONFIG.clip.pollMs;
    fetchClip(this.clip.id).then(
      (clip) => {
        this.asking = false;
        // The clip only grows at its end: the frames that played stay true.
        this.clip = clip;
        if (clip.pinned) this.pin = 'pinned';
      },
      () => {
        this.asking = false;
      },
    );
  }

  /** Looks at the actor of the event, or at the place of the event. */
  private aim(now: number): void {
    const { event, zone } = this.clip;
    const scale = this.rig.fits.street * CONFIG.clip.zoom;
    const actor = event.actor === undefined ? null : followTarget(this.state, event.actor, null, now);
    const home = this.layout.zones.find((known) => known.id === zone);
    const at =
      actor ??
      (event.pos ? { x: event.pos.x + 0.5, y: event.pos.y + 0.5 } : home ? centre(home.bounds) : null);
    if (at === null) return;
    this.rig.lookAt(at.x, at.y, scale);
    if (!this.placed) {
      this.placed = true;
      this.rig.arrive();
    }
  }

  private frame(): void {
    const now = performance.now();
    const dt = this.frameAt === null ? 0 : now - this.frameAt;
    this.frameAt = now;
    if (this.playing) this.elapsed += dt;
    this.poll(now);

    const head = playhead(this.clip, this.elapsed, CONFIG.clip.frameMs, CONFIG.clip.endHoldMs);
    if (head.loop !== this.loop || head.frame < this.applied) {
      this.loop = head.loop;
      this.rewind(now);
    }
    while (this.applied < head.frame) {
      this.applied += 1;
      for (const message of clipFrame(this.clip, this.applied)) this.state = reduce(this.state, message, now);
    }

    const size = sizeOf(this.stage);
    if (size.width !== this.rig.size.width || size.height !== this.rig.size.height) this.rig.resize(size);
    this.aim(now);
    this.rig.step(dt);
    const level = this.rig.level;
    if (now - this.slowAt >= CONFIG.ui.everyMs) {
      this.slowAt = now;
      this.maps.want(zonesToMap(this.rig.targetLevel, zonesInView(this.layout, this.rig.target, size)), now);
      this.bar.update(this.model(head.progress));
    }
    this.maps.paint();
    this.scene.draw({
      camera: this.rig.camera,
      size,
      level,
      now,
      layout: this.layout,
      state: this.state,
      selected: this.clip.event.actor ?? null,
    });
  }

  private model(progress: number): ClipBarModel {
    const zone = this.layout.zones.find((known) => known.id === this.clip.zone);
    const city = zone === undefined ? undefined : this.layout.cities.find((known) => known.id === zone.city);
    const place = [cleanText(city?.name, CONFIG.text.nameMax), cleanText(zone?.name, CONFIG.text.nameMax)]
      .filter((name) => name !== '')
      .join(' · ');
    return {
      line: cleanText(this.clip.line, CONFIG.broadcast.captionMax),
      when: this.state.clock === null ? '' : clockLabel(this.state.clock),
      place,
      progress,
      playing: this.playing,
      pin: this.pin,
      note: this.clip.complete ? '' : 'The end of this clip is still to come.',
    };
  }

  stats(): ClipStats {
    return {
      settled: this.placed && this.rig.near && !this.maps.busy && this.scene.hasMap(this.clip.zone),
      page: 'clip',
      clip: this.clip.id,
      frame: this.applied,
      from: this.clip.from,
      to: this.clip.to,
      complete: this.clip.complete,
      residents: this.state.zones.get(this.clip.zone)?.residents.size ?? 0,
    };
  }
}
