// Speech: what a `society.said` event says.

import { CONFIG } from '../config';
import { cleanText } from './text';
import type { FeedEvent } from './types';

/** The kind of the event for words that a resident said aloud. */
export const SAID = 'society.said';

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/**
 * Returns the raw words of a speech event, or null when the event has
 * none. The society module wraps the words of a resident in an `Untrusted`
 * object (`{ untrusted: true, from, text }`). The words can be the data
 * itself or its `text` field; plain text is accepted too.
 */
export function rawSpeech(data: unknown): string | null {
  if (typeof data === 'string') return data;
  if (!isObject(data)) return null;
  const inner = data.text;
  if (typeof inner === 'string') return inner;
  if (isObject(inner) && typeof inner.text === 'string') return inner.text;
  return null;
}

/** Returns the words of a speech event, safe to show and cut for a bubble. */
export function speechText(event: FeedEvent): string | null {
  if (event.kind !== SAID) return null;
  const raw = rawSpeech(event.data);
  if (raw === null) return null;
  const words = cleanText(raw, CONFIG.text.speechMax);
  return words === '' ? null : words;
}
