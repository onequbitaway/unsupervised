import { describe, expect, it } from 'vitest';
import { CONFIG } from '../config';
import type { Published, PublishedRef } from '../state/types';
import { BOARD_KIND, BroadcastDesk, type DeskApi } from './desk';
import { NameBook } from './names';

const clock = { year: 1, day: 1, hour: 0, minute: 0 };

function board(key: string, frame = 1): Published {
  return { kind: BOARD_KIND, key, frame, clock, data: { rows: [{ name: key, score: frame }] } };
}

/** A server whose answers the test chooses. */
class FakeServer implements DeskApi {
  refs: PublishedRef[] = [];
  boards = new Map<string, Published | null>();
  stats: Published | null = null;
  down = false;
  calls: string[] = [];

  private answer<T>(name: string, value: T): Promise<T> {
    this.calls.push(name);
    return this.down ? Promise.reject(new Error('down')) : Promise.resolve(value);
  }

  index = (): Promise<PublishedRef[]> => this.answer('index', this.refs);
  board = (name: string): Promise<Published | null> => this.answer(`board:${name}`, this.boards.get(name) ?? null);
  economy = (): Promise<Published | null> => this.answer('economy', this.stats);
}

/** Lets the answers of the fake server arrive. */
async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe('BroadcastDesk', () => {
  it('has nothing while no module published anything', async () => {
    const server = new FakeServer();
    const desk = new BroadcastDesk(server);
    desk.tick(0);
    await settle();
    desk.tick(1);
    await settle();
    // The routes answer 404: the api gives null, and the desk has no board.
    expect(server.calls).toEqual(['index', 'economy']);
    expect([desk.boards, desk.economy]).toEqual([[], null]);
  });

  it('reads the boards that the server lists, and the economy', async () => {
    const server = new FakeServer();
    server.refs = [
      { kind: BOARD_KIND, key: 'wealth', frame: 3 },
      { kind: 'finance.stats_published', frame: 3 },
      { kind: BOARD_KIND, key: 'models', frame: 3 },
      { kind: BOARD_KIND, frame: 3 },
    ];
    server.boards.set('wealth', board('wealth'));
    server.boards.set('models', board('models'));
    server.stats = { kind: 'finance.stats_published', frame: 3, clock, data: { gini_bp: 4_000 } };
    const desk = new BroadcastDesk(server);
    desk.tick(0);
    await settle();
    desk.tick(1);
    await settle();
    expect(server.calls).toEqual(['index', 'economy', 'board:models', 'board:wealth']);
    expect(desk.boards.map((entry) => entry.key)).toEqual(['models', 'wealth']);
    expect(desk.economy).toBe(server.stats);

    // Before the time of the next read the desk asks nothing.
    desk.tick(CONFIG.broadcast.pollMs);
    await settle();
    expect(server.calls).toHaveLength(4);
    // Then it reads the newest values.
    server.boards.set('wealth', board('wealth', 9));
    desk.tick(1 + CONFIG.broadcast.pollMs);
    await settle();
    expect(desk.boards.find((entry) => entry.key === 'wealth')?.frame).toBe(9);
  });

  it('drops a board that the server has no more', async () => {
    const server = new FakeServer();
    server.refs = [{ kind: BOARD_KIND, key: 'wealth', frame: 3 }];
    server.boards.set('wealth', board('wealth'));
    const desk = new BroadcastDesk(server);
    desk.tick(0);
    await settle();
    desk.tick(1);
    await settle();
    expect(desk.boards).toHaveLength(1);
    server.refs = [];
    desk.tick(CONFIG.broadcast.indexMs);
    await settle();
    expect(desk.boards).toEqual([]);
  });

  it('keeps what it has when the server does not answer, and reads again later', async () => {
    const server = new FakeServer();
    server.refs = [{ kind: BOARD_KIND, key: 'wealth', frame: 3 }];
    server.boards.set('wealth', board('wealth'));
    const desk = new BroadcastDesk(server);
    desk.tick(0);
    await settle();
    desk.tick(1);
    await settle();
    server.down = true;
    desk.tick(1 + CONFIG.broadcast.pollMs);
    await settle();
    expect(desk.boards.map((entry) => entry.key)).toEqual(['wealth']);
    server.down = false;
    server.boards.set('wealth', board('wealth', 7));
    desk.tick(1 + 2 * CONFIG.broadcast.pollMs);
    await settle();
    expect(desk.boards[0]?.frame).toBe(7);
  });
});

describe('NameBook', () => {
  const profile = (id: number) => ({
    id,
    name: `name of ${id}`,
    alive: true,
    zone: 0,
    zone_name: '',
    city: 0,
    city_name: '',
    pos: { x: 0, y: 0 },
    frame: 1,
    events: [],
    life: { jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] },
  });

  it('asks for a name one time and remembers it', async () => {
    const asked: number[] = [];
    const book = new NameBook((id) => {
      asked.push(id);
      return Promise.resolve(profile(id));
    });
    expect(book.get(7)).toBeNull();
    book.want([7, 7]);
    book.want([7]);
    await settle();
    expect(asked).toEqual([7]);
    expect(book.get(7)).toBe('name of 7');
  });

  it('asks for a few names at the same time, and for the others later', async () => {
    const asked: number[] = [];
    const book = new NameBook(
      (id) => {
        asked.push(id);
        return Promise.resolve(profile(id));
      },
      2,
    );
    book.want([1, 2, 3]);
    expect(asked).toEqual([1, 2]);
    await settle();
    book.want([1, 2, 3]);
    expect(asked).toEqual([1, 2, 3]);
  });

  it('does not ask again for a resident that the server does not know', async () => {
    let calls = 0;
    const book = new NameBook(() => {
      calls += 1;
      return Promise.reject(new Error('404'));
    });
    book.want([9]);
    await settle();
    book.want([9]);
    expect([calls, book.get(9)]).toEqual([1, null]);
  });

  it('keeps a bounded number of names', async () => {
    const book = new NameBook((id) => Promise.resolve(profile(id)), 3, 2);
    book.want([1, 2, 3]);
    await settle();
    // The oldest name left when the third one came.
    expect([book.get(1), book.get(2), book.get(3)]).toEqual([null, 'name of 2', 'name of 3']);
  });
});
