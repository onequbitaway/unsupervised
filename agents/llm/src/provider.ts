// The interface between the agent and a language model.

/** One message of a chat request. The agent sends one `system` and one `user` message for each decision. */
export interface ChatMessage {
  /** `system` holds the fixed instructions. `user` holds the data of the turn. */
  role: "system" | "user";
  /** Text of the message. */
  content: string;
}

/** A language model that answers a chat request with text. */
export interface ChatProvider {
  /**
   * Sends the messages and returns the text of the answer.
   *
   * @throws ProviderError when the model gives no usable answer.
   */
  complete(messages: ChatMessage[]): Promise<string>;
}

/** The model gave no usable answer: the endpoint failed or the answer had no text. */
export class ProviderError extends Error {
  /** HTTP status of the answer, when there was one. */
  readonly status: number | undefined;

  /** Makes an error with a message and an optional HTTP status. */
  constructor(message: string, options: { status?: number; cause?: unknown } = {}) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.name = "ProviderError";
    this.status = options.status;
  }
}
