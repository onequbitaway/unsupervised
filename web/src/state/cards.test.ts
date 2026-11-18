import { describe, expect, it } from 'vitest';
import { CONFIG } from '../config';
import { boardCard, boardResidents, buildCards, cardAt, count, economyCard, electionCard, percent, storiesCard, worldCard } from './cards';
import { story } from './show.test';
import type { Layout, Published } from './types';

const clock = { year: 1, day: 3, hour: 9, minute: 30 };

function published(kind: string, data: unknown, key?: string): Published {
  return key === undefined ? { kind, frame: 5, clock, data } : { kind, key, frame: 5, clock, data };
}

const layout: Layout = {
  frame: 0,
  clock,
  cities: [
    { id: 0, name: 'Latent City', bounds: { x0: 0, y0: 0, x1: 128, y1: 128 }, zones: [0, 1] },
    { id: 1, name: 'Tokentown', bounds: { x0: 200, y0: 0, x1: 328, y1: 128 }, zones: [2] },
  ],
  zones: [],
};
const populations = new Map([[0, 10], [1, 5], [2, 40]]);

describe('boardCard', () => {
  const board = published(
    'missions.leaderboard_published',
    { key: 'wealth', title: 'Richest residents', unit: 'money', rows: [{ resident: 7, score: 980_000 }, { name: 'Bo', score: 12_345 }] },
    'wealth',
  );

  it('ranks the rows with names and scores', () => {
    const card = boardCard(board, (id) => (id === 7 ? 'Ada' : null));
    expect(card).toEqual({
      id: 'board:wealth',
      title: 'Richest residents',
      rows: [
        { rank: '1', label: 'Ada', value: '$9,800.00' },
        { rank: '2', label: 'Bo', value: '$123.45' },
      ],
    });
  });

  it('names a resident that the viewer does not know by its id', () => {
    expect(boardCard(board)?.rows[0]?.label).toBe('Resident 7');
    expect(boardResidents(board)).toEqual([7]);
  });

  it('reads other shapes of a board with care', () => {
    const points = published('missions.leaderboard_published', { entries: [{ model: 'atlas-2', points: 71 }, 'junk', { points: 3 }] }, 'models_by_score');
    expect(boardCard(points)).toEqual({
      id: 'board:models_by_score',
      title: 'Models by score',
      rows: [{ rank: '1', label: 'atlas-2', value: '71' }],
    });
  });

  it('makes no card from data without rows', () => {
    expect(boardCard(published('x_published', null, 'a'))).toBeNull();
    expect(boardCard(published('x_published', { rows: [] }, 'a'))).toBeNull();
    expect(boardCard(published('x_published', { rows: 'many' }, 'a'))).toBeNull();
    expect(boardCard(published('x_published', { rows: [{}, 5, null] }, 'a'))).toBeNull();
    expect(boardResidents(published('x_published', { rows: 'many' }, 'a'))).toEqual([]);
  });

  it('cuts a long board and cleans the names that residents chose', () => {
    const rows = Array.from({ length: 30 }, (_, i) => ({ name: `<b>r${i}</b>\n${'x'.repeat(200)}`, score: i }));
    const card = boardCard(published('x_published', { rows }, 'a'));
    expect(card?.rows).toHaveLength(CONFIG.broadcast.rows);
    const first = card?.rows[0]?.label ?? '';
    expect(first.length).toBe(CONFIG.broadcast.rowMax);
    expect(first).not.toContain('\n');
  });
});

describe('economyCard', () => {
  it('shows the numbers of the statistics', () => {
    const stats = published('finance.stats_published', {
      money_supply: 75_360_000,
      price_index_bp: 10_315,
      gini_bp: 4_080,
      base_rate_bp: 475,
      loans: { count: 50, outstanding: 11_250_000 },
    });
    expect(economyCard(stats)?.rows).toEqual([
      { rank: '', label: 'Money supply', value: '$753,600.00' },
      { rank: '', label: 'Price index', value: '103.2' },
      { rank: '', label: 'Inequality (Gini)', value: '0.41' },
      { rank: '', label: 'Base rate', value: '4.75%' },
      { rank: '', label: 'Loans', value: '50 · $112,500.00' },
    ]);
  });

  it('shows only the numbers that are there, and no card without any', () => {
    expect(economyCard(published('finance.stats_published', { gini_bp: 5_000, money_supply: 'much' }))?.rows).toEqual([
      { rank: '', label: 'Inequality (Gini)', value: '0.50' },
    ]);
    expect(economyCard(published('finance.stats_published', { day: 4 }))).toBeNull();
    expect(economyCard(published('finance.stats_published', 'text'))).toBeNull();
    expect(economyCard(null)).toBeNull();
  });

  it('writes numbers for the screen', () => {
    expect(count(1_234_567.4)).toBe('1,234,567');
    expect(percent(5)).toBe('0.05%');
  });
});

describe('the cards of the stories and of the world', () => {
  it('lists the open stories, the newest first', () => {
    const card = storiesCard([
      story(1, { updated: 5, headline: 'Old news' }),
      story(2, { updated: 9, headline: 'Fresh news', label: 'Feud' }),
      story(3, { updated: 99, status: 'closed' }),
    ]);
    expect(card?.rows).toEqual([
      { rank: '', label: 'FEUD', value: 'Fresh news' },
      { rank: '', label: 'CRIME', value: 'Old news' },
    ]);
    expect(storiesCard([story(3, { status: 'closed' })])).toBeNull();
    expect(storiesCard([])).toBeNull();
  });

  it('shows the elections only on their day', () => {
    const vote = story(4, { kind: 'election', headline: 'Bo won the election for mayor', status: 'closed' });
    expect(electionCard([vote, story(1)], clock)).toEqual({
      id: 'election',
      title: 'Election day',
      rows: [{ rank: '', label: 'RESULT', value: 'Bo won the election for mayor' }],
    });
    expect(electionCard([vote], { ...clock, day: 4 })).toBeNull();
    expect(electionCard([story(1)], clock)).toBeNull();
  });

  it('ranks the cities by their residents', () => {
    expect(worldCard(layout, populations)?.rows).toEqual([
      { rank: '1', label: 'Tokentown', value: '40 residents' },
      { rank: '2', label: 'Latent City', value: '15 residents' },
    ]);
    expect(worldCard({ ...layout, cities: [] }, populations)).toBeNull();
    const one = worldCard(layout, new Map([[2, 1]]));
    expect(one?.rows[0]?.value).toBe('1 resident');
  });
});

describe('buildCards and the rotation', () => {
  const sources = { layout, populations, clock, stories: [], boards: [], economy: null, nameOf: () => null };

  it('has only the card of the world when no module published anything', () => {
    expect(buildCards(sources).map((card) => card.id)).toEqual(['world']);
  });

  it('puts the cards with data in the order of the rotation', () => {
    const cards = buildCards({
      ...sources,
      stories: [story(1), story(2, { kind: 'election' })],
      boards: [
        published('missions.leaderboard_published', { rows: [{ name: 'Ada', score: 1 }] }, 'wealth'),
        published('missions.leaderboard_published', { nothing: true }, 'empty'),
      ],
      economy: published('finance.stats_published', { gini_bp: 4_000 }),
    });
    expect(cards.map((card) => card.id)).toEqual(['election', 'stories', 'board:wealth', 'economy', 'world']);
  });

  it('shows each card for its time, then the next one', () => {
    const cards = buildCards({ ...sources, stories: [story(1)] });
    expect(cards.map((card) => card.id)).toEqual(['stories', 'world']);
    expect(cardAt(cards, 0, 1_000)?.id).toBe('stories');
    expect(cardAt(cards, 999, 1_000)?.id).toBe('stories');
    expect(cardAt(cards, 1_000, 1_000)?.id).toBe('world');
    expect(cardAt(cards, 2_000, 1_000)?.id).toBe('stories');
    expect(cardAt([], 5, 1_000)).toBeNull();
    expect(cardAt(cards, 5_000, 0)?.id).toBe('stories');
  });
});
