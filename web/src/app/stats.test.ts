import { describe, expect, it } from 'vitest';
import { Samples } from './stats';

describe('Samples', () => {
  it('has no numbers before the first sample', () => {
    expect(new Samples(4).summary()).toEqual({ count: 0, mean: 0, p95: 0, max: 0 });
  });

  it('gives the mean, the 95th percentile and the greatest value', () => {
    const samples = new Samples(100);
    for (let i = 1; i <= 100; i++) samples.add(i);
    expect(samples.summary()).toEqual({ count: 100, mean: 50.5, p95: 95, max: 100 });
  });

  it('keeps only the last values', () => {
    const samples = new Samples(3);
    for (const value of [100, 1, 2, 3]) samples.add(value);
    expect(samples.summary()).toEqual({ count: 3, mean: 2, p95: 3, max: 3 });
    samples.clear();
    expect(samples.summary().count).toBe(0);
  });
});
