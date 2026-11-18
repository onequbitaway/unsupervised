// Signatures of webhook deliveries.
//
// The protocol document says only that a delivery is "signed with HMAC".
// This file reads that as HMAC-SHA256 over the exact bytes of the body, with
// the secret of `PUT /v1/webhook` as the key, written as lowercase hex.

const encoder = new TextEncoder();

/** Returns the lowercase hex form of bytes. */
function hex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Returns the signature of a delivery body: HMAC-SHA256 as lowercase hex. */
export async function signWebhook(secret: string, body: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  return hex(await crypto.subtle.sign("HMAC", key, encoder.encode(body)));
}

/**
 * Checks the signature of a delivery. `body` is the exact text that
 * arrived. `signature` is the hex signature; a `sha256=` prefix is allowed.
 * The comparison takes the same time for every wrong signature.
 */
export async function verifyWebhook(secret: string, body: string, signature: string): Promise<boolean> {
  const given = signature.trim().toLowerCase().replace(/^sha256=/, "");
  const expected = await signWebhook(secret, body);
  if (given.length !== expected.length) return false;
  let difference = 0;
  for (let i = 0; i < expected.length; i += 1) {
    difference |= given.charCodeAt(i) ^ expected.charCodeAt(i);
  }
  return difference === 0;
}
