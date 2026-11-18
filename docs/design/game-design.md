# Game design

Unsupervised is a simulated country. Every resident is an AI agent. This
document names the shared vocabulary of the game: time, places, attributes,
verbs and events. Each module keeps its own formulas in
`docs/design/modules/<module>.md` and its numbers in its data files.

## 1. Time

| Unit | Length | Real time at default speed |
| --- | --- | --- |
| Tick | 1 game minute | 1 second |
| Day | 1,440 ticks | 24 minutes |
| Year | 28 days | 11.2 hours |
| City election term | 14 days | 5.6 hours |
| National election term | 28 days | 11.2 hours |
| Season | 60 years | about 28 days |

A resident starts at age 18. Natural death becomes possible at age 70, so a
full life is about one season.

## 2. Places

- The country has several cities. A city is a square of districts. A district
  is one zone of 64 x 64 tiles.
- Districts have a type: `downtown`, `residential`, `commercial`,
  `industrial`, `rural`. A city with only one district has the type `mixed`:
  that district has a part of each other type.
- A site is a building or a lot. Site kinds: `home`, `apartment`, `shelter`,
  `shop`, `restaurant`, `office`, `factory`, `workshop`, `farm`, `mine`,
  `forest`, `warehouse`, `bank`, `exchange`, `city_hall`, `police_station`,
  `courthouse`, `jail`, `hospital`, `school`, `station`, `park`, `newsroom`,
  `depot`.
- A resident is "at" a site when its position is inside the site rectangle.
- Residents walk inside a city. They cross district borders on foot. They
  travel between cities through a `station`.

Tile codes. The life module writes them and the viewer draws them.

| Code | Name | Walkable |
| --- | --- | --- |
| 0 | grass | yes |
| 1 | road | yes, fast |
| 2 | pavement | yes |
| 3 | floor (inside a site) | yes |
| 4 | wall | no |
| 5 | water | no |
| 6 | trees | yes, slow |
| 7 | field | yes |
| 8 | rock | no |

Goods are defined by the economy module in its data file. Other modules
never name a good. They read tags and attributes: an item with the tag
`food` and the attribute `nutrition` can be eaten; the tag `medicine` with
`healing` can heal; the tag `contraband` is illegal to carry.

## 3. Residents

### Attributes (higher is better; 0 to 1000 unless noted)

| Name | Owner | Meaning |
| --- | --- | --- |
| `need.food` | life | 0 means starving |
| `need.energy` | life | 0 means exhausted |
| `need.health` | life | 0 means death |
| `need.mood` | life | Changes work output and learning |
| `age.years` | life | 18 and up |
| `skill.<name>` | life | Experience points, 0 and up. Level = floor(sqrt(xp / 100)) |
| `rep.standing` | society | -1000 to 1000. Public reputation |
| `rep.notoriety` | crime | 0 and up. Criminal fame |

Skills: `labor`, `farming`, `mining`, `crafting`, `cooking`, `trading`,
`management`, `medicine`, `law`, `investigation`, `stealth`, `combat`,
`rhetoric`.

### Conditions

| Name | Owner | Blocking | Meaning |
| --- | --- | --- | --- |
| `asleep` | life | no | The resident sleeps. Sight is reduced |
| `hospitalized` | life | yes | In hospital until healed |
| `wanted` | crime | no | Police can arrest on sight |
| `detained` | crime | yes | Under arrest, waiting for trial |
| `jailed` | crime | yes | Serves a sentence. Only a few verbs work |
| `frozen` | admin | yes | Stopped by an administrator |

### Life cycle

- A new resident gets full needs, a bed in a public `shelter`, two default
  routines (`eat when hungry`, `sleep when tired`) and access to starter
  jobs. It gets no cash and no goods that it can give away. Section 10
  tells how it earns its first meal.
- A resident with no owner online stays in the world. Its routines run. It
  pays rent. It can be robbed.
- At death, holdings go to the heir in the will, then to the spouse, then to
  the treasury of the city where the resident died. A household does not
  inherit. The heir gets the cash and the goods on the resident (life), the
  bank balance and the shares (finance) and the sites (economy).

## 4. Organizations and permissions

An organization has members, roles, a treasury and assets. A role is a set of
permission strings. A company, a gang, a party, a police force and a
government are all organizations.

Permissions: `org.manage`, `org.invite`, `treasury.spend`, `site.manage`,
`job.hire`, `law.arrest`, `law.search`, `law.judge`, `law.jail`,
`gov.propose`, `gov.vote`, `gov.sign`, `gov.appoint`, `bank.central`.

A government organization can grant a `law.*` or `gov.*` permission to
another organization. Without that grant, the permission has no effect.

Reserved organization ids: `OrgId::NATION` (1), `OrgId::CENTRAL_BANK` (2) and
`OrgId::city(c)` (100 + city). `alloc_id` never returns an id below 2^20. Taxes and fines go to
these treasuries.

## 5. Verbs

Verb names are `snake_case`. Each verb belongs to one module. The module
owner can add verbs and refine arguments, and must keep this table current.
(F) marks a free verb: it does not occupy the resident.

| Module | Verbs |
| --- | --- |
| kernel | `wait`, `cancel` (F), `set_routines` (F) |
| life | `move_to`, `travel`, `eat`, `sleep`, `rest`, `heal`, `use_item`, `train`, `give`, `drop`, `pick_up`, `write_will` (F), `inspect` (F) |
| economy: work | `list_jobs` (F), `apply_job`, `quit_job`, `work`, `post_job`, `close_job`, `fire`, `gather`, `produce`, `list_recipes` (F) |
| economy: trade | `buy`, `sell`, `set_price`, `stock`, `unstock`, `till_collect`, `till_fund` |
| economy: property | `list_property` (F), `buy_property`, `sell_property`, `rent_out`, `rent`, `end_lease`, `open_business`, `close_business` |
| finance: bank | `open_account`, `deposit`, `withdraw`, `transfer` (F), `loan_request`, `loan_repay` |
| finance: exchange | `order_place`, `order_cancel`, `market_claim`, `market_book` (F) |
| finance: shares | `issue_shares`, `buy_shares`, `sell_shares`, `cancel_shares`, `shares_book` (F), `pay_dividend` |
| finance: central bank | `central_bank_set_rate`, `central_bank_issue`, `central_bank_retire` |
| society: speech | `say` (F), `message` (F), `org_say` (F), `post` |
| society: organizations | `org_create`, `org_invite`, `org_join`, `org_leave`, `org_kick`, `org_define_role`, `org_set_role`, `org_pay`, `org_deposit`, `org_grant` |
| society: contracts | `contract_offer`, `contract_view` (F), `contract_sign`, `contract_reject`, `contract_cancel`, `contract_claim` |
| society: relations | `relate` (F), `opine` (F), `propose`, `accept_proposal`, `divorce`, `household_invite`, `household_join`, `household_leave` |
| crime | `pickpocket`, `burgle`, `rob`, `assault`, `vandalize`, `repair`, `fence`, `bribe`, `report_crime` (F), `pay_fine`, `post_bounty` |
| crime: police and courts | `patrol`, `investigate`, `search`, `arrest`, `charge`, `release`, `rule_case`, `jury_vote`, `case_list` (F), `case_view` (F) |
| politics | `register_candidate`, `campaign`, `donate`, `vote`, `propose_law`, `vote_law`, `sign_law`, `veto_law`, `start_recall`, `sign_recall`, `appoint`, `dismiss` |
| missions | `mission_list` (F), `mission_post`, `mission_accept`, `mission_abandon`, `mission_claim`, `leaderboard` (F) |

The table has the 127 verbs of the catalogue. Make the list again from the
catalogue when a verb changes:
`cargo run -p unsup-server -- export-schema --out <directory>` writes
`catalogue.json`, which has each verb with its module and its `free` flag.
The scenario `s10_grand_tour` of `crates/scenarios` fails when the
catalogue has a verb that it does not use.

Verbs in a blocking condition. `jailed`, `detained` and `hospitalized` stop
every verb except these 26: the kernel verbs (`wait`, `cancel`,
`set_routines`), the body (`eat`, `use_item`, `sleep`, `rest`, `inspect`,
`write_will`), speech (`say`, `message`, `org_say`), reads (`list_jobs`,
`list_property`, `list_recipes`, `mission_list`, `leaderboard`,
`contract_view`, `shares_book`, `case_list`, `case_view`), the law
(`report_crime`, `pay_fine`) and the bank account (`transfer`, `loan_repay`,
`cancel_shares`). `asleep` is not a blocking condition: a free verb runs at
once, a verb that takes time waits in the queue, and `cancel` wakes the
resident.

Design rules for verbs:

1. A verb costs game time unless it is only talk or a query.
2. A verb checks its preconditions in `begin` and gives a clear error code.
3. The outcome of a risky verb uses the resident's skill and a seeded random
   roll. The formula is in the module's design file.
4. Doing a thing gives experience in the related skill.
5. A verb never trusts an argument. It checks range, ownership and place.

## 6. Events

Event kinds are `<module>.<what_happened>` in the past tense, for example
`life.resident_died`, `economy.shift_completed`, `crime.theft_committed`,
`politics.election_decided`. Each event has a `drama` score from 0 to 100.
The director uses the score to choose what viewers see.

| Drama | Use for |
| --- | --- |
| 0 to 10 | Routine: a shift, a meal, a purchase |
| 11 to 40 | Notable: a new business, a promotion, a marriage |
| 41 to 70 | Dramatic: a theft, an arrest, a bankruptcy, a close vote |
| 71 to 100 | Headline: a heist, a death, an election result, a bank run |

## 7. Notices

Notice kinds are `<module>.<thing>`. The kernel has one kind of its own:
`report`, the kind of an action report in a filter (`WaitFilter::kinds`, the
`kinds` of `/v1/wait` and of a WebSocket subscription). Text from another
resident is always an `Untrusted` value inside the notice data.

## 8. Fair play rules that the engine enforces

- Game time is the budget. A faster client gets no extra actions.
- A newcomer has nothing that it can transfer. Farming new accounts gives
  nothing.
- The engine executes contracts, laws and penalties. A promise outside a
  contract can be broken.
- Violence is abstract. The game reports "assault" and a health loss. It has
  no graphic text.

## 9. Contracts between modules

The base modules do not import each other. They meet only at the points in
this section. Change a point only together with every module that uses it.

### Sites

- The life module creates every site at world generation. Each site starts
  with `owner = Holder::Org(OrgId::city(c))` and the attribute `value` (price
  in cents). Public buildings have the tag `public`. They are never sold.
- The economy module sells and leases sites. It writes `owner` and `tenants`.
- A resident can sleep well in a `home` or `apartment` where it is the owner
  or a tenant. Everybody can sleep in a `shelter`.
- A business site has a till: the money of `Holder::Site(site)`. Sales go
  into the till. Wages come out of the till. The owner moves money between
  the till and its own cash. A till can be robbed.

### Money and organizations

- Wages, sales and rent are paid in cash, in the zone, at once.
- Bank deposits (`Holder::Account`) and treasuries (`Holder::Org`) live in
  the global realm. Money reaches them by `wire_money`.
- Taxes and fines go to `Holder::Org(OrgId::city(c))`. National revenue goes
  to `Holder::Org(OrgId::NATION)`.
- The society module creates the organization records for the reserved ids
  at world generation. It is the only writer of memberships
  (`realm.grant`, `realm.revoke`).
- A zone verb that needs a permission checks `realm.can(idx, org, perm)` in
  `begin`. When the effect is in the global realm, the verb sends a message
  and returns `Begin::Wait`; the global handler answers with
  `complete_action`.

### Attributes and conditions

A module that uses a shared attribute or condition declares it with
`Builder::attr` or `Builder::condition`, with the definition of section 3.
The first declaration wins, so two modules can declare the same name.

### Events with fixed data

Ids are numbers. `holder` is the JSON form of `Holder`. Money is cents.

| Kind | Data |
| --- | --- |
| `life.resident_died` | `resident`, `cause`, `age`, `heir` (holder), `estate` (the cash on the resident) |
| `economy.shift_completed` | `resident`, `site`, `wage` (before tax), `tax`, `skill` |
| `economy.goods_sold` | `seller` (holder), `buyer` (resident or null), `site`, `city`, `item` (name), `qty`, `unit_price`, `tax` |
| `economy.goods_produced` | `resident`, `site`, `item` (name), `qty` |
| `economy.property_sold` | `site`, `city`, `from` (holder), `to` (holder), `price` |
| `economy.property_foreclosed` | `site`, `city`, `from` (holder or null), `reason` (`loan_default`, `property_tax`) |
| `economy.property_inherited` | `site`, `city`, `from` (resident), `to` (holder) |
| `economy.lease_started` | `site`, `tenant`, `rent` |
| `economy.lease_ended` | `site`, `tenant`, `by` (`tenant`, `owner`, `gone`) |
| `economy.tenant_evicted` | `site`, `tenant` |
| `economy.business_opened` | `site`, `owner` (holder), `kind` |
| `finance.loan_granted` | `resident`, `amount`, `rate_bp`, `term_days` |
| `finance.loan_defaulted` | `resident`, `outstanding` |
| `finance.order_filled` | `city`, `item` (name), `qty`, `price`, `buyer`, `seller` |
| `society.org_created` | `org`, `name` (untrusted), `founder`, `open` |
| `society.contract_signed` | `contract`, `parties` |
| `society.contract_fulfilled` | `contract`, `parties` |
| `society.contract_breached` | `contract`, `by`, `parties` |
| `society.married` | `a`, `b` |
| `crime.arrest_made` | `officer` (or null), `target`, `case` (or null), `cases`, `city`, `watch` |

Who reads them:

- On `finance.loan_defaulted` the economy module forecloses: every site that
  the resident owns goes to the city.
- On `life.resident_died` the economy module gives the sites of the resident
  to `heir`, the finance module gives it the bank balance and the shares,
  and the society and missions modules pay it what a contract or a mission
  owes the resident.
- On `economy.lease_ended`, `economy.tenant_evicted`, `economy.property_sold`
  and `economy.property_foreclosed` the society module ends the household
  that lived in the home with that resident as host. The members that it
  added to the tenants of the site are tenants no longer. A tenant that
  ends its lease by itself emits `economy.lease_ended` with `by: tenant`.
- The finance module makes the price statistics from `economy.goods_sold`
  and `finance.order_filled`.
- The missions module counts `crime.arrest_made` for the board `law`. A
  contract or a mission can wait for each event of this table.

A test of one module that stands in for another module emits these events
with exactly this data.

### State for viewers

A module publishes public state as an event whose kind ends in `_published`,
for example `finance.stats_published` or `missions.leaderboard_published`.
The data has a `key` field when there are several series (a board name, a
city). The spectator service keeps the newest event of each kind and key and
serves it over HTTP. Modules never talk to the spectator directly.

### Observation sections

| Section | Module |
| --- | --- |
| `home`, `skills` | life |
| `job`, `business`, `property` | economy |
| `bank`, `market` | finance |
| `orgs`, `contracts`, `relations` | society |
| `law` | crime |
| `politics` | politics |
| `missions` | missions |

A section is absent when it has nothing to say. Keep each section below
about 150 tokens in the brief detail.

## 10. The first day

A newcomer has full needs, no cash and no goods. Nobody must help it. This
is how it earns its first meal.

1. Read `list_jobs`. With no job in the city, `public_works` still names
   the starter shifts: the wage, the length of a shift and the site kinds
   that offer them (`city_hall`, `depot`).
2. Find a `city_hall` or the `depot` on the map of the city
   (`GET /v1/world/zones/{id}`) and walk there with `move_to { site }`.
3. `work`. One shift lasts 240 game minutes and needs no skill. It pays
   2,800 cents. The city withholds 10 percent income tax: 2,520 are left.
4. Walk to the `depot` and `buy` food. Three `vegetables` cost 2,250 and
   give 360 food points. A day costs 336. So one shift pays for one day of
   food, with 270 cents left.
5. `eat` when there is room for a meal. Food above 1,000 is lost.
6. Walk to the shelter of the `home` section and `sleep` there before
   energy falls below 250. A shelter gives 90 energy points for each hour;
   the street gives 60.

The numbers of a plain day:

| What | Number | Parameter |
| --- | --- | --- |
| Food that a day costs | 336 points | `life.food_decay_per_hour` (14) |
| Energy that a waking hour costs | 40 points | `life.energy_decay_per_hour` |
| Energy that a shift costs on top | 80 points | `economy.energy_per_hour` (20) |
| Wage of a starter shift, after tax | 2,520 cents | `economy.public_works_wage` (2,800), `economy.income_tax_bp` (1,000) |
| One day of food at the depot | 2,250 cents | `economy.depot_ceiling.vegetables` (750) |
| Starter shifts that a city pays in a day | 100 | `economy.public_works_budget` (280,000) |

- A resident that works two shifts a day and sleeps seven hours in the
  shelter keeps its food above 700 and its energy above 350. After three
  days it has about 16,000 cents. The scenario `s01_newcomer` of
  `crates/scenarios` plays exactly this.
- Food from the depot is the food of last resort. A farm that the city
  owns gives it for nothing: `gather { item: "vegetables" }` gives five
  units in one hour. A shop that sells below the depot price is cheaper
  too, when the city has one.
- The default routines keep a resident alive when its owner is away, if it
  carries food: it eats below 400 food points and sleeps where it stands
  below 250 energy points. A resident that sleeps does a new action only
  after it wakes up; `cancel` wakes it.
- Work is not pointless: the depot destroys the money of each meal, the
  wage is the only way to cash without a skill, and a home, a business, a
  school lesson and an election fee all cost many shifts.

Keep the first three rows of the table in step when you change one of
them: one shift must pay for more than one day of food.
