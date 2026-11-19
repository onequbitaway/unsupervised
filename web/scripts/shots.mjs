// Counts what the director had on air, from samples of `GET /v1/director`.
//
// A sample is `{ frame, shot }`. Between two samples the director can cut
// more than one time, and two samples can show the same frame. So a sample
// counts for the frames since the sample before it: a shot that was on
// air at frame 120 after a sample at frame 110 gets 10 frames.

/** The kinds of picture, in the order of the summary. */
export const PICTURES = ['wide', 'street', 'close', 'breaking'];

/**
 * Returns the kind of picture of a shot:
 *
 * - `breaking`: breaking news, with every zoom;
 * - `wide`: no subject, the view of a city or of the country;
 * - `close` or `street`: a resident or a place, by the zoom of the shot.
 */
export function pictureOf(shot) {
  if (shot.breaking) return 'breaking';
  if (shot.focus.type === 'none') return 'wide';
  return shot.zoom === 'close' ? 'close' : 'street';
}

/** The tally of a run. */
export class ShotTally {
  constructor() {
    /** The last frame that a sample showed. `null` before the first sample. */
    this.frame = null;
    /** Frames that the samples cover. */
    this.frames = 0;
    /** Frames for each kind of picture. */
    this.pictures = Object.fromEntries(PICTURES.map((picture) => [picture, 0]));
    /** Frames for each zoom of the shot. */
    this.zooms = {};
    /** The shots, by their number, in the order in which they came. */
    this.shots = new Map();
  }

  /** Takes one sample. Samples must come in the order of their frames. */
  add({ frame, shot }) {
    const weight = this.frame === null ? 1 : Math.max(0, frame - this.frame);
    this.frame = this.frame === null ? frame : Math.max(this.frame, frame);
    // Before the first frame the director has no shot.
    if (shot.seq === 0) return;
    this.frames += weight;
    const picture = pictureOf(shot);
    this.pictures[picture] += weight;
    this.zooms[shot.zoom] = (this.zooms[shot.zoom] ?? 0) + weight;
    let entry = this.shots.get(shot.seq);
    if (!entry) {
      entry = { seq: shot.seq, frame: shot.frame, picture, zoom: shot.zoom, focus: shot.focus.type, frames: 0, captions: [] };
      this.shots.set(shot.seq, entry);
    }
    entry.frames += weight;
    // The words of a shot can change while the camera stays.
    const words = { caption: shot.caption, kind: shot.kind ?? null, drama: shot.drama, label: shot.label ?? null, headline: shot.headline ?? null };
    const last = entry.captions[entry.captions.length - 1];
    if (!last || last.caption !== words.caption || last.headline !== words.headline) entry.captions.push(words);
  }

  /** Returns the shots that the samples saw, oldest first. */
  list() {
    return [...this.shots.values()];
  }

  /** Returns the share of the frames of one kind of picture, from 0 to 1. */
  share(picture) {
    return this.frames === 0 ? 0 : this.pictures[picture] / this.frames;
  }

  /** Returns the frames for each kind of event that had a caption. Wide views have the kind `(wide view)`. */
  kinds() {
    const kinds = {};
    for (const shot of this.shots.values()) {
      const kind = shot.captions[0]?.kind ?? '(wide view)';
      kinds[kind] = (kinds[kind] ?? 0) + shot.frames;
    }
    return kinds;
  }

  /** Returns the lines of the summary. */
  summary() {
    const percent = (count) => (this.frames === 0 ? '0.0' : ((count * 100) / this.frames).toFixed(1));
    const lines = [`${this.frames} frames sampled, ${this.shots.size} shots`];
    for (const picture of PICTURES) {
      const shots = this.list().filter((shot) => shot.picture === picture).length;
      lines.push(`  ${picture.padEnd(9)} ${String(this.pictures[picture]).padStart(7)} frames  ${percent(this.pictures[picture]).padStart(5)} %  ${shots} shots`);
    }
    lines.push(`  by zoom: ${Object.entries(this.zooms).map(([zoom, count]) => `${zoom} ${count}`).join(', ') || 'none'}`);
    return lines;
  }
}
