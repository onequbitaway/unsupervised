// Speech bubbles: the words of a `society.said` event above the resident
// that said them.

import { Container, Graphics, Text, TextStyle } from 'pixi.js';
import { CONFIG } from '../config';
import { positionAt } from '../state/motion';
import { findMover, type Speech } from '../state/world';
import { FONT } from './labels';
import { COLORS } from './palette';
import { figureHeight, labelScale, type SceneView, showsDetail } from './view';

/** One bubble. Its origin is the tip of its tail. */
class Bubble {
  readonly container = new Container();
  private readonly back = new Graphics();
  private readonly words: Text;
  private shown = '';
  private fontSize = 0;

  constructor(style: TextStyle) {
    this.words = new Text({ text: '', style });
    this.words.anchor.set(0.5, 1);
    this.container.addChild(this.back, this.words);
  }

  /** Sets the words. `text` is clean text; it is drawn as text only. */
  set(text: string, fontSize: number): void {
    if (text === this.shown && fontSize === this.fontSize) return;
    this.shown = text;
    this.fontSize = fontSize;
    this.words.text = text;
    const padX = Math.round(fontSize * 0.6);
    const padY = Math.round(fontSize * 0.4);
    const tail = Math.round(fontSize * 0.55);
    const width = this.words.width + padX * 2;
    const height = this.words.height + padY * 2;
    this.words.position.set(0, -tail - padY);
    this.back
      .clear()
      .roundRect(-width / 2, -tail - height, width, height, Math.round(fontSize * 0.55))
      .fill({ color: COLORS.bubble })
      .stroke({ color: COLORS.bubbleText, width: 2.5 })
      .poly([-tail * 0.7, -tail - 1.5, tail * 0.7, -tail - 1.5, 0, 0])
      .fill({ color: COLORS.bubble })
      .moveTo(-tail * 0.7, -tail)
      .lineTo(0, 0)
      .lineTo(tail * 0.7, -tail)
      .stroke({ color: COLORS.bubbleText, width: 2.5 });
  }
}

export class BubbleLayer {
  readonly container = new Container();
  private readonly style = new TextStyle({
    fontFamily: FONT,
    fontSize: CONFIG.render.bubbleFontPx,
    fontWeight: '600',
    fill: COLORS.bubbleText,
    wordWrap: true,
    wordWrapWidth: CONFIG.render.bubbleWidthPx,
    // A long word without a space must not make a wide bubble.
    breakWords: true,
  });
  private readonly live = new Map<number, Bubble>();
  private readonly spare: Bubble[] = [];

  draw(view: SceneView): void {
    const { camera, size, state, now } = view;
    const scale = labelScale(size);
    const fontSize = Math.round(CONFIG.render.bubbleFontPx * scale);
    if (this.style.fontSize !== fontSize) {
      this.style.fontSize = fontSize;
      this.style.wordWrapWidth = Math.round(CONFIG.render.bubbleWidthPx * scale);
    }
    const height = figureHeight(camera.scale);

    // The newest speech wins when there are more than the screen can hold.
    let speeches: Speech[] = [];
    if (showsDetail(view.level, camera.scale)) {
      speeches = [...state.speech.values()].sort((a, b) => a.until - b.until).slice(-CONFIG.speech.maxBubbles);
    }
    const used = new Set<number>();
    for (const speech of speeches) {
      const found = findMover(state, speech.resident);
      if (!found) continue;
      const at = positionAt(found.mover, now);
      const sx = (at.x + 0.5 - camera.x) * camera.scale + size.width / 2;
      const sy = (at.y + 0.5 - camera.y) * camera.scale + size.height / 2;
      if (sx < -200 || sx > size.width + 200 || sy < 0 || sy > size.height + 400) continue;
      let bubble = this.live.get(speech.resident);
      if (bubble === undefined) {
        bubble = this.spare.pop() ?? new Bubble(this.style);
        this.live.set(speech.resident, bubble);
      }
      bubble.set(speech.text, fontSize);
      bubble.container.position.set(sx, sy - height - 4);
      bubble.container.alpha = Math.min(1, Math.max(0, (speech.until - now) / CONFIG.render.bubbleFadeMs));
      // Added again each frame, so that a newer bubble is above an older one.
      this.container.addChild(bubble.container);
      used.add(speech.resident);
    }
    for (const [resident, bubble] of this.live) {
      if (used.has(resident)) continue;
      this.live.delete(resident);
      this.container.removeChild(bubble.container);
      if (this.spare.length < CONFIG.speech.maxBubbles) this.spare.push(bubble);
      else bubble.container.destroy({ children: true });
    }
  }

  /** Returns the number of bubbles on the screen. */
  get size(): number {
    return this.live.size;
  }
}
