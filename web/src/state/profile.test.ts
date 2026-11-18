import { describe, expect, it } from 'vitest';
import { panelFacts } from './profile';
import type { FeedEvent, Profile } from './types';

const clock = { year: 2, day: 5, hour: 9, minute: 3 };

function event(id: string, kind: string, data: unknown = null): FeedEvent {
  return { id, frame: 1, clock, kind, actor: 7, actor_name: 'Ada', drama: 20, data };
}

function profile(extra: Partial<Profile> = {}): Profile {
  return {
    id: 7,
    name: 'Ada',
    model: 'atlas-2',
    alive: true,
    zone: 3,
    zone_name: 'Tokentown 1-0',
    city: 1,
    city_name: 'Tokentown',
    pos: { x: 70, y: 9 },
    activity: 'work',
    frame: 40,
    events: [],
    life: { jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] },
    ...extra,
  };
}

describe('panelFacts', () => {
  it('says who, where and what, from the profile', () => {
    expect(panelFacts({ id: 7, live: null, profile: profile(), unknown: false })).toEqual({
      name: 'Ada',
      id: '#7',
      model: 'atlas-2',
      place: 'Tokentown · Tokentown 1-0',
      doing: 'work',
      note: '',
      events: [],
    });
  });

  it('prefers what the feed shows now over the older profile', () => {
    const facts = panelFacts({
      id: 7,
      live: { name: 'Ada B.', model: null, activity: 'move_to' },
      profile: profile(),
      unknown: false,
    });
    expect(facts.name).toBe('Ada B.');
    expect(facts.doing).toBe('move to');
    // The feed has no model for this resident. The profile has one.
    expect(facts.model).toBe('atlas-2');
    // A resident in the feed that does nothing is idle, whatever the profile says.
    expect(panelFacts({ id: 7, live: { name: 'Ada', model: null, activity: null }, profile: profile(), unknown: false }).doing).toBe('idle');
  });

  it('has words before the profile came, and for a resident that is not known', () => {
    expect(panelFacts({ id: 9, live: null, profile: null, unknown: false })).toMatchObject({
      name: 'Resident 9',
      model: 'not given',
      place: '…',
      doing: 'idle',
      note: '',
    });
    expect(panelFacts({ id: 9, live: null, profile: null, unknown: true }).note).toBe('No resident with this id is in the world.');
  });

  it('says that a resident left the world, and why', () => {
    const gone = panelFacts({ id: 7, live: null, profile: profile({ alive: false, cause: 'old_age' }), unknown: false });
    expect(gone.note).toBe('This resident left the world (old age).');
    expect(gone.doing).toBe('—');
    expect(panelFacts({ id: 7, live: null, profile: profile({ alive: false }), unknown: false }).note).toBe('This resident left the world.');
  });

  it('lists the events with the newest first', () => {
    const facts = panelFacts({
      id: 7,
      live: null,
      profile: profile({ events: [event('3-0', 'economy.shift_completed', { wage: 1200 }), event('9-1', 'society.married', { a: 7, b: 8 })] }),
      unknown: false,
    }, (id) => (id === 8 ? 'Bo' : null));
    expect(facts.events).toEqual([
      { id: '9-1', when: 'Year 2 · Day 5 · 09:03', line: 'Ada and Bo married' },
      { id: '3-0', when: 'Year 2 · Day 5 · 09:03', line: 'Ada finished a shift and earned $12.00' },
    ]);
  });

  it('cleans and cuts every text that a resident or an owner chose', () => {
    const facts = panelFacts({
      id: 7,
      live: null,
      profile: profile({
        name: '<img src=x onerror=alert(1)>\u202e' + 'x'.repeat(200),
        model: 'm'.repeat(100),
        city_name: 'City\nwith a break',
        cause: 'x'.repeat(100),
        alive: false,
      }),
      unknown: false,
    });
    expect(facts.name).toHaveLength(40);
    expect(facts.name.startsWith('<img src=x onerror=alert(1)>x')).toBe(true);
    expect(facts.model).toHaveLength(24);
    expect(facts.place).toBe('City with a break · Tokentown 1-0');
    expect(facts.note).toHaveLength('This resident left the world ().'.length + 32);
  });
});
