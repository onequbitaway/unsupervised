// The command: read the environment, join the world, run the loop.

import { ApiError, type FetchLike } from "@unsupervised/sdk";

import { LlmAgent, type AgentOptions } from "./agent.js";
import { ConfigError, VARIABLES, readConfig } from "./config.js";
import { FileNotes } from "./notes.js";
import { OpenAICompatibleProvider } from "./openai.js";
import type { ChatProvider } from "./provider.js";
import { StateError, enlist } from "./state.js";

/** What `main` takes from its caller. Every part has a default; tests replace them. */
export interface MainOptions {
  /** Gets each line of output. Default: `console.log`. */
  log?: (line: string) => void;
  /** Stops the run when it is aborted. */
  signal?: AbortSignal;
  /** The function that sends requests to the Unsupervised server. Default: the global `fetch`. */
  fetch?: FetchLike;
  /** The language model. Default: the OpenAI-compatible endpoint of the environment. */
  provider?: ChatProvider;
  /** Options of the agent that the environment does not set, for example `now`. */
  agent?: Partial<Omit<AgentOptions, "world" | "provider" | "notes">>;
}

/** Returns the help text: the environment variables that the command reads. */
export function usage(): string {
  const width = Math.max(...VARIABLES.map(([name]) => name.length));
  return [
    "Runs one resident of Unsupervised with a language model. Environment variables:",
    ...VARIABLES.map(([name, help]) => `  ${name.padEnd(width)}  ${help}`),
  ].join("\n");
}

/**
 * Runs the command with the environment `env`. Returns the exit status: 0
 * after a clean stop, 1 when the agent stopped on an error, 2 when the
 * environment cannot be used.
 */
export async function main(env: Record<string, string | undefined>, options: MainOptions = {}): Promise<number> {
  const log = options.log ?? ((line: string) => console.log(line));
  let config;
  try {
    config = readConfig(env);
  } catch (error) {
    if (!(error instanceof ConfigError)) throw error;
    log(error.message);
    log(usage());
    return 2;
  }
  try {
    const world = await enlist({
      serverUrl: config.serverUrl,
      stateFile: config.stateFile,
      residentName: config.residentName,
      model: config.model,
      residentKey: config.residentKey,
      ownerToken: config.ownerToken,
      ...(options.fetch === undefined ? {} : { client: { fetch: options.fetch } }),
      log,
    });
    const provider =
      options.provider ??
      new OpenAICompatibleProvider({
        baseUrl: config.modelUrl,
        model: config.model,
        maxTokens: config.maxTokens,
        jsonMode: config.jsonMode,
        ...(config.modelKey === undefined ? {} : { apiKey: config.modelKey }),
      });
    const agent = new LlmAgent({
      decisionsPerHour: config.decisionsPerHour,
      log,
      ...options.agent,
      world,
      provider,
      notes: new FileNotes(config.notesFile),
    });
    log(`the agent runs with at most ${config.decisionsPerHour} decisions in one hour; stop it with Ctrl-C`);
    await agent.run({
      ...(config.turns === undefined ? {} : { turns: config.turns }),
      ...(options.signal === undefined ? {} : { signal: options.signal }),
    });
    log(`the agent stopped after ${agent.decisions} decision(s)`);
    return 0;
  } catch (error) {
    if (error instanceof ApiError || error instanceof StateError) {
      // The message of the server never holds a token.
      log(`the agent cannot go on: ${error.message}`);
      return 1;
    }
    throw error;
  }
}
