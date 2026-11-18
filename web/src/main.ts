// The start of the viewer.

import './styles.css';
import { ClipPlayer } from './app/clipplayer';
import { ResidentPage } from './app/residentpage';
import { Viewer } from './app/viewer';
import { CONFIG } from './config';
import { ApiError, fetchClip, fetchLayout } from './net/api';
import { reconnectDelay } from './net/backoff';
import { createStage } from './render/stage';
import { liveLink, readOptions } from './state/params';
import type { Clip, Layout } from './state/types';
import { el, setText } from './ui/dom';

declare global {
  interface Window {
    /** What the page runs, for scripts that check it. */
    __unsup?: Viewer | ClipPlayer | ResidentPage;
  }
}

/** Asks the server until it answers. `ask` gives up for an error that another try cannot change. */
async function untilAnswer<T>(note: HTMLElement, ask: () => Promise<T>, final: (error: unknown) => boolean): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await ask();
    } catch (error) {
      if (final(error)) throw error;
      setText(note, 'The server does not answer. The viewer tries again.');
      const wait = reconnectDelay(attempt, CONFIG.start.retryMinMs, CONFIG.start.retryMaxMs, Math.random());
      await new Promise((resolve) => setTimeout(resolve, wait));
    }
  }
}

/** Reads the world from the server, and tries again until the server answers. */
function loadLayout(note: HTMLElement): Promise<Layout> {
  return untilAnswer(note, fetchLayout, () => false);
}

/** Reads a clip. Returns null when the server has no clip with this id. */
async function loadClip(note: HTMLElement, id: string): Promise<Clip | null> {
  const gone = (error: unknown): boolean => error instanceof ApiError && error.status >= 400 && error.status < 500;
  try {
    return await untilAnswer(note, () => fetchClip(id), gone);
  } catch {
    return null;
  }
}

async function start(root: HTMLElement): Promise<void> {
  const options = readOptions(globalThis.location.search);
  if (options.resident !== null) {
    // The page of a resident is words only: it needs no picture.
    const page = new ResidentPage(root, options.resident);
    window.__unsup = page;
    page.start();
    return;
  }
  root.classList.toggle('broadcast', options.broadcast);
  const surface = el('div', 'stage');
  const note = el('p', 'loading', 'Connecting to the world…');
  root.append(surface, note);
  let stage;
  try {
    stage = await createStage(surface);
  } catch (error) {
    setText(note, 'This browser cannot draw the world. The viewer needs WebGL.');
    console.error(error);
    return;
  }
  const layout = await loadLayout(note);
  if (options.clip === null) {
    note.remove();
    window.__unsup = new Viewer(root, surface, stage, layout, options);
    return;
  }
  const clip = await loadClip(note, options.clip);
  if (clip === null) {
    setText(note, 'The server does not have this clip any more.');
    const back = el('a', 'loading-link', 'Go to the live map');
    back.setAttribute('href', liveLink(globalThis.location.href));
    note.append(el('br'), back);
    note.classList.add('with-link');
    return;
  }
  note.remove();
  window.__unsup = new ClipPlayer(root, stage, layout, clip);
}

const root = document.getElementById('app');
if (root) void start(root);
