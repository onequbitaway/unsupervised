// One line of news for an event. Every piece of text that came from the
// world is cleaned and cut here. The result is plain text.

import { CONFIG } from '../config';
import { rawSpeech, SAID } from './speech';
import { cleanText } from './text';
import type { FeedEvent } from './types';

type Data = Record<string, unknown>;

function dataOf(event: FeedEvent): Data {
  return typeof event.data === 'object' && event.data !== null ? (event.data as Data) : {};
}

function name(value: unknown): string {
  return cleanText(value, CONFIG.text.nameMax);
}

function word(value: unknown, fallback: string): string {
  const cleaned = cleanText(typeof value === 'string' ? value.replace(/_/g, ' ') : value, 24);
  return cleaned === '' ? fallback : cleaned;
}

/** Writes cents as money: 4500 gives `$45.00`. */
export function money(value: unknown): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 'some money';
  const cents = Math.round(Math.abs(value));
  const whole = Math.floor(cents / 100).toLocaleString('en-US');
  return `${value < 0 ? '-' : ''}$${whole}.${String(cents % 100).padStart(2, '0')}`;
}

/** Turns a kind into words: `crime.theft_committed` gives `Theft committed`. */
export function humanize(kind: string): string {
  const what = kind.includes('.') ? kind.slice(kind.indexOf('.') + 1) : kind;
  const words = cleanText(what.replace(/[_.]+/g, ' '), 48);
  return words === '' ? 'Something happened' : words.charAt(0).toUpperCase() + words.slice(1);
}

/** Finds the name of a resident that an event names by id, if the viewer knows it. */
export type NameOf = (id: number) => string | null;

function who(id: unknown, nameOf: NameOf): string {
  if (typeof id !== 'number') return 'someone';
  const known = nameOf(id);
  return known === null || name(known) === '' ? `resident ${id}` : name(known);
}

/** Returns one line of news for an event. */
export function headline(event: FeedEvent, nameOf: NameOf = () => null): string {
  const data = dataOf(event);
  const actorName = name(event.actor_name);
  const actor = actorName || (event.actor === undefined ? 'Someone' : who(event.actor, nameOf));
  // The event carries the name of its actor. Other residents are looked up.
  const named = (id: unknown): string => (id === event.actor && actorName !== '' ? actorName : who(id, nameOf));
  const line = ((): string => {
    switch (event.kind) {
      case SAID: {
        const words = cleanText(rawSpeech(event.data), CONFIG.text.speechMax);
        return words === '' ? `${actor} said something` : `${actor}: “${words}”`;
      }
      case 'kernel.spawned':
        return `${actor} arrived in the world`;
      case 'kernel.despawned':
        return `${actor} left the world (${word(data.cause, 'unknown cause')})`;
      case 'life.resident_died':
        return typeof data.age === 'number'
          ? `${actor} died at the age of ${Math.floor(data.age)} (${word(data.cause, 'unknown cause')})`
          : `${actor} died (${word(data.cause, 'unknown cause')})`;
      case 'economy.shift_completed':
        return `${actor} finished a shift and earned ${money(data.wage)}`;
      case 'economy.goods_sold':
        return `${named(data.buyer)} bought ${word(data.item, 'goods')} for ${money(data.unit_price)}`;
      case 'economy.goods_produced':
        return `${actor} produced ${word(data.item, 'goods')}`;
      case 'economy.property_sold':
        return `A property was sold for ${money(data.price)}`;
      case 'economy.lease_started':
        return `${named(data.tenant)} rented a place for ${money(data.rent)} a day`;
      case 'economy.tenant_evicted':
        return `${named(data.tenant)} was evicted`;
      case 'economy.business_opened':
        return `${actor} opened a ${word(data.kind, 'business')}`;
      case 'finance.loan_granted':
        return `${named(data.resident)} got a loan of ${money(data.amount)}`;
      case 'finance.loan_defaulted':
        return `${named(data.resident)} defaulted on a loan of ${money(data.outstanding)}`;
      case 'finance.order_filled':
        return `${word(data.item, 'Goods')} traded at ${money(data.price)} on the exchange`;
      case 'society.org_created':
        return `${named(data.founder)} founded ${name(data.name) || 'an organization'}`;
      case 'society.contract_signed':
        return `A contract was signed`;
      case 'society.contract_fulfilled':
        return `A contract was fulfilled`;
      case 'society.contract_breached':
        return `${named(data.by)} broke a contract`;
      case 'society.married':
        return `${named(data.a)} and ${named(data.b)} married`;
      default:
        return event.actor === undefined && event.actor_name === undefined
          ? humanize(event.kind)
          : `${humanize(event.kind)}: ${actor}`;
    }
  })();
  return cleanText(line, CONFIG.text.headlineMax);
}

/** Writes a calendar time for the screen. */
export function clockLabel(clock: { year: number; day: number; hour: number; minute: number }): string {
  const two = (n: number): string => String(n).padStart(2, '0');
  return `Year ${clock.year} · Day ${clock.day} · ${two(clock.hour)}:${two(clock.minute)}`;
}
