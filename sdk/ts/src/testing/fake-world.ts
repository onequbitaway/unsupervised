// A small model of the server, for tests without a network.
//
// `FakeWorld` follows `docs/design/protocol.md` and the real server:
// accounts and keys, the action engine (idempotency keys, the queue, free
// verbs, reports), the inbox with one sequence for notices and reports,
// observations and the long poll. The end-to-end tests in `crates/e2e`
// compare its answers with those of the real server. It runs routines as
// the kernel does: the first routine of an idle resident whose condition
// holds starts its action, and only a failure of that action writes a
// report. It is not the game: it has four verbs (`wait`, `cancel`,
// `set_routines`, `move_to`), a new resident has no routines, and a test
// adds the verbs that it needs with `addVerb`.

import type {
  ActionAck,
  ActionError,
  ActionReport,
  ActionRequest,
  ActionStatus,
  AttrInfo,
  Catalogue,
  CityId,
  Clock,
  CondInfo,
  Condition,
  ErrorCode,
  ItemInfo,
  Notice,
  Observation,
  ObserveOpts,
  OwnerId,
  ParamInfo,
  Pos,
  Rect,
  ResidentId,
  Routine,
  SiteView,
  Value,
  VerbInfo,
  ZoneId,
} from "../generated/protocol.js";
import { defaultVerbs } from "./verbs.js";

/**
 * The resident of a long poll left the world while the call was open. The
 * gateway answers that call with `410 resident_gone`.
 */
export class ResidentGoneError extends Error {
  /** Makes the error for a resident. */
  constructor(readonly resident: ResidentId) {
    super(`the resident ${resident} left the world`);
  }
}

/** A mission that a sponsor published: an owner, not a resident. */
export interface FakeMission {
  /** Id of the mission. */
  mission: number;
  /** The number of the request of the sponsor. */
  request: number;
  /** The owner that is the sponsor. */
  owner: OwnerId;
  /** The frame of the publication. */
  frame: number;
  /** The frame of the deadline. */
  deadline: number;
  /** The definition as the sponsor sent it. */
  definition: Record<string, Value>;
}

/** An action error that a fake verb throws to refuse or to fail. */
export class FakeActionError extends Error {
  /** Makes an error with a protocol error code. */
  constructor(
    readonly code: ErrorCode,
    message: string,
    readonly details?: Value,
  ) {
    super(message);
  }

  /** Returns the wire form of the error. */
  toWire(): ActionError {
    const error: ActionError = { code: this.code, message: this.message };
    if (this.details !== undefined) error.details = this.details;
    return error;
  }
}

/** What `begin` of a fake verb decides. */
export type VerbOutcome =
  /** The action finished at once. */
  | { done: Value }
  /** The action occupies the resident for a number of ticks. `finish` gives the result; it can throw `FakeActionError`. */
  | { busy: number; finish?: () => Value }
  /**
   * The action waits for another party for a number of ticks. The
   * acknowledgement is `pending`, and a report follows. A free verb of the
   * global realm does this, for example `list_jobs`: it does not occupy
   * the resident.
   */
  | { pending: number; finish?: () => Value };

/** A verb of the fake world. */
export interface FakeVerb {
  /** The catalogue entry. */
  info: VerbInfo;
  /** Starts the action. Throw `FakeActionError` to refuse it. */
  begin(resident: FakeResident, args: Record<string, Value>, world: FakeWorld): VerbOutcome;
}

interface Running {
  action_id: number;
  verb: string;
  request_id: string | undefined;
  started: number;
  ends: number;
  waiting: boolean;
  finish: (() => Value) | undefined;
  /** The place of the routine that started the action, in the list of routines. */
  routine?: number | undefined;
}

/** The counters of one routine. */
export interface RoutineStats {
  /** The routine does not fire before this tick. */
  retryAt: number;
  /** Failures in a row. */
  failures: number;
  /** How many times the action of the routine went well. */
  runs: number;
}

interface Queued {
  action_id: number;
  verb: string;
  request_id: string | undefined;
  args: Record<string, Value>;
}

/** One inbox entry: a notice or a report. Both use one sequence. */
export type InboxEntry = { seq: number; notice: Notice } | { seq: number; report: ActionReport };

/** A resident of the fake world. A test can read and change every field. */
export interface FakeResident {
  id: ResidentId;
  owner: OwnerId;
  name: string;
  model: string | undefined;
  city: CityId;
  zone: ZoneId;
  pos: Pos;
  /** Cash in cents. */
  cash: number;
  /** Carried goods: quantity by item name. */
  items: Record<string, number>;
  /** Attributes by name. */
  attrs: Record<string, number>;
  /** Conditions by name. The value is the end tick, or `null` for no end. */
  conditions: Record<string, number | null>;
  /** Extra observation sections, as a game module writes them. */
  sections: Record<string, Value>;
  /** The routines as the owner wrote them, in priority order. Change them with `FakeWorld.setRoutines`. */
  routines: Routine[];
  /** The counters of each routine, at the place of the routine. */
  routineStats: RoutineStats[];
  /** The tick of the last routine that fired. At most one routine fires in one tick. */
  routineTick: number;
  current: Running | undefined;
  /** Free actions that wait for an answer. They do not occupy the resident. */
  background: Running[];
  queue: Queued[];
  inbox: InboxEntry[];
  nextSeq: number;
  /** The last accepted request ids with their acknowledgements. */
  recent: [string, ActionAck][];
  key: string;
  /** Time of the creation, in seconds since 1970. */
  created_at: number;
  webhook: { url: string; secret: string } | undefined;
  freeTick: number;
  freeUsed: number;
}

/** Limits of the fake world. The defaults are the defaults of the kernel. */
export interface FakeLimits {
  queue_max: number;
  free_actions_per_tick: number;
  inbox_max: number;
  routines_max: number;
  sight_radius: number;
  nearby_max: number;
  wait_max: number;
  /** Ticks before a routine fires again after it failed or finished at once. The delay doubles with each failure in a row. */
  routine_backoff: number;
  /** Greatest number of residents of one owner. 0: no limit. This is a limit of the gateway, not a kernel parameter. */
  residents_per_owner: number;
}

/** The defaults of the kernel parameters that the fake world uses. */
export const DEFAULT_LIMITS: FakeLimits = {
  queue_max: 8,
  free_actions_per_tick: 4,
  inbox_max: 32,
  routines_max: 8,
  sight_radius: 12,
  nearby_max: 12,
  wait_max: 1440,
  routine_backoff: 30,
  residents_per_owner: 3,
};

/** The limits that are kernel parameters: the catalogue lists them. */
const KERNEL_LIMITS = [
  "queue_max",
  "free_actions_per_tick",
  "inbox_max",
  "routines_max",
  "sight_radius",
  "nearby_max",
  "wait_max",
  "routine_backoff",
] as const;

/** The time of every sign-up in the fake world, in seconds since 1970. */
const CREATED_AT = 1_800_000_000;

/** Returns a token of the form that the server makes: a prefix and 64 hexadecimal characters. */
function token(prefix: string, ...numbers: number[]): string {
  return `${prefix}${numbers.map((number) => number.toString(16).padStart(8, "0")).join("").padEnd(64, "0")}`;
}

/** Options of a fake world. Every option has a default. */
export interface FakeWorldOptions {
  /** Limits. Default: `DEFAULT_LIMITS`. */
  limits?: Partial<FakeLimits>;
  /** Verbs. Default: `wait`, `cancel`, `set_routines`, `move_to`. */
  verbs?: FakeVerb[];
  /** Attributes of residents. Default: none. */
  attrs?: AttrInfo[];
  /** Items. Default: none. */
  items?: ItemInfo[];
  /** Conditions. Default: `frozen`. */
  conditions?: CondInfo[];
  /** Sites of the one zone. Default: none. */
  sites?: SiteView[];
  /** Where a new resident appears. Default: the centre of the zone. */
  spawn?: Pos;
  /**
   * How time passes during a long poll. `manual`: a `wait` call parks until
   * there is news or `expireWaits` is called. `auto`: a `wait` call steps
   * the world, one tick for each second of its timeout, until there is
   * news. Default: `manual`.
   */
  time?: "manual" | "auto";
}

/** Verbs that work in every condition except `frozen`, and that a routine cannot change. */
const KERNEL_VERBS = ["wait", "cancel", "set_routines"];
const REQUEST_ID_MAX = 64;
const RECENT_MAX = 32;
const NAME_MAX = 64;
const ZONE_SIZE = 64;
const DAYS_PER_YEAR = 28;
const MINUTES_PER_DAY = 1440;
const REPORT_KIND = "report";
/** Greatest number of times that the backoff of a failed routine doubles. */
const BACKOFF_DOUBLINGS_MAX = 16;

const textEncoder = new TextEncoder();

/** Returns the length of a string in bytes. */
export function byteLength(text: string): number {
  return textEncoder.encode(text).length;
}

interface Waiter {
  resident: FakeResident;
  since: number;
  kinds: string[] | undefined;
  resolve: (observation: Observation) => void;
  reject: (error: Error) => void;
}

/** A listener for the inbox of one resident: a WebSocket connection. `gone` says that the resident left the world. */
export type InboxListener = (entry: InboxEntry | "gone") => void;

/** The fake world. See the head of this file. */
export class FakeWorld {
  /** Local tick. One tick is one game minute. The frame number is the same. */
  tick = 0;
  readonly limits: FakeLimits;
  readonly residents = new Map<ResidentId, FakeResident>();
  readonly sites: SiteView[];
  readonly attrs: AttrInfo[];
  readonly items: ItemInfo[];
  readonly conditions: CondInfo[];
  readonly time: "manual" | "auto";
  /** The missions that sponsors published, oldest first. */
  readonly missions: FakeMission[] = [];
  /** The keys of the residents that died, with their owners, until the gateway learns of the death. */
  readonly #dead = new Map<string, OwnerId>();
  #requests = 0;
  readonly #verbs = new Map<string, FakeVerb>();
  readonly #owners = new Map<string, OwnerId>();
  readonly #ownerNames = new Map<OwnerId, string>();
  readonly #keys = new Map<string, ResidentId>();
  readonly #listeners = new Map<ResidentId, Set<InboxListener>>();
  readonly #spawn: Pos;
  #waiters: Waiter[] = [];
  #parked: (() => void)[] = [];
  #ids = 0;
  #owner = 0;
  #secrets = 0;

  /** Makes an empty world at tick 0. */
  constructor(options: FakeWorldOptions = {}) {
    this.limits = { ...DEFAULT_LIMITS, ...options.limits };
    this.sites = options.sites ?? [];
    this.attrs = options.attrs ?? [];
    this.items = options.items ?? [];
    this.conditions = options.conditions ?? [
      { name: "frozen", blocking: true, summary: "An administrator stopped the resident. It cannot act." },
    ];
    this.time = options.time ?? "manual";
    this.#spawn = options.spawn ?? { x: ZONE_SIZE / 2, y: ZONE_SIZE / 2 };
    for (const verb of options.verbs ?? defaultVerbs()) this.addVerb(verb);
  }

  // ---- Catalogue ------------------------------------------------------------

  /** Adds a verb, or replaces the verb with the same name. */
  addVerb(verb: FakeVerb): void {
    this.#verbs.set(verb.info.verb, verb);
  }

  /** Removes a verb. A later request for it gets `unknown_verb`. */
  removeVerb(name: string): void {
    this.#verbs.delete(name);
  }

  /** Returns the public catalogue. */
  catalogue(): Catalogue {
    const verbs = [...this.#verbs.values()]
      .map((verb) => verb.info)
      .sort((a, b) => a.module.localeCompare(b.module) || a.verb.localeCompare(b.verb));
    const params: ParamInfo[] = KERNEL_LIMITS.map((name) => ({
      name: `kernel.${name}`,
      value: this.limits[name],
      policy: false,
      summary: `The kernel parameter ${name}.`,
    }))
      .sort((a, b) => a.name.localeCompare(b.name));
    return { verbs, params, items: this.items, attrs: this.attrs, conditions: this.conditions };
  }

  // ---- Accounts -------------------------------------------------------------

  /** Signs up an owner. Returns the owner and its token, as `POST /v1/owners` does. */
  signUp(name = "Owner"): { owner: { id: OwnerId; name: string; created_at: number }; token: string } {
    this.#owner += 1;
    const secret = token("unsup_own_", this.#owner);
    this.#owners.set(secret, this.#owner);
    this.#ownerNames.set(this.#owner, name);
    return { owner: { id: this.#owner, name, created_at: CREATED_AT }, token: secret };
  }

  /** Returns the residents of an owner that are in the world, in the order of their ids. */
  residentsOf(owner: OwnerId): FakeResident[] {
    return [...this.residents.values()].filter((resident) => resident.owner === owner).sort((a, b) => a.id - b.id);
  }

  /**
   * Removes a resident from the world, as an administrator does. Its key
   * ends at once: the next call with it gets `401 invalid_token`.
   */
  despawn(resident: FakeResident): void {
    this.#remove(resident);
  }

  /**
   * Removes a resident from the world, as a death does. The gateway learns
   * of a death with the next call of the key: that call gets
   * `410 resident_gone`, and each later call gets `401 invalid_token`. A
   * long poll of the resident that is open gets the 410.
   */
  die(resident: FakeResident): void {
    if (!this.residents.has(resident.id)) return;
    this.#dead.set(resident.key, resident.owner);
    this.#remove(resident);
  }

  #remove(resident: FakeResident): void {
    this.residents.delete(resident.id);
    this.#keys.delete(resident.key);
    const open = this.#waiters.filter((waiter) => waiter.resident === resident);
    this.#waiters = this.#waiters.filter((waiter) => waiter.resident !== resident);
    for (const waiter of open) waiter.reject(new ResidentGoneError(resident.id));
    for (const listener of this.#listeners.get(resident.id) ?? []) listener("gone");
    this.#listeners.delete(resident.id);
  }

  /** Returns the owner of a resident that died, when the gateway did not yet learn of the death with this key. */
  deadOwner(key: string | undefined): OwnerId | undefined {
    return key === undefined ? undefined : this.#dead.get(key);
  }

  /** Notes that the gateway learned of a death with a key. The key is unknown from now on. */
  forgetDead(key: string): void {
    this.#dead.delete(key);
  }

  /** Returns the name of an owner. */
  ownerName(owner: OwnerId): string | undefined {
    return this.#ownerNames.get(owner);
  }

  /**
   * Publishes a mission of a sponsor, as the missions module does after its
   * checks. The fake publishes at once: it does not wait for a frame.
   */
  postMission(owner: OwnerId, definition: Record<string, Value>): FakeMission {
    this.#ids += 1;
    this.#requests += 1;
    const mission: FakeMission = {
      mission: this.#ids,
      request: this.#requests,
      owner,
      frame: this.tick,
      deadline: this.tick + (definition["deadline_in"] as number),
      definition,
    };
    this.missions.push(mission);
    return mission;
  }

  /** Returns the owner of a token, if the token is known. */
  ownerOf(token: string | undefined): OwnerId | undefined {
    return token === undefined ? undefined : this.#owners.get(token);
  }

  /** Returns the resident of a key, if the key is known. */
  residentOf(key: string | undefined): FakeResident | undefined {
    const id = key === undefined ? undefined : this.#keys.get(key);
    return id === undefined ? undefined : this.residents.get(id);
  }

  /** Creates a resident for an owner. Returns its id and its key. */
  spawn(owner: OwnerId, spec: { name: string; model?: string; city?: CityId }): FakeResident {
    this.#ids += 1;
    const id = this.#ids;
    const resident: FakeResident = {
      id,
      owner,
      name: spec.name,
      model: spec.model,
      city: spec.city ?? 0,
      zone: 0,
      pos: { ...this.#spawn },
      cash: 0,
      items: {},
      attrs: Object.fromEntries(this.attrs.map((attr) => [attr.name, attr.default])),
      conditions: {},
      sections: {},
      routines: [],
      routineStats: [],
      routineTick: -1,
      current: undefined,
      background: [],
      queue: [],
      inbox: [],
      nextSeq: 1,
      recent: [],
      key: "",
      created_at: CREATED_AT,
      webhook: undefined,
      freeTick: -1,
      freeUsed: 0,
    };
    this.residents.set(id, resident);
    this.rotateKey(resident);
    return resident;
  }

  /** Gives a resident a new key. The old key stops to work. */
  rotateKey(resident: FakeResident): string {
    this.#keys.delete(resident.key);
    this.#secrets += 1;
    resident.key = token("unsup_res_", resident.id, this.#secrets);
    this.#keys.set(resident.key, resident.id);
    return resident.key;
  }

  /** Sets or removes the webhook of a resident. Returns the new secret. */
  setWebhook(resident: FakeResident, url: string | null): string | undefined {
    if (url === null) {
      resident.webhook = undefined;
      return undefined;
    }
    this.#secrets += 1;
    resident.webhook = { url, secret: token("unsup_whs_", resident.id, this.#secrets) };
    return resident.webhook.secret;
  }

  // ---- Actions --------------------------------------------------------------

  /** Handles one action request and returns the acknowledgement. */
  submit(resident: FakeResident, request: ActionRequest): ActionAck {
    const key = request.id ?? undefined;
    if (key !== undefined) {
      if (byteLength(key) > REQUEST_ID_MAX) {
        const error = new FakeActionError("invalid_args", `The request id is longer than ${REQUEST_ID_MAX} bytes.`);
        return { action_id: 0, verb: request.verb, status: { state: "rejected", error: error.toWire() } };
      }
      const repeat = resident.recent.find(([id]) => id === key);
      if (repeat !== undefined) return repeat[1];
    }
    this.#ids += 1;
    const action_id = this.#ids;
    const status = this.#status(resident, request, action_id);
    const ack: ActionAck = { action_id, verb: request.verb, status };
    if (key !== undefined) ack.request_id = key;
    // A rejected request changed nothing, so its key can be used again.
    if (key !== undefined && status.state !== "rejected") {
      if (resident.recent.length >= RECENT_MAX) resident.recent.shift();
      resident.recent.push([key, ack]);
    }
    return ack;
  }

  #status(resident: FakeResident, request: ActionRequest, action_id: number): ActionStatus {
    try {
      const verb = this.#verbs.get(request.verb);
      if (verb === undefined) {
        throw new FakeActionError("unknown_verb", `No verb is named \`${request.verb}\`.`);
      }
      this.#checkConditions(resident, verb.info.verb);
      const args = checkArgs(verb.info, request.args);
      if (verb.info.free) {
        if (resident.freeTick !== this.tick) {
          resident.freeTick = this.tick;
          resident.freeUsed = 0;
        }
        const limit = this.limits.free_actions_per_tick;
        if (resident.freeUsed >= limit) {
          throw new FakeActionError("rate_limited", `At most ${limit} free actions in one tick.`);
        }
        resident.freeUsed += 1;
        const outcome = verb.begin(resident, args, this);
        if ("done" in outcome) return doneStatus(outcome.done);
        if (!("pending" in outcome)) {
          throw new FakeActionError("internal", `The free verb \`${verb.info.verb}\` cannot occupy the resident.`);
        }
        // A free verb of the global realm: the answer comes later, in a
        // report, and the resident stays free.
        resident.background.push({
          action_id,
          verb: verb.info.verb,
          request_id: request.id ?? undefined,
          started: this.tick,
          ends: this.tick + Math.max(1, outcome.pending),
          waiting: true,
          finish: outcome.finish,
        });
        return { state: "pending" };
      }
      if (resident.current !== undefined || resident.queue.length > 0) {
        const max = this.limits.queue_max;
        if (resident.queue.length >= max) {
          throw new FakeActionError("queue_full", `The queue holds at most ${max} planned actions.`);
        }
        resident.queue.push({ action_id, verb: verb.info.verb, request_id: request.id ?? undefined, args });
        return { state: "queued", position: resident.queue.length };
      }
      return this.#begin(resident, verb, request.id ?? undefined, args, action_id);
    } catch (error) {
      if (error instanceof FakeActionError) return { state: "rejected", error: error.toWire() };
      throw error;
    }
  }

  #checkConditions(resident: FakeResident, verb: string): void {
    for (const [name, until] of Object.entries(resident.conditions)) {
      if (until !== null && until <= this.tick) continue;
      const blocking = this.conditions.find((condition) => condition.name === name)?.blocking === true;
      if (name === "frozen" || (blocking && !KERNEL_VERBS.includes(verb))) {
        throw new FakeActionError("blocked", `You cannot use \`${verb}\` while you are ${name}.`, {
          condition: name,
        });
      }
    }
  }

  #begin(
    resident: FakeResident,
    verb: FakeVerb,
    request_id: string | undefined,
    args: Record<string, Value>,
    action_id: number,
    routine?: number,
  ): ActionStatus {
    const outcome = verb.begin(resident, args, this);
    if ("done" in outcome) return doneStatus(outcome.done);
    const waiting = "pending" in outcome;
    const ticks = Math.max(1, waiting ? outcome.pending : outcome.busy);
    resident.current = {
      action_id,
      verb: verb.info.verb,
      request_id,
      started: this.tick,
      ends: this.tick + ticks,
      waiting,
      finish: outcome.finish,
      routine,
    };
    return waiting ? { state: "pending" } : { state: "started", ends_at: this.tick + ticks, ticks };
  }

  /** Stops the running action and clears the queue, as the verb `cancel` does. */
  cancel(resident: FakeResident): Value {
    const cleared = resident.queue.length;
    for (const queued of resident.queue.splice(0)) {
      const error = new FakeActionError("interrupted", "The action was removed from the queue.");
      this.#report(resident, queued, error);
    }
    const running = resident.current;
    resident.current = undefined;
    if (running !== undefined) {
      this.#report(resident, running, new FakeActionError("interrupted", "You cancelled the action."));
    }
    return { cancelled: running?.action_id ?? null, cleared };
  }

  /** Replaces the routines of a resident, as the verb `set_routines` does. */
  setRoutines(resident: FakeResident, routines: Routine[]): Value {
    const max = this.limits.routines_max;
    if (routines.length > max) {
      throw new FakeActionError("invalid_args", `A resident has at most ${max} routines.`);
    }
    for (const routine of routines) {
      if (byteLength(routine.name) > NAME_MAX) {
        throw new FakeActionError("invalid_args", `A routine name has at most ${NAME_MAX} bytes.`);
      }
      this.#checkCondition(routine.when);
      const name = routine.then.verb;
      const verb = this.#verbs.get(name);
      if (verb === undefined) throw new FakeActionError("unknown_verb", `No verb is named \`${name}\`.`);
      if (name === "set_routines" || name === "cancel") {
        throw new FakeActionError("invalid_args", `A routine cannot use the verb \`${name}\`.`);
      }
      checkArgs(verb.info, routine.then.args);
    }
    resident.routines = routines.map((routine) => {
      const { id: _id, ...then } = routine.then;
      return { ...routine, then };
    });
    resident.routineStats = routines.map(() => ({ retryAt: 0, failures: 0, runs: 0 }));
    // The running action goes on, but it no longer counts for a routine.
    if (resident.current !== undefined) resident.current.routine = undefined;
    return { routines: routines.length };
  }

  #checkCondition(condition: Condition): void {
    switch (condition.type) {
      case "always":
        return;
      case "attr_below":
      case "attr_above":
        if (!this.attrs.some((attr) => attr.name === condition.attr)) {
          throw new FakeActionError("invalid_args", `No attribute is named \`${condition.attr}\`.`);
        }
        return;
      case "clock_between":
        if (condition.from_hour > 23 || condition.to_hour > 24) {
          throw new FakeActionError("invalid_args", "`from_hour` is from 0 to 23 and `to_hour` is from 0 to 24.");
        }
        return;
      case "has_item":
        if (!this.items.some((item) => item.name === condition.item)) {
          throw new FakeActionError("invalid_args", `No item is named \`${condition.item}\`.`);
        }
        return;
      case "all":
      case "any":
        condition.of.forEach((part) => this.#checkCondition(part));
        return;
      default:
        throw new FakeActionError("invalid_args", "The condition type is not known.");
    }
  }

  // ---- Time -----------------------------------------------------------------

  /** Runs a number of ticks: actions end, reports are written, queued actions start, routines fire. */
  step(ticks = 1): void {
    for (let i = 0; i < ticks; i += 1) {
      this.tick += 1;
      const residents = [...this.residents.values()].sort((a, b) => a.id - b.id);
      for (const resident of residents) this.#advance(resident);
    }
  }

  #advance(resident: FakeResident): void {
    for (const waiting of resident.background.filter((action) => action.ends <= this.tick)) {
      resident.background.splice(resident.background.indexOf(waiting), 1);
      this.#finish(resident, waiting);
    }
    const running = resident.current;
    if (running !== undefined && running.ends <= this.tick) {
      resident.current = undefined;
      this.#finish(resident, running);
    }
    while (resident.current === undefined && resident.queue.length > 0) {
      const queued = resident.queue.shift() as Queued;
      const verb = this.#verbs.get(queued.verb);
      try {
        if (verb === undefined) {
          throw new FakeActionError("unknown_verb", `No verb is named \`${queued.verb}\`.`);
        }
        this.#checkConditions(resident, queued.verb);
        const status = this.#begin(resident, verb, queued.request_id, queued.args, queued.action_id);
        // The acknowledgement was `queued`, so an action that ends at once reports too.
        if (status.state === "done") this.#report(resident, queued, status.result ?? null);
      } catch (error) {
        if (!(error instanceof FakeActionError)) throw error;
        this.#report(resident, queued, error);
      }
    }
    this.#tryRoutines(resident);
  }

  #finish(resident: FakeResident, running: Running): void {
    let result: Value | FakeActionError;
    try {
      result = running.waiting && running.finish === undefined
        ? new FakeActionError("timeout", "The action waited too long for an answer.")
        : (running.finish?.() ?? null);
    } catch (error) {
      if (!(error instanceof FakeActionError)) throw error;
      result = error;
    }
    const failed = result instanceof FakeActionError;
    if (running.routine !== undefined) this.#noteRoutine(resident, running.routine, !failed, false);
    // A routine action that goes well is only counted. A failure reports.
    if (running.routine === undefined || failed) this.#report(resident, running, result);
  }

  /** Returns true when a routine condition holds for a resident. */
  holds(resident: FakeResident, condition: Condition): boolean {
    switch (condition.type) {
      case "always":
        return true;
      case "attr_below":
        return (resident.attrs[condition.attr] ?? 0) < condition.value;
      case "attr_above":
        return (resident.attrs[condition.attr] ?? 0) > condition.value;
      case "clock_between":
        return hourBetween(this.clock().hour, condition.from_hour, condition.to_hour);
      case "has_item":
        return (resident.items[condition.item] ?? 0) >= condition.qty;
      case "all":
        return condition.of.every((part) => this.holds(resident, part));
      case "any":
        return condition.of.some((part) => this.holds(resident, part));
      default:
        return false;
    }
  }

  /** Fires the first routine of an idle resident whose condition holds. At most one routine fires in one tick. */
  #tryRoutines(resident: FakeResident): void {
    if (resident.routines.length === 0 || resident.current !== undefined || resident.queue.length > 0) return;
    if (resident.routineTick === this.tick || this.#isFrozen(resident)) return;
    const index = resident.routines.findIndex(
      (routine, place) => this.#stats(resident, place).retryAt <= this.tick && this.holds(resident, routine.when),
    );
    if (index < 0) return;
    resident.routineTick = this.tick;
    this.#ids += 1;
    const action_id = this.#ids;
    const then = (resident.routines[index] as Routine).then;
    const action = { action_id, verb: then.verb, request_id: undefined };
    try {
      const verb = this.#verbs.get(then.verb);
      if (verb === undefined) throw new FakeActionError("unknown_verb", `No verb is named \`${then.verb}\`.`);
      this.#checkConditions(resident, then.verb);
      const status = this.#begin(resident, verb, undefined, checkArgs(verb.info, then.args), action_id, index);
      // An action that finished at once holds the routine back, so that it cannot fire every tick.
      if (status.state === "done") this.#noteRoutine(resident, index, true, true);
    } catch (error) {
      if (!(error instanceof FakeActionError)) throw error;
      this.#noteRoutine(resident, index, false, true);
      this.#report(resident, action, error);
    }
  }

  #isFrozen(resident: FakeResident): boolean {
    const until = resident.conditions["frozen"];
    return until !== undefined && (until === null || until > this.tick);
  }

  /** Returns the counters of the routine at a place of the list. */
  #stats(resident: FakeResident, place: number): RoutineStats {
    return (resident.routineStats[place] ??= { retryAt: 0, failures: 0, runs: 0 });
  }

  #noteRoutine(resident: FakeResident, place: number, ok: boolean, instant: boolean): void {
    if (place >= resident.routines.length) return;
    const stats = this.#stats(resident, place);
    const backoff = Math.max(1, this.limits.routine_backoff);
    if (ok) {
      stats.failures = 0;
      stats.runs += 1;
      if (instant) stats.retryAt = this.tick + backoff;
    } else {
      const doublings = Math.min(stats.failures, BACKOFF_DOUBLINGS_MAX);
      stats.failures += 1;
      stats.retryAt = this.tick + backoff * 2 ** doublings;
    }
  }

  #report(
    resident: FakeResident,
    action: { action_id: number; verb: string; request_id: string | undefined },
    result: Value | FakeActionError,
  ): void {
    const report: ActionReport = { action_id: action.action_id, verb: action.verb, tick: this.tick, ok: true };
    if (action.request_id !== undefined) report.request_id = action.request_id;
    if (result instanceof FakeActionError) {
      report.ok = false;
      report.error = result.toWire();
    } else if (result !== null) {
      report.result = result;
    }
    this.#push(resident, (seq) => ({ seq, report: { ...report, seq } }));
  }

  // ---- Inbox ----------------------------------------------------------------

  /** Writes a notice to the inbox of a resident. Returns its sequence number. */
  notify(to: ResidentId, kind: string, data?: Value, from?: ResidentId): number {
    const resident = this.#resident(to);
    return this.#push(resident, (seq) => {
      const notice: Notice = { seq, tick: this.tick, kind };
      if (from !== undefined) notice.from = from;
      if (data !== undefined && data !== null) notice.data = data;
      return { seq, notice };
    });
  }

  /**
   * Writes a notice with text of another resident, as the verbs `say`
   * (`society.said`) and `message` (`society.message`) do. The text is in
   * the field `text` of the data, in an object with `untrusted: true`.
   */
  tell(to: ResidentId, from: ResidentId, text: string, kind = "society.said"): number {
    return this.notify(to, kind, { text: { untrusted: true, from, text } }, from);
  }

  /** Listens to the inbox of a resident, as a WebSocket connection does. Returns a function that stops it. */
  listen(resident: ResidentId, listener: InboxListener): () => void {
    const set = this.#listeners.get(resident) ?? new Set<InboxListener>();
    this.#listeners.set(resident, set);
    set.add(listener);
    return () => set.delete(listener);
  }

  #push(resident: FakeResident, make: (seq: number) => InboxEntry): number {
    const entry = make(resident.nextSeq);
    resident.nextSeq += 1;
    resident.inbox.push(entry);
    while (resident.inbox.length > Math.max(1, this.limits.inbox_max)) resident.inbox.shift();
    for (const listener of this.#listeners.get(resident.id) ?? []) listener(entry);
    this.#wake(resident);
    return entry.seq;
  }

  #resident(id: ResidentId): FakeResident {
    const resident = this.residents.get(id);
    if (resident === undefined) throw new Error(`the fake world has no resident ${id}`);
    return resident;
  }

  // ---- Observation ----------------------------------------------------------

  /** Returns the calendar time. */
  clock(): Clock {
    const minute = this.tick % MINUTES_PER_DAY;
    const day = Math.floor(this.tick / MINUTES_PER_DAY);
    return {
      year: Math.floor(day / DAYS_PER_YEAR) + 1,
      day: (day % DAYS_PER_YEAR) + 1,
      hour: Math.floor(minute / 60),
      minute: minute % 60,
    };
  }

  /** Returns the observation of a resident. It changes nothing. */
  observe(resident: FakeResident, opts: ObserveOpts = {}): Observation {
    const since = opts.since_notice ?? 0;
    const wants = (section: string): boolean => opts.sections == null || opts.sections.includes(section);
    const sections: Record<string, Value> = {};
    const notices: Notice[] = [];
    const reports: ActionReport[] = [];
    for (const entry of resident.inbox) {
      if (entry.seq < since) continue;
      if ("notice" in entry) notices.push(entry.notice);
      else reports.push(entry.report);
    }
    if (wants("self")) sections["self"] = this.#self(resident, opts.detail === "full");
    if (wants("nearby")) sections["nearby"] = this.#nearby(resident);
    if (wants("reports")) sections["reports"] = reports as unknown as Value;
    for (const [name, value] of Object.entries(resident.sections)) {
      if (wants(name)) sections[name] = value;
    }
    return {
      frame: this.tick,
      tick: this.tick,
      clock: this.clock(),
      resident: resident.id,
      sections,
      notices,
      next_notice: resident.nextSeq,
    };
  }

  #self(resident: FakeResident, full: boolean): Value {
    const site = this.sites.find((candidate) => contains(candidate.rect, resident.pos));
    const running = resident.current;
    const conditions: Record<string, Value> = {};
    for (const [name, until] of Object.entries(resident.conditions)) {
      if (until === null || until > this.tick) conditions[name] = until;
    }
    const out: Record<string, Value> = {
      id: resident.id,
      name: resident.name,
      pos: { ...resident.pos },
      zone: resident.zone,
      city: resident.city,
    };
    if (site !== undefined) out["site"] = { id: site.id, name: site.name, kind: site.kind };
    out["activity"] =
      running === undefined
        ? null
        : {
            verb: running.verb,
            action_id: running.action_id,
            started: running.started,
            ends: running.ends,
            waiting: running.waiting,
          };
    out["queue"] = resident.queue.length;
    out["conditions"] = conditions;
    out["attrs"] = { ...resident.attrs };
    out["cash"] = resident.cash;
    out["items"] = { ...resident.items };
    if (full) {
      out["zone_name"] = "Fake District";
      out["owner"] = resident.owner;
      out["model"] = resident.model ?? null;
      out["born"] = 0;
      out["queued"] = resident.queue.map((queued) => ({ verb: queued.verb, action_id: queued.action_id }));
      out["routines"] = resident.routines.map((routine, place) => {
        const stats = this.#stats(resident, place);
        return { ...routine, runs: stats.runs, failures: stats.failures };
      }) as unknown as Value;
      out["memberships"] = [];
    }
    return out;
  }

  #nearby(resident: FakeResident): Value {
    const radius = this.limits.sight_radius;
    const max = this.limits.nearby_max;
    const seen = [...this.residents.values()]
      .filter((other) => other.id !== resident.id && distance(other.pos, resident.pos) <= radius)
      .sort((a, b) => distance(a.pos, resident.pos) - distance(b.pos, resident.pos) || a.id - b.id);
    const sites = this.sites
      .map((site) => ({ site, pos: nearest(site.rect, resident.pos) }))
      .filter(({ pos }) => distance(pos, resident.pos) <= radius)
      .sort((a, b) => distance(a.pos, resident.pos) - distance(b.pos, resident.pos) || a.site.id - b.site.id);
    const out: Record<string, Value> = {
      residents: seen.slice(0, max).map((other) => {
        const entry: Record<string, Value> = { id: other.id, name: other.name, pos: { ...other.pos } };
        if (other.current !== undefined) entry["activity"] = other.current.verb;
        return entry;
      }),
    };
    if (seen.length > max) out["more_residents"] = seen.length - max;
    out["sites"] = sites
      .slice(0, max)
      .map(({ site, pos }) => ({ id: site.id, name: site.name, kind: site.kind, pos: { ...pos } }));
    if (sites.length > max) out["more_sites"] = sites.length - max;
    return out;
  }

  // ---- Long poll ------------------------------------------------------------

  /** Returns true when the inbox has an entry from `since` that the kinds let through. */
  hasNews(resident: FakeResident, since: number, kinds?: string[]): boolean {
    return resident.inbox.some((entry) => {
      if (entry.seq < since) return false;
      if (kinds === undefined || kinds.length === 0) return true;
      return kinds.includes("notice" in entry ? entry.notice.kind : REPORT_KIND);
    });
  }

  /**
   * The long poll. Resolves with an observation when the resident has news
   * from `since`, or without news when the timeout ends. Without `since`,
   * only news after the call counts. Rejects with `ResidentGoneError` when
   * the resident leaves the world while the call is open.
   */
  wait(
    resident: FakeResident,
    options: { timeout: number; since?: number; kinds?: string[] },
  ): Promise<Observation> {
    const since = options.since ?? resident.nextSeq;
    const answer = (): Observation => this.observe(resident, { since_notice: since });
    if (this.hasNews(resident, since, options.kinds)) return Promise.resolve(answer());
    // A wait of zero seconds answers at once, without news.
    if (options.timeout <= 0) return Promise.resolve(answer());
    if (this.time === "auto") {
      for (let i = 0; i < options.timeout && !this.hasNews(resident, since, options.kinds); i += 1) {
        this.step(1);
        // A rule of the test can end the life of the resident in a tick.
        if (!this.residents.has(resident.id)) return Promise.reject(new ResidentGoneError(resident.id));
      }
      return Promise.resolve(answer());
    }
    return new Promise((resolve, reject) => {
      this.#waiters.push({ resident, since, kinds: options.kinds, resolve, reject });
      for (const parked of this.#parked.splice(0)) parked();
    });
  }

  /** Resolves when a `wait` call is parked. A test awaits it before it makes news. */
  parked(): Promise<void> {
    if (this.#waiters.length > 0) return Promise.resolve();
    return new Promise((resolve) => this.#parked.push(resolve));
  }

  /** Ends every parked `wait` call, as the timeout does. */
  expireWaits(): void {
    for (const waiter of this.#waiters.splice(0)) {
      waiter.resolve(this.observe(waiter.resident, { since_notice: waiter.since }));
    }
  }

  /** Removes the parked `wait` calls of a resident that gave up. */
  abandonWaits(resident: FakeResident): void {
    this.#waiters = this.#waiters.filter((waiter) => waiter.resident !== resident);
  }

  #wake(resident: FakeResident): void {
    const woken = this.#waiters.filter(
      (waiter) => waiter.resident === resident && this.hasNews(resident, waiter.since, waiter.kinds),
    );
    this.#waiters = this.#waiters.filter((waiter) => !woken.includes(waiter));
    for (const waiter of woken) {
      waiter.resolve(this.observe(resident, { since_notice: waiter.since }));
    }
  }
}

function doneStatus(result: Value): ActionStatus {
  return result === null ? { state: "done" } : { state: "done", result };
}

/** Checks arguments against the top level of the schema of a verb. Returns them as an object. */
function checkArgs(info: VerbInfo, args: Value | undefined): Record<string, Value> {
  const given = args ?? {};
  if (typeof given !== "object" || Array.isArray(given)) {
    throw new FakeActionError("invalid_args", `The arguments of \`${info.verb}\` must be an object.`);
  }
  const schema = info.args_schema as {
    properties?: Record<string, { type?: string | string[] }>;
    required?: string[];
  } | null;
  for (const name of schema?.required ?? []) {
    if (!(name in given)) {
      throw new FakeActionError("invalid_args", `\`${info.verb}\` needs the argument \`${name}\`.`);
    }
  }
  const fits = (type: string, value: Value): boolean =>
    (type === "null" && value === null) ||
    (type === "integer" && Number.isInteger(value)) ||
    (type === "number" && typeof value === "number") ||
    (type === "string" && typeof value === "string") ||
    (type === "boolean" && typeof value === "boolean") ||
    (type === "array" && Array.isArray(value)) ||
    (type === "object" && typeof value === "object" && value !== null && !Array.isArray(value));
  for (const [name, value] of Object.entries(given)) {
    const type = schema?.properties?.[name]?.type;
    // A schema names one type, or a list of types such as `["integer", "null"]`.
    const types = type === undefined ? [] : Array.isArray(type) ? type : [type];
    if (types.length > 0 && !types.some((one) => fits(one, value))) {
      throw new FakeActionError("invalid_args", `The argument \`${name}\` of \`${info.verb}\` must be of type ${types.join(" or ")}.`);
    }
  }
  return given;
}

/** Distance in tiles. A diagonal step counts as one. */
export function distance(a: Pos, b: Pos): number {
  return Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y));
}

/**
 * Returns true when `hour` is in the range from `from` (inclusive) to `to`
 * (exclusive). The range wraps past midnight when `from` is greater than
 * `to`. It is empty when the two are equal.
 */
function hourBetween(hour: number, from: number, to: number): boolean {
  return from < to ? from <= hour && hour < to : from > to && (hour >= from || hour < to);
}

function contains(rect: Rect, pos: Pos): boolean {
  return pos.x >= rect.x0 && pos.x < rect.x1 && pos.y >= rect.y0 && pos.y < rect.y1;
}

function nearest(rect: Rect, pos: Pos): Pos {
  return {
    x: Math.min(Math.max(pos.x, rect.x0), rect.x1 - 1),
    y: Math.min(Math.max(pos.y, rect.y0), rect.y1 - 1),
  };
}
