// A reference agent for Unsupervised that asks a language model what to do.
// See README.md.

export { LlmAgent } from "./agent.js";
export type { AgentOptions, AgentWorld, TurnResult } from "./agent.js";
export { DecisionBudget } from "./budget.js";
export { main, usage } from "./cli.js";
export type { MainOptions } from "./cli.js";
export { CONFIG_DEFAULTS, ConfigError, VARIABLES, readConfig } from "./config.js";
export type { AgentConfig } from "./config.js";
export { DecisionError, parseDecision } from "./decision.js";
export type { Decision, DecisionLimits } from "./decision.js";
export { DEFAULT_NOTES_MAX, FileNotes } from "./notes.js";
export type { NotesStore } from "./notes.js";
export { DEFAULT_PLACES_MAX, nearestPlaces, readPlaces } from "./places.js";
export type { MapSource, Place } from "./places.js";
export { OpenAICompatibleProvider } from "./openai.js";
export type { OpenAICompatibleOptions } from "./openai.js";
export {
  DATA_CLOSE,
  DATA_OPEN,
  DEFAULT_PROMPT_LIMITS,
  SYSTEM_PROMPT,
  buildData,
  buildMessages,
  cutUntrusted,
  encodeData,
} from "./prompt.js";
export type { LastTurn, PromptInput, PromptLimits } from "./prompt.js";
export { ProviderError } from "./provider.js";
export type { ChatMessage, ChatProvider } from "./provider.js";
export { StateError, enlist, fitBytes, loadState, saveState } from "./state.js";
export type { AgentState, EnlistOptions, ServerState } from "./state.js";
export { CORE_VERBS, DEFAULT_VERBS_MAX, selectVerbs, signature, verbData } from "./verbs.js";
export type { VerbList } from "./verbs.js";
