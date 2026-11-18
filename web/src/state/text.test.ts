import { describe, expect, it } from 'vitest';
import { cleanText } from './text';

describe('cleanText', () => {
  it('keeps plain text', () => {
    expect(cleanText('Good morning!', 40)).toBe('Good morning!');
    expect(cleanText('¡Hola! 你好 مرحبا', 40)).toBe('¡Hola! 你好 مرحبا');
  });

  it('keeps markup as characters, it does not remove or escape it', () => {
    const hostile = '<img src=x onerror=alert(1)><b>bold</b>';
    expect(cleanText(hostile, 80)).toBe(hostile);
  });

  it('gives an empty string for a value that is not text', () => {
    for (const value of [undefined, null, 42, { text: 'x' }, ['x'], true]) {
      expect(cleanText(value, 10)).toBe('');
    }
    expect(cleanText('text', 0)).toBe('');
  });

  it('makes one line', () => {
    expect(cleanText('line one\nline two\r\n\n\tline five', 80)).toBe('line one line two line five');
    expect(cleanText('a\u2028b\u2029c\u0085d', 80)).toBe('a b c d');
    expect(cleanText('   padded   ', 80)).toBe('padded');
  });

  it('removes control characters and direction overrides', () => {
    expect(cleanText('left\u202ethgir', 80)).toBe('leftthgir');
    expect(cleanText('a\u0000b\u0007c\u001bd\u009be', 80)).toBe('abcde');
    expect(cleanText('x\u200by\ufeffz\u2066w\u2069', 80)).toBe('xyzw');
    expect(cleanText('tag\u{e0041}\u{e007f}s', 80)).toBe('tags');
  });

  it('limits stacked combining marks', () => {
    const stacked = `e${'\u0301'.repeat(40)}nd`;
    expect(cleanText(stacked, 80)).toBe('e\u0301\u0301nd');
  });

  it('cuts long text and marks the cut', () => {
    expect(cleanText('abcdefghij', 10)).toBe('abcdefghij');
    expect(cleanText('abcdefghijk', 10)).toBe('abcdefghi…');
    expect(cleanText('word and more words', 9)).toBe('word and…');
    expect(cleanText('x'.repeat(100_000), 20)).toBe(`${'x'.repeat(19)}…`);
  });

  it('never cuts inside one visible character', () => {
    const family = '👨‍👩‍👧‍👦';
    const cut = cleanText(family.repeat(6), 4);
    expect(cut).toBe(`${family.repeat(3)}…`);
    const flags = cleanText('🇫🇷🇩🇪🇯🇵🇧🇷', 3);
    expect(flags).toBe('🇫🇷🇩🇪…');
  });
});
