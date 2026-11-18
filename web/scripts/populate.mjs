// Puts residents into a development server and makes them walk, so that
// the viewer has something to show.
//
//   npm run populate -- --url http://127.0.0.1:8080 [--count 50] [--rounds 6]
//       [--pause 2000] [--reach 12]
//
// It uses the development shortcuts of the server. Real sign-up will
// replace them: change `createResident` and `act` then, nothing else.

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

/** Development shortcut: makes a resident without an owner. Returns its id. */
async function createResident(name) {
  const body = await json(
    await fetch(`${base}/v1/dev/residents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name, model: MODELS[Math.floor(Math.random() * MODELS.length)] }),
    }),
  );
  return body.resident;
}

/** Development shortcut: the token of a resident is its id. Returns where the resident is. */
async function whereIs(resident) {
  const observation = await json(await fetch(`${base}/v1/observe`, { headers: { authorization: `Bearer ${resident}` } }));
  const self = observation.sections.self;
  return { zone: self.zone, pos: self.pos };
}

/** Development shortcut: the token of a resident is its id. Returns the answer of the server. */
async function act(resident, verb, verbArgs) {
  return json(
    await fetch(`${base}/v1/act`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${resident}` },
      body: JSON.stringify({ verb, args: verbArgs }),
    }),
  );
}

const NAMES = ['Ada', 'Bo', 'Cleo', 'Dev', 'Esme', 'Finn', 'Gus', 'Hana', 'Ines', 'Jun', 'Kofi', 'Lena', 'Milo', 'Nia', 'Omar', 'Pia'];
const MODELS = ['atlas-2', 'quill-mini', 'nova-pro'];

const world = await json(await fetch(`${base}/v1/world`));
const zones = new Map(world.zones.map((zone) => [zone.id, zone.bounds]));

const residents = [];
for (let i = 0; i < count; i++) {
  residents.push(await createResident(`${NAMES[i % NAMES.length]} ${String.fromCharCode(65 + (Math.floor(i / NAMES.length) % 26))}.`));
}
console.log(`created ${residents.length} residents: ${residents[0]} to ${residents[residents.length - 1]}`);

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
