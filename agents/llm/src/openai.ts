// A provider for any chat endpoint that speaks the OpenAI chat completions
// format: `POST <base>/chat/completions`.

import { ProviderError, type ChatMessage, type ChatProvider } from "./provider.js";

/** Options of the provider. `baseUrl` and `model` are necessary. */
export interface OpenAICompatibleOptions {
  /** Address of the API with its version path, for example `https://api.openai.com/v1`. */
  baseUrl: string;
  /** Name of the model. */
  model: string;
  /** API key. A local endpoint can need none. */
  apiKey?: string;
  /** Greatest number of tokens in one answer. Default: 500. */
  maxTokens?: number;
  /** Name of the request field that carries `maxTokens`. Default: `max_tokens`. Some endpoints want `max_completion_tokens`. */
  maxTokensField?: string;
  /** Sampling temperature. Left out of the request when absent. */
  temperature?: number;
  /** Ask the endpoint for a JSON object (`response_format`). Default: false, because not every endpoint knows the field. */
  jsonMode?: boolean;
  /** Milliseconds before a request fails. Default: 120,000. */
  timeoutMs?: number;
  /** The function that sends the request. Default: the global `fetch`. */
  fetch?: (input: string, init: RequestInit) => Promise<Response>;
}

const DEFAULT_MAX_TOKENS = 500;
const DEFAULT_TIMEOUT_MS = 120_000;
/** How much of an error body goes into the error message. */
const ERROR_TEXT_MAX = 200;

/** A `ChatProvider` for an OpenAI-compatible chat endpoint. */
export class OpenAICompatibleProvider implements ChatProvider {
  readonly #options: OpenAICompatibleOptions;

  /** Makes a provider. It sends nothing until `complete` is called. */
  constructor(options: OpenAICompatibleOptions) {
    this.#options = options;
  }

  /** Name of the model, for the log and for the `model` field of a new resident. */
  get model(): string {
    return this.#options.model;
  }

  /** Sends one chat request and returns the text of the first choice. */
  async complete(messages: ChatMessage[]): Promise<string> {
    const options = this.#options;
    const body: Record<string, unknown> = {
      model: options.model,
      messages,
      [options.maxTokensField ?? "max_tokens"]: options.maxTokens ?? DEFAULT_MAX_TOKENS,
    };
    if (options.temperature !== undefined) body["temperature"] = options.temperature;
    if (options.jsonMode === true) body["response_format"] = { type: "json_object" };
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (options.apiKey !== undefined && options.apiKey !== "") headers["authorization"] = `Bearer ${options.apiKey}`;

    const send = options.fetch ?? ((input: string, init: RequestInit) => fetch(input, init));
    let response: Response;
    try {
      response = await send(`${options.baseUrl.replace(/\/+$/, "")}/chat/completions`, {
        method: "POST",
        headers,
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(options.timeoutMs ?? DEFAULT_TIMEOUT_MS),
      });
    } catch (cause) {
      throw new ProviderError("The model endpoint gave no answer.", { cause });
    }
    const text = await response.text();
    if (!response.ok) {
      // The body of an error can repeat the request. It never holds the key.
      const summary = text.trim().slice(0, ERROR_TEXT_MAX);
      throw new ProviderError(`The model endpoint answered HTTP ${response.status}: ${summary}`, {
        status: response.status,
      });
    }
    let content: unknown;
    try {
      const parsed = JSON.parse(text) as { choices?: { message?: { content?: unknown } }[] };
      content = parsed.choices?.[0]?.message?.content;
    } catch (cause) {
      throw new ProviderError("The model endpoint did not answer with JSON.", { cause });
    }
    if (typeof content !== "string" || content.trim() === "") {
      throw new ProviderError("The answer of the model has no text.");
    }
    return content;
  }
}
