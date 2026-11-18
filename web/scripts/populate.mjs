// Puts residents into a server and makes them walk and talk, so that the
// viewer has something to show.
//
//   npm run populate -- --url http://127.0.0.1:8080 [--count 15] [--rounds 6]
//       [--pause 2000] [--reach 12] [--token unsup_own_...]
//
// It uses the public API of `docs/design/protocol.md`, section 2: it signs
// up owners with `POST /v1/owners` and creates residents with
// `POST /v1/residents`.
//
// A server has limits. With the defaults, one owner has 3 residents in the
// world, and one address can sign up 5 owners at once and then one more
// each 3 minutes. So the script gets 15 residents from a server with the
// default limits. When the server refuses more sign-ups, the script goes
// on with the residents that it has.
//
// For more residents, give one owner token with `--token` and raise the
// limit of residents of one owner in the configuration of the server:
//
//   [gateway.limits]
//   residents_per_owner = 100
//
// `unsupervised run --dev` prints the token of a ready owner. For a town
// that lives by itself (work, food, sleep), run the house residents of
// `agents/scripted` instead.

import { number, readArgs } from './args.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string') {
  console.error('Give the address of the server: --url http://127.0.0.1:8080');
  process.exit(2);
}
const base = args.url.replace(/\/$/, '');
const count = number(args.count, 15);
const rounds = number(args.rounds, 6);
const pause = number(args.pause, 2_000);
const reach = number(args.reach, 12);

/** Greatest number of seconds that the script waits when the server says "too many requests". */
const PATIENCE_SECONDS = 15;

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

/** An answer of the server that is not a success. */
class Refusal extends Error {
  constructor(response, body) {
    super(`${response.url} answered ${response.status}: ${JSON.stringify(body)}`);
    this.status = response.status;
    this.code = body?.error?.code;
    this.retryAfter = Number(response.headers.get('retry-after') ?? '0');
  }
}

/**
 * Sends a request and returns the JSON answer. When the server answers
 * `rate_limited` with a short wait, the script waits and sends the request
 * again. Throws a `Refusal` for every other answer that is not a success.
 */
async function call(method, path, token, body) {
  const headers = {};
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (token !== null) headers.authorization = `Bearer ${token}`;
  for (;;) {
    const response = await fetch(`${base}${path}`, {
      method,
      headers,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const answer = await response.json();
    if (response.ok) return answer;
    const refusal = new Refusal(response, answer);
    if (refusal.status !== 429 || refusal.retryAfter > PATIENCE_SECONDS) throw refusal;
    await sleep(Math.max(1, refusal.retryAfter) * 1000);
  }
}

/** The token of the owner that gets the next resident. */
let ownerToken = typeof args.token === 'string' ? args.token : null;
let owners = 0;

/**
 * Creates a resident for the current owner. When that owner has its
 * greatest number of residents, the script signs up a new owner one time
 * and tries again. Returns the id and the key of the resident, or `null`
 * when the limits of the server permit no more residents.
 */
async function createResident(name) {
  const request = { name, model: MODELS[Math.floor(Math.random() * MODELS.length)] };
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      if (ownerToken === null) {
        ownerToken = (await call('POST', '/v1/owners', null, { name: `Populate ${owners + 1}` })).token;
        owners += 1;
      }
      const body = await call('POST', '/v1/residents', ownerToken, request);
      return { id: body.resident.id, key: body.key, name };
    } catch (error) {
      if (!(error instanceof Refusal)) throw error;
      if (error.code === 'resident_limit') {
        // A token from the command line has no next owner behind it.
        if (typeof args.token === 'string') {
          console.error(
            'The owner of --token has its greatest number of residents. Raise `residents_per_owner` in the ' +
              '`[gateway.limits]` table of the server configuration.',
          );
          return null;
        }
        ownerToken = null;
      } else if (error.code === 'rate_limited') {
        console.error(
          `The server takes no more sign-ups from this address now (the next one in ${error.retryAfter} seconds). ` +
            'For more residents, give one owner token with --token and raise `residents_per_owner`.',
        );
        return null;
      } else {
        throw error;
      }
    }
  }
  throw new Error('A new owner cannot create a resident. Look at the limits of the gateway.');
}

/** Returns where a resident is, from its own observation. */
async function whereIs(resident) {
  const observation = await call('GET', '/v1/observe?sections=self', resident.key);
  const self = observation.sections.self;
  return { zone: self.zone, pos: self.pos };
}

/** Sends one action of a resident. Returns its acknowledgement. */
async function act(resident, verb, verbArgs) {
  const answer = await call('POST', '/v1/act?sections=self', resident.key, { verb, args: verbArgs });
  return answer.acks[0];
}

const NAMES = ['Ada', 'Bo', 'Cleo', 'Dev', 'Esme', 'Finn', 'Gus', 'Hana', 'Ines', 'Jun', 'Kofi', 'Lena', 'Milo', 'Nia', 'Omar', 'Pia'];
const MODELS = ['atlas-2', 'quill-mini', 'nova-pro'];
// Words without a link and without a long row of one character: the text filters of a public server let them through.
const WORDS = ['Good day.', 'Where is the depot?', 'I look for work.', 'The park is nice today.', 'See you at the city hall.'];

const world = await call('GET', '/v1/world', null);
const zones = new Map(world.zones.map((zone) => [zone.id, zone.bounds]));
const verbs = new Set((await call('GET', '/v1/catalogue', null)).verbs.map((verb) => verb.verb));

const residents = [];
for (let i = 0; i < count; i++) {
  const name = `${NAMES[i % NAMES.length]} ${String.fromCharCode(65 + (Math.floor(i / NAMES.length) % 26))}.`;
  const resident = await createResident(name);
  if (resident === null) break;
  residents.push(resident);
}
if (residents.length === 0) {
  console.error('The script created no resident.');
  process.exit(1);
}
console.log(
  `created ${residents.length} of ${count} residents: ${residents[0].id} to ${residents[residents.length - 1].id}` +
    (owners > 0 ? `, for ${owners} new owners` : ''),
);

const between = (low, high) => low + Math.floor(Math.random() * (high - low + 1));

for (let round = 1; round <= rounds; round++) {
  let moved = 0;
  let spoke = 0;
  for (const resident of residents) {
    const { zone, pos } = await whereIs(resident);
    const bounds = zones.get(zone);
    if (!bounds) continue;
    // A new walk replaces the walk of the round before.
    await act(resident, 'cancel', {});
    // A target near the resident, inside its district.
    const x = Math.min(bounds.x1 - 1, Math.max(bounds.x0, pos.x + between(-reach, reach)));
    const y = Math.min(bounds.y1 - 1, Math.max(bounds.y0, pos.y + between(-reach, reach)));
    const walk = await act(resident, 'move_to', { x, y });
    if (walk.status.state !== 'rejected') moved += 1;
    // One resident in four says something. The residents near it hear it.
    if (verbs.has('say') && between(0, 3) === 0) {
      const said = await act(resident, 'say', { text: WORDS[between(0, WORDS.length - 1)] });
      if (said.status.state === 'done') spoke += 1;
    }
  }
  console.log(`round ${round} of ${rounds}: ${moved} residents walk, ${spoke} speak`);
  if (round < rounds) await sleep(pause);
}
