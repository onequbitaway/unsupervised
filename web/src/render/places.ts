// The names of the cities and of the districts, with their numbers of
// residents, and the lines around them.

import { Container, Graphics } from 'pixi.js';
import { CONFIG } from '../config';
import { visibleRect, worldToScreen } from '../state/camera';
import { cleanText } from '../state/text';
import type { Rect } from '../state/types';
import { intersects } from '../state/zones';
import { LabelPool, outlined } from './labels';
import { COLORS } from './palette';
import { labelScale, type SceneView } from './view';

/** Writes a number of residents: `1,234 residents`. */
export function residentsLabel(count: number): string {
  return `${count.toLocaleString('en-US')} ${count === 1 ? 'resident' : 'residents'}`;
}

export class PlaceLayer {
  /** The lines around cities and districts. They go below the residents. */
  readonly lines = new Graphics();
  /** The names. They go above the residents. */
  readonly names = new Container();
  private readonly cities = new LabelPool(outlined(CONFIG.render.cityFontPx, COLORS.placeText, COLORS.labelStroke, '800'), 0.5, 1);
  private readonly counts = new LabelPool(outlined(CONFIG.render.districtFontPx, COLORS.siteText, COLORS.labelStroke), 0.5, 0);
  private readonly districts = new LabelPool(outlined(CONFIG.render.districtFontPx, COLORS.placeText, COLORS.labelStroke), 0.5, 0.5);

  constructor() {
    this.names.addChild(this.districts.container, this.cities.container, this.counts.container);
  }

  draw(view: SceneView): void {
    const { camera, size, level, layout, state } = view;
    const scale = labelScale(size);
    this.cities.setFontSize(Math.round(CONFIG.render.cityFontPx * scale));
    this.counts.setFontSize(Math.round(CONFIG.render.districtFontPx * scale));
    this.districts.setFontSize(Math.round(CONFIG.render.districtFontPx * scale));
    const seen = visibleRect(camera, size);
    const onScreen = (rect: Rect): { x: number; y: number; width: number; height: number } => {
      const a = worldToScreen(camera, size, rect.x0, rect.y0);
      return { x: a.x, y: a.y, width: (rect.x1 - rect.x0) * camera.scale, height: (rect.y1 - rect.y0) * camera.scale };
    };

    this.lines.clear();
    this.cities.begin();
    this.counts.begin();
    this.districts.begin();

    // Cities: an outline and the name above it, at the two far levels.
    if (level !== 'street') {
      for (const city of layout.cities) {
        if (!intersects(city.bounds, seen)) continue;
        const box = onScreen(city.bounds);
        const pad = 5;
        this.lines
          .rect(box.x - pad, box.y - pad, box.width + pad * 2, box.height + pad * 2)
          .stroke({ color: COLORS.cityOutline, width: 3, alpha: level === 'country' ? 0.95 : 0.6 });
        let residents = 0;
        for (const zone of city.zones) residents += state.populations.get(zone) ?? 0;
        const name = cleanText(city.name, CONFIG.text.nameMax) || `City ${city.id}`;
        // The name stays on the screen when the top of the city is above it.
        const top = Math.max(box.y - pad - 4, Math.round(CONFIG.render.cityFontPx * scale) + 8);
        this.cities.place(city.id, name, box.x + box.width / 2, top);
        if (level === 'country') {
          this.counts.place(city.id, residentsLabel(residents), box.x + box.width / 2, box.y + box.height + pad + 6);
        }
      }
    }

    // Districts: lines between them, and their names at the city level.
    if (level !== 'country') {
      for (const zone of layout.zones) {
        if (!intersects(zone.bounds, seen)) continue;
        const box = onScreen(zone.bounds);
        this.lines
          .rect(box.x, box.y, box.width, box.height)
          .stroke({ color: COLORS.districtLine, width: level === 'city' ? 3 : 2, alpha: level === 'city' ? 0.9 : 0.45 });
        if (level !== 'city') continue;
        const name = cleanText(zone.name, CONFIG.text.nameMax) || `District ${zone.id}`;
        const count = state.populations.get(zone.id) ?? 0;
        this.districts.place(zone.id, `${name} · ${count.toLocaleString('en-US')}`, box.x + box.width / 2, box.y + box.height / 2);
      }
    }

    this.cities.end();
    this.counts.end();
    this.districts.end();
  }
}
