import { describe, expect, it } from 'vitest';
import { activityGroup, activityLabel } from './activity';
import { clockLabel, headline, humanize, money } from './headline';
import type { FeedEvent } from './types';

function event(kind: string, data: unknown = null, extra: Partial<FeedEvent> = {}): FeedEvent {
  return { id: '1-0', frame: 1, clock: { year: 1, day: 1, hour: 0, minute: 1 }, kind, drama: 10, data, ...extra };
}

const ada = { actor: 7, actor_name: 'Ada' };

describe('headline', () => {
  it('writes the events with fixed data', () => {
    expect(headline(event('kernel.spawned', { resident: 7, name: 'Ada' }, ada))).toBe('Ada arrived in the world');
    expect(headline(event('kernel.despawned', { cause: 'old_age' }, ada))).toBe('Ada left the world (old age)');
    expect(headline(event('life.resident_died', { cause: 'starvation', age: 44.9 }, ada))).toBe(
      'Ada died at the age of 44 (starvation)',
    );
    expect(headline(event('economy.shift_completed', { wage: 1250 }, ada))).toBe('Ada finished a shift and earned $12.50');
    expect(headline(event('economy.business_opened', { kind: 'police_station' }, ada))).toBe('Ada opened a police station');
    expect(headline(event('finance.loan_defaulted', { resident: 7, outstanding: 123456 }))).toBe(
      'resident 7 defaulted on a loan of $1,234.56',
    );
  });

  it('uses the names that the viewer knows for residents named by id', () => {
    const names = (id: number): string | null => (id === 3 ? 'Bo' : null);
    expect(headline(event('society.married', { a: 3, b: 4 }), names)).toBe('Bo and resident 4 married');
    expect(headline(event('society.contract_breached', {}), names)).toBe('someone broke a contract');
  });

  it('uses the name in the event when the data names the actor by id', () => {
    // The viewer does not know resident 7, but the event carries its name.
    expect(headline(event('society.married', { a: 7, b: 4 }, ada))).toBe('Ada and resident 4 married');
    expect(headline(event('finance.loan_granted', { resident: 7, amount: 50_000 }, ada))).toBe('Ada got a loan of $500.00');
    // An event without a name for its actor still looks the actor up.
    expect(headline(event('society.married', { a: 7, b: 4 }, { actor: 7 }), (id) => (id === 7 ? 'Ada B.' : null))).toBe(
      'Ada B. and resident 4 married',
    );
  });

  it('writes an unknown kind in words', () => {
    expect(headline(event('crime.theft_committed', { amount: 5 }, ada))).toBe('Theft committed: Ada');
    expect(headline(event('politics.election_decided'))).toBe('Election decided');
    expect(headline(event('strange', null, { actor: 9 }))).toBe('Strange: resident 9');
    expect(humanize('')).toBe('Something happened');
  });

  it('quotes speech', () => {
    expect(headline(event('society.said', { text: { untrusted: true, from: 7, text: 'Hello.' } }, ada))).toBe('Ada: “Hello.”');
    expect(headline(event('society.said', {}, ada))).toBe('Ada said something');
  });

  it('cleans and cuts every text from the world', () => {
    const hostile = { actor: 7, actor_name: `Eve\u202e\n<script>${'x'.repeat(200)}` };
    const line = headline(event('kernel.spawned', null, hostile));
    // A name has at most 22 characters on the screen.
    expect(line).toBe('Eve <script>xxxxxxxxx… arrived in the world');
    const long = headline(event('society.said', { text: 'la '.repeat(400) }, ada));
    expect(long.length).toBeLessThanOrEqual(120);
    expect(long.startsWith('Ada: “la la')).toBe(true);
    // Data of the wrong type does not break the line.
    expect(headline(event('economy.business_opened', { kind: { a: 1 } }, ada))).toBe('Ada opened a business');
    expect(headline(event('life.resident_died', 'not an object', ada))).toBe('Ada died (unknown cause)');
    expect(headline(event('economy.shift_completed', { wage: 'much' }, ada))).toBe('Ada finished a shift and earned some money');
  });

  it('writes money and the clock', () => {
    expect(money(0)).toBe('$0.00');
    expect(money(-250)).toBe('-$2.50');
    expect(money(100000005)).toBe('$1,000,000.05');
    expect(clockLabel({ year: 2, day: 14, hour: 7, minute: 5 })).toBe('Year 2 · Day 14 · 07:05');
  });
});

describe('activity', () => {
  it('puts verbs in groups', () => {
    expect(activityGroup('move_to')).toBe('move');
    expect(activityGroup('work')).toBe('work');
    expect(activityGroup('sleep')).toBe('rest');
    expect(activityGroup('rob')).toBe('crime');
    expect(activityGroup('vote')).toBe('civic');
    expect(activityGroup(null)).toBe('idle');
    expect(activityGroup('')).toBe('idle');
    expect(activityGroup('juggle')).toBe('other');
    expect(activityGroup('constructor')).toBe('other');
  });

  it('writes a verb in words', () => {
    expect(activityLabel('move_to')).toBe('move to');
    expect(activityLabel(undefined)).toBe('idle');
  });
});
