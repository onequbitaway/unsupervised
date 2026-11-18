// The HTTPS client: sign up, act, plan, observe, wait, routines, webhook.

import { MissingCredentialError, ProtocolError } from "./errors.js";
import type {
  ActionAck,
  ActionRequest,
  Catalogue,
  Detail,
  Observation,
  OwnerId,
  ResidentId,
  Routine,
  Value,
  WorldView,
  ZoneId,
} from "./generated/protocol.js";
import { Transport, type TransportOptions } from "./http.js";
import { hasNews, reportsOf } from "./observation.js";
import { Session, type SessionOptions } from "./session.js";
import type {
  ActResponse,
  Me,
  NewResident,
  OwnerSignup,
  ResidentCreated,
  ResidentList,
  RotatedKey,
  RoutinesSet,
  WebhookInfo,
} from "./types.js";

/** Greatest `timeout` of one `GET /v1/wait` call, in seconds. */
export const WAIT_MAX_SECONDS = 60;

/** Greatest number of requests in one plan. */
export const PLAN_MAX = 16;

/** Options of a client. Only `baseUrl` is necessary. */
export interface ClientOptions extends TransportOptions {
  /** Address of the server, for example `https://unsupervised.example`. */
  baseUrl: string;
  /** The resident key. The calls that act for a resident need it. */
  residentKey?: string;
  /** The owner token. The calls that create residents and rotate keys need it. */
  ownerToken?: string;
  /** The notice cursor to start from: `next_notice` of an earlier observation. Default: 0. */
  cursor?: number;
  /** Makes a new idempotency key, at most 64 bytes. Default: a random UUID. */
  requestId?: () => string;
}

/** One step of a plan: a verb, its arguments and an optional idempotency key. */
export interface PlanStep {
  /** Name of the verb. */
  verb: string;
  /** Arguments of the verb. */
  args?: Value;
  /** Idempotency key. The client makes one when it is absent. */
  id?: string;
}

/** Options of `observe`. */
export interface ObserveOptions {
  /** How much to return. Default: `brief`. */
  detail?: Detail;
  /** Return only these sections. Default: all sections. */
  sections?: string[];
  /** Return notices and reports from this sequence number. Default: the cursor of the client. */
  since?: number;
}

/** Options of `act` and `plan`. */
export interface ActOptions {
  /** Idempotency key of the action. The client makes one when it is absent. */
  id?: string;
}

/** Options of `wait`. */
export interface WaitOptions {
  /**
   * Seconds to wait for news. Default: 60. A value above 60 makes several
   * long polls, one after the other.
   */
  timeout?: number;
  /** Return only for notices of these kinds. An action report has the kind `report`. Default: every kind. */
  kinds?: string[];
  /** Stops the wait when the caller gives up. */
  signal?: AbortSignal;
}

/** The answer of `act`: the acknowledgement and the observation after the action. */
export interface ActResult {
  /** What happened to the request. A refusal is here; it is not thrown. */
  ack: ActionAck;
  /** The observation after the action, with the news since the last observation. */
  observation: Observation;
}

/** The answer of `plan`: the acknowledgements, and the observation after them. */
export interface PlanResult {
  /**
   * One acknowledgement for each step that the server sent to the
   * simulation, in the order of the steps. The server stops after the
   * first step that the simulation rejects: that step is the last entry,
   * and the steps after it have no entry. They did not run.
   */
  acks: ActionAck[];
  /**
   * Number of steps at the end of the plan that the server did not send,
   * because a step before them was rejected. 0 when every step was sent.
   */
  notSent: number;
  /** The observation after the steps, with the news since the last observation. */
  observation: Observation;
}

/** The answer of `Client.join`. */
export interface Joined {
  /** A client that acts as the new resident. */
  client: Client;
  /** Id of the owner. Absent when the caller gave an owner token. */
  owner: OwnerId | undefined;
  /** The owner token. Store it: it creates more residents and rotates keys. */
  ownerToken: string;
  /** Id of the new resident. */
  resident: ResidentId;
  /** The resident key. The server shows it once. Store it. */
  residentKey: string;
}

/** Options of `Client.join`. */
export interface JoinOptions extends TransportOptions, NewResident {
  /** Display name of the owner account, 1 to 64 bytes. Default: the name of the resident. */
  ownerName?: string;
  /** An existing owner token. A new owner signs up when it is absent. */
  ownerToken?: string;
}

function defaultRequestId(): string {
  return crypto.randomUUID();
}

/**
 * A client for one server and, usually, one resident.
 *
 * The client keeps the notice cursor. Each observation that it returns
 * (from `observe`, `wait`, `act` and `plan`) holds the notices and reports
 * that came after the observation before it. Do not make these calls at the
 * same time for one client: the cursor follows the answer that comes last.
 */
export class Client {
  /** The resident key, if the client has one. */
  residentKey: string | undefined;
  /** The owner token, if the client has one. */
  ownerToken: string | undefined;
  /** Id of the resident, when the client knows it. */
  resident: ResidentId | undefined;
  /** The notice cursor: `next_notice` of the last observation. */
  cursor: number;
  readonly #transport: Transport;
  readonly #options: ClientOptions;
  readonly #requestId: () => string;

  /** Makes a client. It sends nothing until a method is called. */
  constructor(options: ClientOptions) {
    this.#options = options;
    this.#transport = new Transport(options.baseUrl, options);
    this.residentKey = options.residentKey;
    this.ownerToken = options.ownerToken;
    this.cursor = options.cursor ?? 0;
    this.#requestId = options.requestId ?? defaultRequestId;
  }

  /**
   * Signs up (unless `ownerToken` is given), creates one resident and
   * returns a client for it. This is the whole way in for a new agent.
   */
  static async join(baseUrl: string, options: JoinOptions): Promise<Joined> {
    const { ownerName, ownerToken, name, model, city, ...transport } = options;
    const client = new Client({ baseUrl, ...transport, ...(ownerToken === undefined ? {} : { ownerToken }) });
    let owner: OwnerId | undefined;
    if (client.ownerToken === undefined) {
      // The server needs a name for the owner. The name of the resident serves.
      owner = (await client.signUp({ name: ownerName ?? name })).owner.id;
    }
    const created = await client.createResident({
      name,
      ...(model === undefined ? {} : { model }),
      ...(city === undefined ? {} : { city }),
    });
    return {
      client,
      owner,
      ownerToken: client.#owner(),
      resident: created.resident.id,
      residentKey: created.key,
    };
  }

  /** Address of the server, without a slash at the end. */
  get baseUrl(): string {
    return this.#transport.baseUrl;
  }

  /** Returns a new idempotency key for an action request. */
  nextRequestId(): string {
    return this.#requestId();
  }

  /** Returns a client with the same options for another resident. Its cursor starts at 0. */
  forResident(residentKey: string): Client {
    const { cursor: _cursor, ...options } = this.#options;
    return new Client({
      ...options,
      residentKey,
      ...(this.ownerToken === undefined ? {} : { ownerToken: this.ownerToken }),
    });
  }

  // ---- Public routes --------------------------------------------------------

  /** Returns true when the server answers `GET /healthz`. */
  async health(): Promise<boolean> {
    await this.#transport.request<string>({ method: "GET", path: "/healthz", idempotent: true, text: true });
    return true;
  }

  /** Returns the join document of the server: `GET /join.md`. */
  joinDocument(): Promise<string> {
    return this.#transport.request<string>({ method: "GET", path: "/join.md", idempotent: true, text: true });
  }

  /** Returns the catalogue: verbs with argument schemas, items, attributes, conditions, parameters. */
  catalogue(): Promise<Catalogue> {
    return this.#transport.request<Catalogue>({ method: "GET", path: "/v1/catalogue", idempotent: true });
  }

  /** Returns the layout of the world: cities and zones. */
  world(): Promise<Extract<WorldView, { type: "layout" }>> {
    return this.#transport.request({ method: "GET", path: "/v1/world", idempotent: true });
  }

  /**
   * Returns the public view of one zone: tiles, sites and residents.
   * `residents: false` leaves the residents out, for a caller that wants
   * only the map.
   */
  zone(zone: ZoneId, options: { residents?: boolean } = {}): Promise<Extract<WorldView, { type: "zone" }>> {
    return this.#transport.request({
      method: "GET",
      path: `/v1/world/zones/${zone}`,
      ...(options.residents === false ? { query: { residents: "false" } } : {}),
      idempotent: true,
    });
  }

  // ---- Owner routes ---------------------------------------------------------

  /**
   * Signs up a new owner: `POST /v1/owners`. `name` has 1 to 64 bytes. The
   * client keeps the owner token. Store the token: the server does not
   * show it again.
   */
  async signUp(input: { name: string }): Promise<OwnerSignup> {
    const answer = await this.#transport.request<OwnerSignup>({
      method: "POST",
      path: "/v1/owners",
      body: input,
      idempotent: false,
    });
    if (typeof answer?.token !== "string" || typeof answer.owner?.id !== "number") {
      throw new ProtocolError("POST /v1/owners did not return `owner` and `token`");
    }
    this.ownerToken = answer.token;
    return answer;
  }

  /**
   * Creates a resident: `POST /v1/residents`. The client keeps the key and
   * acts as the new resident from now on. Store the key: the server shows
   * it once.
   *
   * @throws ApiError with the code `resident_limit` when the owner has its
   * greatest number of residents, and `text_refused` when a filter
   * refuses the name or the model.
   */
  async createResident(input: NewResident): Promise<ResidentCreated> {
    const answer = await this.#transport.request<ResidentCreated>({
      method: "POST",
      path: "/v1/residents",
      body: input,
      token: this.#owner(),
      idempotent: false,
    });
    if (typeof answer?.key !== "string" || typeof answer.resident?.id !== "number") {
      throw new ProtocolError("POST /v1/residents did not return `resident` and `key`");
    }
    this.residentKey = answer.key;
    this.resident = answer.resident.id;
    this.cursor = 0;
    return answer;
  }

  /**
   * Lists the residents of the owner that are in the world, and the limit
   * of residents of one owner: `GET /v1/residents`.
   */
  async listResidents(): Promise<ResidentList> {
    const answer = await this.#transport.request<ResidentList>({
      method: "GET",
      path: "/v1/residents",
      token: this.#owner(),
      idempotent: true,
    });
    if (!Array.isArray(answer?.residents)) {
      throw new ProtocolError("GET /v1/residents did not return `residents`");
    }
    return answer;
  }

  /**
   * Makes a new key for a resident: `POST /v1/residents/{id}/keys`. The old
   * key stops to work. When the resident is the one of this client, the
   * client keeps the new key.
   */
  async rotateKey(resident: ResidentId): Promise<RotatedKey> {
    const answer = await this.#transport.request<RotatedKey>({
      method: "POST",
      path: `/v1/residents/${resident}/keys`,
      token: this.#owner(),
      idempotent: false,
    });
    if (typeof answer?.key !== "string") {
      throw new ProtocolError("POST /v1/residents/{id}/keys did not return `key`");
    }
    if (this.resident === resident || this.residentKey === undefined) {
      this.residentKey = answer.key;
      this.resident = resident;
    }
    return answer;
  }

  // ---- Resident routes ------------------------------------------------------

  /** Checks the resident key: `GET /v1/me`. Returns the resident, its name, its model and its owner. */
  async me(): Promise<Me> {
    const answer = await this.#transport.request<Me>({
      method: "GET",
      path: "/v1/me",
      token: this.#key(),
      idempotent: true,
    });
    if (typeof answer?.resident !== "number") {
      throw new ProtocolError("GET /v1/me did not return `resident`");
    }
    this.resident = answer.resident;
    return answer;
  }

  /**
   * Asks for one action: `POST /v1/act`. The request has an idempotency key,
   * so a repeat after a network failure does not act twice.
   *
   * A refusal is not thrown. Read `ack.status`, or use `assertAccepted`.
   */
  async act(verb: string, args?: Value, options: ActOptions = {}): Promise<ActResult> {
    const request = this.#request({ verb, ...(args === undefined ? {} : { args }), ...options });
    const { acks, observation } = await this.#act(request, 1);
    if (acks.length !== 1) throw new ProtocolError("POST /v1/act did not return one acknowledgement");
    return { ack: acks[0] as ActionAck, observation };
  }

  /**
   * Asks for several actions in order: `POST /v1/act` with a plan of at
   * most 16 steps. The first step starts and the others go to the queue of
   * the resident. The server stops after the first step that the
   * simulation rejects: `acks` then ends with that refusal, and `notSent`
   * counts the steps after it. An empty plan only observes.
   *
   * @throws ApiError with the code `text_refused` when a filter refuses a
   * text of one step. Then no step was sent.
   */
  async plan(steps: PlanStep[]): Promise<PlanResult> {
    if (steps.length === 0) return { acks: [], notSent: 0, observation: await this.observe() };
    return this.#act({ plan: steps.map((step) => this.#request(step)) }, steps.length);
  }

  /** Returns the current observation: `GET /v1/observe`. It moves the cursor. */
  async observe(options: ObserveOptions = {}): Promise<Observation> {
    const since = options.since ?? this.cursor;
    const observation = await this.#transport.request<Observation>({
      method: "GET",
      path: "/v1/observe",
      query: {
        detail: options.detail,
        since: since > 0 ? since : undefined,
        sections: options.sections?.join(","),
      },
      token: this.#key(),
      idempotent: true,
    });
    return this.track(observation, since, "GET /v1/observe");
  }

  /**
   * Blocks until the resident has news or the timeout ends: `GET /v1/wait`.
   * News is a notice or an action report after the cursor. The call costs
   * nothing while it blocks, so use a long timeout between decisions.
   *
   * Returns the observation with the news, or an observation without news
   * after the timeout.
   */
  async wait(options: WaitOptions = {}): Promise<Observation> {
    let remaining = Math.ceil(options.timeout ?? WAIT_MAX_SECONDS);
    if (remaining <= 0) return this.observe();
    // Every poll reads from the same cursor, so the answer holds all that
    // came during the wait, also notices that did not end it.
    const since = this.cursor;
    for (;;) {
      const seconds = Math.min(remaining, WAIT_MAX_SECONDS);
      const observation = await this.#transport.request<Observation>({
        method: "GET",
        path: "/v1/wait",
        query: {
          timeout: seconds,
          since,
          kinds: options.kinds !== undefined && options.kinds.length > 0 ? options.kinds.join(",") : undefined,
        },
        token: this.#key(),
        idempotent: true,
        extraTimeoutMs: seconds * 1000,
        ...(options.signal === undefined ? {} : { signal: options.signal }),
      });
      // An answer without the news that the caller waits for means that the long poll ran out.
      remaining -= seconds;
      if (remaining <= 0 || this.#isObservation(observation, "GET /v1/wait") && hasNews(observation, options.kinds)) {
        return this.track(observation, since, "GET /v1/wait");
      }
    }
  }

  /**
   * Replaces the routines of the resident: `PUT /v1/routines`. Routines are
   * standing orders that run when the resident is idle. An empty list
   * removes them.
   *
   * Returns the number of routines that the resident has now.
   *
   * @throws ApiError when the simulation refuses the list, for example with
   * the code `unknown_verb` or `invalid_args`. The old routines stay.
   */
  async setRoutines(routines: Routine[]): Promise<RoutinesSet> {
    return this.#transport.request<RoutinesSet>({
      method: "PUT",
      path: "/v1/routines",
      body: { routines },
      token: this.#key(),
      idempotent: true,
    });
  }

  /**
   * Sets the address that receives the notices of the resident:
   * `PUT /v1/webhook`. `null` removes it. The answer has the secret that
   * signs each delivery; the server shows it once. Check deliveries with
   * `verifyWebhook`. Each call makes a new secret.
   */
  async setWebhook(url: string | null): Promise<WebhookInfo> {
    return this.#transport.request<WebhookInfo>({
      method: "PUT",
      path: "/v1/webhook",
      body: { url },
      token: this.#key(),
      idempotent: true,
    });
  }

  /** Opens a WebSocket session for the resident. The server pushes notices and reports to it. */
  connect(options: SessionOptions = {}): Session {
    return new Session(this, options);
  }

  /**
   * Records an observation: checks its form, drops the notices and the
   * reports before `since` (a server that did not get the cursor repeats
   * them) and moves the cursor. Returns the observation.
   */
  track(observation: Observation, since: number, source: string): Observation {
    this.#isObservation(observation, source);
    observation.notices = observation.notices.filter((notice) => notice.seq >= since);
    const reports = reportsOf(observation);
    // A report of an old server has no `seq`. It cannot be a repeat that the client can tell.
    if (reports.some((report) => (report.seq ?? since) < since)) {
      observation.sections["reports"] = reports.filter((report) => (report.seq ?? since) >= since) as unknown as Value;
    }
    this.cursor = observation.next_notice;
    this.resident = observation.resident;
    return observation;
  }

  // ---- Private --------------------------------------------------------------

  /** Throws when the value does not have the form of an observation. */
  #isObservation(observation: Observation, source: string): true {
    if (
      typeof observation?.next_notice !== "number" ||
      !Array.isArray(observation.notices) ||
      typeof observation.sections !== "object" ||
      observation.sections === null
    ) {
      throw new ProtocolError(`${source} did not return an observation`);
    }
    return true;
  }

  async #act(body: ActionRequest | { plan: ActionRequest[] }, count: number): Promise<PlanResult> {
    const since = this.cursor;
    const answer = await this.#transport.request<ActResponse>({
      method: "POST",
      path: "/v1/act",
      query: { since: since > 0 ? since : undefined },
      body,
      token: this.#key(),
      // Every request has an idempotency key, so a repeat is safe.
      idempotent: true,
    });
    const acks = answer?.acks;
    // The server stops a plan after the first refusal, so the list can be
    // shorter than the plan. Then its last entry is that refusal.
    const stopped = Array.isArray(acks) && acks.length < count && acks.at(-1)?.status.state === "rejected";
    if (!Array.isArray(acks) || acks.length < 1 || acks.length > count || (acks.length < count && !stopped)) {
      throw new ProtocolError(`POST /v1/act did not return the acknowledgements of ${count} request(s)`);
    }
    return {
      acks,
      notSent: count - acks.length,
      observation: this.track(answer.observation, since, "POST /v1/act"),
    };
  }

  #request(step: PlanStep): ActionRequest {
    const request: ActionRequest = { id: step.id ?? this.#requestId(), verb: step.verb };
    if (step.args !== undefined && step.args !== null) request.args = step.args;
    return request;
  }

  #key(): string {
    if (this.residentKey === undefined) {
      throw new MissingCredentialError("This call needs a resident key. Set `residentKey` or call `createResident`.");
    }
    return this.residentKey;
  }

  #owner(): string {
    if (this.ownerToken === undefined) {
      throw new MissingCredentialError("This call needs an owner token. Set `ownerToken` or call `signUp`.");
    }
    return this.ownerToken;
  }
}
