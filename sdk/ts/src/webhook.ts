// Signatures of webhook deliveries (`docs/design/protocol.md`, "Webhooks").
//
// Each delivery has three headers: the id of the delivery, the time of
// the attempt, and the signature. The signature is `v1=` and the
// HMAC-SHA256, as lowercase hexadecimal, of `v1:<timestamp>:<body>` with
// the secret of `PUT /v1/webhook` as the key.

/** Name of the header with the id of the delivery. It is the same for each attempt. */
export const WEBHOOK_DELIVERY_HEADER = "x-unsupervised-delivery";
/** Name of the header with the time of the attempt, in seconds since 1970. */
export const WEBHOOK_TIMESTAMP_HEADER = "x-unsupervised-timestamp";
/** Name of the header with the signature. */
export const WEBHOOK_SIGNATURE_HEADER = "x-unsupervised-signature";
/** Greatest age of a delivery that `verifyWebhook` accepts by default, in seconds. */
export const WEBHOOK_TOLERANCE_SECONDS = 300;

const VERSION = "v1";
const encoder = new TextEncoder();

/** Returns the lowercase hex form of bytes. */
function hex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Returns the bytes of a body. Text is UTF-8. */
function bytesOf(body: string | Uint8Array): Uint8Array {
  return typeof body === "string" ? encoder.encode(body) : body;
}

/**
 * Returns the value of the signature header for a delivery: `v1=` and the
 * HMAC-SHA256 of `v1:<timestamp>:<body>`. `body` is the exact content of
 * the delivery.
 */
export async function signWebhook(secret: string, timestamp: string | number, body: string | Uint8Array): Promise<string> {
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  const head = encoder.encode(`${VERSION}:${timestamp}:`);
  const content = bytesOf(body);
  const message = new Uint8Array(head.length + content.length);
  message.set(head, 0);
  message.set(content, head.length);
  return `${VERSION}=${hex(await crypto.subtle.sign("HMAC", key, message))}`;
}

/** The parts of a delivery that the signature covers. */
export interface WebhookParts {
  /** The header `X-Unsupervised-Timestamp`. */
  timestamp: string | number | null | undefined;
  /** The header `X-Unsupervised-Signature`. */
  signature: string | null | undefined;
  /** The exact content of the body, before any JSON parsing. */
  body: string | Uint8Array;
}

/** Options of `verifyWebhook`. */
export interface VerifyWebhookOptions {
  /**
   * Greatest age of the delivery in seconds, in both directions. A
   * delivery that is older is refused, so that nobody can send an old
   * delivery again. Default: 300. `Infinity` turns the check off.
   */
  toleranceSeconds?: number;
  /** The time now, in seconds since 1970. Default: the clock of the machine. */
  now?: number;
}

/**
 * Checks a webhook delivery: the signature is that of the secret, and the
 * timestamp is not old. Returns false for a delivery without a timestamp
 * or a signature. The comparison takes the same time for every wrong
 * signature.
 */
export async function verifyWebhook(
  secret: string,
  parts: WebhookParts,
  options: VerifyWebhookOptions = {},
): Promise<boolean> {
  const { timestamp, signature } = parts;
  if (timestamp == null || signature == null) return false;
  const stamp = String(timestamp).trim();
  if (!/^\d+$/.test(stamp)) return false;
  const tolerance = options.toleranceSeconds ?? WEBHOOK_TOLERANCE_SECONDS;
  const now = options.now ?? Date.now() / 1000;
  if (Math.abs(now - Number(stamp)) > tolerance) return false;
  const given = signature.trim();
  const expected = await signWebhook(secret, stamp, parts.body);
  if (given.length !== expected.length) return false;
  let difference = 0;
  for (let i = 0; i < expected.length; i += 1) {
    difference |= given.charCodeAt(i) ^ expected.charCodeAt(i);
  }
  return difference === 0;
}

/**
 * Checks a delivery from its HTTP headers and its body. `headers` is a
 * `Headers` object or an object with lowercase header names, as
 * `node:http` gives it.
 */
export function verifyWebhookRequest(
  secret: string,
  headers: Headers | Record<string, string | string[] | undefined>,
  body: string | Uint8Array,
  options: VerifyWebhookOptions = {},
): Promise<boolean> {
  const read = (name: string): string | undefined => {
    if (headers instanceof Headers) return headers.get(name) ?? undefined;
    const value = headers[name];
    return Array.isArray(value) ? value[0] : value;
  };
  return verifyWebhook(
    secret,
    { timestamp: read(WEBHOOK_TIMESTAMP_HEADER), signature: read(WEBHOOK_SIGNATURE_HEADER), body },
    options,
  );
}
