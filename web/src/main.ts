// The start of the viewer.

import './styles.css';
import { Viewer } from './app/viewer';
import { CONFIG } from './config';
import { fetchLayout } from './net/api';
import { reconnectDelay } from './net/backoff';
import { createStage } from './render/stage';
import { readOptions } from './state/params';
import type { Layout } from './state/types';
import { el, setText } from './ui/dom';

declare global {
  interface Window {
    /** The running viewer, for scripts that check it. */
    __unsup?: Viewer;
  }
}

/** Reads the world from the server, and tries again until the server answers. */
async function loadLayout(note: HTMLElement): Promise<Layout> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await fetchLayout();
    } catch {
      setText(note, 'The server does not answer. The viewer tries again.');
      const wait = reconnectDelay(attempt, CONFIG.start.retryMinMs, CONFIG.start.retryMaxMs, Math.random());
      await new Promise((resolve) => setTimeout(resolve, wait));
    }
  }
}

async function start(root: HTMLElement): Promise<void> {
  const options = readOptions(globalThis.location.search);
  root.classList.toggle('broadcast', options.broadcast);
  const surface = el('div', 'stage');
  const note = el('p', 'loading', 'Connecting to the world…');
  root.append(surface, note);
  try {
    const stage = await createStage(surface);
    const layout = await loadLayout(note);
    note.remove();
    window.__unsup = new Viewer(root, surface, stage, layout, options);
  } catch (error) {
    setText(note, 'This browser cannot draw the world. The viewer needs WebGL.');
    console.error(error);
  }
}

const root = document.getElementById('app');
if (root) void start(root);
