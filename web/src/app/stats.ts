// Numbers about the speed of the viewer: the time between two frames and
// the time that the viewer works for one frame.

/** The last values of one measurement. */
export class Samples {
  private readonly values: Float32Array;
  private next = 0;
  private count = 0;

  constructor(capacity: number) {
    this.values = new Float32Array(Math.max(1, capacity));
  }

  add(value: number): void {
    this.values[this.next] = value;
    this.next = (this.next + 1) % this.values.length;
    this.count = Math.min(this.count + 1, this.values.length);
  }

  clear(): void {
    this.next = 0;
    this.count = 0;
  }

  /** Returns the mean, the value that 95 of 100 samples stay below, and the greatest value. */
  summary(): { count: number; mean: number; p95: number; max: number } {
    if (this.count === 0) return { count: 0, mean: 0, p95: 0, max: 0 };
    const sorted = Array.from(this.values.subarray(0, this.count)).sort((a, b) => a - b);
    const sum = sorted.reduce((total, value) => total + value, 0);
    const at = Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1);
    return { count: this.count, mean: sum / sorted.length, p95: sorted[at] ?? 0, max: sorted[sorted.length - 1] ?? 0 };
  }
}
