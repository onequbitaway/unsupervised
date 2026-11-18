import { describe, expect, it } from 'vitest';
import { readOptions, watchLink } from './params';

describe('readOptions', () => {
  it('has no option when the address has none', () => {
    expect(readOptions('')).toEqual({ watch: null, broadcast: false, view: null, zone: null });
  });

  it('reads the resident to follow, the broadcast flag, the level and the zone', () => {
    expect(readOptions('?watch=1048577&broadcast=1&view=city&zone=3')).toEqual({
      watch: 1048577,
      broadcast: true,
      view: 'city',
      zone: 3,
    });
    expect(readOptions('?broadcast').broadcast).toBe(true);
    expect(readOptions('?broadcast=true').broadcast).toBe(true);
    expect(readOptions('?broadcast=0').broadcast).toBe(false);
    expect(readOptions('?broadcast=false').broadcast).toBe(false);
  });

  it('refuses an id that is not a plain number', () => {
    for (const bad of ['', 'abc', '-4', '1.5', '1e3', ' 7', '0x10', '99999999999999999', '<script>']) {
      expect(readOptions(`?watch=${encodeURIComponent(bad)}`).watch, bad).toBeNull();
    }
    expect(readOptions('?watch=0').watch).toBe(0);
    expect(readOptions('?view=moon').view).toBeNull();
  });
});

describe('watchLink', () => {
  it('sets the resident and keeps the other options', () => {
    expect(watchLink('http://host:8080/?broadcast=1', 42)).toBe('http://host:8080/?broadcast=1&watch=42');
    expect(watchLink('http://host/?watch=7&broadcast=1', 9)).toBe('http://host/?watch=9&broadcast=1');
  });

  it('removes the place, and removes the resident when there is none', () => {
    expect(watchLink('http://host/?zone=3&view=street', 5)).toBe('http://host/?watch=5');
    expect(watchLink('http://host/?watch=7&broadcast=1', null)).toBe('http://host/?broadcast=1');
  });
});
