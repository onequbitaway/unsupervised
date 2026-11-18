// Text from residents is untrusted. This module makes it safe to show:
// plain characters only, one line, a bounded length. The viewer still
// renders the result only as text, never as HTML.

// C0 and C1 controls, the characters that change the direction of text,
// and characters that have no width.
const HIDDEN = /[\u0000-\u0008\u000e-\u001f\u007f-\u009f\u061c\u200b\u200e\u200f\u202a-\u202e\u2060-\u206f\ufeff\ufff9-\ufffb]|\udb40[\udc00-\udc7f]/g;
// Line breaks, tabs and the Unicode line separators.
const BREAKS = /[\t\n\u000b\u000c\r\u0085\u2028\u2029]+/g;
// More than two combining marks in a row.
const STACKED = /(\p{M}{2})\p{M}+/gu;

const segmenter =
  typeof Intl !== 'undefined' && 'Segmenter' in Intl
    ? new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    : null;

/** Splits text into the characters that a reader sees. */
function graphemes(value: string): string[] {
  if (segmenter) return Array.from(segmenter.segment(value), (part) => part.segment);
  return Array.from(value);
}

/**
 * Returns text that is safe to put on the screen: no control characters,
 * no direction overrides, one line, at most `max` characters. A value that
 * is not text gives an empty string. Text that was cut ends in "…".
 */
export function cleanText(value: unknown, max: number): string {
  if (typeof value !== 'string' || max <= 0) return '';
  // A cut before the cleaning bounds the work on a very long input.
  const bounded = value.length > max * 16 ? value.slice(0, max * 16) : value;
  const plain = bounded
    .replace(BREAKS, ' ')
    .replace(HIDDEN, '')
    .replace(STACKED, '$1')
    .replace(/\s+/g, ' ')
    .trim();
  if (plain.length <= max) return plain;
  const parts = graphemes(plain);
  if (parts.length <= max) return plain;
  return `${parts.slice(0, Math.max(1, max - 1)).join('').trimEnd()}…`;
}
