// Plays a short drama in a running server, so that the director, the
// stories and a broadcast have something to show.
//
//   node scripts/drama.mjs --url http://127.0.0.1:8080 --token unsup_own_...
//       [--city 0] [--seed-money 30000] [--model drama]
//
// The script uses only the public API of `docs/design/protocol.md`,
// section 2. It creates eight residents for the owner of `--token` and
// drives them through events that the game modules make public:
//
// - Day 1. A sponsor mission pays seed money. Two residents run for mayor.
//   One of them speaks to a crowd. A thief picks a pocket in that crowd,
//   and the residents that saw it report the case. One resident posts a
//   mission and another one completes it. One resident buys a site and
//   opens a business.
// - Day 2. The two candidates hold a rally.
// - Day 3. The cast votes. After the count, the new mayor appoints a
//   police officer.
//
// The game does the rest of the crime story by itself. When the reports
// name the thief, the court issues a warrant, the city watch arrests the
// thief, and the evidence decides the case.
//
// Start the script in the first hours of day 1 of a new world: the
// registration for the first city election is open on day 1 only. A step
// that the game refuses is printed with the error code, and the script
// goes on with the next step.
//
// What the server must permit:
//
// - Eight more residents for the owner (`residents_per_owner` in the
//   `[gateway.limits]` table of the server configuration).
// - Two sponsor missions on one day (`missions_per_day` in `[sponsors]`).
//   The second one pays the price of the cheapest site that is for sale,
//   which is more than the default of `max_mission_cents`. When the server
//   refuses it, no business opens and the script says why.

import { clockText, connect, minutesOf, Refusal, sleep } from './api.mjs';
import { number, readArgs } from './args.mjs';

const args = readArgs(process.argv.slice(2));
if (typeof args.url !== 'string' || typeof args.token !== 'string') {
  console.error('Give the server and an owner token: --url http://127.0.0.1:8080 --token unsup_own_...');
  process.exit(2);
}
const call = connect(args.url);
const ownerToken = args.token;
const cityId = number(args.city, 0);
const seedMoney = number(args['seed-money'], 30_000);
const model = typeof args.model === 'string' ? args.model : 'drama';

/** Real seconds that the script waits for the end of one action. */
const REPORT_PATIENCE_SECONDS = 240;
/** Greatest number of tries of the thief. A failed try is a crime too. */
const THEFT_TRIES = 6;
/** Real milliseconds between two looks at the clock. */
const CLOCK_POLL_MS = 400;
/** The kinds of site that `open_business` takes. */
const BUSINESS_KINDS = ['shop', 'restaurant', 'workshop', 'factory', 'office', 'warehouse'];

/** One resident of the cast. */
class Member {
  constructor(name, id, key) {
    this.name = name;
    this.id = id;
    this.key = key;
    this.cursor = 0;
    /** Every notice that the resident got, oldest first. */
    this.notices = [];
    /** The reports of ended actions, by action id. */
    this.reports = new Map();
  }

  /** Keeps the notices and the reports of an observation. */
  take(observation) {
    this.notices.push(...observation.notices);
    for (const report of observation.sections.reports ?? []) this.reports.set(report.action_id, report);
    this.cursor = observation.next_notice;
    return observation;
  }

  /** Returns a fresh observation. */
  async look(detail = 'brief') {
    return this.take(await call('GET', `/v1/observe?since=${this.cursor}&detail=${detail}`, this.key));
  }

  /** Waits for the end of an acknowledged action. Returns `{ ok, result }` or `{ ok, error }`. */
  async end(ack) {
    const state = ack.status.state;
    if (state === 'done') return { ok: true, result: ack.status.result ?? {} };
    if (state === 'rejected') return { ok: false, error: ack.status.error };
    const giveUp = Date.now() + REPORT_PATIENCE_SECONDS * 1000;
    for (;;) {
      const report = this.reports.get(ack.action_id);
      if (report) {
        this.reports.delete(ack.action_id);
        return report.ok ? { ok: true, result: report.result ?? {} } : { ok: false, error: report.error };
      }
      if (Date.now() > giveUp) {
        return { ok: false, error: { code: 'no_report', message: `no report for ${ack.verb} in ${REPORT_PATIENCE_SECONDS} s` } };
      }
      this.take(await call('GET', `/v1/wait?since=${this.cursor}&kinds=report&timeout=20`, this.key));
    }
  }

  /** Sends one action and waits for its end. */
  async act(verb, verbArgs = {}) {
    const { acks } = await call('POST', '/v1/act?observe=false', this.key, { verb, args: verbArgs });
    return this.end(acks[0]);
  }

  /**
   * Stops what the resident does, then sends the steps as one plan and
   * waits for the end of the last one. A resident that sleeps wakes up.
   */
  async plan(steps) {
    const requests = [{ verb: 'cancel', args: {} }, ...steps.map(([verb, verbArgs]) => ({ verb, args: verbArgs ?? {} }))];
    const { acks } = await call('POST', '/v1/act?observe=false', this.key, { plan: requests });
    const refused = acks.find((ack) => ack.status.state === 'rejected');
    if (refused) return { ok: false, error: refused.status.error };
    // A step that fails cancels the steps behind it: their reports say so.
    let last = { ok: true, result: {} };
    for (const ack of acks.slice(1)) {
      last = await this.end(ack);
      if (!last.ok) return last;
    }
    return last;
  }

  /** Returns the newest notice of a kind, or `undefined`. */
  notice(kind) {
    return this.notices.findLast((notice) => notice.kind === kind);
  }
}

/** Returns the clock of the world. */
async function clock() {
  return (await call('GET', '/v1/world', null)).clock;
}

/** Waits until the game clock is at or after a moment of a day. Returns false when that moment is in the past. */
async function until(day, hour, minute = 0) {
  const target = ((day - 1) * 24 + hour) * 60 + minute;
  let now = await clock();
  if (minutesOf(now) > target + 30) return false;
  while (minutesOf(now) < target) {
    await sleep(CLOCK_POLL_MS);
    now = await clock();
  }
  return true;
}

const done = [];
const failed = [];

/** Prints one line with the game clock. */
async function say(text) {
  console.log(`[${clockText(await clock())}] ${text}`);
}

/** Writes why the game refused a step. */
function why(outcome) {
  return outcome.error ? `${outcome.error.code}: ${outcome.error.message}` : 'unknown';
}

/**
 * Runs one step of the drama. A step that throws or returns a refusal is
 * printed and counted; the drama goes on.
 */
async function beat(name, step) {
  try {
    const text = await step();
    done.push(name);
    await say(`${name}: ${text}`);
    return true;
  } catch (error) {
    failed.push(name);
    const reason = error instanceof Refusal ? `${error.status} ${error.code}` : (error?.message ?? String(error));
    await say(`${name}: NOT DONE (${reason})`);
    return false;
  }
}

/** Throws when the game refused an action. Returns the result. */
function must(outcome, what) {
  if (!outcome.ok) throw new Error(`${what}: ${why(outcome)}`);
  return outcome.result;
}

// ---- The world ----------------------------------------------------------------

const world = await call('GET', '/v1/world', null);
const catalogue = await call('GET', '/v1/catalogue', null);
const param = (name, fallback) => catalogue.params.find((entry) => entry.name === name)?.value ?? fallback;
const city = world.cities.find((entry) => entry.id === cityId);
if (!city) {
  console.error(`The world has no city ${cityId}.`);
  process.exit(1);
}
const sites = [];
for (const zone of city.zones) {
  const view = await call('GET', `/v1/world/zones/${zone}?residents=false`, null);
  sites.push(...view.sites);
}
const siteOf = (kind) => sites.find((site) => site.kind === kind);
const hall = siteOf('city_hall');
const depot = siteOf('depot');
if (!hall || !depot) {
  console.error(`The city ${city.name} has no city hall or no depot.`);
  process.exit(1);
}

// ---- The cast -----------------------------------------------------------------

const NAMES = ['Vera', 'Sly', 'Wit', 'Cora', 'Milo', 'Mina', 'Bea', 'Otto'];
const cast = [];
for (const name of NAMES) {
  const body = await call('POST', '/v1/residents', ownerToken, { name, model, city: cityId });
  cast.push(new Member(name, body.resident.id, body.key));
}
const [vera, sly, wit, cora, milo, mina, bea, otto] = cast;
console.log(`cast in ${city.name}: ${cast.map((member) => `${member.name} ${member.id}`).join(', ')}`);
for (const member of cast) await member.look();

/** Publishes a sponsor mission that pays each taker that writes a will. */
async function sponsor(title, reward, takers) {
  return call('POST', '/v1/missions', ownerToken, {
    title,
    text: 'Write a will and get the money.',
    objectives: [{ event: 'life.will_written', count: 1 }],
    reward,
    deadline_in: 720,
    takers,
  });
}

/** Lets a resident take a sponsor mission and do its objective. Returns the cash after it. */
async function earn(member, mission) {
  must(await member.act('mission_accept', { mission }), `${member.name} takes the mission`);
  must(await member.act('write_will', {}), `${member.name} writes a will`);
  // The reward comes by wire in the next frames.
  for (let tries = 0; tries < 50; tries++) {
    const observation = await member.look();
    if (member.notice('missions.mission_completed')) return observation.sections.self.cash;
    await sleep(CLOCK_POLL_MS);
  }
  throw new Error(`${member.name} got no reward`);
}

// ---- Day 1 --------------------------------------------------------------------

let seeded = false;
await beat('Seed money', async () => {
  const posted = await sponsor('Seed money for the founders', seedMoney, cast.length);
  const cash = await Promise.all(cast.map((member) => earn(member, posted.mission)));
  seeded = true;
  return `the sponsor mission ${posted.mission} paid ${seedMoney} cents to each of ${cash.length} residents`;
});

// Food for the days of the drama. The routines of the game eat it.
const bread = param('economy.depot_ceiling.bread', 2_000);
if (seeded) {
  await beat('Food', async () => {
    const bought = await Promise.all(
      cast.map((member) => member.plan([['move_to', { site: depot.id }], ['buy', { item: 'bread', qty: 4, max_price: bread }]])),
    );
    const refused = bought.filter((outcome) => !outcome.ok);
    if (refused.length === cast.length) throw new Error(why(refused[0]));
    return `${cast.length - refused.length} residents bought bread at the depot`;
  });
}

/** The election for mayor of the city, as a resident sees it. */
async function mayorElection(member) {
  const observation = await member.look('full');
  const elections = observation.sections.politics?.elections ?? [];
  return elections.find((election) => election.office === 'mayor' && election.city === cityId);
}

let election = null;
const candidates = [];
for (const [candidate, platform] of [
  [cora, 'A park that is open at night, and a watch that sleeps less.'],
  [milo, 'Lower taxes. The depot pays more for your grain.'],
]) {
  await beat(`${candidate.name} runs for mayor`, async () => {
    const open = await mayorElection(candidate);
    if (!open) throw new Error('no election for mayor is open in this city');
    if (open.phase !== 'registration') throw new Error(`the election is in the phase ${open.phase}`);
    election = open.election;
    const result = must(
      await candidate.plan([['register_candidate', { election, platform }]]),
      'register_candidate',
    );
    candidates.push(candidate);
    // Money for the rally of day 2.
    const fund = await candidate.act('donate', { election, candidate: candidate.id, amount: 1_500 });
    return `election ${election}, fee ${result.fee ?? param('politics.candidate_fee', 0)} cents${fund.ok ? ', 1500 cents in the campaign fund' : `, no fund (${why(fund)})`}`;
  });
}

// The crowd: everybody walks to the city hall and then to the tile of Vera.
let crowd = false;
await beat('The crowd meets at the city hall', async () => {
  must(await vera.plan([['move_to', { site: hall.id }]]), 'Vera walks to the city hall');
  const spot = (await vera.look()).sections.self.pos;
  const others = cast.filter((member) => member !== vera);
  const walks = await Promise.all(others.map((member) => member.plan([['move_to', { x: spot.x, y: spot.y }]])));
  const there = walks.filter((outcome) => outcome.ok).length;
  crowd = there > 0;
  return `${there + 1} residents stand at ${spot.x},${spot.y} in ${hall.name}`;
});

await beat('Cora speaks', async () => {
  must(await cora.act('say', { text: 'Friends, vote for me on day three. The park stays open at night.' }), 'say');
  must(await milo.act('say', { text: 'Do not listen. Vote for lower taxes.' }), 'say');
  const posted = await cora.act('post', { text: 'I run for mayor. Meet me at the city hall.' });
  return `two speeches${posted.ok ? ' and one post on the public feed' : ''}`;
});

// The mission of Mina: one shift of public works. Wit takes it.
let shift = null;
await beat('Mina posts a mission', async () => {
  const result = must(
    await mina.act('mission_post', {
      title: 'Sweep the square of the city hall',
      text: 'One shift of public works.',
      objectives: [{ event: 'economy.shift_completed', count: 1 }],
      reward: 1_500,
      deadline_in: 1_440,
      takers: 1,
    }),
    'mission_post',
  );
  must(await wit.act('mission_accept', { mission: result.mission }), 'Wit takes the mission');
  return `mission ${result.mission} for 1500 cents; Wit took it`;
});

const cases = [];
if (crowd) {
  await beat('Sly picks the pocket of Vera', async () => {
    for (let attempt = 1; attempt <= THEFT_TRIES; attempt++) {
      const outcome = await sly.act('pickpocket', { target: vera.id });
      const result = must(outcome, 'pickpocket');
      if (result.case !== undefined) cases.push(result.case);
      if (result.success) return `try ${attempt} took ${result.cash} cents, case ${result.case}, ${result.witnesses} witnesses`;
    }
    throw new Error(`${THEFT_TRIES} tries failed; the cases ${cases.join(', ')} are attempts`);
  });

  await beat('The crowd reports the theft', async () => {
    let reports = 0;
    let named = 0;
    for (const member of cast.filter((entry) => entry !== sly)) {
      await member.look();
      const known = new Set(member.notices.filter((notice) => notice.kind === 'crime.witnessed').map((notice) => notice.data.case));
      for (const number of known) {
        const outcome = await member.act('report_crime', { case: number });
        if (!outcome.ok) continue;
        reports += 1;
        if (outcome.result.named_suspect) named += 1;
      }
    }
    if (reports === 0) throw new Error('nobody saw a crime');
    return `${reports} reports, ${named} of them name the thief`;
  });
}

// Wit works the shift of the mission in the city hall.
shift = wit.plan([['move_to', { site: hall.id }], ['work', {}]]);

// The business of Bea: the cheapest site that is for sale.
await beat('Bea opens a business', async () => {
  // The board lists the cheapest sites first, and homes are the cheapest.
  // So the script asks for each kind of business site.
  const offers = [];
  for (const kind of BUSINESS_KINDS) {
    const board = must(await bea.act('list_property', { offer: 'sale', kind, limit: 3 }), 'list_property');
    offers.push(...(board.listings ?? []).filter((offer) => typeof offer.price === 'number'));
  }
  if (offers.length === 0) throw new Error('no site for a business is for sale');
  offers.sort((a, b) => a.price - b.price || a.site - b.site);
  const offer = offers[0];
  const fee = param('economy.permit_fee', 5_000);
  const need = offer.price + fee + 2_000;
  const have = (await bea.look()).sections.self.cash;
  if (have < need) {
    const posted = await sponsor('A new business for the town', need - have, 1);
    await earn(bea, posted.mission);
  }
  must(await bea.plan([['move_to', { site: offer.site }], ['buy_property', { max_price: offer.price }]]), 'buy_property');
  const opened = must(await bea.act('open_business', {}), 'open_business');
  return `site ${offer.site} (${opened.kind ?? offer.kind}) for ${offer.price} cents, permit ${opened.fee ?? fee} cents`;
});

await beat('Wit completes the mission of Mina', async () => {
  const worked = must(await shift, 'work');
  return `one shift for ${worked.net ?? worked.wage} cents; the mission pays 1500 cents`;
});

// ---- Day 2: the rallies ---------------------------------------------------------

if (election !== null && (await until(2, 9))) {
  for (const candidate of candidates) {
    await beat(`${candidate.name} holds a rally`, async () => {
      const result = must(
        await candidate.plan([['move_to', { site: hall.id }], ['campaign', { election, spend: 600 }]]),
        'campaign',
      );
      return `${result.heard ?? 0} residents heard it`;
    });
  }
}

// ---- Day 3: the vote, the count and the arrest ----------------------------------

let mayor = null;
if (election !== null && candidates.length > 0 && (await until(3, 6, 5))) {
  // The voters of the second candidate go first, so the lead changes.
  const second = candidates[1] ?? candidates[0];
  const first = candidates[0];
  const ballots = [
    [milo, second],
    [mina, second],
    [bea, second],
    [cora, first],
    [vera, first],
    [wit, first],
    [otto, first],
    [sly, first],
  ];
  await beat('The cast votes', async () => {
    let votes = 0;
    for (const [voter, choice] of ballots) {
      const outcome = await voter.plan([['vote', { election, candidate: choice.id }]]);
      if (outcome.ok) votes += 1;
      else console.log(`  ${voter.name} did not vote: ${why(outcome)}`);
    }
    if (votes === 0) throw new Error('no vote was cast');
    return `${votes} votes in the election ${election}`;
  });

  if (await until(3, 20, 3)) {
    await beat('The count', async () => {
      for (const candidate of candidates) {
        const offices = (await candidate.look('full')).sections.politics?.offices ?? [];
        if (offices.some((office) => office.office === 'mayor')) mayor = candidate;
      }
      if (!mayor) throw new Error('no candidate of the cast is the mayor');
      return `${mayor.name} is the mayor of ${city.name}`;
    });
  }
}

if (mayor) {
  await beat(`${mayor.name} appoints Otto`, async () => {
    must(await mayor.plan([['appoint', { post: 'police_officer', resident: otto.id, city: cityId }]]), 'appoint');
    return 'Otto is a police officer';
  });
}

await say(`the drama is over: ${done.length} steps done, ${failed.length} not done${failed.length > 0 ? ` (${failed.join('; ')})` : ''}`);
console.log(`watch a resident: ?watch=<id> with ${cast.map((member) => `${member.name}=${member.id}`).join(', ')}`);
process.exit(failed.length === 0 ? 0 : 1);
