// The state of the viewer and the pure reducer that applies the feed.
//
// `reduce` never changes its input. It returns a new state that shares
// every part that did not change, so a caller can keep an old state.

import { CONFIG } from '../config';
import { type Motion, measureTick, moveTo, positionAt, standing } from './motion';
import { speechText } from './speech';
import type { Clock, ResidentView, ServerMessage } from './types';

/** A resident as the viewer draws it. */
export interface Mover extends Motion {
  id: number;
  /** Untrusted: clean it before it goes on the screen. */
  name: string;
  model: string | null;
  /** Verb of the running action, or null. */
  activity: string | null;
}

export interface ZoneState {
  id: number;
  /** The frame of the last snapshot or delta. */
  frame: number;
  residents: ReadonlyMap<number, Mover>;
}

/** Words that a resident said, for a speech bubble. */
export interface Speech {
  resident: number;
  zone: number;
  /** Already safe to show. */
  text: string;
  /** The bubble goes away at this time, in milliseconds. */
  until: number;
}

/** Where a resident was drawn when it left a zone. */
interface Trace {
  x: number;
  y: number;
  frame: number;
}

export interface ViewerState {
  /** The last frame that the viewer knows. */
  frame: number;
  clock: Clock | null;
  /** Residents per zone, for every zone of the world. */
  populations: ReadonlyMap<number, number>;
  /** The subscribed zones that have a snapshot. */
  zones: ReadonlyMap<number, ZoneState>;
  /** The newest speech of each resident. */
  speech: ReadonlyMap<number, Speech>;
  /** Residents that left a zone in the current frame. */
  traces: ReadonlyMap<number, Trace>;
  /** Estimate of the real time between two frames, in milliseconds. */
  tickMs: number;
  /** When the last `frame` message came, or null. */
  lastFrameAt: number | null;
  /** Viewers that are connected to the server. */
  viewers: number;
  /** Frames that this viewer lost, as the server counted them. */
  dropped: number;
}

export function initialState(): ViewerState {
  return {
    frame: 0,
    clock: null,
    populations: new Map(),
    zones: new Map(),
    speech: new Map(),
    traces: new Map(),
    tickMs: CONFIG.motion.defaultTickMs,
    lastFrameAt: null,
    viewers: 0,
    dropped: 0,
  };
}

function mover(view: ResidentView, motion: Motion): Mover {
  return {
    ...motion,
    id: view.id,
    name: view.name,
    model: view.model ?? null,
    activity: view.activity ?? null,
  };
}

/** Finds a resident in any zone. */
export function findMover(state: ViewerState, id: number): { zone: number; mover: Mover } | null {
  for (const zone of state.zones.values()) {
    const found = zone.residents.get(id);
    if (found) return { zone: zone.id, mover: found };
  }
  return null;
}

function withPopulations(
  current: ReadonlyMap<number, number>,
  changes: readonly [number, number][],
): ReadonlyMap<number, number> {
  if (changes.length === 0) return current;
  const next = new Map(current);
  for (const [zone, count] of changes) next.set(zone, count);
  return next;
}

/**
 * Makes the mover of a resident that is new in a zone. A resident that the
 * viewer already draws somewhere (it crossed a border, or the zone was sent
 * again) goes on from that place.
 */
function arrive(
  state: ViewerState,
  traces: ReadonlyMap<number, Trace>,
  view: ResidentView,
  now: number,
): Mover {
  const known = findMover(state, view.id);
  if (known) {
    return mover(view, moveTo(known.mover, view.pos.x, view.pos.y, now, state.tickMs));
  }
  const trace = traces.get(view.id);
  if (trace) {
    const from = standing(trace.x, trace.y, now);
    return mover(view, moveTo(from, view.pos.x, view.pos.y, now, state.tickMs));
  }
  return mover(view, standing(view.pos.x, view.pos.y, now));
}

type Delta = Extract<ServerMessage, { op: 'delta' }>;

function applyDelta(state: ViewerState, delta: Delta, now: number): ViewerState {
  const zone = state.zones.get(delta.zone);
  // A delta for a zone without a snapshot is from before an unsubscribe.
  if (!zone) return state;

  const residents = new Map(zone.residents);
  let traces = state.traces;
  if (delta.left.length > 0) {
    const next = new Map(traces);
    for (const id of delta.left) {
      const gone = residents.get(id);
      if (!gone) continue;
      const at = positionAt(gone, now);
      next.set(id, { x: at.x, y: at.y, frame: delta.frame });
      residents.delete(id);
    }
    traces = next;
  }
  for (const view of delta.entered) {
    residents.set(view.id, arrive(state, traces, view, now));
  }
  for (const [id, x, y] of delta.moved) {
    const who = residents.get(id);
    if (who) residents.set(id, { ...who, ...moveTo(who, x, y, now, state.tickMs) });
  }
  for (const [id, verb] of delta.activity) {
    const who = residents.get(id);
    if (who) residents.set(id, { ...who, activity: verb });
  }

  let speech = state.speech;
  for (const event of delta.events) {
    const words = speechText(event);
    if (words === null || event.actor === undefined) continue;
    if (speech === state.speech) speech = new Map(speech);
    (speech as Map<number, Speech>).set(event.actor, {
      resident: event.actor,
      zone: delta.zone,
      text: words,
      until: now + CONFIG.speech.holdMs,
    });
  }

  const zones = new Map(state.zones);
  zones.set(delta.zone, { id: delta.zone, frame: delta.frame, residents });
  return { ...state, zones, speech, traces };
}

/** Removes what is over: old speech, and traces of earlier frames. */
function prune(state: ViewerState, frame: number, now: number): ViewerState {
  let speech = state.speech;
  for (const [id, entry] of state.speech) {
    if (entry.until > now) continue;
    if (speech === state.speech) speech = new Map(speech);
    (speech as Map<number, Speech>).delete(id);
  }
  let traces = state.traces;
  for (const [id, trace] of state.traces) {
    if (trace.frame >= frame) continue;
    if (traces === state.traces) traces = new Map(traces);
    (traces as Map<number, Trace>).delete(id);
  }
  return speech === state.speech && traces === state.traces ? state : { ...state, speech, traces };
}

/**
 * Applies one message of the feed. `now` is the time at which the message
 * came, in milliseconds on any steady clock.
 *
 * Within a delta the order is: `left`, `entered`, `moved`, `activity`.
 * Entries for residents that the zone does not have are ignored.
 */
export function reduce(state: ViewerState, message: ServerMessage, now: number): ViewerState {
  switch (message.op) {
    case 'world':
      return {
        ...state,
        frame: message.frame,
        clock: message.clock,
        populations: new Map(message.populations),
        viewers: message.viewers,
        dropped: state.dropped + message.dropped,
        // The time between this message and the next frame is not a tick.
        lastFrameAt: null,
      };
    case 'snapshot': {
      const residents = new Map<number, Mover>();
      for (const view of message.residents) {
        residents.set(view.id, arrive(state, state.traces, view, now));
      }
      const zones = new Map(state.zones);
      zones.set(message.zone, { id: message.zone, frame: message.frame, residents });
      return { ...state, zones };
    }
    case 'delta':
      return applyDelta(state, message, now);
    case 'frame': {
      const tickMs =
        state.lastFrameAt === null ? state.tickMs : measureTick(state.tickMs, now - state.lastFrameAt);
      return prune(
        {
          ...state,
          frame: message.frame,
          clock: message.clock,
          populations: withPopulations(state.populations, message.populations),
          viewers: message.viewers,
          tickMs,
          lastFrameAt: now,
        },
        message.frame,
        now,
      );
    }
    case 'unsubscribed': {
      if (!message.zones.some((zone) => state.zones.has(zone))) return state;
      const zones = new Map(state.zones);
      for (const zone of message.zones) zones.delete(zone);
      return { ...state, zones };
    }
    case 'error':
    case 'pong':
      return state;
  }
}

/** Forgets the residents of the zones that are not in `keep`. */
export function keepZones(state: ViewerState, keep: ReadonlySet<number>): ViewerState {
  let zones = state.zones;
  for (const id of state.zones.keys()) {
    if (keep.has(id)) continue;
    if (zones === state.zones) zones = new Map(zones);
    (zones as Map<number, ZoneState>).delete(id);
  }
  return zones === state.zones ? state : { ...state, zones };
}

/** Returns the number of residents in the whole world. */
export function totalPopulation(state: ViewerState): number {
  let total = 0;
  for (const count of state.populations.values()) total += count;
  return total;
}
