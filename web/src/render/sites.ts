// The sites of the districts: a coloured roof and an outline for each
// site, and its label at street level.

import { Container, Graphics } from 'pixi.js';
import { CONFIG } from '../config';
import { visibleRect, worldToScreen } from '../state/camera';
import { siteLabel } from '../state/sites';
import type { SiteView, ZoneMap } from '../state/types';
import { intersects } from '../state/zones';
import { LabelPool, outlined } from './labels';
import { COLORS, siteColor } from './palette';
import { labelScale, type SceneView, showsDetail } from './view';

interface District {
  roofs: Graphics;
  outlines: Graphics;
  sites: readonly { site: SiteView; label: string }[];
}

/** The part of the font size that one character is wide, on average. */
const CHAR_WIDTH = 0.56;

export class SiteLayer {
  /** In the world: roofs and outlines. */
  readonly shapes = new Container();
  /** On the screen: labels. */
  readonly labels = new LabelPool(outlined(CONFIG.render.siteFontPx, COLORS.siteText, COLORS.labelStroke), 0.5, 0.5);
  private readonly districts = new Map<number, District>();

  /** Shows the sites of a district. */
  add(map: ZoneMap): void {
    this.remove(map.zone.id);
    const roofs = new Graphics();
    const outlines = new Graphics();
    for (const site of map.sites) {
      const { x0, y0, x1, y1 } = site.rect;
      if (x1 <= x0 || y1 <= y0) continue;
      const color = siteColor(site.kind);
      roofs.rect(x0, y0, x1 - x0, y1 - y0).fill({ color });
      // The outline is inside the site, a quarter of a tile wide.
      outlines.rect(x0 + 0.125, y0 + 0.125, x1 - x0 - 0.25, y1 - y0 - 0.25).stroke({ color, width: 0.25 });
    }
    this.shapes.addChild(roofs, outlines);
    this.districts.set(map.zone.id, {
      roofs,
      outlines,
      sites: map.sites.map((site) => ({ site, label: siteLabel(site) })),
    });
  }

  /** Forgets the districts that are not in `zones`. */
  keep(zones: ReadonlySet<number>): void {
    for (const zone of [...this.districts.keys()]) {
      if (!zones.has(zone)) this.remove(zone);
    }
  }

  private remove(zone: number): void {
    const district = this.districts.get(zone);
    if (district === undefined) return;
    this.districts.delete(zone);
    this.shapes.removeChild(district.roofs, district.outlines);
    district.roofs.destroy();
    district.outlines.destroy();
  }

  draw(view: SceneView): void {
    const { camera, size, level } = view;
    // From far a site is a roof. From near the viewer looks into it.
    const roof = level === 'street' ? CONFIG.render.roofStreet : CONFIG.render.roofCity;
    for (const district of this.districts.values()) district.roofs.alpha = roof;

    const fontSize = Math.round(CONFIG.render.siteFontPx * labelScale(size));
    this.labels.setFontSize(fontSize);
    this.labels.begin();
    if (showsDetail(level, camera.scale)) {
      const seen = visibleRect(camera, size);
      let shown = 0;
      for (const district of this.districts.values()) {
        for (const { site, label } of district.sites) {
          if (shown >= CONFIG.render.siteLabelBudget) break;
          if (!intersects(site.rect, seen)) continue;
          const width = (site.rect.x1 - site.rect.x0) * camera.scale;
          if (label.length * fontSize * CHAR_WIDTH > width * CONFIG.render.siteLabelOverflow) continue;
          const at = worldToScreen(camera, size, (site.rect.x0 + site.rect.x1) / 2, (site.rect.y0 + site.rect.y1) / 2);
          this.labels.place(site.id, label, at.x, at.y);
          shown += 1;
        }
      }
    }
    this.labels.end();
  }
}
