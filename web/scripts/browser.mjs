// Opens the viewer in the Google Chrome of this machine, without a window.
// No browser is downloaded: `playwright-core` only drives the one that is
// installed. Set CHROME_PATH when Chrome is in another place.

import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chromium } from 'playwright-core';

const CHROME = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

// The options for Chrome. The script gives its own short list and not the
// defaults of Playwright: with those defaults, Chrome 158 without a window
// stops about five seconds after a page opens (`--disable-field-trial-config`
// is one cause). The three `backgrounding` options keep the frames of a
// page without a window at full speed, which the measurement needs.
const OPTIONS = [
  '--headless',
  '--remote-debugging-pipe',
  '--no-startup-window',
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-search-engine-choice-screen',
  '--hide-scrollbars',
  '--mute-audio',
  '--force-color-profile=srgb',
  '--disable-background-timer-throttling',
  '--disable-backgrounding-occluded-windows',
  '--disable-renderer-backgrounding',
];

/**
 * Opens the viewer at `url` and waits until the picture shows everything
 * that the camera asks for. Returns the page and a function that closes
 * the browser and removes its temporary profile.
 */
export async function openViewer(url, { width, height, timeoutMs }) {
  if (!existsSync(CHROME)) {
    throw new Error(`Google Chrome is not at ${CHROME}. Set CHROME_PATH to the Chrome program.`);
  }
  const profile = await mkdtemp(join(tmpdir(), 'unsupervised-viewer-'));
  let browser = null;
  const close = async () => {
    // The browser may be gone already. The profile must go in both cases.
    await browser?.close().catch(() => {});
    await rm(profile, { recursive: true, force: true });
  };
  try {
    browser = await chromium.launch({
      executablePath: CHROME,
      ignoreDefaultArgs: true,
      args: [...OPTIONS, `--user-data-dir=${profile}`],
    });
    const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
    page.on('pageerror', (error) => console.error(`page error: ${error.message}`));
    page.on('console', (message) => {
      if (message.type() === 'error' || message.type() === 'warning') console.error(`page ${message.type()}: ${message.text()}`);
    });
    await page.goto(url, { waitUntil: 'load', timeout: timeoutMs });
    await page.waitForFunction(() => window.__unsup?.stats().settled === true, null, { timeout: timeoutMs });
    return { page, close };
  } catch (error) {
    await close();
    throw error;
  }
}

/** Returns the address of the viewer with more options in its query. */
export function viewerUrl(base, query) {
  const url = new URL(base);
  for (const [name, value] of new URLSearchParams(query ?? '')) url.searchParams.set(name, value);
  return url.toString();
}
