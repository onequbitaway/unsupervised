import { describe, expect, it } from 'vitest';
import type { ServerMessage } from '../state/types';
import { reconnectDelay } from './backoff';
import { FeedClient, type FeedEnv, type FeedStatus, feedUrl, type SocketLike } from './feed';

class FakeSocket implements SocketLike {
  sent: unknown[] = [];
  closed = false;
  onopen: ((event: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: ((event: unknown) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }

  close(): void {
    this.closed = true;
  }
}

/** An environment with a clock that the test moves by hand. */
class FakeEnv implements FeedEnv {
  sockets: FakeSocket[] = [];
  now = 0;
  roll = 1;
  private timers: { at: number; run: () => void; handle: number }[] = [];
  private next = 1;

  createSocket(): SocketLike {
    const socket = new FakeSocket();
    this.sockets.push(socket);
    return socket;
  }

  setTimeout(run: () => void, ms: number): unknown {
    const handle = this.next++;
    this.timers.push({ at: this.now + ms, run, handle });
    return handle;
  }

  clearTimeout(handle: unknown): void {
    this.timers = this.timers.filter((timer) => timer.handle !== handle);
  }

  random(): number {
    return this.roll;
  }

  /** Moves the clock and runs the timers that are due, in order. */
  advance(ms: number): void {
    const end = this.now + ms;
    for (;;) {
      const due = this.timers.filter((timer) => timer.at <= end).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      this.timers = this.timers.filter((timer) => timer !== due);
      this.now = due.at;
      due.run();
    }
    this.now = end;
  }

  get last(): FakeSocket {
    const socket = this.sockets.at(-1);
    if (!socket) throw new Error('no socket');
    return socket;
  }
}

function client(): { env: FakeEnv; feed: FeedClient; messages: ServerMessage[]; statuses: FeedStatus[] } {
  const env = new FakeEnv();
  const messages: ServerMessage[] = [];
  const statuses: FeedStatus[] = [];
  const feed = new FeedClient(
    'ws://test/v1/spectate',
    { onMessage: (m) => messages.push(m), onStatus: (s) => statuses.push(s) },
    env,
  );
  return { env, feed, messages, statuses };
}

describe('reconnectDelay', () => {
  it('doubles up to the longest wait', () => {
    expect([0, 1, 2, 3, 4, 5, 6, 60].map((n) => reconnectDelay(n, 500, 15_000, 1))).toEqual([
      500, 1_000, 2_000, 4_000, 8_000, 15_000, 15_000, 15_000,
    ]);
  });

  it('spreads the viewers over the second half of the step', () => {
    expect(reconnectDelay(2, 500, 15_000, 0)).toBe(1_000);
    expect(reconnectDelay(2, 500, 15_000, 0.5)).toBe(1_500);
    expect(reconnectDelay(-3, 500, 15_000, 7)).toBe(500);
  });
});

describe('FeedClient', () => {
  it('builds the address of the feed from the address of the page', () => {
    expect(feedUrl({ protocol: 'http:', host: 'localhost:8080' })).toBe('ws://localhost:8080/v1/spectate');
    expect(feedUrl({ protocol: 'https:', host: 'unsupervised.example' })).toBe('wss://unsupervised.example/v1/spectate');
  });

  it('subscribes when the connection opens and sends only differences after that', () => {
    const { env, feed, statuses } = client();
    feed.setZones([1, 2]);
    feed.start();
    expect(env.last.sent).toEqual([]);
    env.last.onopen?.(null);
    expect(env.last.sent).toEqual([{ op: 'subscribe', zones: [1, 2] }]);
    feed.setZones([2, 3]);
    expect(env.last.sent.slice(1)).toEqual([
      { op: 'unsubscribe', zones: [1] },
      { op: 'subscribe', zones: [3] },
    ]);
    feed.setZones([3, 2]);
    expect(env.last.sent).toHaveLength(3);
    feed.setZones([]);
    expect(env.last.sent[3]).toEqual({ op: 'unsubscribe', zones: [2, 3] });
    expect(statuses).toEqual(['connecting', 'open']);
  });

  it('gives the messages that it understands to the viewer', () => {
    const { env, feed, messages } = client();
    feed.start();
    env.last.onopen?.(null);
    env.last.onmessage?.({ data: '{"op":"pong"}' });
    env.last.onmessage?.({ data: 'not json' });
    env.last.onmessage?.({ data: '{"op":"teleport"}' });
    env.last.onmessage?.({ data: new ArrayBuffer(4) });
    env.last.onmessage?.({ data: '{"op":"unsubscribed","zones":[4]}' });
    expect(messages).toEqual([{ op: 'pong' }, { op: 'unsubscribed', zones: [4] }]);
  });

  it('connects again with a longer wait each time, and subscribes again', () => {
    const { env, feed, statuses } = client();
    feed.setZones([5]);
    feed.start();
    env.last.onopen?.(null);
    env.last.onclose?.(null);
    expect(statuses).toEqual(['connecting', 'open', 'waiting']);
    expect(env.sockets).toHaveLength(1);
    env.advance(499);
    expect(env.sockets).toHaveLength(1);
    env.advance(1);
    expect(env.sockets).toHaveLength(2);
    // This attempt fails before it opens: the next wait is twice as long.
    env.last.onerror?.(null);
    env.last.onclose?.(null);
    env.advance(999);
    expect(env.sockets).toHaveLength(2);
    env.advance(1);
    expect(env.sockets).toHaveLength(3);
    env.last.onopen?.(null);
    // The new connection knows nothing: the client asks for its zones again.
    expect(env.last.sent).toEqual([{ op: 'subscribe', zones: [5] }]);
    // After a connection that opened, the wait starts short again.
    env.last.onclose?.(null);
    env.advance(500);
    expect(env.sockets).toHaveLength(4);
  });

  it('pings, and replaces a connection that says nothing', () => {
    const { env, feed } = client();
    feed.start();
    const first = env.last;
    first.onopen?.(null);
    env.advance(10_000);
    expect(first.sent).toEqual([{ op: 'ping' }]);
    first.onmessage?.({ data: '{"op":"pong"}' });
    env.advance(20_000);
    // The connection said something in the first 30 seconds: it stays.
    expect(env.sockets).toHaveLength(1);
    expect(first.sent).toHaveLength(3);
    // Nothing for the next 30 seconds: the client drops it and waits.
    env.advance(30_000);
    expect(first.closed).toBe(true);
    env.advance(500);
    expect(env.sockets).toHaveLength(2);
  });

  it('chooses the channels of the show one time on each connection', () => {
    const { env, feed } = client();
    feed.setChannels(['director', 'stories']);
    feed.setZones([4]);
    feed.start();
    env.last.onopen?.(null);
    expect(env.last.sent).toEqual([
      { op: 'subscribe', zones: [4] },
      { op: 'tune', channels: ['director', 'stories'] },
    ]);
    // A change of the zones does not send the channels again.
    feed.setZones([4, 5]);
    expect(env.last.sent.slice(2)).toEqual([{ op: 'subscribe', zones: [5] }]);
    // Other channels are sent at once.
    feed.setChannels(['director']);
    expect(env.last.sent.at(-1)).toEqual({ op: 'tune', channels: ['director'] });
    // A new connection knows nothing: the client sends its channels again.
    env.last.onclose?.(null);
    env.advance(500);
    env.last.onopen?.(null);
    expect(env.last.sent).toEqual([
      { op: 'subscribe', zones: [4, 5] },
      { op: 'tune', channels: ['director'] },
    ]);
  });

  it('sends no channels for a viewer that wants none', () => {
    const { env, feed } = client();
    feed.start();
    env.last.onopen?.(null);
    feed.setZones([1]);
    expect(env.last.sent).toEqual([{ op: 'subscribe', zones: [1] }]);
  });

  it('gives the messages of the show to the viewer', () => {
    const { env, feed, messages } = client();
    feed.start();
    env.last.onopen?.(null);
    env.last.onmessage?.({ data: '{"op":"stories","stories":[],"reset":true}' });
    expect(messages).toEqual([{ op: 'stories', stories: [], reset: true }]);
  });

  it('stop ends the connection and every timer', () => {
    const { env, feed } = client();
    feed.start();
    env.last.onopen?.(null);
    feed.stop();
    expect(env.last.closed).toBe(true);
    env.advance(120_000);
    expect(env.sockets).toHaveLength(1);
    expect(env.last.sent).toEqual([]);
  });
});
