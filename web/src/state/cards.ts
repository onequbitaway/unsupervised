// The cards of the side panel of a broadcast: the stories, the boards,
// the numbers of the economy, the elections of the day and the cities.
// Each card is plain data with clean text. A card without data is not
// made, so the panel never shows an empty box.

import { CONFIG } from '../config';
import { type NameOf, humanize, money } from './headline';
import { electionsToday, sortStories } from './show';
import { cleanText } from './text';
import type { Clock, Layout, Published, Story } from './types';

export interface CardRow {
  /** The place in a ranking, or an empty text. */
  rank: string;
  label: string;
  value: string;
}

export interface Card {
  /** A stable name, for example `board:wealth`. */
  id: string;
  title: string;
  rows: CardRow[];
}

type Data = Record<string, unknown>;

function isObject(value: unknown): value is Data {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function finite(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function label(value: unknown): string {
  return cleanText(value, CONFIG.broadcast.rowMax);
}

/** Writes a whole number with groups: 1234567 gives `1,234,567`. */
export function count(value: number): string {
  return Math.round(value).toLocaleString('en-US');
}

/** Writes basis points as percent: 475 gives `4.75%`. */
export function percent(basisPoints: number): string {
  return `${(basisPoints / 100).toFixed(2)}%`;
}

/** The fields of a row of a board that can hold its score, in the order in which they are tried. */
const SCORE_FIELDS = ['score', 'value', 'points', 'amount', 'wealth', 'total', 'count'] as const;

/** The fields of the data of a board that can hold its rows. */
const ROW_FIELDS = ['rows', 'entries', 'top', 'ranking', 'leaders'] as const;

function scoreOf(row: Data, unit: unknown): string {
  for (const field of SCORE_FIELDS) {
    const value = finite(row[field]);
    if (value === null) continue;
    return unit === 'money' || field === 'wealth' || field === 'amount' ? money(value) : count(value);
  }
  return '';
}

function nameOfRow(row: Data, nameOf: NameOf): string {
  // A name that the board gives wins. A resident chose it: it is cleaned.
  const given = label(row.name ?? row.label ?? row.title);
  if (given !== '') return given;
  const resident = finite(row.resident ?? row.id);
  if (resident !== null) return label(nameOf(resident)) || `Resident ${resident}`;
  const model = label(row.model ?? row.owner);
  return model;
}

/**
 * Returns the card of one board, or null when the published data has no
 * rows that the viewer can read. The shape of a board is the choice of the
 * module that publishes it, so the card reads it with care: a list of rows
 * under a usual name, each with a name or a resident and a number.
 */
export function boardCard(board: Published, nameOf: NameOf = () => null): Card | null {
  const data = isObject(board.data) ? board.data : {};
  const source = ROW_FIELDS.map((field) => data[field]).find((value) => Array.isArray(value));
  if (!Array.isArray(source)) return null;
  const rows: CardRow[] = [];
  for (const entry of source) {
    if (!isObject(entry) || rows.length >= CONFIG.broadcast.rows) continue;
    const name = nameOfRow(entry, nameOf);
    if (name === '') continue;
    rows.push({ rank: String(rows.length + 1), label: name, value: scoreOf(entry, data.unit) });
  }
  if (rows.length === 0) return null;
  const key = board.key ?? 'board';
  const title = label(data.title) || humanize(key);
  return { id: `board:${key}`, title, rows };
}

/** Returns the residents that the rows of a board name by id only. */
export function boardResidents(board: Published): number[] {
  const data = isObject(board.data) ? board.data : {};
  const source = ROW_FIELDS.map((field) => data[field]).find((value) => Array.isArray(value));
  if (!Array.isArray(source)) return [];
  const ids: number[] = [];
  for (const entry of source.slice(0, CONFIG.broadcast.rows)) {
    if (!isObject(entry) || label(entry.name ?? entry.label ?? entry.title) !== '') continue;
    const resident = finite(entry.resident ?? entry.id);
    if (resident !== null && Number.isSafeInteger(resident) && resident >= 0) ids.push(resident);
  }
  return ids;
}

/**
 * Returns the card with the numbers of the economy, or null when the
 * statistics have none of the numbers that the card shows. The fields are
 * those of `finance.stats_published`.
 */
export function economyCard(stats: Published | null): Card | null {
  const data = stats !== null && isObject(stats.data) ? stats.data : null;
  if (data === null) return null;
  const rows: CardRow[] = [];
  const add = (name: string, value: string | null): void => {
    if (value !== null) rows.push({ rank: '', label: name, value });
  };
  const number = (field: string): number | null => finite(data[field]);
  const supply = number('money_supply');
  add('Money supply', supply === null ? null : money(supply));
  const prices = number('price_index_bp');
  add('Price index', prices === null ? null : (prices / 100).toFixed(1));
  const gini = number('gini_bp');
  add('Inequality (Gini)', gini === null ? null : (gini / 10_000).toFixed(2));
  const rate = number('base_rate_bp');
  add('Base rate', rate === null ? null : percent(rate));
  const loans = isObject(data.loans) ? data.loans : {};
  const outstanding = finite(loans.outstanding);
  const open = finite(loans.count);
  if (outstanding !== null) add('Loans', open === null ? money(outstanding) : `${count(open)} · ${money(outstanding)}`);
  if (rows.length === 0) return null;
  return { id: 'economy', title: 'The economy', rows };
}

/** Returns the card of the running stories, or null when there is none. */
export function storiesCard(stories: Iterable<Story>): Card | null {
  const open = sortStories([...stories]).filter((story) => story.status === 'open');
  const rows = open
    .slice(0, CONFIG.broadcast.rows)
    .map((story) => ({ rank: '', label: label(story.label).toUpperCase(), value: label(story.headline) }))
    .filter((row) => row.value !== '');
  return rows.length === 0 ? null : { id: 'stories', title: 'Running stories', rows };
}

/** Returns the card of the elections of the game day, or null on a day without one. */
export function electionCard(stories: Iterable<Story>, clock: Clock | null): Card | null {
  const rows = electionsToday(stories, clock)
    .slice(0, CONFIG.broadcast.rows)
    .map((story) => ({ rank: '', label: story.status === 'open' ? 'NOW' : 'RESULT', value: label(story.headline) }))
    .filter((row) => row.value !== '');
  return rows.length === 0 ? null : { id: 'election', title: 'Election day', rows };
}

/** Returns the card of the cities with their numbers of residents. The layout always has them. */
export function worldCard(layout: Layout, populations: ReadonlyMap<number, number>): Card | null {
  const cities = layout.cities
    .map((city) => ({
      name: label(city.name),
      residents: city.zones.reduce((sum, zone) => sum + (populations.get(zone) ?? 0), 0),
    }))
    .filter((city) => city.name !== '')
    .sort((a, b) => b.residents - a.residents || a.name.localeCompare(b.name))
    .slice(0, CONFIG.broadcast.cities);
  if (cities.length === 0) return null;
  const rows = cities.map((city, at) => ({
    rank: String(at + 1),
    label: city.name,
    value: `${count(city.residents)} ${city.residents === 1 ? 'resident' : 'residents'}`,
  }));
  return { id: 'world', title: 'The country', rows };
}

export interface CardSources {
  layout: Layout;
  populations: ReadonlyMap<number, number>;
  clock: Clock | null;
  stories: Iterable<Story>;
  /** The boards that the server has, or an empty list. */
  boards: readonly Published[];
  /** The statistics of the economy, or null until a module publishes them. */
  economy: Published | null;
  nameOf: NameOf;
}

/**
 * Returns the cards that have data, in the order of the rotation: the
 * elections of the day first, then the stories, the boards, the economy
 * and the cities. Data that is absent gives no card.
 */
export function buildCards(sources: CardSources): Card[] {
  const stories = [...sources.stories];
  const cards: (Card | null)[] = [
    electionCard(stories, sources.clock),
    storiesCard(stories),
    ...sources.boards.slice(0, CONFIG.broadcast.maxBoards).map((board) => boardCard(board, sources.nameOf)),
    economyCard(sources.economy),
    worldCard(sources.layout, sources.populations),
  ];
  return cards.filter((card): card is Card => card !== null);
}

/**
 * Returns the card that the panel shows at a time: each card stays for
 * `cardMs`, then the next one comes. Null when there is no card.
 */
export function cardAt(cards: readonly Card[], now: number, cardMs: number): Card | null {
  if (cards.length === 0) return null;
  const turn = cardMs <= 0 ? 0 : Math.floor(Math.max(0, now) / cardMs);
  return cards[turn % cards.length] ?? null;
}
