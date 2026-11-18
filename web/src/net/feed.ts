// The client of `/v1/spectate`: one WebSocket that connects again when it
// breaks, and that keeps the subscription of the viewer on the server.

import { CONFIG } from '../config';
import type { Channel, ClientMessage, ServerMessage } from '../state/types';
import { reconnectDelay } from './backoff';
import { parseServerMessage } from './messages';

/** The part of a WebSocket that the client uses. */
export interface SocketLike {
  send(data: string): void;
  close(): void;
  onopen: ((event: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event: unknown) => void) | null;
  onerror: ((event: unknown) => void) | null;
}

export type FeedStatus = 'connecting' | 'open' | 'waiting';

export interface FeedHooks {
  /** A message of the feed came. */
  onMessage(message: ServerMessage): void;
  /** The connection changed. */
  onStatus(status: FeedStatus): void;
}

/** What the client needs from its environment. Tests give fakes. */
export interface FeedEnv {
  createSocket(url: string): SocketLike;
  setTimeout(run: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
  /** A number from 0 to 1. */
  random(): number;
}

const browserEnv: FeedEnv = {
  createSocket: (url) => new WebSocket(url) as unknown as SocketLike,
  setTimeout: (run, ms) => globalThis.setTimeout(run, ms),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as number),
  random: () => Math.random(),
};

/** Returns the address of the feed for a page address. */
export function feedUrl(page: { protocol: string; host: string }): string {
  return `${page.protocol === 'https:' ? 'wss:' : 'ws:'}//${page.host}/v1/spectate`;
}

export class FeedClient {
  private socket: SocketLike | null = null;
  private open = false;
  private stopped = true;
  private attempt = 0;
  private retry: unknown = null;
  private watchdog: unknown = null;
  private heard = false;
  /** The zones that the viewer wants. */
  private wanted = new Set<number>();
  /** The zones that the server was asked for on this connection. */
  private asked = new Set<number>();
  /** The channels of the show that the viewer wants. */
  private channels: readonly Channel[] = [];
  /** True when the server knows the channels on this connection. */
  private tuned = false;

  constructor(
    private readonly url: string,
    private readonly hooks: FeedHooks,
    private readonly env: FeedEnv = browserEnv,
  ) {}

  /** Connects, and connects again whenever the connection breaks. */
  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.connect();
  }

  /** Closes the connection for good. */
  stop(): void {
    this.stopped = true;
    this.env.clearTimeout(this.retry);
    this.env.clearTimeout(this.watchdog);
    this.drop();
  }

  /**
   * Sets the zones that the viewer wants. The client sends only the
   * difference to the server, now or as soon as the connection is open.
   */
  setZones(zones: Iterable<number>): void {
    this.wanted = new Set(zones);
    this.sync();
  }

  /**
   * Sets the channels of the show that the viewer wants: the shots of the
   * director, the stories. The server then sends the state of each channel
   * and every change, on this connection and on every later one.
   */
  setChannels(channels: readonly Channel[]): void {
    this.channels = [...channels];
    this.tuned = false;
    this.sync();
  }

  private connect(): void {
    this.hooks.onStatus('connecting');
    const socket = this.env.createSocket(this.url);
    this.socket = socket;
    socket.onopen = () => {
      if (this.socket !== socket) return;
      this.open = true;
      this.attempt = 0;
      this.asked = new Set();
      this.tuned = false;
      this.hooks.onStatus('open');
      this.sync();
      this.watch();
    };
    socket.onmessage = (event) => {
      if (this.socket !== socket || typeof event.data !== 'string') return;
      this.heard = true;
      const message = parseServerMessage(event.data);
      if (message !== null) this.hooks.onMessage(message);
    };
    const lost = (): void => {
      if (this.socket !== socket) return;
      this.drop();
      if (this.stopped) return;
      const wait = reconnectDelay(
        this.attempt,
        CONFIG.feed.reconnectMinMs,
        CONFIG.feed.reconnectMaxMs,
        this.env.random(),
      );
      this.attempt += 1;
      this.hooks.onStatus('waiting');
      this.retry = this.env.setTimeout(() => this.connect(), wait);
    };
    socket.onclose = lost;
    socket.onerror = lost;
  }

  /** Forgets the socket without a new attempt. */
  private drop(): void {
    const socket = this.socket;
    this.socket = null;
    this.open = false;
    this.env.clearTimeout(this.watchdog);
    if (socket === null) return;
    socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null;
    try {
      socket.close();
    } catch {
      // The socket was closed already.
    }
  }

  private send(message: ClientMessage): void {
    if (this.open) this.socket?.send(JSON.stringify(message));
  }

  /** Makes the subscription on the server equal to the wanted zones. */
  private sync(): void {
    if (!this.open) return;
    const byId = (a: number, b: number): number => a - b;
    const remove = [...this.asked].filter((zone) => !this.wanted.has(zone)).sort(byId);
    const add = [...this.wanted].filter((zone) => !this.asked.has(zone)).sort(byId);
    // Unsubscribe first, so that the number of zones stays below the limit.
    if (remove.length > 0) this.send({ op: 'unsubscribe', zones: remove });
    if (add.length > 0) this.send({ op: 'subscribe', zones: add });
    this.asked = new Set(this.wanted);
    // A viewer without channels sends nothing: the server has none either.
    if (!this.tuned && this.channels.length > 0) this.send({ op: 'tune', channels: [...this.channels] });
    this.tuned = true;
  }

  /**
   * Sends a ping from time to time. A connection that says nothing at all
   * between two checks is dead: the client opens a new one.
   */
  private watch(): void {
    this.heard = true;
    const pingsPerCheck = Math.max(1, Math.round(CONFIG.feed.staleMs / CONFIG.feed.pingMs));
    let pings = 0;
    const tick = (): void => {
      if (!this.open) return;
      pings += 1;
      if (pings % pingsPerCheck === 0) {
        if (!this.heard) {
          const socket = this.socket;
          socket?.onclose?.(null);
          return;
        }
        this.heard = false;
      }
      this.send({ op: 'ping' });
      this.watchdog = this.env.setTimeout(tick, CONFIG.feed.pingMs);
    };
    this.watchdog = this.env.setTimeout(tick, CONFIG.feed.pingMs);
  }
}
