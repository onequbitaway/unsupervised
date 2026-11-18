// The loop of the agent: observe, think, act, wait.
//
// One turn asks the language model once. Between turns the agent blocks in a
// long `wait` call of the server, which costs nothing. A budget of decisions
// for each hour bounds the cost.

import {
  ApiError,
  MissingCredentialError,
  REPORT_KIND,
  UnsupervisedError,
  hasNews,
  reportsOf,
  selfOf,
  type ActionAck,
  type ActionReport,
  type Catalogue,
  type Notice,
  type Observation,
  type PlanResult,
  type PlanStep,
  type WaitOptions,
} from "@unsupervised/sdk";

import { DecisionBudget } from "./budget.js";
import { DecisionError, parseDecision, type Decision, type DecisionLimits } from "./decision.js";
import type { NotesStore } from "./notes.js";
import { buildMessages, DEFAULT_PROMPT_LIMITS, type LastTurn, type PromptLimits } from "./prompt.js";
import { ProviderError, type ChatProvider } from "./provider.js";

/** The part of the SDK client that the agent uses. `Client` fits. */
export interface AgentWorld {
  /** Returns the catalogue: the verbs with their argument schemas. */
  catalogue(): Promise<Catalogue>;
  /** Returns the current observation, with the news since the last one. */
  observe(): Promise<Observation>;
  /** Asks for several actions in order. */
  plan(steps: PlanStep[]): Promise<PlanResult>;
  /** Blocks until the resident has news or the timeout ends. */
  wait(options: WaitOptions): Promise<Observation>;
}

/** Options of the agent. `world`, `provider` and `notes` are necessary. */
export interface AgentOptions {
  /** The client of the resident. */
  world: AgentWorld;
  /** The language model. */
  provider: ChatProvider;
  /** The memory of the agent. */
  notes: NotesStore;
  /** Greatest number of decisions in one hour. Default: 12. */
  decisionsPerHour?: number;
  /** Least seconds between two decisions. Default: 30. */
  minDecisionSeconds?: number;
  /** Least wait that the model can choose, in seconds. Default: 60. */
  minWaitSeconds?: number;
  /** Greatest wait that the model can choose, in seconds. Default: 3600. */
  maxWaitSeconds?: number;
  /** The wait when the model gives none or gives no usable answer, in seconds. Default: 300. */
  defaultWaitSeconds?: number;
  /** Greatest number of actions in one turn. Default: 8, the default queue length of the server. */
  maxActions?: number;
  /**
   * Notice kinds that end a wait early. An action report has the kind
   * `report`. Default: `["report"]`. An empty list means every kind. A
   * report of an action that went well does not end the wait while other
   * planned actions still run.
   */
  wakeKinds?: string[];
  /** Decisions between two reads of the catalogue. Default: 50. */
  catalogueEvery?: number;
  /** Seconds that `run` waits after an error of the network or the server. Default: 30. */
  errorSeconds?: number;
  /** Waits for a number of milliseconds, or until the signal is aborted. Default: a timer. */
  sleep?: (milliseconds: number, signal?: AbortSignal) => Promise<void>;
  /** Limits of the prompt. */
  promptLimits?: PromptLimits;
  /** The time in milliseconds. Default: `Date.now`. */
  now?: () => number;
  /** Gets one line for each turn. Default: no log. */
  log?: (line: string) => void;
}

/** What one turn did. */
export interface TurnResult {
  /** True when the turn asked the model. False when the budget or the least interval held it back. */
  decided: boolean;
  /** The decision of the model, when the turn asked it and the answer was usable. */
  decision: Decision | undefined;
  /** The acknowledgements of the planned actions. */
  acks: ActionAck[];
  /** Greatest number of seconds that the turn waited for news at its end. News can end the wait earlier. */
  waitedSeconds: number;
}

const DEFAULTS = {
  decisionsPerHour: 12,
  minDecisionSeconds: 30,
  minWaitSeconds: 60,
  maxWaitSeconds: 3600,
  defaultWaitSeconds: 300,
  maxActions: 8,
  catalogueEvery: 50,
  errorSeconds: 30,
};
/** HTTP statuses that mean: the key does not work. `run` stops. */
const FATAL_STATUSES = [401, 403];
/** How many verbs of recent reports and answers stay in focus. */
const FOCUS_MAX = 10;
/** Greatest number of notices and of reports that wait for the next decision. */
const PENDING_MAX = 50;

/** An agent that asks a language model what its resident does. */
export class LlmAgent {
  /** How many times the agent asked the model. */
  decisions = 0;
  readonly #options: AgentOptions;
  readonly #budget: DecisionBudget;
  readonly #limits: DecisionLimits;
  readonly #now: () => number;
  #catalogue: Catalogue | undefined;
  #catalogueAt = 0;
  #observation: Observation | undefined;
  #notices: Notice[] = [];
  #reports: ActionReport[] = [];
  #focus: string[] = [];
  #last: LastTurn | undefined;
  #nextDecisionAt = 0;

  /** Makes an agent. It sends nothing until `turn` or `run` is called. */
  constructor(options: AgentOptions) {
    this.#options = options;
    this.#now = options.now ?? Date.now;
    this.#budget = new DecisionBudget(options.decisionsPerHour ?? DEFAULTS.decisionsPerHour, this.#now);
    this.#limits = {
      minWaitSeconds: options.minWaitSeconds ?? DEFAULTS.minWaitSeconds,
      maxWaitSeconds: options.maxWaitSeconds ?? DEFAULTS.maxWaitSeconds,
      defaultWaitSeconds: options.defaultWaitSeconds ?? DEFAULTS.defaultWaitSeconds,
      maxActions: options.maxActions ?? DEFAULTS.maxActions,
    };
  }

  /** Decisions that are left in the current hour. */
  get budgetLeft(): number {
    return this.#budget.left;
  }

  /**
   * Runs turns until `turns` are done or `signal` is aborted.
   *
   * An error of the network or the server does not stop the run: the agent
   * waits `errorSeconds` and makes the next turn. A key that the server
   * refuses (HTTP 401 or 403) stops the run: the error is thrown.
   */
  async run(options: { turns?: number; signal?: AbortSignal } = {}): Promise<void> {
    const { signal } = options;
    for (let turn = 0; options.turns === undefined || turn < options.turns; turn += 1) {
      if (isAborted(signal)) return;
      try {
        await this.turn(signal);
      } catch (error) {
        // An abort during the long poll ends the run without an error.
        if (isAborted(signal)) return;
        if (!canRecover(error)) throw error;
        const seconds = this.#options.errorSeconds ?? DEFAULTS.errorSeconds;
        this.#options.log?.(`error: ${error.message} | wait ${seconds}s`);
        // The answer of the server was lost. The next turn observes again.
        this.#observation = undefined;
        await (this.#options.sleep ?? defaultSleep)(seconds * 1000, signal);
      }
    }
  }

  /**
   * One turn: observe, think (when the budget allows), act, wait.
   *
   * An error of the model does not end the turn: the agent plans nothing
   * and waits. An error of the server is thrown.
   */
  async turn(signal?: AbortSignal): Promise<TurnResult> {
    const { world } = this.#options;
    if (this.#catalogue === undefined || this.decisions - this.#catalogueAt >= (this.#options.catalogueEvery ?? DEFAULTS.catalogueEvery)) {
      this.#catalogue = await world.catalogue();
      this.#catalogueAt = this.decisions;
    }
    if (this.#observation === undefined) this.#take(await world.observe());

    let decision: Decision | undefined;
    let acks: ActionAck[] = [];
    let waitSeconds: number;
    const early = Math.ceil((this.#nextDecisionAt - this.#now()) / 1000);
    const decided = early <= 0 && this.#budget.take();
    if (decided) {
      decision = await this.#think(this.#catalogue);
      if (decision !== undefined && decision.actions.length > 0) {
        const result = await world.plan(decision.actions);
        acks = result.acks;
        this.#last = { actions: acks.map(summary) };
        this.#take(result.observation);
      }
      waitSeconds = decision?.waitSeconds ?? this.#limits.defaultWaitSeconds;
      this.#nextDecisionAt = this.#now() + (this.#options.minDecisionSeconds ?? DEFAULTS.minDecisionSeconds) * 1000;
    } else {
      // No decision now: wait until the least interval is over or the budget has room.
      const held = Math.max(early, this.#budget.secondsUntilFree(), 1);
      waitSeconds = Math.min(this.#limits.maxWaitSeconds, held);
    }
    this.#options.log?.(
      decided
        ? `decision ${this.decisions}: ${decision?.thought ?? this.#last?.error ?? ""} | actions: ${
            acks.map((ack) => `${ack.verb}=${ack.status.state}`).join(", ") || "none"
          } | wait ${waitSeconds}s | budget left ${this.#budget.left}`
        : `no decision: wait ${waitSeconds}s | budget left ${this.#budget.left}`,
    );

    await this.#wait(waitSeconds, decided, signal);
    return { decided, decision, acks, waitedSeconds: waitSeconds };
  }

  /**
   * Blocks in the long poll of the server for at most `seconds`.
   *
   * After a decision, news ends the wait, with one exception: while planned
   * actions still run and every report is a success, the agent waits on.
   * So a plan of several actions costs one decision, not one for each
   * action. Without a decision, the agent cannot think before the time is
   * over, so it waits the whole time and only collects the news.
   */
  async #wait(seconds: number, decided: boolean, signal?: AbortSignal): Promise<void> {
    const kinds = this.#options.wakeKinds ?? [REPORT_KIND];
    const deadline = this.#now() + seconds * 1000;
    for (let left = seconds; left > 0; left = Math.ceil((deadline - this.#now()) / 1000)) {
      const observation = await this.#options.world.wait({
        timeout: left,
        ...(kinds.length > 0 ? { kinds } : {}),
        ...(signal === undefined ? {} : { signal }),
      });
      this.#take(observation);
      // An answer without news means that the time is over. The agent trusts
      // the server here: it does not ask again, so it cannot spin.
      if (!hasNews(observation, kinds)) return;
      if (decided && !planRunsWell(observation, kinds)) return;
    }
  }

  /** Asks the model once. Returns `undefined` when the answer is not usable. */
  async #think(catalogue: Catalogue): Promise<Decision | undefined> {
    const observation = this.#observation as Observation;
    const messages = buildMessages({
      observation,
      notices: this.#notices,
      reports: this.#reports,
      catalogue,
      notes: await this.#options.notes.read(),
      focus: this.#focus,
      turnsLeft: this.#budget.left,
      ...(this.#last === undefined ? {} : { last: this.#last }),
      limits: this.#options.promptLimits ?? DEFAULT_PROMPT_LIMITS,
    });
    this.decisions += 1;
    // The model saw the news. The next prompt shows only what comes later.
    this.#notices = [];
    this.#reports = [];
    this.#last = undefined;
    let decision: Decision;
    try {
      decision = parseDecision(await this.#options.provider.complete(messages), this.#limits);
    } catch (error) {
      if (error instanceof DecisionError) {
        this.#last = { error: error.message };
        return undefined;
      }
      if (error instanceof ProviderError) {
        this.#options.log?.(`the model gave no answer: ${error.message}`);
        return undefined;
      }
      throw error;
    }
    if (decision.note !== undefined) {
      const { clock } = observation;
      await this.#options.notes.add(decision.note, `year ${clock.year} day ${clock.day} ${clock.hour}:${String(clock.minute).padStart(2, "0")}`);
    }
    this.#remember([...decision.describe, ...decision.actions.map((action) => action.verb)]);
    return decision;
  }

  /** Keeps an observation and collects its news for the next decision. */
  #take(observation: Observation): void {
    this.#observation = observation;
    this.#notices = [...this.#notices, ...observation.notices].slice(-PENDING_MAX);
    const reports = reportsOf(observation);
    this.#reports = [...this.#reports, ...reports].slice(-PENDING_MAX);
    this.#remember(reports.map((report) => report.verb));
  }

  /** Puts verbs at the front of the focus list. */
  #remember(verbs: string[]): void {
    this.#focus = [...new Set([...verbs, ...this.#focus])].slice(0, FOCUS_MAX);
  }
}

/**
 * Returns true when the news of an observation is no reason to think again:
 * the resident still has planned actions, no action failed, and no notice
 * came that the owner wants to be woken for.
 */
function planRunsWell(observation: Observation, kinds: string[]): boolean {
  const me = selfOf(observation);
  if (me === undefined || (me.activity === null && me.queue === 0)) return false;
  if (reportsOf(observation).some((report) => !report.ok)) return false;
  return !observation.notices.some((notice) => kinds.length === 0 || kinds.includes(notice.kind));
}

/** Returns true when the caller gave up. A function, because the answer changes while the agent waits. */
function isAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

/** Returns true for an error that a later turn can get past: the network failed or the server had a problem. */
function canRecover(error: unknown): error is UnsupervisedError {
  if (!(error instanceof UnsupervisedError) || error instanceof MissingCredentialError) return false;
  return !(error instanceof ApiError && error.status !== undefined && FATAL_STATUSES.includes(error.status));
}

/** Waits for a number of milliseconds. Ends early, without an error, when the signal is aborted. */
function defaultSleep(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted === true) {
      resolve();
      return;
    }
    const done = (): void => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", done);
      resolve();
    };
    const timer = setTimeout(done, milliseconds);
    signal?.addEventListener("abort", done);
  });
}

/** Returns what the next prompt says about one acknowledgement. */
function summary(ack: ActionAck): NonNullable<LastTurn["actions"]>[number] {
  const { status } = ack;
  const entry: NonNullable<LastTurn["actions"]>[number] = { verb: ack.verb, state: status.state };
  if (status.state === "rejected") entry.error = `${status.error.code}: ${status.error.message}`;
  if (status.state === "done" && status.result !== undefined) entry.result = status.result;
  return entry;
}
