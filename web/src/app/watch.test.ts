import { describe, expect, it } from 'vitest';
import { ApiError } from '../net/api';
import type { Profile } from '../state/types';
import { ProfileWatch } from './watch';

function profile(id: number, x: number, alive = true): Profile {
  return {
    id,
    name: `r${id}`,
    alive,
    zone: 0,
    zone_name: 'z',
    city: 0,
    city_name: 'c',
    pos: { x, y: 1 },
    frame: 1,
    events: [],
    life: { jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] },
  };
}

class FakeServer {
  asked: number[] = [];
  private waiting: { resolve: (p: Profile) => void; reject: (e: Error) => void }[] = [];
  fetch = (resident: number): Promise<Profile> => {
    this.asked.push(resident);
    return new Promise((resolve, reject) => this.waiting.push({ resolve, reject }));
  };
  async answer(answer: Profile | Error): Promise<void> {
    const call = this.waiting.shift();
    if (!call) throw new Error('no request waits');
    if (answer instanceof Error) call.reject(answer);
    else call.resolve(answer);
    await Promise.resolve();
    await Promise.resolve();
  }
}

describe('ProfileWatch', () => {
  it('asks nothing without a selected resident', () => {
    const server = new FakeServer();
    const watch = new ProfileWatch(server.fetch);
    watch.tick(0, true);
    expect(server.asked).toEqual([]);
  });

  it('reads the profile at once and then from time to time', async () => {
    const server = new FakeServer();
    const watch = new ProfileWatch(server.fetch);
    watch.select(7);
    watch.tick(0, false);
    watch.tick(1, false);
    expect(server.asked).toEqual([7]);
    await server.answer(profile(7, 30));
    expect(watch.lastKnown).toEqual({ x: 30, y: 1 });
    watch.tick(3_999, false);
    expect(server.asked).toEqual([7]);
    watch.tick(4_000, false);
    expect(server.asked).toEqual([7, 7]);
  });

  it('asks more often while it searches a followed resident', async () => {
    const server = new FakeServer();
    const watch = new ProfileWatch(server.fetch);
    watch.select(7);
    watch.tick(0, true);
    await server.answer(profile(7, 30));
    watch.tick(999, true);
    expect(server.asked).toHaveLength(1);
    watch.tick(1_000, true);
    expect(server.asked).toHaveLength(2);
    // A resident that left the world is not searched: the wait is the long one again.
    await server.answer(profile(7, 30, false));
    watch.tick(2_000, true);
    watch.tick(4_999, true);
    expect(server.asked).toHaveLength(2);
    watch.tick(5_000, true);
    expect(server.asked).toHaveLength(3);
  });

  it('starts to search at once when a followed resident goes out of sight', async () => {
    const server = new FakeServer();
    const watch = new ProfileWatch(server.fetch);
    watch.select(7);
    watch.tick(0, false);
    await server.answer(profile(7, 30));
    // The feed shows the resident: the next read is four seconds away.
    watch.tick(900, false);
    expect(server.asked).toHaveLength(1);
    // The resident goes out of sight. The last read is one second old: read again now.
    watch.tick(1_000, true);
    expect(server.asked).toHaveLength(2);
  });

  it('says when the server does not know the resident, and keeps the profile on other errors', async () => {
    const server = new FakeServer();
    const watch = new ProfileWatch(server.fetch);
    watch.select(7);
    watch.tick(0, false);
    await server.answer(profile(7, 30));
    watch.tick(4_000, false);
    await server.answer(new ApiError(503, 'busy'));
    expect(watch.unknown).toBe(false);
    expect(watch.lastKnown).toEqual({ x: 30, y: 1 });
    watch.tick(8_000, false);
    await server.answer(new ApiError(404, 'unknown'));
    expect(watch.unknown).toBe(true);
  });

  it('forgets the profile of another resident, also when its answer comes late', async () => {
    const server = new FakeServer();
    const watch = new ProfileWatch(server.fetch);
    watch.select(7);
    watch.tick(0, false);
    watch.select(8);
    await server.answer(profile(7, 30));
    expect(watch.profile).toBeNull();
    watch.tick(1, false);
    expect(server.asked).toEqual([7, 8]);
    await server.answer(profile(8, 50));
    expect(watch.profile?.id).toBe(8);
  });
});
