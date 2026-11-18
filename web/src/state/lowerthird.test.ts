import { describe, expect, it } from 'vitest';
import { CONFIG } from '../config';
import { breakingBanner, lowerThird, shotPlace } from './lowerthird';
import { emptyShow, reduceShow } from './show';
import type { Layout, Shot } from './types';

const clock = { year: 1, day: 1, hour: 0, minute: 0 };
const layout: Layout = {
  frame: 0,
  clock,
  cities: [{ id: 0, name: 'Latent City', bounds: { x0: 0, y0: 0, x1: 128, y1: 128 }, zones: [3] }],
  zones: [{ id: 3, city: 0, name: 'Downtown', bounds: { x0: 0, y0: 0, x1: 64, y1: 64 }, residents: 0 }],
};

function shot(extra: Partial<Shot> = {}): Shot {
  return {
    seq: 2,
    frame: 20,
    clock,
    zone: 3,
    focus: { type: 'resident', id: 7, name: 'Ada' },
    zoom: 'close',
    caption: 'Bo stole $45.00 from Ada',
    breaking: false,
    drama: 55,
    ...extra,
  };
}

describe('lowerThird', () => {
  it('shows nothing without a shot or without a caption', () => {
    expect(lowerThird(null, layout).visible).toBe(false);
    expect(lowerThird(shot({ caption: '  \n ' }), layout).visible).toBe(false);
  });

  it('says what happens, where, and in which story', () => {
    const third = lowerThird(shot({ story: 4, label: 'Crime', headline: 'The bank job' }), layout);
    expect(third).toEqual({
      visible: true,
      tag: 'CRIME',
      caption: 'Bo stole $45.00 from Ada',
      headline: 'The bank job',
      place: 'Latent City · Downtown',
      breaking: false,
    });
  });

  it('leaves out a headline that repeats the caption', () => {
    const third = lowerThird(shot({ label: 'Crime', headline: 'Bo stole $45.00 from Ada' }), layout);
    expect([third.caption, third.headline]).toEqual(['Bo stole $45.00 from Ada', '']);
  });

  it('names a shot without a story LIVE and a wide view OVERVIEW', () => {
    expect(lowerThird(shot(), layout).tag).toBe('LIVE');
    const wide = lowerThird(shot({ focus: { type: 'none' }, zoom: 'city', city: 0, caption: 'Latent City · 30 residents' }), layout);
    expect([wide.tag, wide.place]).toEqual(['OVERVIEW', '']);
    // Breaking news without a place still says where the camera is.
    const news = lowerThird(shot({ focus: { type: 'none' }, zoom: 'city', city: 0, zone: undefined, breaking: true }), layout);
    expect([news.place, news.breaking]).toEqual(['Latent City', true]);
  });

  it('cleans and cuts text that residents chose', () => {
    const hostile = `<img src=x onerror=alert(1)>\n\u202e${'x'.repeat(400)}`;
    const third = lowerThird(shot({ caption: hostile, headline: hostile.replace('x', 'y'), label: 'crime\nwave of the century' }), layout);
    expect(third.caption.length).toBe(CONFIG.broadcast.captionMax);
    expect(third.caption.startsWith('<img src=x onerror=alert(1)> xxx')).toBe(true);
    expect(third.caption.endsWith('…')).toBe(true);
    expect(third.caption).not.toMatch(/[\n\u202e]/);
    expect(third.tag).toBe('CRIME WAVE OF THE…');
  });

  it('finds the place of a shot from its district or its city', () => {
    expect(shotPlace(shot({ zone: 99, city: 0 }), layout)).toBe('Latent City');
    expect(shotPlace(shot({ zone: 99 }), layout)).toBe('');
  });
});

describe('breakingBanner', () => {
  it('shows the caption of breaking news for a time', () => {
    const news = shot({ breaking: true, caption: 'Ada died at the age of 81' });
    const show = reduceShow(emptyShow(), { op: 'director', shot: news }, 10_000);
    expect(breakingBanner(show, 10_000)).toBe('Ada died at the age of 81');
    expect(breakingBanner(show, 10_000 + CONFIG.broadcast.breakingMs - 1)).toBe('Ada died at the age of 81');
    expect(breakingBanner(show, 10_000 + CONFIG.broadcast.breakingMs)).toBe('');
    // New words of the same shot do not start the time again.
    const later = reduceShow(show, { op: 'director', shot: { ...news, caption: 'More news' } }, 20_000);
    expect(breakingBanner(later, 20_000)).toBe('More news');
    expect(breakingBanner(later, 10_000 + CONFIG.broadcast.breakingMs)).toBe('');
  });

  it('shows nothing for a shot that is not breaking news', () => {
    const show = reduceShow(emptyShow(), { op: 'director', shot: shot() }, 0);
    expect(breakingBanner(show, 1)).toBe('');
    expect(breakingBanner(emptyShow(), 1)).toBe('');
  });
});
