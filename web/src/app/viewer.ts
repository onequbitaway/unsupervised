// The viewer: it connects the feed, the state, the camera, the picture
// and the page, and runs them once in each frame.

import { UPDATE_PRIORITY } from 'pixi.js';
import { CONFIG } from '../config';
import { fetchFeed, fetchLayout, fetchProfile, fetchZoneMap } from '../net/api';
import { FeedClient, type FeedStatus, feedUrl } from '../net/feed';
import { residentColor } from '../render/palette';
import { Scene } from '../render/scene';
import { sizeOf, type Stage } from '../render/stage';
import { drawsDots, figureHeight } from '../render/view';
import { type Level, screenToWorld, type Size } from '../state/camera';
import { followTarget } from '../state/follow';
import { type PageOptions, watchLink } from '../state/params';
import { pickResident } from '../state/pick';
import { panelFacts } from '../state/profile';
import { cleanText } from '../state/text';
import type { TickerItem } from '../state/ticker';
import type { Layout, ServerMessage, ZoneView } from '../state/types';
import { findMover, initialState, keepZones, reduce, totalPopulation, type ViewerState } from '../state/world';
import { busiestZone, contains, nearestCity, nearestZone, zoneAt, zonesInView, zonesToFollow, zonesToMap } from '../state/zones';
import { Controls } from '../ui/controls';
import { el } from '../ui/dom';
import { Hud } from '../ui/hud';
import { attachInput, type KeyAction } from '../ui/input';
import { cssColor, Legend } from '../ui/legend';
import { ResidentPanel } from '../ui/panel';
import { TickerBar } from '../ui/tickerbar';
import { MapLoader } from './maps';
import { NewsDesk } from './news';
import { CameraRig } from './rig';
import { Samples } from './stats';
import { ProfileWatch } from './watch';

/** The numbers that `window.__unsup.stats()` gives, for scripts that check the viewer. */
export interface ViewerStats {
  /** True when the picture shows everything that the camera asks for. */
  settled: boolean;
  frame: number;
  level: Level;
  /** Where the camera looks, in tiles, and how many pixels one tile has. */
  camera: { x: number; y: number };
  scale: number;
  status: FeedStatus;
  /** Milliseconds between two frames of the screen. */
  frameMs: ReturnType<Samples['summary']>;
  /** Milliseconds that the viewer works for one frame, with the drawing. */
  workMs: ReturnType<Samples['summary']>;
  residents: { world: number; known: number; drawn: number };
  labels: { names: number; bubbles: number; sites: number };
  zones: number;
  selected: number | null;
  following: boolean;
}

/** Returns a text that changes when the cities or the zones of a world change. */
function shapeOf(layout: Layout): string {
  return JSON.stringify([
    layout.cities.map((city) => [city.id, city.name, city.bounds]),
    layout.zones.map((zone) => [zone.id, zone.city, zone.name, zone.bounds]),
  ]);
}

export class Viewer {
  private state: ViewerState;
  private readonly rig: CameraRig;
  private readonly scene: Scene;
  private readonly maps: MapLoader;
  private readonly news: NewsDesk;
  private readonly watch = new ProfileWatch(fetchProfile);
  private readonly feed: FeedClient;
  private readonly hud = new Hud();
  private readonly legend = new Legend();
  private readonly controls: Controls;
  private readonly panel: ResidentPanel;
  private readonly ticker: TickerBar;
  private readonly frameMs = new Samples(CONFIG.stats.samples);
  private readonly workMs = new Samples(CONFIG.stats.samples);
  private selected: number | null = null;
  private following = false;
  /** True until the camera went near to a resident that the viewer started to follow. */
  private closeUp = false;
  private status: FeedStatus = 'connecting';
  private opened = 0;
  private gotWorld = false;
  private outdated = false;
  private followed: readonly number[] = [];
  private slowAt = -Infinity;
  private frameAt: number | null = null;
  private size: Size;

  constructor(
    root: HTMLElement,
    surface: HTMLElement,
    private readonly stage: Stage,
    private readonly layout: Layout,
    options: PageOptions,
  ) {
    this.size = sizeOf(stage);
    this.state = {
      ...initialState(),
      frame: layout.frame,
      clock: layout.clock,
      populations: new Map(layout.zones.map((zone) => [zone.id, zone.residents])),
    };
    this.rig = new CameraRig(layout, this.size);
    this.scene = new Scene(stage, layout);
    this.maps = new MapLoader(this.scene, fetchZoneMap);
    this.news = new NewsDesk((id) => findMover(this.state, id)?.mover.name ?? null, fetchFeed);
    this.controls = new Controls({
      zoom: (factor) => this.rig.zoomCentre(factor),
      level: (level) => this.showLevel(level),
    });
    this.panel = new ResidentPanel({
      close: () => this.select(null),
      follow: (on) => this.setFollow(on),
    });
    this.ticker = new TickerBar((item) => this.goToNews(item));
    const corner = el('div', 'corner');
    corner.append(this.hud.root, this.legend.root);
    root.append(corner, this.controls.root, this.panel.root, this.ticker.root);

    attachInput(surface, {
      pan: (dx, dy) => {
        this.stopFollowing();
        this.rig.drag(dx, dy);
      },
      zoomAt: (factor, x, y) => this.rig.zoom(factor, x, y),
      click: (x, y) => this.click(x, y),
      key: (action) => this.key(action),
    });

    this.startView(options);
    this.feed = new FeedClient(feedUrl(globalThis.location), {
      onMessage: (message) => this.onMessage(message),
      onStatus: (status) => this.onStatus(status),
    });
    this.feed.start();
    stage.app.ticker.add(() => this.frame(), undefined, UPDATE_PRIORITY.HIGH);
    // The renderer draws between these two calls.
    stage.app.ticker.add(() => this.afterDraw(), undefined, UPDATE_PRIORITY.UTILITY);
  }

  /** Chooses what the screen shows at the start. */
  private startView(options: PageOptions): void {
    if (options.watch !== null) {
      this.select(options.watch);
      this.setFollow(true);
      return;
    }
    const asked = options.zone === null ? undefined : this.layout.zones.find((zone) => zone.id === options.zone);
    const focus = asked ?? busiestZone(this.layout, this.state.populations);
    // The street is where things happen. An empty world shows the country.
    const level = options.view ?? (asked || totalPopulation(this.state) > 0 ? 'street' : 'country');
    if (level === 'country' || focus === null) this.rig.showCountry();
    else if (level === 'city') this.showCityOf(focus);
    else this.rig.showZone(focus);
    this.rig.arrive();
  }

  private showCityOf(zone: ZoneView): void {
    const city = this.layout.cities.find((candidate) => candidate.id === zone.city);
    if (city) this.rig.showCity(city);
    else this.rig.showZone(zone);
  }

  /** Goes to a level, near to what the camera shows now. */
  private showLevel(level: Level): void {
    this.stopFollowing();
    const { x, y } = this.rig.target;
    if (level === 'country') {
      this.rig.showCountry();
      return;
    }
    // From the country view, the busiest place is the best guess.
    const zone =
      this.rig.targetLevel === 'country' ? busiestZone(this.layout, this.state.populations) : nearestZone(this.layout, x, y);
    if (zone === null) return;
    if (level === 'city') this.showCityOf(zone);
    else this.rig.showZone(zone);
  }

  private key(action: KeyAction): void {
    switch (action) {
      case 'in':
        this.rig.zoomCentre(CONFIG.view.zoomStep);
        break;
      case 'out':
        this.rig.zoomCentre(1 / CONFIG.view.zoomStep);
        break;
      case 'country':
      case 'city':
      case 'street':
        this.showLevel(action);
        break;
      case 'escape':
        this.select(null);
        break;
      default: {
        this.stopFollowing();
        const dx = action === 'left' ? -1 : action === 'right' ? 1 : 0;
        const dy = action === 'up' ? -1 : action === 'down' ? 1 : 0;
        this.rig.nudge(dx, dy);
      }
    }
  }

  /** A click selects a resident, or goes one level nearer to the place of the click. */
  private click(sx: number, sy: number): void {
    const camera = this.rig.camera;
    const at = screenToWorld(camera, this.size, sx, sy);
    // A figure stands on its tile: its body is above the point of its feet.
    const lift = drawsDots(this.rig.level, camera.scale) ? 0 : (figureHeight(camera.scale) * 0.45) / camera.scale;
    const radius = CONFIG.render.pickPx / camera.scale;
    const resident = pickResident(this.state, at.x, at.y + lift, radius, performance.now());
    if (resident !== null) {
      this.select(resident);
      return;
    }
    const level = this.rig.level;
    if (level === 'country') {
      const city = nearestCity(this.layout, at.x, at.y);
      if (city && contains(city.bounds, at.x, at.y)) {
        this.stopFollowing();
        this.rig.showCity(city);
      }
    } else if (level === 'city') {
      const zone = zoneAt(this.layout, at.x, at.y);
      if (zone) {
        this.stopFollowing();
        this.rig.showZone(zone);
      }
    } else {
      this.select(null);
    }
  }

  /** Selects a resident for the panel, or none. */
  private select(resident: number | null): void {
    if (resident === this.selected) return;
    this.selected = resident;
    this.watch.select(resident);
    if (this.following) this.setFollow(false);
    this.slowAt = -Infinity;
  }

  /** Starts or stops to follow the selected resident, and puts it into the address. */
  private setFollow(on: boolean): void {
    const next = on && this.selected !== null;
    if (next === this.following) return;
    this.following = next;
    this.closeUp = next;
    this.slowAt = -Infinity;
    const address = watchLink(globalThis.location.href, next ? this.selected : null);
    globalThis.history.replaceState(null, '', address);
  }

  private stopFollowing(): void {
    if (this.following) this.setFollow(false);
  }

  /** The viewer pressed a line of news: follow its resident, or go to its place. */
  private goToNews(item: TickerItem): void {
    if (item.actor !== null) {
      this.select(item.actor);
      this.setFollow(true);
      return;
    }
    const zone = item.zone === null ? undefined : this.layout.zones.find((candidate) => candidate.id === item.zone);
    if (zone) {
      this.stopFollowing();
      this.rig.showZone(zone);
    }
  }

  private onMessage(message: ServerMessage): void {
    const now = performance.now();
    this.state = reduce(this.state, message, now);
    switch (message.op) {
      case 'world':
        this.gotWorld = true;
        this.outdated = message.protocol !== CONFIG.feed.protocol;
        break;
      case 'delta':
        this.news.add(message.events, now);
        break;
      case 'frame':
        this.news.add(message.headlines, now);
        break;
      case 'error':
        console.warn(`The feed refused a request (${message.code}): ${message.message}`);
        break;
      default:
        break;
    }
  }

  private onStatus(status: FeedStatus): void {
    this.status = status;
    if (status !== 'open') return;
    this.opened += 1;
    // After a new connection the server can hold another world.
    if (this.opened > 1) void this.checkWorld();
  }

  /** Loads the page again when the server has a world of another shape. */
  private async checkWorld(): Promise<void> {
    try {
      const layout = await fetchLayout();
      if (shapeOf(layout) !== shapeOf(this.layout)) globalThis.location.reload();
    } catch {
      // The server does not answer yet. The next connection checks again.
    }
  }

  /** Moves the camera with the followed resident. */
  private follow(now: number): boolean {
    if (!this.following || this.selected === null) return false;
    const gone = this.watch.profile !== null && !this.watch.profile.alive;
    const target = followTarget(this.state, this.selected, this.watch.lastKnown, now);
    if (target === null) return true;
    if (this.closeUp) {
      this.rig.lookAt(target.x, target.y, Math.max(this.rig.target.scale, this.rig.followScale));
      this.closeUp = false;
    } else {
      this.rig.lookAt(target.x, target.y);
    }
    // True while the feed does not show the resident and it may be somewhere.
    return !target.live && !gone;
  }

  /** Tells the feed and the map loader which zones the camera needs. */
  private syncZones(now: number): void {
    const level = this.rig.targetLevel;
    const inView = zonesInView(this.layout, this.rig.target, this.size);
    this.followed = zonesToFollow(level, inView);
    this.feed.setZones(this.followed);
    this.state = keepZones(this.state, new Set(this.followed));
    this.maps.want(zonesToMap(level, inView), now);
  }

  private notice(): string {
    if (this.outdated) return 'This viewer is older than the server. Load the page again.';
    if (this.status === 'waiting') return 'The connection is lost. The viewer connects again.';
    if (this.status === 'connecting' || !this.gotWorld) return 'Connecting…';
    return '';
  }

  private place(level: Level): string {
    if (level === 'country') return 'The whole country';
    const zone = nearestZone(this.layout, this.rig.camera.x, this.rig.camera.y);
    if (zone === null) return '';
    const city = this.layout.cities.find((candidate) => candidate.id === zone.city);
    const cityName = cleanText(city?.name, CONFIG.text.nameMax);
    const zoneName = cleanText(zone.name, CONFIG.text.nameMax);
    return level === 'city' || zoneName === '' ? cityName : `${cityName} · ${zoneName}`;
  }

  /** Updates the parts of the page that do not need every frame. */
  private slowUpdate(now: number, level: Level): void {
    this.syncZones(now);
    this.hud.update({
      clock: this.state.clock,
      residents: totalPopulation(this.state),
      viewers: this.state.viewers,
      place: this.place(level),
      notice: this.notice(),
    });
    this.controls.setLevel(level);
    this.legend.update(level === 'country', this.state.populations);
    if (this.selected === null) {
      this.panel.hide();
      return;
    }
    const found = findMover(this.state, this.selected);
    const facts = panelFacts(
      { id: this.selected, live: found?.mover ?? null, profile: this.watch.profile, unknown: this.watch.unknown },
      (id) => findMover(this.state, id)?.mover.name ?? null,
    );
    this.panel.show({
      facts,
      color: cssColor(residentColor(this.selected)),
      following: this.following,
      link: watchLink(globalThis.location.href, this.selected),
    });
  }

  private frame(): void {
    const now = performance.now();
    if (this.frameAt !== null) this.frameMs.add(now - this.frameAt);
    const elapsed = this.frameAt === null ? 0 : now - this.frameAt;
    this.frameAt = now;

    const size = sizeOf(this.stage);
    if (size.width !== this.size.width || size.height !== this.size.height) {
      this.size = size;
      this.rig.resize(size);
    }

    const searching = this.follow(now);
    this.rig.step(elapsed);
    const level = this.rig.level;
    if (now - this.slowAt >= CONFIG.ui.everyMs) {
      this.slowAt = now;
      this.slowUpdate(now, level);
    }
    this.maps.paint();
    this.news.tick(now);
    this.watch.tick(now, searching);
    this.ticker.update(this.news.state);
    this.scene.draw({
      camera: this.rig.camera,
      size,
      level,
      now,
      layout: this.layout,
      state: this.state,
      selected: this.selected,
    });
  }

  private afterDraw(): void {
    if (this.frameAt !== null) this.workMs.add(performance.now() - this.frameAt);
  }

  /** Returns numbers about the viewer, for scripts that check it. */
  stats(): ViewerStats {
    let known = 0;
    for (const zone of this.state.zones.values()) known += zone.residents.size;
    const counts = this.scene.counts();
    const found = this.selected !== null && findMover(this.state, this.selected) !== null;
    const gone = this.watch.unknown || (this.watch.profile !== null && !this.watch.profile.alive);
    return {
      settled:
        this.gotWorld &&
        this.status === 'open' &&
        (this.following ? this.rig.near : this.rig.arrived) &&
        !this.maps.busy &&
        this.followed.every((zone) => this.state.zones.has(zone)) &&
        (!this.following || found || gone),
      frame: this.state.frame,
      level: this.rig.level,
      camera: { x: this.rig.camera.x, y: this.rig.camera.y },
      scale: this.rig.camera.scale,
      status: this.status,
      frameMs: this.frameMs.summary(),
      workMs: this.workMs.summary(),
      residents: { world: totalPopulation(this.state), known, drawn: counts.residents },
      labels: { names: counts.names, bubbles: counts.bubbles, sites: counts.siteLabels },
      zones: this.followed.length,
      selected: this.selected,
      following: this.following,
    };
  }

  /** Forgets the measured frame times, so that a script measures from now. */
  resetStats(): void {
    this.frameMs.clear();
    this.workMs.clear();
  }
}
