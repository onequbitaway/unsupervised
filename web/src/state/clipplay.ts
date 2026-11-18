// A replay clip as a sequence of feed messages. The player gives them to
// the same reducer as the live feed, so a clip looks like the live view.

import type { Clip, ServerMessage } from './types';

/** Returns the `snapshot` message that starts a clip: the residents before its first frame. */
export function clipStart(clip: Clip): ServerMessage {
  return { op: 'snapshot', zone: clip.zone, frame: clip.from - 1, residents: clip.residents };
}

/**
 * Returns the messages of one frame of a clip: the `delta` of the zone,
 * when the zone changed in that frame, and the `frame` message. The clock
 * of a frame without a change is the clock of the frame before it.
 */
export function clipFrame(clip: Clip, frame: number): ServerMessage[] {
  const messages: ServerMessage[] = [];
  const entry = clip.frames.find((known) => known.frame === frame);
  if (entry) {
    messages.push({
      op: 'delta',
      frame,
      zone: clip.zone,
      tick: entry.tick,
      moved: entry.moved,
      entered: entry.entered,
      left: entry.left,
      activity: entry.activity,
      events: entry.events,
    });
  }
  let clock = clip.event.clock;
  for (const known of clip.frames) {
    if (known.frame > frame) break;
    clock = known.clock;
  }
  // Before the first frame with a change the clip has no clock of its own.
  const first = clip.frames[0];
  if (first !== undefined && frame < first.frame) clock = first.clock;
  messages.push({ op: 'frame', frame, clock, populations: [], headlines: [], viewers: 0 });
  return messages;
}

export interface Playhead {
  /** The last frame that the player shows. `from - 1`: only the start. */
  frame: number;
  /** How far the clip is, 0 to 1. */
  progress: number;
  /** True while the last picture stays, before the clip starts again. */
  ended: boolean;
  /** How many times the clip started again. */
  loop: number;
}

/**
 * Returns where a clip is after it played for `elapsedMs`. Each frame
 * takes `frameMs`. After the last frame the picture stays for `endHoldMs`.
 * Then the clip starts again.
 */
export function playhead(clip: Clip, elapsedMs: number, frameMs: number, endHoldMs: number): Playhead {
  const frames = Math.max(0, clip.to - clip.from + 1);
  const length = frames * frameMs;
  const cycle = length + Math.max(0, endHoldMs);
  if (frames === 0 || frameMs <= 0 || cycle <= 0) {
    return { frame: clip.to, progress: 1, ended: true, loop: 0 };
  }
  const time = Math.max(0, elapsedMs);
  const loop = Math.floor(time / cycle);
  const within = time - loop * cycle;
  if (within >= length) return { frame: clip.to, progress: 1, ended: true, loop };
  // The first frame shows at once. The start alone is never on the screen.
  const shown = Math.floor(within / frameMs) + 1;
  return { frame: clip.from - 1 + shown, progress: within / length, ended: false, loop };
}
