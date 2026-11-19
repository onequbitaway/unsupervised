// Calls to the public API of a server (`docs/design/protocol.md`, section
// 2), for the development scripts.

/** Waits for a number of milliseconds. */
export const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

/** An answer of the server that is not a success. */
export class Refusal extends Error {
  constructor(response, body) {
    super(`${response.url} answered ${response.status}: ${JSON.stringify(body)}`);
    this.status = response.status;
    this.code = body?.error?.code;
    this.retryAfter = Number(response.headers.get('retry-after') ?? '0');
  }
}

/**
 * Returns a function that sends one request to the server at `base` and
 * returns the JSON answer.
 *
 * When the server answers `rate_limited` with a wait of at most
 * `patienceSeconds`, the function waits and sends the request again. It
 * throws a `Refusal` for every other answer that is not a success.
 */
export function connect(base, { patienceSeconds = 15 } = {}) {
  const root = base.replace(/\/$/, '');
  return async function call(method, path, token, body) {
    const headers = {};
    if (body !== undefined) headers['content-type'] = 'application/json';
    if (token !== null && token !== undefined) headers.authorization = `Bearer ${token}`;
    for (;;) {
      const response = await fetch(`${root}${path}`, {
        method,
        headers,
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
      const text = await response.text();
      let answer;
      try {
        answer = JSON.parse(text);
      } catch {
        // A proxy or a crash page can answer with plain text.
        answer = { error: { code: 'not_json', message: text.slice(0, 200) } };
      }
      if (response.ok) return answer;
      const refusal = new Refusal(response, answer);
      if (refusal.status !== 429 || refusal.retryAfter > patienceSeconds) throw refusal;
      await sleep(Math.max(1, refusal.retryAfter) * 1000);
    }
  };
}

/** Returns the minutes of a game clock since the start of its year. */
export function minutesOf(clock) {
  return ((clock.day - 1) * 24 + clock.hour) * 60 + clock.minute;
}

/** Writes a game clock as `day 3 20:00`. */
export function clockText(clock) {
  const two = (number) => String(number).padStart(2, '0');
  return `day ${clock.day} ${two(clock.hour)}:${two(clock.minute)}`;
}
