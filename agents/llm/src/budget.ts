// A limit on the number of decisions in one hour. Each decision is one
// request to the language model, so the limit bounds the cost.

const HOUR_MS = 3_600_000;

/** Counts decisions in a sliding window of one hour. */
export class DecisionBudget {
  readonly #perHour: number;
  readonly #now: () => number;
  #taken: number[] = [];

  /**
   * Makes a budget of `perHour` decisions in any 60 minutes. `now` gives the
   * time in milliseconds.
   */
  constructor(perHour: number, now: () => number = Date.now) {
    this.#perHour = perHour;
    this.#now = now;
  }

  #forget(): void {
    const start = this.#now() - HOUR_MS;
    this.#taken = this.#taken.filter((time) => time > start);
  }

  /** How many decisions are left in the current hour. */
  get left(): number {
    this.#forget();
    return Math.max(0, this.#perHour - this.#taken.length);
  }

  /** Takes one decision from the budget. Returns false when none is left. */
  take(): boolean {
    if (this.left <= 0) return false;
    this.#taken.push(this.#now());
    return true;
  }

  /** Seconds until one decision is free again. 0 when one is free now. */
  secondsUntilFree(): number {
    if (this.left > 0) return 0;
    if (this.#perHour <= 0) return HOUR_MS / 1000;
    const oldest = this.#taken[0] as number;
    return Math.max(1, Math.ceil((oldest + HOUR_MS - this.#now()) / 1000));
  }
}
