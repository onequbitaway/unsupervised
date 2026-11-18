// A pool of text labels on the screen. A label belongs to a key (a
// resident, a site, a city). It keeps its text object while it is shown,
// so the text is drawn once and then only moved.

import { Container, Text, TextStyle, type TextStyleOptions } from 'pixi.js';

/** The font of every label: the font of the system, no download. */
export const FONT = 'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';

export class LabelPool {
  readonly container = new Container();
  private readonly style: TextStyle;
  private readonly live = new Map<number | string, Text>();
  private readonly used = new Set<number | string>();
  private readonly spare: Text[] = [];

  /** `anchorX` and `anchorY` say which point of a label sits at its position, from 0 to 1. */
  constructor(
    style: TextStyleOptions,
    private readonly anchorX: number,
    private readonly anchorY: number,
  ) {
    this.style = new TextStyle(style);
  }

  /** Sets the size of the font. Every label is drawn again. */
  setFontSize(pixels: number): void {
    if (this.style.fontSize !== pixels) this.style.fontSize = pixels;
  }

  /** Starts a new round. Labels that are not placed before `end` go away. */
  begin(): void {
    this.used.clear();
  }

  /**
   * Shows the label of `key` with `content` at a point of the screen.
   * `content` must be clean text already. It is drawn as text; nothing in
   * it is ever read as markup.
   */
  place(key: number | string, content: string, x: number, y: number, alpha = 1): void {
    let label = this.live.get(key);
    if (label === undefined) {
      label = this.spare.pop() ?? new Text({ text: '', style: this.style });
      label.anchor.set(this.anchorX, this.anchorY);
      label.visible = true;
      this.container.addChild(label);
      this.live.set(key, label);
    }
    if (label.text !== content) label.text = content;
    label.position.set(x, y);
    label.alpha = alpha;
    this.used.add(key);
  }

  /** Ends the round: removes the labels that were not placed. */
  end(): void {
    for (const [key, label] of this.live) {
      if (this.used.has(key)) continue;
      this.live.delete(key);
      this.container.removeChild(label);
      if (this.spare.length < 64) this.spare.push(label);
      else label.destroy();
    }
  }

  /** Returns the number of labels on the screen. */
  get size(): number {
    return this.live.size;
  }
}

/** The style of a label that must be readable on any ground: light text, dark edge. */
export function outlined(fontSize: number, fill: number, stroke: number, weight: '600' | '700' | '800' = '700'): TextStyleOptions {
  return {
    fontFamily: FONT,
    fontSize,
    fontWeight: weight,
    fill,
    stroke: { color: stroke, width: Math.max(3, Math.round(fontSize * 0.28)), join: 'round' },
  };
}
