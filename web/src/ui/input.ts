// The pointer, the wheel and the keys: how a viewer moves the camera.

import { CONFIG } from '../config';

export type KeyAction = 'in' | 'out' | 'country' | 'city' | 'street' | 'left' | 'right' | 'up' | 'down' | 'escape';

const KEYS: Record<string, KeyAction> = {
  '+': 'in',
  '=': 'in',
  '-': 'out',
  _: 'out',
  '1': 'country',
  '2': 'city',
  '3': 'street',
  ArrowLeft: 'left',
  ArrowRight: 'right',
  ArrowUp: 'up',
  ArrowDown: 'down',
  a: 'left',
  d: 'right',
  w: 'up',
  s: 'down',
  Escape: 'escape',
};

/** Returns what a key does, or null when it does nothing in the viewer. */
export function keyAction(key: string): KeyAction | null {
  return Object.hasOwn(KEYS, key) ? (KEYS[key] ?? null) : null;
}

/** Returns the zoom factor of a turn of the wheel: above 1 zooms in. */
export function wheelFactor(deltaY: number, pinch: boolean): number {
  const bounded = Math.max(-240, Math.min(240, deltaY));
  // A pinch on a touch pad gives small numbers.
  return Math.exp(-bounded * (pinch ? 0.01 : 0.0018));
}

export interface InputHandlers {
  /** The pointer dragged the picture by this many pixels. */
  pan(dx: number, dy: number): void;
  /** Zoom by a factor around a point of the screen. */
  zoomAt(factor: number, x: number, y: number): void;
  /** A click at a point of the screen. */
  click(x: number, y: number): void;
  key(action: KeyAction): void;
}

/** Listens on `target` and on the keys of the page. Returns a function that stops it. */
export function attachInput(target: HTMLElement, handlers: InputHandlers): () => void {
  const pointers = new Map<number, { x: number; y: number }>();
  let travelled = 0;
  let pinch = 0;

  const local = (event: MouseEvent): { x: number; y: number } => {
    const box = target.getBoundingClientRect();
    return { x: event.clientX - box.left, y: event.clientY - box.top };
  };
  const spread = (): number => {
    const [a, b] = [...pointers.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  };

  const down = (event: PointerEvent): void => {
    if (event.pointerType === 'mouse' && event.button !== 0) return;
    target.setPointerCapture(event.pointerId);
    pointers.set(event.pointerId, local(event));
    if (pointers.size === 1) travelled = 0;
    pinch = spread();
  };
  const move = (event: PointerEvent): void => {
    const last = pointers.get(event.pointerId);
    if (!last) return;
    const at = local(event);
    pointers.set(event.pointerId, at);
    if (pointers.size === 1) {
      travelled += Math.abs(at.x - last.x) + Math.abs(at.y - last.y);
      if (travelled >= CONFIG.view.clickPx) handlers.pan(at.x - last.x, at.y - last.y);
    } else {
      // Two fingers: the distance between them is the zoom.
      const now = spread();
      const [a, b] = [...pointers.values()];
      if (pinch > 0 && now > 0 && a && b) handlers.zoomAt(now / pinch, (a.x + b.x) / 2, (a.y + b.y) / 2);
      pinch = now;
      travelled = Infinity;
    }
  };
  const up = (event: PointerEvent): void => {
    if (!pointers.delete(event.pointerId)) return;
    if (pointers.size === 0 && travelled < CONFIG.view.clickPx && event.type === 'pointerup') {
      const at = local(event);
      handlers.click(at.x, at.y);
    }
    pinch = spread();
  };
  const wheel = (event: WheelEvent): void => {
    event.preventDefault();
    const at = local(event);
    handlers.zoomAt(wheelFactor(event.deltaY, event.ctrlKey), at.x, at.y);
  };
  const key = (event: KeyboardEvent): void => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    // A key in a field or on a button belongs to that element.
    const on = event.target instanceof HTMLElement ? event.target.tagName : '';
    if (on === 'INPUT' || on === 'TEXTAREA') return;
    const action = keyAction(event.key);
    if (action === null) return;
    event.preventDefault();
    handlers.key(action);
  };

  target.addEventListener('pointerdown', down);
  target.addEventListener('pointermove', move);
  target.addEventListener('pointerup', up);
  target.addEventListener('pointercancel', up);
  target.addEventListener('wheel', wheel, { passive: false });
  globalThis.addEventListener('keydown', key);
  return () => {
    target.removeEventListener('pointerdown', down);
    target.removeEventListener('pointermove', move);
    target.removeEventListener('pointerup', up);
    target.removeEventListener('pointercancel', up);
    target.removeEventListener('wheel', wheel);
    globalThis.removeEventListener('keydown', key);
  };
}
