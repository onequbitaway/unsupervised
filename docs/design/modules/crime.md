# The crime module

Crate `unsup-crime`, path `crates/modules/crime`. The module depends on the
kernel, on `unsup-life` (death by violence, names, skill levels) and on
`unsup-society` (the names of permissions, the registry of organizations).
It reads the goods, the tills and the depot prices of the economy module
through kernel primitives only.

This document gives the rules and the formulas of the module. The code is
the final word. Every number is a default. The numbers are parameters in
`crates/modules/crime/data/balance.toml` with the prefix `crime.`. The name
of a parameter is in backticks, without the prefix where the text is clear.

All divisions round down, unless the text says "rounded up". One tick is one
game minute. A chance is in basis points (bp): 10,000 bp is 100 percent.

## 1. Purpose

Crime is legal to try and the world punishes it.

- A resident can steal, burgle, rob, attack, vandalize, bribe and carry
  contraband.
- Each crime makes a **case**. The perpetrator of a case is secret.
- The world makes **evidence**: witnesses, traces, loot, officers that saw
  the crime.
- **Police** are residents with permissions. They investigate, search and
  arrest. An arrest needs grounds.
- **Courts** decide: a judge, a jury, or the engine at the deadline.
- The **engine** carries out each penalty by itself: the fine, the
  restitution, the jail time.
- The **city watch** is the engine in the role of the police, for a world
  that has no police.

Two rules hold in every part of the module:

1. Nothing depends on the goodwill of a resident. An agent that does
   nothing cannot stop a penalty.
2. Nothing waits without a limit. Each step has a deadline, and the engine
   acts at the deadline. An empty office does not stop the law.

## 2. State

| State | Kind | Realm | Content |
| --- | --- | --- | --- |
| `crime.cases` (`Cases`) | resource | global | The case registry: cases, custody, trials, bounties, deadlines |
| `crime.record` (`Record`) | component | zone | Per resident: hot value, warrants, known cases, debts, custody, trials, jury duties |
| `crime.scenes` (`Scenes`) | resource | zone | The traces of the crimes of the zone |
| `crime.law_sites` (`LawSites`) | resource | zone | The jail, the police stations and the courthouse of each city |
| `rep.notoriety` | attribute | zone | 0 and up, default 0. Each conviction raises it |
| `wanted` | condition | zone | Not blocking. A warrant names the resident |
| `detained` | condition | zone | Blocking. Under arrest |
| `jailed` | condition | zone | Blocking. It has an end tick: the end of the sentence |

The registry of the global realm is the source of truth for cases, custody
and trials. The record is what the zone of a resident must know at once. It
moves with the resident.

The module declares the shared attributes `need.health`, `need.food`,
`need.energy`, `skill.stealth`, `skill.combat`, `skill.investigation`,
`skill.law`, `skill.labor`, `rep.standing` and the condition `asleep` with
the definitions of their owners.

All times of the registry count frames. One frame is one game minute of the
calendar. A slow zone does not move a deadline. Only the end of the
condition `jailed` is a tick of the zone of the inmate; the kernel keeps it
right when the inmate changes zone.

## 3. How a verb runs

| Kind | Verbs | How it runs |
| --- | --- | --- |
| Zone, takes time | `pickpocket`, `burgle`, `rob`, `assault`, `vandalize`, `repair`, `fence`, `patrol`, `investigate` | The zone checks the request when the action starts. The work is done when the action ends |
| Zone, work at the start | `bribe`, `search`, `arrest`, `release` | The work is done when the action starts, so that the target cannot walk away. The resident is then busy for the time of the verb. The report has the result |
| Zone, at once | `report_crime` (F), `pay_fine` | The answer is in the acknowledgement |
| Zone, then global | `charge`, `rule_case`, `post_bounty` | The zone checks the powers and the cash. The registry checks the rest and makes the change. The report comes one frame later |
| Global | `jury_vote`, `case_list` (F), `case_view` (F) | The kernel sends the request to the registry. The report comes one frame later |

(F) marks a free verb.

`report_crime`, `pay_fine`, `case_list` and `case_view` work in the blocking
conditions `jailed`, `detained` and `hospitalized`. No other verb of the
module does.

A verb that names another resident (`target`, `to`) needs that resident in
the same zone, at most the range of the verb away (1 tile). The errors are
`invalid_args` (the resident itself) and `not_here`.

## 4. Crimes

A crime checks its preconditions at the start. At the end it checks them
again, rolls its outcome and makes a case. A crime that fails is still a
crime: the report is `ok` with `success: false`, and the case has
`attempt: true`.

Each chance is put between `chance_min_bp` (500) and `chance_max_bp` (9,500).
`level` is the level of a skill: `floor(sqrt(experience / 100))`. It is night
from `night_from_hour` (22) to `night_to_hour` (6).

A crime that worked gives `xp_success` (12) experience, one that failed
gives `xp_fail` (4): in `skill.stealth` for `pickpocket`, `burgle` and
`vandalize`, in `skill.combat` for `rob` and `assault`.

Nobody can use `pickpocket`, `rob` or `assault` against a resident that is
`detained` or `jailed` (`not_allowed`). The guards protect it.

### 4.1 `pickpocket`

Argument: `target`. Time: `pickpocket_ticks` (3).

```
chance = pickpocket_base_bp (4500)
       + level(stealth of the thief) * stealth_level_bp (600)
       - level(investigation of the victim) * pickpocket_alert_bp (400)
       + night_bonus_bp (1500)              at night
       + pickpocket_asleep_bonus_bp (2500)  when the victim sleeps
cash   = min(cash of the victim * pickpocket_pct (25) / 100, pickpocket_max (5000))
```

Kind of the case: `theft`. Result: `success`, `case`, `witnesses`, `target`,
`cash`. A victim that walked away before the end gives `not_here`, and no
case.

### 4.2 `burgle`

Argument: `item` (optional). Time: `burgle_ticks` (20). The resident stands
in a site that is not its own: it is not the owner, and it is not a member
of the owner with `site.manage`.

```
chance = burgle_base_bp (3500)
       + level(stealth) * stealth_level_bp (600)
       + night_bonus_bp (1500)                       at night
       + burgle_empty_bonus_bp (2000)                when nobody else in the site is awake
       - occupants * burgle_occupant_malus_bp (1500) else
       + burgle_tool_bonus_bp (1500)                 when the burglar carries an item with the tag `tool`
       - burgle_public_malus_bp (2000)               in a site with the tag `public`
cash   = till * burgle_till_pct (50) / 100
goods  = at most burgle_units_max (10) units
```

`occupants` is the number of other residents inside the site that do not
sleep. The burglar takes the goods with the highest `base_price` first, then
the lower item id. With `item`, it takes only that good.

Kind of the case: `burglary`. The victim is the owner of the site. Result:
`success`, `case`, `witnesses`, `site`, `cash`, `goods`, `value`. Errors:
`not_here` (not in a site), `not_allowed` (your site), `not_found` (no such
item).

### 4.3 `rob`

Argument: `target`. Time: `rob_ticks` (5).

```
chance = rob_base_bp (5000)
       + (level(combat of the robber) - level(combat of the victim)) * combat_level_bp (800)
       + force_asleep_bonus_bp (2000)   when the victim sleeps
cash   = cash of the victim * rob_cash_pct (60) / 100
goods  = at most rob_units_max (5) units, the highest `base_price` first
```

- Success: the victim loses `rob_injury` (100) health. Its action stops.
- Failure: the robber loses `rob_fail_injury` (60) health, but keeps 1
  point. A failed attack never kills the attacker.

Kind of the case: `robbery`, or `homicide` when the victim dies. Result:
`success`, `case`, `witnesses`, `target`, `cash`, `goods`, `value`,
`injury`, `fatal`, `hurt`.

### 4.4 `assault`

Argument: `target`. Time: `assault_ticks` (3).

```
chance = assault_base_bp (6000)
       + (level(combat of the attacker) - level(combat of the victim)) * combat_level_bp (800)
       + force_asleep_bonus_bp (2000)   when the victim sleeps
damage = assault_damage (150) + level(combat of the attacker) * assault_level_damage (25)
```

- Success: the victim loses `damage` health. Its action stops.
- Failure: the attacker loses `assault_fail_injury` (60) health, but keeps 1
  point.

The loss of the victim, for a restitution, is the lost health times the
hospital fee `life.heal_fee_per_point`.

Kind of the case: `assault`, or `homicide` when the victim dies. Result:
`success`, `case`, `witnesses`, `target`, `damage`, `fatal`, `hurt`.

**Death by violence.** A victim whose health reaches 0 dies through
`unsup_life::die(realm, idx, "violence")`, in the tick of the attack. The
life module emits `life.resident_died` with the cause `violence` and gives
the estate to the heir. The loot of a robbery is with the robber before
that. Violence is abstract: the game reports a loss of health and has no
other text.

### 4.5 `vandalize` and `repair`

`vandalize` has no argument. Time: `vandalize_ticks` (10). The resident
stands in a site that is not its own.

```
chance    = vandalize_base_bp (6000) + level(stealth) * stealth_level_bp (600)
          + night_bonus_bp (1500) at night
destroyed = for each good in the stock: stock * vandalize_stock_pct (25) / 100, rounded up
whole     = value + damage                       (the value of the site without damage)
room      = whole * vandalize_damage_max_bp (1000) / 10000 - damage
loss      = min(whole * vandalize_value_bp (50) / 10000, room, value)
```

- The destroyed goods leave the world through the sink `destruction`.
- The site attribute `value` falls by `loss`. The site attribute `damage`
  grows by `loss`. The economy module reads `value` for the property tax
  and for the price of a site of a government.
- The loss of the victim is the normal price of the destroyed goods plus
  `loss`.

Kind of the case: `vandalism`. Result: `success`, `case`, `witnesses`,
`site`, `destroyed`, `damage`, `value`. Errors: `not_here`, `not_allowed`
(your site), `conflict` (the stock is empty and the damage is at its
limit).

`repair` has the argument `amount` (optional, cents of value; without it,
all the damage). Time: `repair_ticks` (30). Each resident can repair the
site where it stands.

```
repaired = min(amount, damage)
cost     = repaired * repair_cost_bp (10000) / 10000, rounded up
```

The cost leaves the world through the money sink `repair.materials`. Then
`value` grows and `damage` falls by `repaired`. The repairer gets
`repair_xp` (6) experience in `skill.labor`. Result: `site`, `repaired`,
`cost`, `damage`. Errors: `not_here`, `conflict` (no damage), `invalid_args`
(`amount` below 1), `insufficient` (not enough cash).

With a cost of 100 percent, a resident that damages a site of the city to
buy it for less pays the same amount to repair it.

### 4.6 `defraud`

The first verb table of the game design had `defraud`. This module does not
have it, and the table does not have it any more. A fraud is a promise that a resident breaks. A resident can break a
promise without a verb, and the engine cannot know that a promise was made
unless it is a contract. A contract of the society module already punishes
a breach. A verb `defraud` would only let a resident tell the engine about
its own crime.

### 4.7 Contraband

It is a crime to carry goods with the tag `contraband` while the policy
parameter `contraband_illegal` is 1. The crime has no verb. It becomes a
case of the kind `smuggling` when somebody finds the goods:

- an officer with `search` (section 7.3), or
- the city watch in a public building (section 10).

The goods go to the police station. With `contraband_illegal` at 0, nobody
takes them.

## 5. Hot value and `fence`

### 5.1 Hot value

The ledger does not know where a coin came from. So the record of a thief
counts the stolen cents and the stolen units of each good, by case. This is
the **hot** value.

- Stolen cash and stolen goods are hot from the moment of the theft.
- Value that leaves the hands of the thief is no longer hot: cash that it
  spends or puts in the bank, goods that it gives, drops, eats or sells.
  The count of a good is never above what the resident holds. The oldest
  lot shrinks first. The module corrects the count in each game hour and
  before each search, sale to a fence and payment to a court.
- A lot is cold after `hot_ticks` (4,320) frames.
- A lot is gone when the trial of its case ends.
- A record has at most `hot_lots_max` (16) lots.

A search finds only hot value (section 7.3). The mark is on the thief, not
on the goods: a resident that gets stolen goods from the thief holds clean
goods. See section 19.

### 5.2 `fence`

Arguments: `item`, `qty` (optional; without it, every hot unit of the good
that the resident still holds). Time: `fence_ticks` (10).

```
reference  = economy.depot_floor.<item>                 when the depot trades the good
           = base_price * fence_base_bp (0) / 10000     else
unit price = reference * fence_bp (6000) / 10000
```

The goods leave the world through the goods sink `export.sale`. The money
enters the world through the money faucet `export.sale`. These are the sink
and the faucet of a sale to the depot in the economy module. The module adds
no new faucet.

- The fence buys only hot units. The depot buys clean goods for more.
- The fence pays 60 percent of the depot price. So no chain through the
  fence makes more money than the depot does. Keep `fence_bp` at or below
  10,000.
- The depot does not trade contraband, and with `fence_base_bp` at 0 the
  fence does not buy it. A system buyer for contraband would be a new
  source of money. A law can open one.
- The cash of a sale is clean. The event `crime.goods_fenced` has no actor.

Result: `item`, `qty`, `unit_price`, `paid`. Errors: `not_found` (no such
item), `invalid_args` (`qty` below 1), `not_here` (in a site with the tag
`public`), `insufficient` (no hot unit of the good), `not_allowed` (the
price is 0).

## 6. Cases and evidence

### 6.1 The case

Each crime makes a case in the registry. The zone of the crime gives the id.

| Field | Content |
| --- | --- |
| `kind` | `theft`, `burglary`, `robbery`, `assault`, `homicide`, `vandalism`, `bribery`, `smuggling` |
| `city`, `zone`, `pos`, `site` | The place |
| `victim` | The holder that lost value: a resident, or the owner of the site |
| `value` | Cents that the victim lost |
| `attempt` | True when the crime failed |
| `witnesses` | Residents that noticed the crime |
| `evidence` | Points by source |
| `status` | See the table below |

The perpetrator is in the case, but nobody can read it. Verbs, events and
notices name the **suspect** only when the status is `wanted`, `held`,
`trial`, `convicted` or `acquitted`.

| Status | Meaning | Next |
| --- | --- | --- |
| `open` | The evidence is below the warrant threshold | `wanted`, `held`, `trial`, `cold`, `moot` |
| `wanted` | A warrant names the suspect | `held`, `cold`, `moot` |
| `held` | The suspect is under arrest and not charged in this case | `trial`, `wanted`, `open`, `moot` |
| `trial` | A trial runs | `convicted`, `acquitted`, `moot` |
| `convicted`, `acquitted` | The trial ended | closed |
| `cold` | No charge in `case_ttl_ticks` (20,160) frames after the crime. The warrant ends | closed |
| `moot` | The perpetrator left the world | closed |

A closed case leaves the registry after `case_keep_ticks` (2,880) frames. An
acquitted resident is not wanted again for the same case.

### 6.2 Witnesses

At the end of a crime, the residents around its place can notice it.

- A resident that is awake notices a crime at most `witness_radius` (8)
  tiles away.
- A resident that sleeps notices only a crime against itself.
- An officer on patrol notices a crime at most `patrol_radius` (10) tiles
  away (section 7.1).
- At most `witness_max` (8) residents notice one crime: the victim first,
  then the nearest, then the lower id.

Each of them rolls one time to recognize the perpetrator:

```
chance = identify_base_bp (6000)
       - level(stealth of the perpetrator) * identify_stealth_bp (700)
       - distance in tiles * identify_distance_bp (300)
       - identify_night_malus_bp (2000)    at night
       + identify_fail_bonus_bp (2500)     when the crime failed
       + identify_force_bonus_bp (3000)    for the victim of `rob` or `assault`
       - identify_asleep_malus_bp (4000)   when the resident sleeps
```

For an officer on patrol the chance is
`patrol_detect_bp (8500) - level(stealth) * patrol_stealth_bp (400)`. These
chances are between 0 and 10,000 bp.

Each resident that noticed the crime gets the notice `crime.witnessed`
(`crime.patrol_alert` on patrol) with `case`, `kind`, `role` (`victim` or
`witness`), `pos`, and `suspect` only when it recognized the perpetrator.
The record keeps the last `known_max` (8) such cases.

**The victim gets a notice in each case.** The owner of a burgled or
damaged site gets it wherever it is, also in another zone.

A bribe and contraband have no witnesses.

### 6.3 `report_crime` (free)

Argument: `case`.

A resident can report only a case in its record: it was the victim, it saw
the crime, or it got the bribe. The report names the perpetrator only when
the resident recognized it. So a resident cannot put the blame on somebody
that it did not see. A resident reports a case one time.

Result: `case`, `reported`, `named_suspect`. Errors: `not_found` (the
resident does not know the case), `conflict` (reported already).

### 6.4 Evidence

Evidence is a number of points against the perpetrator of a case.

| Source | Points | Rule |
| --- | --- | --- |
| `testimony` | `evidence_victim` (40) | A victim, or the receiver of a bribe, that recognized the perpetrator reports the case |
| `testimony` | `evidence_witness` (25) | A witness that recognized the perpetrator reports the case. Each resident counts one time |
| `investigation` | up to `evidence_investigate` (30) | `investigate` reads the trace. The sum of a case is at most `evidence_investigate_max` (60) |
| `search` | `evidence_search` (50) | A search finds hot value of the case on a resident. One time |
| `caught` | `evidence_caught` (100) | A resident with `law.arrest` or `law.search` in the city saw who did it, or found contraband. One time |
| `watch` | `evidence_watch` (60) | The city watch saw the crime, or found contraband. One time |

A report without a name gives no points. It sets `reported`.

Thresholds (policy parameters, for the nation or for a city):

| Parameter | Points | Effect |
| --- | --- | --- |
| `charge_evidence` | 20 | An officer can charge a suspect in custody |
| `warrant_evidence` | 40 | The case gets a warrant. The engine charges a suspect in custody |
| `convict_evidence` | 60 | The engine convicts at the deadline of a trial |

So the report of a victim that saw the robber gives a warrant. Two
witnesses give a warrant. A witness and a fresh trace give a warrant. The
engine convicts by itself with a victim and a witness, with the loot and
one more source, with an officer that saw the crime, or with the watch.

### 6.5 Warrants

When the evidence of an `open` case reaches `warrant_evidence`:

- The case is `wanted`. The suspect gets the condition `wanted` and the
  notice `crime.wanted`. Viewers see `crime.warrant_issued` without a name.
- When the suspect is under arrest already, the case is `held` instead.

The condition `wanted` ends when the last warrant of the resident ends: an
arrest, a charge, a verdict, or a case that goes cold. Each resident can see
the condition with `inspect`, and the docket names the suspect of a case
with a warrant.

One function of this module sets the condition (`enforce::sync_wanted`). It
keeps this rule after each change of a warrant and of a custody: a resident
is `wanted` exactly while a warrant names it and it is not under arrest. No
other module, no verb and no administrator command sets the condition. A
warrant names only the resident that did the crime (section 19). So a
resident cannot make another resident wanted, and each arrest that the
condition allows has a case behind it.

### 6.6 The docket: `case_list` and `case_view` (free)

`case_list { status }` lists the cases of the city where the resident is,
the newest first, at most `case_list_max` (10); `more` counts the rest.
Without `status`, it lists the cases that are not closed. Each entry has
`case`, `kind`, `status`, `age`, `zone`, `pos`, `site`, `evidence`, and
`suspect` and `decided_in` when they apply. Error: `invalid_args` (no such
status).

`case_view { case }` gives one case with `value`, `attempt`, `witnesses`,
`reported`, `victim`, `evidence_by_source`, `thresholds`, `trial`
(`charged_by`, `jury`, `jurors`, `votes`, and `your_vote` for a juror) and
`sentence`. Error: `not_found`.

A bribe is not on the docket until somebody reports it or the first
evidence exists.

## 7. Police

An officer is a resident with a power of the law **in the city where it
acts**. The society module writes a power to the memberships of a resident
one time for each government that it comes from: `law.arrest@100` is
`law.arrest` by the power of the city 0. The module accepts the power from
the government of the city or from the nation, and checks it with
`realm.has_perm`. An officer of one city has no power in another city.

| Verb | Needs |
| --- | --- |
| `patrol`, `investigate` | `law.arrest` or `law.search` |
| `search` | `law.search` |
| `arrest`, `charge` | `law.arrest` |
| `release` | `law.arrest` for a suspect, `law.jail` for an inmate |

Each verb answers `not_allowed` to a resident without the power.

### 7.1 `patrol`

Argument: `ticks` (optional, 1 to `patrol_max_ticks` (240); the default is
`patrol_ticks` (60)).

While the action runs, the officer notices each crime at most
`patrol_radius` (10) tiles away and recognizes the perpetrator with the
chance of section 6.2. A crime that an officer saw has 100 evidence points,
so the perpetrator is wanted one frame later, and an officer with
`law.arrest` can arrest it at once.

At the end, the officer gets `patrol_xp_per_hour` (6) experience in
`skill.investigation` for each hour. Result: `ticks`, `wanted` (the wanted
residents at most `patrol_radius` tiles away, at most `patrol_list_max`
(5)), `more_wanted`.

### 7.2 `investigate`

Argument: `case` (optional). Time: `investigate_ticks` (30).

Each crime leaves a trace at its place, also a bribe. A trace lasts
`trace_ticks` (2,880) frames. A zone keeps at most `traces_max` (256).

The officer reads the newest trace at most `investigate_radius` (6) tiles
away that it did not read yet, or the trace of `case`.

```
chance = investigate_base_bp (5000)
       + level(investigation of the officer) * investigate_level_bp (700)
       - level(stealth of the perpetrator at the time of the crime) * investigate_stealth_bp (500)
points = evidence_investigate (30) * (trace_ticks - age) / trace_ticks, at least 1
```

- Success: the case gets `points`. The officer gets `investigate_xp` (10)
  experience. An officer reads a trace one time. Another officer can read
  it too.
- Failure: half of the experience. The officer can try again; the trace is
  older then.

The result has `case`, `kind`, `found` and `evidence`. It does not name the
perpetrator. Error: `not_found` (no trace to read here).

### 7.3 `search`

Argument: `target`. Time: `search_ticks` (5). The search happens at the
start of the action.

- The hot cash and the hot goods of the target go to the police station.
  Each case of that loot gets `evidence_search` (50) points.
- With `contraband_illegal` at 1, each good with the tag `contraband` goes
  to the police station too, and a case of the kind `smuggling` starts with
  100 points.
- For `caught_ticks` (120) frames, this officer has grounds to arrest the
  target for those cases.

"The police station" is the one of the city that is nearest: cash goes into
its till and goods into its stock (`Holder::Site`). In another zone they
arrive one frame later. A city without a police station sends both to its
treasury. The economy module moves the tills of the sites of a city to the
treasury at the start of each day.

The target gets the notice `crime.searched`. The event `crime.search_made`
names the officer and not the target. The officer gets `search_xp` (4)
experience. Result: `target`, `found`, `money`, `goods`, `contraband`,
`cases`, `smuggling_case`.

A search needs no grounds. A search that finds nothing costs the officer
its time.

### 7.4 `arrest`

Argument: `target`. Time: `arrest_ticks` (5). The arrest happens at the
start of the action.

**Grounds.** The engine refuses an arrest (`not_allowed`) unless one of
these is true:

1. A warrant names the target: a case with evidence at or above
   `warrant_evidence`.
2. The target has the condition `wanted` for another reason.
3. This officer caught the target in the act: it saw the crime and
   recognized the perpetrator, or its search found loot or contraband, at
   most `caught_ticks` (120) frames ago.

The engine then:

1. removes the planned actions of the target and stops its action
   (`realm.clear_queue`, `realm.interrupt`);
2. sets `detained` and clears `wanted`;
3. moves the target to the jail of the city, also in another zone. A city
   without a jail holds the target where it stands;
4. tells the target (`crime.arrested`) and the registry;
5. emits `crime.arrest_made { officer, target, case, cases, city, watch }`.

The officer gets `arrest_xp` (8) experience in `skill.law`. Result:
`target`, `cases`, `jail`. Other errors: `conflict` (the target is in
custody already).

### 7.5 `charge`

Arguments: `target`, `case`. The officer can be at each place of the city.

The registry starts the trial when the target is under arrest in this city,
the case is `open`, `wanted` or `held`, and the evidence against the target
in this case is at least `charge_evidence` (20). The officer gets `court_xp`
(5) experience. Result: `case`, `target`, `decided_in`.

Errors: `not_found` (not under arrest in this city; no such case of this
city), `conflict` (the case is on trial or closed), `insufficient` (too
little evidence). The answer is `insufficient` also for a target that is
not the perpetrator, so the verb does not tell who did it.

**When the officer does not charge.** `hold_ticks` (720) frames after the
arrest, the engine charges each case of the suspect with evidence at or
above `warrant_evidence`. When there is none, the suspect is free
(`crime.suspect_released`, reason `hold_expired`).

### 7.6 `release`

Argument: `target`. Time: `arrest_ticks` (5). The officer stands next to
the target, for example in the jail.

- A suspect that is `detained` and not charged: the officer needs
  `law.arrest`. The suspect is free. Its cases are open again, and those
  with enough evidence get their warrant back.
- A suspect that waits for a verdict: `conflict`. Only the court ends a
  trial.
- An inmate that is `jailed`: the officer needs `law.jail`. The rest of the
  sentence is gone.

Other errors: `conflict` (the target is not in custody), `not_here`.

## 8. Courts

### 8.1 The trial

Each charge is a trial with a deadline: `trial_ticks` (720) frames after the
charge. The defendant stays `detained`. It gets the notice `crime.charged`.

1. **The judge.** A resident with `law.judge` in the city of the case can
   end the trial with `rule_case` at each moment before the verdict.
2. **The jury.** When no member of the government of the city, and no
   member of the nation, has `law.judge`, the engine calls a jury in the
   frame of the charge. When one has, the engine calls the jury after
   `trial_bench_ticks` (240) frames without a ruling.
3. **The deadline.** At the deadline the engine decides. When at least
   `jury_quorum` (3) jurors voted, the votes decide: guilty when more
   jurors voted for guilty than for not guilty. Else the evidence decides:
   guilty when it is at or above `convict_evidence` (60).

So a trial ends at its deadline at the latest, with or without a judge,
with or without jurors.

### 8.2 `rule_case`

Arguments: `case`, `guilty`, and optional `fine`, `jail_ticks`,
`restitution`.

Limits of a conviction (policy parameters for each kind of crime):

```
0 <= fine        <= crime.<kind>.fine_max
0 <= jail_ticks  <= crime.<kind>.jail_max_ticks
0 <= restitution <= the loss of the victim
```

A number that is absent has the value of the engine (section 9.1). An
acquittal has no numbers. The judge gets `court_xp` (5) experience in
`skill.law`.

Errors: `not_allowed` (no `law.judge` in the city of the case; your own
case), `not_found`, `conflict` (the case is not on trial), `invalid_args`
(a number outside its limits).

### 8.3 The jury and `jury_vote`

The engine draws up to `jury_size` (5) residents of the city with the
random stream of the global realm, in at most `jury_draws` (64) draws. Each
draw is `Directory::sample_alive`: one of the residents that live now, each
with the same chance. A resident that left the world costs no draw, and the
draw does not depend on how the kernel numbers the residents. These
residents do not sit on a jury: the defendant, the victim, the officer that
charged, residents of the owner of the defendant, and residents under
arrest. A draw that names one of them, a resident of another city or a
juror of this trial is lost. A city with too few residents has a smaller
jury, or none.

A juror gets the notice `crime.jury_duty` and reads the case with
`case_view`. `jury_vote { case, guilty }` gives its vote. It can vote from
each place. When each juror voted, the trial ends at once. A juror gets
`court_xp` (5) experience.

Errors: `not_found`, `conflict` (the case is not on trial; you voted
already), `not_allowed` (not a juror of this trial).

## 9. Sentences

### 9.1 The sentence of the engine

When a jury or the evidence convicts, and for each number that a judge
leaves out:

```
fine        = crime.<kind>.fine
jail_ticks  = crime.<kind>.jail_ticks
restitution = the loss of the victim (`value` of the case)
```

| Kind | Fine | Jail | Greatest fine | Greatest jail | Notoriety |
| --- | --- | --- | --- | --- | --- |
| `theft` | 3,000 | 240 | 12,000 | 1,440 | 20 |
| `burglary` | 8,000 | 720 | 30,000 | 4,320 | 40 |
| `robbery` | 10,000 | 1,440 | 40,000 | 7,200 | 60 |
| `assault` | 6,000 | 720 | 25,000 | 4,320 | 50 |
| `homicide` | 50,000 | 14,400 | 200,000 | 40,320 | 200 |
| `vandalism` | 4,000 | 240 | 16,000 | 1,440 | 15 |
| `bribery` | 5,000 | 480 | 50,000 | 2,880 | 30 |
| `smuggling` | 4,000 | 240 | 20,000 | 1,440 | 15 |

### 9.2 What the engine does at a verdict

The registry sends the verdict to the zone of the defendant, wherever it
is. That zone does the work one frame later.

For a conviction:

1. `rep.notoriety` grows by `crime.<kind>.notoriety`. `rep.standing` falls
   by `conviction_standing` (60).
2. The convict owes the restitution to the victim and the fine to the
   treasury of the city of the case.
3. The engine takes the cash that the convict has on it now: first for the
   restitution, then for the fine. A victim that is a resident gets the
   notice `crime.restitution`.
4. With jail time, the convict gets the condition `jailed` until
   `now + jail_ticks`, in the jail. A convict that serves a sentence
   already serves the new one after it. A convict that is not in the jail
   (an officer released it during the trial) is moved there.

For each verdict:

5. What is left of the loot of the case is no longer hot.
6. When no other trial holds the resident, `detained` ends. A resident
   without jail time is free (`crime.suspect_released`, reason `acquitted`
   or `no_jail_time`).

When an officer charged only a part of the cases, the engine charges the
other cases with evidence at or above `warrant_evidence` before the custody
ends.

### 9.3 Debts and `pay_fine`

A debt is due `fine_due_ticks` (1,440) frames after the verdict.

`pay_fine { amount }` pays from the cash of the convict, the restitution
first. Without `amount`, it pays all that the convict owes, or all its
cash. It works in jail. A gift (`give`) reaches an inmate, so another
resident can help. Result: `paid`, `owed`. Errors: `conflict` (no debt),
`insufficient` (no cash), `invalid_args` (`amount` below 1).

**Where the money goes.** A fine goes to the treasury of the city of the
case. A restitution goes to the victim: a resident, an organization, or a
site without an owner.

- A site is reached in two ways: it is a site of the zone where the
  convict is, or the debt names the zone of the site and the world has
  that zone. The money then goes to the site, by wire when the site is in
  another zone.
- Every other site cannot be reached: the site of the zone of the convict
  is gone, the debt names a zone that the world does not have, or the debt
  names no zone and the site is not in the zone of the convict. That
  restitution goes to a city treasury: that of the city of the zone that
  the debt names, when the world has that zone, else that of the city
  where the convict is. The ledger label stays `crime.restitution`.
- So a payment fails only when the convict has no cash. A debt never
  becomes jail time because its payee cannot be reached.

**A debt that is not paid in time becomes jail time.** When the time is
over, the engine takes the cash that the convict has. Then:

```
jail_ticks = owed / fine_cents_per_jail_tick (10), rounded up,
             at most fine_jail_max_ticks (2880)
```

The debt is gone. The engine stops the action of the convict, moves it to
the jail of the city where it is, and sets `jailed`. The victim does not
get the part of the restitution that jail replaced.

### 9.4 Jail

`detained` and `jailed` are blocking conditions. In both, a resident can
still use:

| Module | Verbs |
| --- | --- |
| kernel | `wait`, `cancel`, `set_routines` |
| life | `eat`, `use_item`, `sleep`, `rest`, `write_will`, `inspect` |
| society | `say`, `message`, `org_say`, `contract_view` |
| economy | `list_jobs`, `list_property`, `list_recipes` |
| finance | `transfer`, `loan_repay`, `cancel_shares`, `shares_book` |
| missions | `mission_list`, `leaderboard` |
| crime | `report_crime`, `pay_fine`, `case_list`, `case_view` |

These are 26 verbs. Each module lists the two conditions in `allowed_in`
of its verbs. No other verb works: an inmate cannot walk, trade, work,
give, vote or commit a crime. The scenario `s02_heist` of
`crates/scenarios` asks every verb of the catalogue and fails when the
list changes. The finance verbs use only the bank account: an inmate can
send money to a friend, who brings it as cash for the fine.

**The jail keeps an inmate alive.** An inmate cannot buy food or walk to a
bed. Once in each game hour, in each zone, for each resident that is
`detained` or `jailed`:

```
food   += jail_meal_food (300)     when food   < jail_meal_below (500)
energy += jail_rest_energy (150)   when energy < jail_rest_below (250)
```

The meal is not a good of the ledger. The pass runs after the hourly pass
of the life module, so an inmate never starves. It does not depend on the
reflexes of the life module.

The sentence ends at its tick: the kernel ends the condition, the inmate
gets `crime.released` and viewers see `crime.sentence_served`. The inmate
walks out of the jail by itself.

## 10. The city watch

The watch is the engine in the role of the police. It needs no resident. It
is weak by default, and each of its numbers is a policy parameter.

| What | Parameter | Default |
| --- | --- | --- |
| It sees a crime that no officer saw. The case gets `evidence_watch` points: a warrant | `watch_detect_bp` | 1,000 (10 percent) |
| Once in each game hour, it arrests a wanted resident that stands in a site with the tag `public` | `watch_arrest_bp` | 2,500 (25 percent) |
| In the same round, it takes the contraband of a resident in such a site and makes a `smuggling` case | `watch_contraband_bp` | 200 (2 percent) |
| The points of a crime that the watch saw | `evidence_watch` | 60 |

An arrest by the watch has `officer: null` in `crime.arrest_made`. The
registry charges a suspect of the watch in the frame of the arrest. With no
judge and no jury, the evidence decides at the deadline: 60 points convict.

So in a world with no police, a criminal that the watch saw is wanted, is
taken when it enters a public building (the shelter, the station, the
hospital, the bank), is tried by the engine and serves its sentence. A
wanted resident that stays out of public buildings is safe from the watch,
until the case is cold.

Residents with powers are much stronger: an officer on patrol sees 85
percent of the crimes near it, an investigation and a search add evidence
to each case, and an officer arrests at each place.

## 11. `bribe` and `post_bounty`

### 11.1 `bribe`

Arguments: `to`, `amount`, `note` (optional, at most `bribe_note_max` (200)
bytes). Time: `bribe_ticks` (1). The cash moves at the start.

A gift with `give` is public. A bribe is secret: no event, no witness, no
chance for the watch. Only the receiver gets a notice
(`crime.bribe_received` with `case`, `from`, `amount` and the note as
untrusted text).

A bribe is a crime of the giver, of the kind `bribery`. The case is hidden
until:

- the receiver reports it (`report_crime`): 40 points, a warrant; or
- officers read the trace at the place (`investigate`).

The receiver keeps the money in each case. The engine does not enforce what
the bribe was for.

Errors: `invalid_args` (yourself; `amount` below 1; a bad note), `not_here`,
`insufficient`.

### 11.2 `post_bounty`

Arguments: `target`, `amount`, `ticks` (optional, 1 to `bounty_max_ticks`
(20,160); the default is `bounty_ticks` (4,320)).

The cash goes to `Holder::Escrow(bounty id)` in the global realm. When the
event `crime.arrest_made` names the target and an officer, the escrow goes
to that officer. The city watch earns nothing: the bounty stays open. When
the time is over, or when the target leaves the world, the money goes back
to the poster.

A bounty gives no grounds for an arrest. It pays only for a suspect that
the police can lawfully take.

The contract engine of the society module pays only a party that signed
the contract. A bounty has no such party, so the module keeps its own
escrow. A resident that wants to pay one named officer can also use a
contract with a `pay_on_event` clause on `crime.arrest_made`.

Result: `bounty`, `target`, `amount`, `open_for`. Errors: `invalid_args`
(yourself; `amount` below `bounty_min` (500); bad `ticks`), `insufficient`,
`not_found` (no such resident), `conflict` (more than `bounties_open_max`
(3) open bounties). A refused request gives the cash back.

## 12. Events

All events are public. The drama score of an event is a parameter.

**A public event never names a hidden perpetrator.** The events of a crime
have no actor, and their data has no field with the perpetrator. The first
event that names the resident is `crime.arrest_made`.

| Kind | Data | Actor | Drama |
| --- | --- | --- | --- |
| `crime.theft_committed` | `case`, `kind` (`theft`, `burglary`, `robbery`), `city`, `site`, `value`, `attempt` | none | `crime.<kind>.drama`: 45, 55, 60 |
| `crime.assault_committed` | `case`, `kind` (`assault`, `homicide`), `city`, `victim`, `fatal`, `attempt` | none | 60, 85 |
| `crime.vandalism_committed` | `case`, `city`, `site`, `damage`, `attempt` | none | 42 |
| `crime.contraband_seized` | `case`, `city`, `value` | none | 45 |
| `crime.goods_fenced` | `city`, `value` | none | 20 |
| `crime.warrant_issued` | `case`, `kind`, `city` | none | 40 |
| `crime.search_made` | `officer`, `city`, `found` | the officer | 20 |
| `crime.arrest_made` | `officer` (or null), `target`, `case` (or null), `cases`, `city`, `watch` | the officer | 55 |
| `crime.suspect_charged` | `case`, `kind`, `city`, `target`, `officer` (or null), `decided_in` | the target | 45 |
| `crime.jury_summoned` | `case`, `kind`, `city`, `jurors` | none | 35 |
| `crime.verdict_given` | `case`, `kind`, `city`, `target`, `guilty`, `by` (`judge`, `jury`, `evidence`), `judge`, `fine`, `jail_ticks`, `restitution`, `votes_guilty`, `votes_not_guilty` | the target | 65 |
| `crime.suspect_released` | `target`, `reason` (`released`, `paroled`, `hold_expired`, `acquitted`, `no_jail_time`, `no_case`), `by` | the target | 30 |
| `crime.sentence_served` | `resident`, `free` | the resident | 30 |
| `crime.fine_paid` | `resident`, `amount`, `owed` | the resident | 10 |
| `crime.fine_defaulted` | `resident`, `case`, `owed`, `jail_ticks` | the resident | 40 |
| `crime.site_repaired` | `site`, `by`, `value`, `cost` | the repairer | 10 |
| `crime.bounty_posted` | `bounty`, `by`, `target`, `amount`, `open_for` | the poster | 40 |
| `crime.bounty_paid` | `bounty`, `target`, `officer`, `amount` | the officer | 50 |
| `crime.bounty_expired` | `bounty`, `target`, `amount`, `reason` (`expired`, `target_gone`) | none | 10 |

- A crime that failed has the drama `drama.attempt` (30).
- A crime with a loss of `heist_value` (50,000) or more has the drama
  `drama.heist` (75).
- A robbery that kills is an `crime.assault_committed` with the kind
  `homicide`. The life module also emits `life.resident_died`.
- A bribe has no event.

The society module can wait for each of these events in a contract.
`crime.arrest_made` has the fields `officer`, `target` and `case`.

## 13. Notices

| Kind | To | Data |
| --- | --- | --- |
| `crime.witnessed` | A victim or a witness | `case`, `kind`, `role`, `pos`, `site`, `value` (victim), `attempt`, `suspect`, `can_arrest` |
| `crime.patrol_alert` | An officer on patrol | The same data |
| `crime.bribe_received` | The receiver of a bribe | `case`, `from`, `amount`, `note` (untrusted) |
| `crime.wanted` | The suspect | `case`, `kind`, `wanted` |
| `crime.searched` | The target of a search | The result of the search |
| `crime.arrested` | The suspect | `officer`, `cases`, `jail` |
| `crime.charged` | The defendant | `case`, `kind`, `decided_in` |
| `crime.jury_duty` | A juror | `case`, `kind`, `vote_in` |
| `crime.verdict` | The defendant | `case`, `kind`, `guilty`, `fine`, `restitution`, `jail_ticks`, `paid`, `owed`, `jailed_until` |
| `crime.verdict` | A victim that is a resident | `case`, `kind`, `guilty`, `defendant`, `restitution` |
| `crime.restitution` | A victim that is a resident | `case`, `amount` |
| `crime.fine_defaulted` | The convict | `case`, `owed`, `jail_ticks` |
| `crime.released` | A resident that leaves custody | `reason`, `free` |
| `crime.bounty` | The poster and the officer | `bounty`, `target`, and `officer`, `amount` or `refunded`, `reason` |

The only text of a resident in a notice is the note of a bribe. It is an
`Untrusted` value.

## 14. The observation section `law`

The section is absent when it has nothing to say. Times are ticks from now.

| Field | Content |
| --- | --- |
| `warrants` | `case`, `kind` of each warrant that names the resident |
| `custody` | `state` (`detained`, `jailed`), `site`, `arrested_by`, and for jail `until` (tick) and `ticks_left` |
| `trials` | `case`, `kind`, `decided_in` of each trial of the resident |
| `jury_duty` | `case`, `kind`, `vote_in` of each trial in which the resident must vote |
| `debts` | `owed`, `due_in` |
| `hot` | `money` and `items`: the stolen value that the resident holds |
| `known_cases` | `case`, `kind`, `role`, `suspect`, `reported` of the cases that the resident can report |
| `powers_here` | The powers of the law that the resident holds in this city |

With brief detail, `warrants` and `known_cases` have the newest
`observe_list_max` (3) entries; `more_warrants` and `more_known_cases` count
the rest. A section with five warrants, or with five known cases, has less
than 400 bytes with brief detail.

## 15. Faucets, sinks and ledger labels

The module makes no money and no goods, except through `fence`.

| Name | Kind | What | When |
| --- | --- | --- | --- |
| `destruction` | sink | goods | `vandalize` destroys stock. New in this module; the economy model names it |
| `repair.materials` | sink | money | `repair`. New in this module |
| `export.sale` | sink | goods | `fence`. The sink of the economy module |
| `export.sale` | faucet | money | `fence`. The faucet of the economy module |

Each other movement is a transfer:

| Label | Movement |
| --- | --- |
| `crime.loot` | Stolen cash and goods to the thief |
| `crime.bribe` | The cash of a bribe |
| `crime.seized` | Hot value and contraband to the police station or the treasury |
| `crime.fine` | A fine to the city treasury |
| `crime.restitution` | A restitution to the victim |
| `crime.bounty` | Money of a bounty: into its escrow, to the officer, back to the poster |
| `crime.hold`, `crime.return` | Cash on its way to the registry and back |

## 16. Parameters

All are in `data/balance.toml`. The sections above give the default of each
parameter that a formula uses. These are policy parameters, which a law can
change for the nation or for one city:

- Evidence: `evidence_watch`, `warrant_evidence`, `charge_evidence`,
  `convict_evidence`.
- Police: `hold_ticks`, `contraband_illegal`.
- Courts: `trial_ticks`, `trial_bench_ticks`, `jury_size`, `jury_quorum`,
  `fine_due_ticks`, `fine_cents_per_jail_tick`, `fine_jail_max_ticks`,
  `case_ttl_ticks`.
- The watch: `watch_detect_bp`, `watch_arrest_bp`, `watch_contraband_bp`.
- Sentences: `<kind>.fine`, `<kind>.jail_ticks`, `<kind>.fine_max`,
  `<kind>.jail_max_ticks` for each kind of crime.

A zone uses the value of its city. The registry uses the value of the city
of the case.

## 17. Verbs

| Group | Verbs |
| --- | --- |
| Crimes | `pickpocket`, `burgle`, `rob`, `assault`, `vandalize`, `bribe` |
| After a crime | `fence`, `repair`, `report_crime` (F), `pay_fine`, `post_bounty` |
| Police | `patrol`, `investigate`, `search`, `arrest`, `charge`, `release` |
| Courts | `rule_case`, `jury_vote` |
| Queries | `case_list` (F), `case_view` (F) |

The verb table of `docs/design/game-design.md`, section 5, has these 21
verbs. Three points differ from the first game design:

- `defraud` is not there (section 4.6).
- `repair` is new: the verb that gives the value of a damaged site back.
- `case_list` and `case_view` are new: an officer must find the cases, and
  a judge and a juror must read a case before they decide.

## 18. For other modules

A crate that depends on `unsup-crime` can use:

| Item | Use |
| --- | --- |
| `Cases`, `Case`, `Status`, `Evidence`, `Trial`, `Sentence`, `Bounty` | The registry of the global realm, for reads |
| `Record` and its parts | The record of a resident: `warrants()`, `debts()`, `hot()`, `custody()` |
| `LawSites` | Where the jail, the police stations and the courthouse of a city are |
| `CrimeKind` | The kinds of crime |
| `names` | Conditions, permissions, tags, events, notices, ledger labels |

What the module needs from the others:

- **Sites.** The module finds the sites of the kinds `jail`,
  `police_station` and `courthouse` in each zone when the world is made, and
  again in each game hour. Each zone tells all zones.
- **Powers.** The politics module gives the offices: a role of a city
  government with `law.arrest`, `law.search`, `law.judge` or `law.jail`
  (`unsup_society::offices`). A police force that is not the government
  gets the powers with `org_grant`.
- **The condition `wanted`.** Another module can set it on a resident. It is
  grounds for an arrest. With no case to charge, the suspect is free after
  `hold_ticks`. No module does this today. A module that does it gives each
  officer, and the city watch, the right to hold that resident one time for
  `hold_ticks` without a case: the arrest ends the condition, and this
  module does not set it again without a warrant. The arrest counts like
  each other arrest (`arrest_xp`, the event `crime.arrest_made`). Set the
  condition only for a resident that the rules of that module let the
  police hold.
- **Laws.** Each policy parameter can be changed with `set_param`.

The module also works in a world without the other modules: without a jail
a suspect is held in place, and without the registry of organizations each
trial has a jury at once.

## 19. Limits

- **Hot value is a count on the thief.** Goods and cash that the thief
  gives to another resident, drops in a site or puts in the bank are clean.
  A second resident that holds the loot is not found by a search.
- **Evidence never points at the wrong resident.** A report names only a
  perpetrator that the witness saw, and a `charge` needs evidence against
  the target. So the engine never convicts an innocent resident, and a
  resident cannot frame another. A judge or a jury can acquit a guilty
  resident.
- **A search needs no grounds.** An officer can search each resident.
- **A released or paroled resident keeps what it has.** `law.jail` ends a
  sentence early. This is a power that an official can sell.
- **Unpaid restitution is lost to the victim** when it becomes jail time.
- **A zone cannot see the sites of another zone.** A restitution for a site
  of another zone goes there by wire, also when that site is gone. The
  money then stays on the holder of the site in the ledger of that zone.
- **A fine comes only from cash.** The module does not take money of a bank
  account or a site of a convict.
- **Confiscated goods stay in the police station.** A member of the city
  government with `site.manage` can take them out. The module does not
  destroy seized contraband.
- **The engine waits only for a judge of the government.** A judge of
  another organization with a grant can rule, but the jury is called at
  once when the government of the city has no judge.
- **A juror in jail cannot vote.** The draw does not know who serves a
  sentence. Such a juror counts as one that did not vote.
- **Walls do not stop sight.** A witness in the street can see a burglary
  in a building.
- **The watch looks only into sites with the tag `public`.**
- **A resident that the kernel moves to another zone in the tick of its
  arrest** stays where it is and is held there.
- **Zone passes.** The round of the watch, the care of the jail and the
  check of hot value each look at every resident of a zone once in each
  game hour.
