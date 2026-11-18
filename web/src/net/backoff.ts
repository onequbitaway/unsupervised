// How long to wait before the feed connects again.

/**
 * Returns the wait before attempt number `attempt` (from 0): the shortest
 * wait doubled each time, up to the longest wait. `random` is a number
 * from 0 to 1; it spreads the viewers so that they do not all come back in
 * the same moment after a restart of the server. The wait is between half
 * of the step and the whole step.
 */
export function reconnectDelay(attempt: number, minMs: number, maxMs: number, random: number): number {
  const step = Math.min(maxMs, minMs * 2 ** Math.min(Math.max(attempt, 0), 30));
  return Math.round(step / 2 + (step / 2) * Math.min(Math.max(random, 0), 1));
}
