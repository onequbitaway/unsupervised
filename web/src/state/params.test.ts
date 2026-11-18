import { describe, expect, it } from 'vitest';
import { clipLink, liveLink, readOptions, residentLink, watchLink } from './params';

describe('readOptions', () => {
  it('has no option when the address has none', () => {
    expect(readOptions('')).toEqual({
      watch: null,
      broadcast: false,
      resident: null,
      clip: null,
      view: null,
      zone: null,
    });
  });

  it('reads the resident to follow, the broadcast flag, the level and the zone', () => {
    expect(readOptions('?watch=1048577&broadcast=1&view=city&zone=3')).toEqual({
      watch: 1048577,
      broadcast: true,
      resident: null,
      clip: null,
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

describe('the page of a resident and the player of a clip', () => {
  it('reads the resident of the page and the event of the clip', () => {
    expect(readOptions('?resident=1048577').resident).toBe(1048577);
    expect(readOptions('?clip=120-3').clip).toBe('120-3');
  });

  it('refuses a clip id that is not two plain numbers with a dash', () => {
    for (const bad of ['', '120', '120-', '-3', 'a-3', '1-2-3', '1.5-2', ' 1-2', '1-2 ', '<b>-1', '1-99999999990']) {
      expect(readOptions(`?clip=${encodeURIComponent(bad)}`).clip, bad).toBeNull();
    }
    expect(readOptions('?resident=x').resident).toBeNull();
  });

  it('makes links that keep the server and drop the other views', () => {
    const here = 'https://unsupervised.example/play/?broadcast=1&watch=7&zone=3&theme=dark#top';
    expect(residentLink(here, 12)).toBe('https://unsupervised.example/play/?theme=dark&resident=12');
    expect(clipLink(here, '120-3')).toBe('https://unsupervised.example/play/?theme=dark&clip=120-3');
    expect(liveLink(here)).toBe('https://unsupervised.example/play/?theme=dark');
    // The link of a clip that came from the page itself is the same link.
    expect(clipLink(clipLink(here, '120-3'), '120-3')).toBe(clipLink(here, '120-3'));
  });
});
