// The surface that the viewer draws on, and the two spaces of the scene:
// the world, in tiles, which the camera moves; and the screen, in pixels.

import { Application, Container } from 'pixi.js';
import type { Camera, Size } from '../state/camera';
import { COLORS } from './palette';

export interface Stage {
  app: Application;
  /** Children are placed in tiles. The camera moves and scales this space. */
  world: Container;
  /** Children are placed in pixels of the screen. It is drawn over the world. */
  screen: Container;
}

/** Makes the drawing surface and puts it into `parent`. */
export async function createStage(parent: HTMLElement): Promise<Stage> {
  const app = new Application();
  await app.init({
    resizeTo: parent,
    background: COLORS.background,
    antialias: true,
    autoDensity: true,
    // More than two device pixels for one pixel costs much and shows little.
    resolution: Math.min(globalThis.devicePixelRatio || 1, 2),
    preference: 'webgl',
    powerPreference: 'high-performance',
  });
  parent.appendChild(app.canvas);
  const world = new Container();
  const screen = new Container();
  app.stage.addChild(world, screen);
  return { app, world, screen };
}

/** Returns the size of the drawing surface in pixels of the page. */
export function sizeOf(stage: Stage): Size {
  return { width: stage.app.screen.width, height: stage.app.screen.height };
}

/** Moves the world so that the camera looks at its point. */
export function applyCamera(world: Container, camera: Camera, size: Size): void {
  world.scale.set(camera.scale);
  world.position.set(size.width / 2 - camera.x * camera.scale, size.height / 2 - camera.y * camera.scale);
}
