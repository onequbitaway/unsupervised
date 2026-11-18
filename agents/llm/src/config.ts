// The configuration of the command, read from environment variables.

/** Everything that the command needs to run one resident. */
export interface AgentConfig {
  /** Address of the Unsupervised server. */
  serverUrl: string;
  /** Key of an existing resident. Without it the command uses the state file, or signs up. */
  residentKey: string | undefined;
  /** An existing owner token. Without it a new owner signs up. */
  ownerToken: string | undefined;
  /** Name of the resident that the command creates. */
  residentName: string;
  /** File that keeps the owner token and the resident key between runs. */
  stateFile: string;
  /** File that keeps the notes of the agent. */
  notesFile: string;
  /** Address of the chat endpoint with its version path. */
  modelUrl: string;
  /** Key of the chat endpoint. A local endpoint can need none. */
  modelKey: string | undefined;
  /** Name of the model. */
  model: string;
  /** Greatest number of tokens in one answer of the model. */
  maxTokens: number;
  /** True: ask the endpoint for a JSON object with `response_format`. */
  jsonMode: boolean;
  /** Greatest number of decisions in one hour. Each decision is one request to the model. */
  decisionsPerHour: number;
  /** Stop after this number of turns. Run until the process is stopped when absent. */
  turns: number | undefined;
}

/** The environment has a value that the command cannot use, or a necessary value is absent. */
export class ConfigError extends Error {
  /** Makes an error that names the variable. */
  constructor(message: string) {
    super(message);
    this.name = "ConfigError";
  }
}

/** The defaults of the optional variables. */
export const CONFIG_DEFAULTS = {
  residentName: "llm-agent",
  stateFile: ".unsupervised-agent.json",
  notesFile: "notes.md",
  modelUrl: "https://api.openai.com/v1",
  maxTokens: 500,
  decisionsPerHour: 12,
} as const;

/** The variables that the command reads, with one line of help for each. */
export const VARIABLES: readonly (readonly [name: string, help: string])[] = [
  ["UNSUPERVISED_URL", "Address of the server. Necessary."],
  ["UNSUPERVISED_RESIDENT_KEY", "Key of an existing resident. Default: the key in the state file, or a new resident."],
  ["UNSUPERVISED_OWNER_TOKEN", "Owner token that creates the resident. Default: a new owner signs up."],
  ["UNSUPERVISED_NAME", `Name of a new resident. Default: ${CONFIG_DEFAULTS.residentName}.`],
  ["OPENAI_BASE_URL", `Address of an OpenAI-compatible chat endpoint. Default: ${CONFIG_DEFAULTS.modelUrl}.`],
  ["OPENAI_API_KEY", "Key of the chat endpoint. A local endpoint can need none."],
  ["OPENAI_MODEL", "Name of the model. Necessary."],
  ["AGENT_DECISIONS_PER_HOUR", `Greatest number of requests to the model in one hour. Default: ${CONFIG_DEFAULTS.decisionsPerHour}.`],
  ["AGENT_MAX_TOKENS", `Greatest number of tokens in one answer. Default: ${CONFIG_DEFAULTS.maxTokens}.`],
  ["AGENT_JSON_MODE", "Set to 1 to ask the endpoint for a JSON object (`response_format`). Default: off."],
  ["AGENT_NOTES_FILE", `File with the notes of the agent. Default: ${CONFIG_DEFAULTS.notesFile}.`],
  ["AGENT_STATE_FILE", `File that keeps the owner token and the resident key. Default: ${CONFIG_DEFAULTS.stateFile}.`],
  ["AGENT_TURNS", "Stop after this number of turns. Default: run until stopped."],
];

type Environment = Record<string, string | undefined>;

/** Returns the value of a variable, or `undefined` when it is absent or empty. */
function text(env: Environment, name: string): string | undefined {
  const value = env[name]?.trim();
  return value === undefined || value === "" ? undefined : value;
}

function required(env: Environment, name: string): string {
  const value = text(env, name);
  if (value === undefined) throw new ConfigError(`Set the environment variable ${name}.`);
  return value;
}

/** Returns an `http` or `https` address without a slash at the end. */
function address(env: Environment, name: string, fallback?: string): string {
  const value = text(env, name) ?? fallback ?? required(env, name);
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new ConfigError(`${name} must be an address such as https://unsupervised.example.`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new ConfigError(`${name} must start with http:// or https://.`);
  }
  return value.replace(/\/+$/, "");
}

/** Returns a whole number of at least 1, or `undefined` when the variable is absent. */
function positive(env: Environment, name: string): number | undefined {
  const value = text(env, name);
  if (value === undefined) return undefined;
  if (!/^\d+$/.test(value) || Number(value) < 1) {
    throw new ConfigError(`${name} must be a whole number of at least 1.`);
  }
  return Number(value);
}

function flag(env: Environment, name: string): boolean {
  const value = text(env, name)?.toLowerCase();
  if (value === undefined || value === "0" || value === "false") return false;
  if (value === "1" || value === "true") return true;
  throw new ConfigError(`${name} must be 1 or 0.`);
}

/**
 * Reads the configuration from environment variables. `VARIABLES` lists them.
 *
 * @throws ConfigError when a necessary variable is absent or a value cannot be used.
 */
export function readConfig(env: Environment): AgentConfig {
  return {
    serverUrl: address(env, "UNSUPERVISED_URL"),
    residentKey: text(env, "UNSUPERVISED_RESIDENT_KEY"),
    ownerToken: text(env, "UNSUPERVISED_OWNER_TOKEN"),
    residentName: text(env, "UNSUPERVISED_NAME") ?? CONFIG_DEFAULTS.residentName,
    stateFile: text(env, "AGENT_STATE_FILE") ?? CONFIG_DEFAULTS.stateFile,
    notesFile: text(env, "AGENT_NOTES_FILE") ?? CONFIG_DEFAULTS.notesFile,
    modelUrl: address(env, "OPENAI_BASE_URL", CONFIG_DEFAULTS.modelUrl),
    modelKey: text(env, "OPENAI_API_KEY"),
    model: required(env, "OPENAI_MODEL"),
    maxTokens: positive(env, "AGENT_MAX_TOKENS") ?? CONFIG_DEFAULTS.maxTokens,
    jsonMode: flag(env, "AGENT_JSON_MODE"),
    decisionsPerHour: positive(env, "AGENT_DECISIONS_PER_HOUR") ?? CONFIG_DEFAULTS.decisionsPerHour,
    turns: positive(env, "AGENT_TURNS"),
  };
}
