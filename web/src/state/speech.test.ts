import { describe, expect, it } from 'vitest';
import { rawSpeech, speechText } from './speech';
import type { FeedEvent } from './types';

function said(data: unknown, kind = 'society.said'): FeedEvent {
  return { id: '1-0', frame: 1, clock: { year: 1, day: 1, hour: 0, minute: 1 }, kind, drama: 3, data };
}

describe('speech', () => {
  it('reads the words in every documented shape', () => {
    expect(rawSpeech({ text: { untrusted: true, from: 7, text: 'nested' } })).toBe('nested');
    expect(rawSpeech({ untrusted: true, from: 7, text: 'direct' })).toBe('direct');
    expect(rawSpeech({ text: 'plain field' })).toBe('plain field');
    expect(rawSpeech('plain')).toBe('plain');
  });

  it('gives null when there are no words', () => {
    for (const data of [null, undefined, 5, {}, { text: 5 }, { text: { text: [] } }, []]) {
      expect(rawSpeech(data)).toBeNull();
    }
  });

  it('only a speech event has speech', () => {
    expect(speechText(said({ text: 'hi' }))).toBe('hi');
    expect(speechText(said({ text: 'hi' }, 'society.posted'))).toBeNull();
    expect(speechText(said({ text: ' \n\u0000 ' }))).toBeNull();
  });

  it('cuts the words for a bubble and keeps them one line', () => {
    const long = speechText(said({ text: `first line\n${'y'.repeat(500)}` }));
    expect(long).toHaveLength(84);
    expect(long?.startsWith('first line yyy')).toBe(true);
    expect(long?.endsWith('…')).toBe(true);
  });
});
