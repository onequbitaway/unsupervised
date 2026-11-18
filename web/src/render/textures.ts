// From painted pixels to textures of the renderer.

import { ImageSource } from 'pixi.js';

/** Copies painted pixels into a canvas. */
export function toCanvas(data: Uint8ClampedArray, width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext('2d');
  if (context === null) throw new Error('This browser cannot draw on a canvas.');
  const image = context.createImageData(width, height);
  image.data.set(data);
  context.putImageData(image, 0, 0);
  return canvas;
}

/**
 * Makes the source of a texture from a canvas. Small copies of the image
 * are made, so that it stays calm when it is drawn small. `crisp` keeps
 * the pixels sharp when the image is drawn large.
 */
export function sourceOf(canvas: HTMLCanvasElement, crisp: boolean): ImageSource {
  return new ImageSource({
    resource: canvas,
    autoGenerateMipmaps: true,
    minFilter: 'linear',
    mipmapFilter: 'linear',
    magFilter: crisp ? 'nearest' : 'linear',
  });
}
