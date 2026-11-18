import { describe, expect, it } from 'vitest';
import { CONFIG } from '../config';
import { readLife } from '../net/show';
import { lifePage } from './life';
import type { LifeEntry, LifeStory, Profile } from './types';

const clock = { year: 2, day: 5, hour: 9, minute: 3 };

function entry(frame: number, line: string): LifeEntry {
  return { event: `${frame}-0`, frame, clock, kind: 'x.y', drama: 20, line };
}

function profile(life: Partial<LifeStory>, extra: Partial<Profile> = {}): Profile {
  return {
    id: 7,
    name: 'Ada',
    model: 'atlas-2',
    alive: true,
    zone: 3,
    zone_name: 'Downtown',
    city: 1,
    city_name: 'Tokentown',
    pos: { x: 70, y: 9 },
    activity: 'buy_property',
    frame: 40,
    events: [],
    life: { ...readLife(null), ...life },
    ...extra,
  };
}

describe('lifePage', () => {
  it('says who the resident is and what it does', () => {
    const page = lifePage(profile({}));
    expect([page.name, page.id, page.model, page.alive]).toEqual(['Ada', '#7', 'atlas-2', true]);
    expect(page.status).toBe('In Tokentown · Downtown · buy property');
    // A life without a history has no chapter.
    expect(page.chapters).toEqual([]);
  });

  it('puts the chapters in the order of a life and leaves empty ones out', () => {
    const page = lifePage(
      profile({
        born: entry(1, 'Ada arrived in Plaza'),
        crimes: [entry(20, 'Bo stole $45.00 from Ada')],
        jobs: [entry(5, 'Ada got a job at First Bank'), entry(9, 'Ada left the job')],
        death: entry(40, 'Ada died at the age of 81 (old age)'),
      }),
    );
    expect(page.chapters.map((chapter) => [chapter.id, chapter.title, chapter.lines.length])).toEqual([
      ['born', 'Arrival', 1],
      ['jobs', 'Work', 2],
      ['crimes', 'Crime and law', 1],
      ['death', 'The end', 1],
    ]);
    expect(page.chapters[1]?.lines[0]).toEqual({
      event: '5-0',
      when: 'Year 2 · Day 5 · 09:03',
      line: 'Ada got a job at First Bank',
    });
  });

  it('says how a resident left the world', () => {
    const gone = lifePage(profile({ age: 81.7 }, { alive: false, cause: 'old_age' }));
    expect(gone.status).toBe('Left the world at the age of 81 (old age) · last seen in Tokentown · Downtown');
    expect(gone.alive).toBe(false);
    const plain = lifePage(profile({}, { alive: false, zone_name: '', city_name: '' }));
    expect(plain.status).toBe('Left the world · last seen in zone 3');
  });

  it('cleans and cuts the text of residents', () => {
    const hostile = `<script>alert(1)</script>\n\u202e${'z'.repeat(900)}`;
    const page = lifePage(profile({ moments: [entry(3, hostile), entry(4, ' \n ')] }, { name: hostile, model: hostile }));
    expect(page.name.length).toBe(40);
    expect(page.model.length).toBe(CONFIG.text.modelMax);
    const lines = page.chapters[0]?.lines ?? [];
    // The line that is only spaces is left out.
    expect(lines).toHaveLength(1);
    expect(lines[0]?.line.length).toBe(CONFIG.page.lineMax);
    expect(lines[0]?.line).not.toMatch(/[\n\u202e]/);
  });
});

describe('readLife', () => {
  it('gives an empty life story for a value that is not one', () => {
    for (const bad of [null, undefined, 5, 'life', []]) {
      expect(readLife(bad)).toEqual({ jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] });
    }
  });

  it('reads the chapters and drops entries that are not valid', () => {
    const good = { event: '5-0', frame: 5, clock, kind: 'economy.worker_hired', drama: 12, line: 'hired', zone: 2, site: 9 };
    const life = readLife({ born: good, jobs: [good, { frame: 5 }, 'x'], crimes: 'none', age: 81, death: { line: 'no id' } });
    expect(life.born).toEqual(good);
    expect(life.jobs).toEqual([good]);
    expect(life.crimes).toEqual([]);
    expect(life.age).toBe(81);
    expect(life.death).toBeUndefined();
  });
});
