import { describe, expect, it } from 'vitest';
import type { FeedEvent, FeedPage } from '../state/types';
import { NewsDesk } from './news';

const clock = { year: 1, day: 1, hour: 0, minute: 0 };

function event(id: string, drama: number, kind = 'crime.theft_committed', extra: Partial<FeedEvent> = {}): FeedEvent {
  return { id, frame: 1, clock, kind, drama, data: null, ...extra };
}

/** A server whose answers the test gives by hand. */
class FakeFeed {
  calls: { after: string | null; minDrama: number }[] = [];
  private waiting: { resolve: (page: FeedPage) => void; reject: (error: Error) => void }[] = [];
  fetch = (after: string | null, minDrama: number): Promise<FeedPage> => {
    this.calls.push({ after, minDrama });
    return new Promise((resolve, reject) => this.waiting.push({ resolve, reject }));
  };
  async answer(page: FeedPage | Error): Promise<void> {
    const call = this.waiting.shift();
    if (!call) throw new Error('no request waits');
    if (page instanceof Error) call.reject(page);
    else call.resolve(page);
    await Promise.resolve();
    await Promise.resolve();
  }
}

describe('NewsDesk', () => {
  it('writes a clean line for an event and shows the most dramatic one first', () => {
    const desk = new NewsDesk((id) => (id === 5 ? 'Bo' : null), new FakeFeed().fetch);
    desk.add(
      [
        event('1-0', 5, 'society.said', { actor: 5, zone: 2, data: { text: { untrusted: true, from: 5, text: '<b>hi</b>\nthere' } } }),
        event('1-1', 80, 'life.resident_died', { actor: 6, actor_name: 'Cleo', data: { cause: 'old_age', age: 81 } }),
      ],
      1_000,
    );
    expect(desk.state.items.map((item) => item.text)).toEqual(['Bo: “<b>hi</b> there”', 'Cleo died at the age of 81 (old age)']);
    expect(desk.state.items[0]).toMatchObject({ zone: 2, actor: 5, drama: 5 });
    desk.tick(1_000);
    expect(desk.state.current?.id).toBe('1-1');
    // The same event from another source is not added again.
    desk.add([event('1-1', 80, 'life.resident_died')], 2_000);
    expect(desk.state.items).toHaveLength(2);
  });

  it('asks the server for notable events with a cursor, from time to time', async () => {
    const feed = new FakeFeed();
    const desk = new NewsDesk(() => null, feed.fetch);
    desk.tick(0);
    desk.tick(10);
    // One request at a time.
    expect(feed.calls).toEqual([{ after: null, minDrama: 11 }]);
    await feed.answer({ events: [event('4-0', 30)], next: '4-0', more: false, gap: false });
    expect(desk.state.items.map((item) => item.id)).toEqual(['4-0']);
    desk.tick(7_999);
    expect(feed.calls).toHaveLength(1);
    desk.tick(8_000);
    expect(feed.calls[1]).toEqual({ after: '4-0', minDrama: 11 });
  });

  it('asks again at once when more events wait, and later after an error', async () => {
    const feed = new FakeFeed();
    const desk = new NewsDesk(() => null, feed.fetch);
    desk.tick(0);
    await feed.answer({ events: [event('4-0', 30)], next: '4-0', more: true, gap: false });
    desk.tick(1);
    expect(feed.calls).toHaveLength(2);
    await feed.answer(new Error('offline'));
    desk.tick(2);
    expect(feed.calls).toHaveLength(2);
    desk.tick(8_001);
    // The cursor did not move.
    expect(feed.calls[2]).toEqual({ after: '4-0', minDrama: 11 });
  });
});
