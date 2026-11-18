import { describe, expect, it } from 'vitest';
import { clipFrame, clipStart, playhead } from './clipplay';
import type { Clip, ClipFrame } from './types';
import { initialState, reduce } from './world';

const clock = (minute: number) => ({ year: 1, day: 1, hour: 0, minute });

function frame(number: number, extra: Partial<ClipFrame> = {}): ClipFrame {
  return { frame: number, clock: clock(number), tick: number, moved: [], entered: [], left: [], activity: [], events: [], ...extra };
}

const clip: Clip = {
  id: '5-0',
  event: { id: '5-0', frame: 5, clock: clock(5), kind: 'crime.theft_committed', drama: 55, data: null, zone: 2, actor: 8 },
  line: 'Bo stole $45.00 from Ada',
  zone: 2,
  from: 3,
  to: 8,
  complete: true,
  pinned: false,
  residents: [{ id: 7, name: 'Ada', pos: { x: 10, y: 4 } }],
  frames: [
    frame(4, { moved: [[7, 11, 4]], entered: [{ id: 8, name: 'Bo', pos: { x: 20, y: 4 } }] }),
    frame(5, { activity: [[8, 'rob']] }),
    frame(7, { left: [8], moved: [[7, 12, 4]] }),
  ],
  link: '/?clip=5-0',
};

describe('the messages of a clip', () => {
  it('starts with the residents before the first frame', () => {
    expect(clipStart(clip)).toEqual({ op: 'snapshot', zone: 2, frame: 2, residents: clip.residents });
  });

  it('gives a delta for a frame with a change, and always the frame message', () => {
    expect(clipFrame(clip, 4).map((message) => message.op)).toEqual(['delta', 'frame']);
    expect(clipFrame(clip, 6).map((message) => message.op)).toEqual(['frame']);
    const delta = clipFrame(clip, 5)[0];
    expect(delta).toMatchObject({ op: 'delta', zone: 2, frame: 5, activity: [[8, 'rob']] });
  });

  it('keeps the clock of the last frame with a change', () => {
    const clockOf = (number: number) => {
      const last = clipFrame(clip, number).at(-1);
      return last?.op === 'frame' ? last.clock.minute : null;
    };
    // Frame 3 is before the first change: it has the clock of that change.
    expect([3, 4, 6, 8].map(clockOf)).toEqual([4, 4, 5, 7]);
  });

  it('plays through the reducer of the live feed', () => {
    let state = reduce(initialState(), clipStart(clip), 0);
    const names = (): string[] => [...(state.zones.get(2)?.residents.values() ?? [])].map((mover) => `${mover.name}@${mover.x}`);
    expect(names()).toEqual(['Ada@10']);
    for (let number = clip.from; number <= clip.to; number++) {
      for (const message of clipFrame(clip, number)) state = reduce(state, message, number * 400);
      if (number === 5) {
        expect(names()).toEqual(['Ada@11', 'Bo@20']);
        expect(state.zones.get(2)?.residents.get(8)?.activity).toBe('rob');
      }
    }
    expect(names()).toEqual(['Ada@12']);
    expect(state.frame).toBe(8);
  });
});

describe('playhead', () => {
  it('shows one more frame after each frame time', () => {
    // 6 frames of 400 ms, then 1,000 ms on the last picture.
    expect(playhead(clip, 0, 400, 1_000)).toEqual({ frame: 3, progress: 0, ended: false, loop: 0 });
    expect(playhead(clip, 399, 400, 1_000).frame).toBe(3);
    expect(playhead(clip, 400, 400, 1_000).frame).toBe(4);
    expect(playhead(clip, 1_200, 400, 1_000)).toEqual({ frame: 6, progress: 0.5, ended: false, loop: 0 });
    expect(playhead(clip, 2_399, 400, 1_000).frame).toBe(8);
  });

  it('holds the last picture, then starts again', () => {
    expect(playhead(clip, 2_400, 400, 1_000)).toEqual({ frame: 8, progress: 1, ended: true, loop: 0 });
    expect(playhead(clip, 3_399, 400, 1_000).ended).toBe(true);
    expect(playhead(clip, 3_400, 400, 1_000)).toEqual({ frame: 3, progress: 0, ended: false, loop: 1 });
    expect(playhead(clip, 3_400 * 3 + 800, 400, 1_000)).toMatchObject({ frame: 5, loop: 3 });
  });

  it('stands at the end of a clip without frames', () => {
    const empty = { ...clip, from: 9, to: 8 };
    expect(playhead(empty, 5_000, 400, 1_000)).toEqual({ frame: 8, progress: 1, ended: true, loop: 0 });
    expect(playhead(clip, 5_000, 0, 1_000).ended).toBe(true);
    expect(playhead(clip, -50, 400, 1_000).frame).toBe(3);
  });
});
