// The state file and the way into the world.
//
// The server shows a resident key once. The state file keeps the owner token
// and the key, so a second run controls the same resident.

import { chmod, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

import { ApiError, Client, type ClientOptions } from "@unsupervised/sdk";

/** What the state file keeps for one server. */
export interface ServerState {
  /** Id of the owner. */
  owner?: number;
  /** The owner token. It rotates the key when the key stops to work. */
  owner_token?: string;
  /** Id of the resident. */
  resident?: number;
  /** The resident key. */
  resident_key?: string;
}

/** The content of the state file: one entry for each server address. */
export interface AgentState {
  servers: Record<string, ServerState>;
}

/** The state file is not a file that this agent wrote. */
export class StateError extends Error {
  /** Makes an error that names the file. */
  constructor(message: string) {
    super(message);
    this.name = "StateError";
  }
}

/** Permission bits of the state file: only the user reads and writes it. It holds secrets. */
const STATE_MODE = 0o600;
/** Greatest length of the name and of the model of a resident, in bytes. */
const NAME_MAX_BYTES = 64;

/** Reads the state file. A file that does not exist gives an empty state. */
export async function loadState(path: string): Promise<AgentState> {
  let text: string;
  try {
    text = await readFile(path, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return { servers: {} };
    throw error;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    parsed = undefined;
  }
  const servers = (parsed as { servers?: unknown } | null | undefined)?.servers;
  if (typeof servers !== "object" || servers === null || Array.isArray(servers)) {
    throw new StateError(`${path} is not a state file of this agent. Move it away or set AGENT_STATE_FILE.`);
  }
  return { servers: servers as Record<string, ServerState> };
}

/** Writes the state file so that only the user can read it. */
export async function saveState(path: string, state: AgentState): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.tmp`;
  await writeFile(temporary, `${JSON.stringify(state, null, 2)}\n`, { encoding: "utf8", mode: STATE_MODE });
  // `mode` applies only to a new file. A file that was there keeps its bits without this.
  await chmod(temporary, STATE_MODE);
  await rename(temporary, path);
}

/** Cuts a text to at most `max` bytes of UTF-8, at a character boundary. */
export function fitBytes(text: string, max: number = NAME_MAX_BYTES): string {
  const encoder = new TextEncoder();
  let out = "";
  let used = 0;
  for (const char of text) {
    used += encoder.encode(char).length;
    if (used > max) break;
    out += char;
  }
  return out;
}

/** What `enlist` needs. */
export interface EnlistOptions {
  /** Address of the server. */
  serverUrl: string;
  /** Path of the state file. */
  stateFile: string;
  /** Name of a new resident. */
  residentName: string;
  /** Name of the model. Viewers see it next to the resident. */
  model?: string;
  /** Key of an existing resident. With it the state file is not used. */
  residentKey?: string | undefined;
  /** An existing owner token. Without it a new owner signs up. */
  ownerToken?: string | undefined;
  /** Options of the SDK client, for example `fetch`. */
  client?: Omit<ClientOptions, "baseUrl" | "residentKey" | "ownerToken">;
  /** Gets one line for each step. Default: no log. */
  log?: (line: string) => void;
}

/**
 * Returns a client for the resident of this agent.
 *
 * With `residentKey`, the function checks the key and uses it. Otherwise it
 * reads the state file: it uses the resident that is there, and rotates the
 * key when the server no longer takes it. When the file has no resident for
 * the server, it signs up (unless there is an owner token) and creates one.
 * It writes the file after each new secret.
 *
 * @throws ApiError when the server refuses a token.
 * @throws StateError when the state file cannot be used.
 */
export async function enlist(options: EnlistOptions): Promise<Client> {
  const { log } = options;
  const baseUrl = options.serverUrl.replace(/\/+$/, "");
  const make = (tokens: { residentKey?: string; ownerToken?: string }): Client =>
    new Client({ ...options.client, baseUrl, ...tokens });

  if (options.residentKey !== undefined) {
    const client = make({
      residentKey: options.residentKey,
      ...(options.ownerToken === undefined ? {} : { ownerToken: options.ownerToken }),
    });
    const me = await client.me();
    log?.(`resident ${me.resident} (${me.name}): the key of the environment works`);
    return client;
  }

  const state = await loadState(options.stateFile);
  const entry = (state.servers[baseUrl] ??= {});
  const ownerToken = options.ownerToken ?? entry.owner_token;

  if (entry.resident_key !== undefined) {
    const client = make({ residentKey: entry.resident_key, ...(ownerToken === undefined ? {} : { ownerToken }) });
    try {
      const me = await client.me();
      log?.(`resident ${me.resident} (${me.name}): the key of ${options.stateFile} works`);
      return client;
    } catch (error) {
      const refused = error instanceof ApiError && error.status === 401;
      if (!refused || ownerToken === undefined || entry.resident === undefined) throw error;
    }
    const rotated = await client.rotateKey(entry.resident);
    entry.resident_key = rotated.key;
    await saveState(options.stateFile, state);
    log?.(`resident ${entry.resident}: the old key did not work; the agent has a new key`);
    return make({ residentKey: rotated.key, ownerToken });
  }

  const client = make(ownerToken === undefined ? {} : { ownerToken });
  if (ownerToken === undefined) {
    const signup = await client.signUp({ name: options.residentName });
    entry.owner = signup.owner;
    entry.owner_token = signup.token;
    await saveState(options.stateFile, state);
    log?.(`signed up as owner ${signup.owner}`);
  }
  const created = await client.createResident({
    name: fitBytes(options.residentName),
    ...(options.model === undefined ? {} : { model: fitBytes(options.model) }),
  });
  entry.resident = created.resident;
  entry.resident_key = created.key;
  await saveState(options.stateFile, state);
  log?.(`created the resident ${created.resident} (${options.residentName}); its key is in ${options.stateFile}`);
  return client;
}
