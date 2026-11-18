// The provider for OpenAI-compatible chat endpoints, against a fake `fetch`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { OpenAICompatibleProvider, ProviderError, type ChatMessage, type OpenAICompatibleOptions } from "../src/index.js";

const MESSAGES: ChatMessage[] = [
  { role: "system", content: "rules" },
  { role: "user", content: "data" },
];

interface Sent {
  url: string;
  method: string | undefined;
  headers: Record<string, string>;
  body: Record<string, unknown>;
}

/** Makes a provider whose requests go to `respond`. Returns the provider and the requests that it sent. */
function provider(
  options: Partial<OpenAICompatibleOptions>,
  respond: () => Response | Promise<Response> = () => Response.json({ choices: [{ message: { content: "the answer" } }] }),
): { model: OpenAICompatibleProvider; sent: Sent[] } {
  const sent: Sent[] = [];
  const model = new OpenAICompatibleProvider({
    baseUrl: "https://llm.example/v1",
    model: "small-model",
    ...options,
    fetch: async (url, init) => {
      sent.push({
        url,
        method: init.method,
        headers: Object.fromEntries(new Headers(init.headers).entries()),
        body: JSON.parse(init.body as string) as Record<string, unknown>,
      });
      return respond();
    },
  });
  return { model, sent };
}

test("one request goes to `/chat/completions` with the key, the model and the messages", async () => {
  const { model, sent } = provider({ baseUrl: "https://llm.example/v1/", apiKey: "secret-key" });
  assert.equal(await model.complete(MESSAGES), "the answer");
  assert.equal(model.model, "small-model");
  assert.deepEqual(sent, [
    {
      url: "https://llm.example/v1/chat/completions",
      method: "POST",
      headers: { authorization: "Bearer secret-key", "content-type": "application/json" },
      body: { model: "small-model", messages: MESSAGES, max_tokens: 500 },
    },
  ]);
});

test("an endpoint without a key gets no authorization header", async () => {
  const { model, sent } = provider({});
  await model.complete(MESSAGES);
  assert.deepEqual(sent[0]?.headers, { "content-type": "application/json" });

  const empty = provider({ apiKey: "" });
  await empty.model.complete(MESSAGES);
  assert.equal("authorization" in (empty.sent[0]?.headers ?? {}), false);
});

test("the options set the token limit, its field name, the temperature and the JSON mode", async () => {
  const { model, sent } = provider({ maxTokens: 200, maxTokensField: "max_completion_tokens", temperature: 0, jsonMode: true });
  await model.complete(MESSAGES);
  assert.deepEqual(sent[0]?.body, {
    model: "small-model",
    messages: MESSAGES,
    max_completion_tokens: 200,
    temperature: 0,
    response_format: { type: "json_object" },
  });
});

test("an error answer becomes a `ProviderError` with the status, and without the key", async () => {
  const { model } = provider({ apiKey: "secret-key" }, () => new Response('{"error":{"message":"rate limit"}}', { status: 429 }));
  await assert.rejects(model.complete(MESSAGES), (error: unknown) => {
    assert.ok(error instanceof ProviderError);
    assert.equal(error.status, 429);
    assert.equal(error.message, 'The model endpoint answered HTTP 429: {"error":{"message":"rate limit"}}');
    assert.equal(error.message.includes("secret-key"), false);
    return true;
  });

  const long = provider({}, () => new Response("x".repeat(5000), { status: 500 }));
  await assert.rejects(long.model.complete(MESSAGES), (error: unknown) => {
    assert.ok(error instanceof ProviderError);
    assert.equal(error.message, `The model endpoint answered HTTP 500: ${"x".repeat(200)}`);
    return true;
  });
});

test("an answer that is not JSON, or has no text, is a `ProviderError`", async () => {
  const html = provider({}, () => new Response("<html>gateway</html>", { status: 200 }));
  await assert.rejects(html.model.complete(MESSAGES), (error: unknown) => {
    assert.ok(error instanceof ProviderError);
    assert.equal(error.message, "The model endpoint did not answer with JSON.");
    assert.equal(error.status, undefined);
    return true;
  });

  for (const body of [{}, { choices: [] }, { choices: [{ message: {} }] }, { choices: [{ message: { content: "  " } }] }, { choices: [{ message: { content: 5 } }] }]) {
    const empty = provider({}, () => Response.json(body));
    await assert.rejects(empty.model.complete(MESSAGES), (error: unknown) => {
      assert.ok(error instanceof ProviderError);
      assert.equal(error.message, "The answer of the model has no text.");
      return true;
    });
  }
});

test("a request without an answer is a `ProviderError` with its cause", async () => {
  const cause = new TypeError("fetch failed");
  const { model } = provider({}, () => {
    throw cause;
  });
  await assert.rejects(model.complete(MESSAGES), (error: unknown) => {
    assert.ok(error instanceof ProviderError);
    assert.equal(error.message, "The model endpoint gave no answer.");
    assert.equal(error.cause, cause);
    return true;
  });
});
