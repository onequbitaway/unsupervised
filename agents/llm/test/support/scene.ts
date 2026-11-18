// Shared set-up of the agent tests: a fake world, a fake model and a client
// that needs no network.

import { Client } from "@unsupervised/sdk";
import {
  FakeGateway,
  FakeWorld,
  type FakeResident,
  type FakeWorldOptions,
  type LoggedRequest,
} from "@unsupervised/sdk/testing";

import {
  DATA_CLOSE,
  DATA_OPEN,
  LlmAgent,
  type AgentOptions,
  type ChatMessage,
  type ChatProvider,
  type NotesStore,
} from "../../src/index.js";

/** Address of the fake server. No request leaves the process. */
export const FAKE_URL = "http://fake";

/** The fixed text that follows the data block in the user message. */
export const AFTER_DATA = "\nAnswer with one JSON object.";

/** Returns the answer of a model: the JSON object that the prompt asks for. */
export function answer(fields: {
  thought?: string;
  actions?: { verb: string; args?: Record<string, unknown> }[];
  wait_seconds?: number;
  note?: string;
  describe?: string[];
}): string {
  return JSON.stringify({ thought: "test", actions: [], ...fields });
}

/** A model that gives prepared answers and records what it was asked. */
export class FakeProvider implements ChatProvider {
  /** The messages of each request, in order. */
  readonly calls: ChatMessage[][] = [];
  /** Called at the start of each request, with its number from 1. */
  onCall: ((call: number) => void) | undefined;
  readonly #answers: (string | Error)[];

  /** Makes a model that gives `answers` in order. The last one repeats. */
  constructor(...answers: (string | Error)[]) {
    this.#answers = answers.length > 0 ? answers : [answer({ wait_seconds: 600 })];
  }

  async complete(messages: ChatMessage[]): Promise<string> {
    this.calls.push(messages);
    this.onCall?.(this.calls.length);
    const next = this.#answers.length > 1 ? (this.#answers.shift() as string | Error) : (this.#answers[0] as string | Error);
    if (next instanceof Error) throw next;
    return next;
  }

  /** Returns the data document of request `call` (from 1). */
  data(call: number = this.calls.length): Record<string, any> {
    return dataOf(this.calls[call - 1] as ChatMessage[]);
  }
}

/** Notes in memory. */
export class MemoryNotes implements NotesStore {
  readonly notes: string[] = [];

  async read(): Promise<string[]> {
    return [...this.notes];
  }

  async add(text: string, stamp: string): Promise<void> {
    this.notes.push(`[${stamp}] ${text}`);
  }
}

/**
 * Returns the JSON document between the data tags of the user message.
 * Throws when the message does not have exactly the form
 * `<data>\n...\n</data>` and the fixed last line.
 */
export function dataOf(messages: ChatMessage[]): Record<string, any> {
  const user = messages[1]?.content ?? "";
  const head = `${DATA_OPEN}\n`;
  const tail = `\n${DATA_CLOSE}${AFTER_DATA}`;
  if (!user.startsWith(head) || !user.endsWith(tail)) throw new Error(`the user message has another form: ${user}`);
  return JSON.parse(user.slice(head.length, user.length - tail.length)) as Record<string, any>;
}

/** A fake world with one resident, a client for it and an agent with a fake model. */
export interface Scene {
  world: FakeWorld;
  gateway: FakeGateway;
  resident: FakeResident;
  client: Client;
  provider: FakeProvider;
  notes: MemoryNotes;
  agent: LlmAgent;
  /** Delays that the agent asked for after an error, in milliseconds. */
  sleeps: number[];
  /** The lines that the agent logged. */
  logs: string[];
  /** Returns `METHOD path` of every request since the last call of `forget`. */
  paths(): string[];
  /** Returns the requests to one path since the last call of `forget`. */
  requestsTo(path: string): LoggedRequest[];
  /** Returns the verbs of every action that the agent asked for. */
  sentVerbs(): string[];
  /** Forgets the requests so far. */
  forget(): void;
}

/**
 * Makes a scene. Time is automatic by default: a long poll of N seconds
 * steps the world N ticks, or fewer when news ends it. The clock of the
 * agent is the tick of the world, so one tick is one second.
 */
export function scene(
  options: {
    world?: FakeWorldOptions;
    answers?: (string | Error)[];
    agent?: Partial<AgentOptions>;
  } = {},
): Scene {
  const world = new FakeWorld({ time: "auto", ...options.world });
  const gateway = new FakeGateway(world);
  const resident = world.spawn(world.signUp().owner.id, { name: "Ada" });
  // The client does not sleep between attempts.
  const client = new Client({
    baseUrl: FAKE_URL,
    fetch: gateway.fetch,
    residentKey: resident.key,
    sleep: async () => undefined,
    maxRetries: 1,
  });
  const provider = new FakeProvider(...(options.answers ?? []));
  const notes = new MemoryNotes();
  const sleeps: number[] = [];
  const logs: string[] = [];
  const agent = new LlmAgent({
    world: client,
    provider,
    notes,
    log: (line) => {
      logs.push(line);
    },
    now: () => world.tick * 1000,
    sleep: async (milliseconds) => {
      sleeps.push(milliseconds);
    },
    ...options.agent,
  });
  return {
    world,
    gateway,
    resident,
    client,
    provider,
    notes,
    agent,
    sleeps,
    logs,
    paths: () => gateway.requests.map((request) => `${request.method} ${request.path}`),
    requestsTo: (path) => gateway.requests.filter((request) => request.path === path),
    sentVerbs: () =>
      gateway.requests
        .filter((request) => request.path === "/v1/act")
        .flatMap((request) => {
          const body = request.body as { plan?: { verb: string }[]; verb?: string };
          return body.plan === undefined ? [body.verb as string] : body.plan.map((step) => step.verb);
        }),
    forget: () => {
      gateway.requests.length = 0;
    },
  };
}
