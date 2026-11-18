// The whole picture: every layer, in the order in which it is drawn.

import type { Layout, ZoneMap } from '../state/types';
import { BubbleLayer } from './bubbles';
import { GroundLayer } from './ground';
import { LandLayer } from './land';
import { NameLayer } from './names';
import { PlaceLayer } from './places';
import { ResidentLayer } from './residents';
import { SiteLayer } from './sites';
import { applyCamera, type Stage } from './stage';
import type { SceneView } from './view';

export class Scene {
  private readonly land: LandLayer;
  private readonly ground = new GroundLayer();
  private readonly sites = new SiteLayer();
  private readonly places = new PlaceLayer();
  private readonly residents = new ResidentLayer();
  private readonly names = new NameLayer();
  private readonly bubbles = new BubbleLayer();

  constructor(
    private readonly stage: Stage,
    layout: Layout,
  ) {
    this.land = new LandLayer(layout);
    this.residents.setZones(layout.zones);
    // From the bottom to the top.
    stage.world.addChild(this.land.container, this.ground.container, this.sites.shapes);
    stage.screen.addChild(
      this.places.lines,
      this.residents.container,
      this.sites.labels.container,
      this.names.labels.container,
      this.places.names,
      this.bubbles.container,
    );
  }

  /** Returns true when the map of a zone is painted. */
  hasMap(zone: number): boolean {
    return this.ground.has(zone);
  }

  /** Paints the map of a zone: its tiles and its sites. */
  addMap(map: ZoneMap): void {
    this.ground.add(map);
    this.sites.add(map);
  }

  /** Frees the maps of the zones that are not in `zones`. */
  keepMaps(zones: ReadonlySet<number>): void {
    this.ground.keep(zones);
    this.sites.keep(zones);
  }

  /** Draws one frame. */
  draw(view: SceneView): void {
    applyCamera(this.stage.world, view.camera, view.size);
    this.land.update(view.level, view.state.populations);
    // The heat map shows numbers of residents, not streets.
    this.ground.container.visible = view.level !== 'country';
    this.sites.shapes.visible = view.level !== 'country';
    this.sites.draw(view);
    this.places.draw(view);
    const candidates = this.names.due(view) ? [] : null;
    this.residents.draw(view, view.state.speech, candidates);
    if (candidates !== null) this.names.choose(candidates, view);
    this.names.draw(view);
    this.bubbles.draw(view);
  }

  /** Returns what the last frame drew, for the check of the performance. */
  counts(): { residents: number; names: number; bubbles: number; siteLabels: number } {
    return {
      residents: this.residents.drawn,
      names: this.names.labels.size,
      bubbles: this.bubbles.size,
      siteLabels: this.sites.labels.size,
    };
  }
}
