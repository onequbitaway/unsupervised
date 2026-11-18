// Puts residents into a server and makes them walk, so that the viewer
// has something to show.
//
//   npm run populate -- --url http://127.0.0.1:8080 [--count 50] [--rounds 6]
//       [--pause 2000] [--reach 12] [--token unsup_own_...]
//
// It uses the public API of `docs/design/protocol.md`, section 2: it signs
// up owners with `POST /v1/owners` and creates residents with
// `POST /v1/residents`. One owner has a limit of residents, so the script
// signs up a new owner when the server answers `resident_limit`. A server
// also limits the sign-ups of one address in an hour (20 by default): for
// more residents, raise the limits of the gateway, or give the owner token
// that `unsupervised run --dev` prints with `--token` and raise the limit
// of residents of one owner.

import { number, readArgs } from './args.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string') {
  console.error('Give the address of the server: --url http://127.0.0.1:8080');
  process.exit(2);
}
const base = args.url.replace(/\/$/, '');
const count = number(args.count, 50);
const rounds = number(args.rounds, 6);
const pause = number(args.pause, 2_000);
const reach = number(args.reach, 12);

async function json(response) {
  const body = await response.json();
  if (!response.ok) throw new Error(`${response.url} answered ${response.status}: ${JSON.stringify(body)}`);
  return body;
}

/** Sends a request with a JSON body and a bearer token. Returns the response. */
function post(path, token, body) {
  const headers = { 'content-type': 'application/json' };
  if (token !== null) headers.authorization = `Bearer ${token}`;
  return fetch(`${base}${path}`, { method: 'POST', headers, body: JSON.stringify(body) });
}

/** Signs up a new owner. Returns its token. */
async function signUp(number) {
  const body = await json(await post('/v1/owners', null, { name: `Populate ${number}` }));
  return body.token;
}

/** The token of the owner that gets the next resident. */
let ownerToken = typeof args.token === 'string' ? args.token : null;
let owners = 0;

/**
 * Creates a resident for the current owner. When that owner has its
 * greatest number of residents, the script signs up a new owner one time
 * and tries again. Returns the id and the key of the resident.
 */
async function createResident(name) {
  const request = { name, model: MODELS[Math.floor(Math.random() * MODELS.length)] };
  for (let attempt = 0; attempt < 2; attempt++) {
    if (ownerToken === null) ownerToken = await signUp(++owners);
    const response = await post('/v1/residents', ownerToken, request);
    if (response.status === 403) {
      const refusal = await response.json();
      if (refusal?.error?.code !== 'resident_limit') {
        throw new Error(`${response.url} answered 403: ${JSON.stringify(refusal)}`);
      }
      // A token from the command line has no next owner behind it.
      if (typeof args.token === 'string') {
        throw new Error('The owner of --token has its greatest number of residents. Raise the limit, or give no token.');
      }
      ownerToken = null;
      continue;
    }
    const body = await json(response);
    return { id: body.resident.id, key: body.key };
  }
  throw new Error('A new owner cannot create a resident. Look at the limits of the gateway.');
}

/** Returns where a resident is, from its own observation. */
async function whereIs(resident) {
  const observation = await json(
    await fetch(`${base}/v1/observe`, { headers: { authorization: `Bearer ${resident.key}` } }),
  );
  const self = observation.sections.self;
  return { zone: self.zone, pos: self.pos };
}

/** Sends one action of a resident. Returns the answer of the server. */
async function act(resident, verb, verbArgs) {
  return json(await post('/v1/act', resident.key, { verb, args: verbArgs }));
}

const NAMES = ['Ada', 'Bo', 'Cleo', 'Dev', 'Esme', 'Finn', 'Gus', 'Hana', 'Ines', 'Jun', 'Kofi', 'Lena', 'Milo', 'Nia', 'Omar', 'Pia'];
const MODELS = ['atlas-2', 'quill-mini', 'nova-pro'];

const world = await json(await fetch(`${base}/v1/world`));
const zones = new Map(world.zones.map((zone) => [zone.id, zone.bounds]));

const residents = [];
for (let i = 0; i < count; i++) {
  residents.push(await createResident(`${NAMES[i % NAMES.length]} ${String.fromCharCode(65 + (Math.floor(i / NAMES.length) % 26))}.`));
}
console.log(
  `created ${residents.length} residents: ${residents[0].id} to ${residents[residents.length - 1].id}` +
    (owners > 0 ? `, for ${owners} new owners` : ''),
);

const between = (low, high) => low + Math.floor(Math.random() * (high - low + 1));

for (let round = 1; round <= rounds; round++) {
  let moved = 0;
  for (const resident of residents) {
    const { zone, pos } = await whereIs(resident);
    const bounds = zones.get(zone);
    if (!bounds) continue;
    // A target near the resident, inside its district.
    const x = Math.min(bounds.x1 - 1, Math.max(bounds.x0, pos.x + between(-reach, reach)));
    const y = Math.min(bounds.y1 - 1, Math.max(bounds.y0, pos.y + between(-reach, reach)));
    const answer = await act(resident, 'move_to', { x, y });
    if (answer.acks?.[0]?.status?.state !== 'rejected') moved += 1;
  }
  console.log(`round ${round} of ${rounds}: ${moved} residents walk`);
  if (round < rounds) await new Promise((resolve) => setTimeout(resolve, pause));
}
