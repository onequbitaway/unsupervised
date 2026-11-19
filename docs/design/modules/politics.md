# The politics module

Crate `unsup-politics`, path `crates/modules/politics`. The module depends
on the kernel and on the society module.

This document gives the calendar and the rules of the module. The code is
the final word. The numbers are in
`crates/modules/politics/data/balance.toml`; section 15 lists them. This
document names each parameter without the prefix `politics.`.

Read `docs/design/architecture.md`, `docs/design/game-design.md` and
`docs/design/modules/society.md` first.

## 1. Purpose

Residents govern themselves.

- **Offices** are won in elections on a fixed calendar.
- A **law** is data. It sets one policy parameter to one value, for one
  city or for the nation. The simulation obeys the parameter.
- The executive **appoints** the police, the judges and the central
  banker.
- Residents can **recall** an office: enough signatures open an early
  election.

The engine executes all of this. An official has exactly the powers of its
role, and a law has exactly the effect of its parameter.

## 2. Rules for the whole module

- The world must load the society module. The society module makes the
  government organizations and owns all memberships. This module changes
  them only through `unsup_society::offices`.
- All state of the module is one resource of the global realm,
  `politics.state`. The holders of the offices are not in it. The registry
  of the society module has them.
- Every window and every deadline counts frames. One frame is one game
  minute of the calendar (`realm.clock()`). Time dilation of a zone does
  not move a deadline.
- A **day** is a game day of 1,440 game minutes. The days of the world
  count from 1: day 1 is day 1 of year 1. With the default year of 28
  days, day 31 of the world is day 3 of year 2.
- Money is in cents. The module creates no money and destroys none. It has
  no faucet and no sink.
- A refused verb changes nothing. Cash that left the zone for a request
  comes back by wire when the global realm refuses the request.
- Text that a resident wrote (a platform, the reason of a bill) leaves the
  module only as an `Untrusted` value.
- A blocking condition (`jailed`, `detained`, `hospitalized`) stops every
  verb of this module.

### A government

A **government** is the nation (`OrgId::NATION`) or one city
(`OrgId::city(c)`). In events, notices and results a government is the
field `gov` (`nation` or `city`) and, for a city, the field `city`.

A parameter that this module reads for a city government is read in that
city: the override of the city when a city law made one, else the national
value. So a city law changes the rules of that city only.

### How a verb runs

| Kind | Verbs | How it runs |
| --- | --- | --- |
| Global | `propose_law`, `vote_law`, `sign_law`, `veto_law`, `appoint`, `dismiss` | The kernel sends the request to the global realm. The answer is a report in the next frame. |
| Zone, then global | `register_candidate`, `donate`, `vote`, `start_recall`, `sign_recall` | The zone checks what only the zone knows: the cash and the age of the resident. The global realm does the rest. The answer is a report in the next frame. |
| Zone, global, zone | `campaign` | See section 6. |

No verb of the module is free. Each verb occupies the resident until its
report comes.

## 3. Offices and posts

Every office and every post is a role in a government organization. The
module defines the roles at world generation.

| Office or post | Role | Organization | Seats | Permissions | Filled by |
| --- | --- | --- | --- | --- | --- |
| `mayor` | `mayor` | city | 1 | `gov.sign`, `gov.appoint`, `treasury.spend` | election |
| `council` | `councillor` | city | `council_seats` | `gov.propose`, `gov.vote` | election |
| `police_chief` | `police_chief` | city | 1 | `law.arrest`, `law.search`, `law.jail` | the mayor |
| `police_officer` | `police_officer` | city | `police_officers_max` | `law.arrest`, `law.search` | the mayor |
| `judge` | `judge` | city | `judges_max` | `law.judge`, `law.jail` | the mayor |
| `president` | `president` | nation | 1 | `gov.sign`, `gov.appoint`, `treasury.spend` | election |
| `legislature` | `legislator` | nation | `legislature_seats` | `gov.propose`, `gov.vote` | election |
| `central_banker` | `central_banker` | central bank | 1 | `bank.central` | the president |

- The mayor and the president are the **executives**. The council and the
  legislature are the **members**.
- `unsup_politics::roles` has the role names for other modules.
- A zone checks a power with `realm.can(idx, OrgId::city(c), "law.arrest")`.
  The copy in the zone also has the name with its source, for example
  `law.arrest@100`. See `docs/design/modules/society.md`, section 4.4.
- The zones see a new holder one frame after the change.
- A resident has one role in an organization. So a resident holds one
  office or post of one city at a time. It can hold an office of a city
  and an office of the nation at the same time.
- An official leaves its office with the verb `org_leave` of the society
  module. The society module removes a resident that left the world.
- An office that nobody holds is empty. Nothing else happens. The next
  election, or an early election after a recall, fills it.

Section 17 says why the police and the court are roles of the city
government and not organizations of their own.

## 4. The election calendar

Each government has one voting day in each term. On that day the
government elects its executive and its members: two elections, one for
each office.

| Government | First voting day | Term | Voting days with the default year |
| --- | --- | --- | --- |
| City | day `city_first_vote_day` (3) | `city_term_days` (14) | day 3 and day 17 of each year |
| Nation | day `nation_first_vote_day` (5) | `nation_term_days` (28) | day 5 of each year |

The windows of an election with the voting day `V`:

| Window | From | To | Verbs |
| --- | --- | --- | --- |
| Registration | 00:00 of day `V - campaign_days - registration_days` | 00:00 of day `V - campaign_days` | `register_candidate`, `donate` |
| Campaign | 00:00 of day `V - campaign_days` | the polls open | `campaign`, `donate` |
| Voting | minute `polls_open_minute` of day `V` (06:00) | minute `polls_close_minute` of day `V` (20:00) | `vote` |
| Count | when the polls close | | |

With the numbers of the data file, the first city election is:

| Day | What happens |
| --- | --- |
| Day 1 | Registration |
| Day 2 | Campaign |
| Day 3, 00:00 to 06:00 | Campaign |
| Day 3, 06:00 to 20:00 | Voting |
| Day 3, 20:00 | Count. The winners take office |

Rules:

- The module opens an election when its registration window starts. It
  emits `politics.election_called`. The windows and the number of seats
  are fixed at that moment. A law that changes the calendar or the seats
  applies from the next election.
- When the count of a regular election is done, the next voting day of
  that government is `V` plus the term in days.
- An election that opens after the start of its registration window has a
  shorter window. An election whose registration window is over when it
  opens has no candidate, and the office stays empty.
- A world that is restored from a snapshot without this module gets the
  roles and the calendar in its first frame. The next voting day is the
  first day on the grid of the table that is not in the past.

## 5. Candidates and funds

### 5.1 `register_candidate`

`register_candidate { election, platform }` puts the resident on the
ballot.

- The election is in its registration window.
- For a city office the resident is in that city. For a national office
  the place does not matter.
- `platform` is a short text for the voters, at most `platform_max` bytes.
  It is not blank and has no control character except a line break.
- The resident pays `candidate_fee` in cash. The fee is the value in the
  city in which the resident stands. The fee goes to the treasury of the
  government of the office.
- A resident runs for one office of a government at a time.
- An election has at most `candidates_max` candidates.

### 5.2 Campaign funds and `donate`

Each candidate has a campaign fund. The fund is an escrow in the global
realm. It is not cash of the candidate.

- `donate { election, candidate, amount }` moves cash of the donor to the
  fund. Every resident can give, also the candidate. Donations are open in
  the registration window and in the campaign window.
- Only `campaign` takes money out of a fund.
- When the election is decided, the rest of each fund goes to the treasury
  of the government of the office. The same happens to the fund of a
  candidate that leaves the world.
- Each donation is the public event `politics.donation_made`, with the
  donor and the amount.

## 6. `campaign`

`campaign { election, spend }` holds a rally at the place where the
candidate stands.

1. The zone checks that `spend` is `campaign_cost_min` or more, and asks
   the global realm.
2. The global realm checks that the election is in its campaign window,
   that the resident is a candidate, that the resident is in the city of a
   city election and that the fund has `spend`. It moves `spend` from the
   fund to the treasury of the city in which the rally is.
3. The candidate is busy for `campaign_minutes` game minutes.
4. Then the zone of the candidate emits `politics.rally_held` at the
   position of the candidate. Each resident within the reach gets a notice
   of that kind with the platform of the candidate. Viewers see the event.

The reach in tiles is

```
reach = min(campaign_radius_max, campaign_radius_base + spend / campaign_cents_per_tile)
```

The division rounds down. With the data file, 600 cents reach 12 tiles.

- The result has `heard`: the residents of the zone within the reach,
  without the candidate. Residents on the other side of a zone border get
  the notice too (at most the sight radius away), but `heard` does not
  count them.
- A candidate that stops the action holds no rally. The money is spent.
- A refusal comes in the next frame and spends nothing.

## 7. `vote` and the count

`vote { election, candidate }` casts one vote.

- The polls of the election are open.
- For a city office the voter is in that city.
- The voter lived `voter_min_age` game minutes or more. This stops an
  owner that makes new residents on the voting day.
- A resident has one vote in an election. It also has one vote for each
  office: a resident that voted for the mayor of one city cannot vote for
  the mayor of another city while both elections are open.
- A candidate can vote for itself.
- The ballot is secret. No event and no published state says who voted
  for whom. The counts are public while the polls are open.

The count runs when the polls close.

1. A candidate that left the world is not counted. A candidate needs
   `win_votes_min` votes to win a seat.
2. The candidates are put in order: most votes first.
3. The first `seats` candidates win.
4. When more candidates have the votes of the last seat than seats are
   left, a draw decides between them. The draw uses the random number
   generator of the global realm, so the same seed and the same inputs
   give the same winner.
5. The winners take office at once. The residents that held the office
   before lose the role. An office that nobody won is empty.
6. Each candidate gets the notice `politics.election_won` or
   `politics.election_lost`.
7. The module emits `politics.election_decided` and publishes the last
   state of the election.

A winner that holds another role of the same government loses that role:
a councillor that wins the election for mayor leaves the council.

The drama of `politics.election_decided` is a headline score. It is
`drama_election_close` after a draw or when the last winner leads the best
loser by `close_margin` votes or fewer, `drama_election_empty` when nobody
won, and `drama_election_decided` in every other case.

## 8. Laws

### 8.1 What a law is

A bill names a **policy parameter** (the catalogue marks them with
`policy: true`), a new value inside the bounds of the parameter, and a
government.

- A city bill is decided by the council of that city. When it becomes law,
  the module calls `realm.set_param(param, value, Some(city))`. The
  parameter has the new value in that city only.
- A national bill is decided by the legislature. When it becomes law, the
  module calls `realm.set_param(param, value, None)`. The national value
  changes. A city that has its own value keeps it.
- The global realm has the new value at once. The zones have it in the
  next frame.

### 8.2 The way of a bill

```
propose_law -> vote -> sign -> law
                 |       |
                 |       +-> veto -> override -> law
                 |                      |
                 +-> rejected           +-> rejected
```

| Stage | Who acts | Ends | Then |
| --- | --- | --- | --- |
| `vote` | The electorate: `vote_law` | When nobody is left to vote, or after `bill_voting_minutes` | Passed: `sign`. Failed: `rejected` |
| `sign` | Executive: `sign_law` or `veto_law` | At the action, or after `bill_sign_minutes` | Signed: law. Vetoed: `override`. No action before the deadline: law |
| `override` | The electorate: `vote_law` | When nobody is left to vote, or after `bill_override_minutes` | Two thirds: law. Else: `rejected` |

Rules:

- `propose_law { param, value, city, reason }` needs `gov.propose`. With
  `city` it is a city bill. Without `city` it is a national bill. `reason`
  is an optional text of at most `bill_text_max` bytes.
- A government has at most `bills_open_max` open bills, and one open bill
  for one parameter.
- **Electorate.** Each stage with a vote has an electorate. It is fixed
  when the stage begins and does not change:
  - for the stage `vote`, the residents with `gov.vote` at the proposal;
  - for the stage `override`, the residents with `gov.vote` at the veto.
- `vote_law { bill, vote }` needs `gov.vote` now and a place in the
  electorate of the stage. `vote` is `yes` or `no`. A member votes one
  time in a stage.
  - A resident that got its seat after the stage began is not in the
    electorate. Its vote is refused with `not_allowed`. It votes on the
    next bill, and on the override when the veto comes after its seat.
  - A member of the electorate that lost its seat cannot vote any more.
- **A vote stays.** A vote that a member cast counts, also when the member
  leaves the council or the legislature, or the world, before the count.
- **Quorum.** A decision needs the votes of `quorum_permille` thousandths
  of the electorate, rounded up, and one vote or more. A member that
  leaves does not make the quorum smaller.
- **Majority.** A bill passes the stage `vote` when the votes in favour
  are more than the votes against. A tie fails.
- **Override.** The members override a veto when the votes in favour are
  `override_num / override_den` (two thirds) of the votes cast or more,
  with a quorum. The override is a new vote with a new electorate: the
  votes of the first stage do not count in it.
- **The end of a stage.** A stage ends at once when a vote comes in and
  nobody is left to vote: each member of the electorate has voted or has
  no seat any more. Else it ends at its deadline.
- `members` in `politics.bill_passed` and `politics.bill_rejected` is the
  size of the electorate of the stage.
- A bill of a snapshot from before this rule has no electorate. It gets
  the residents with `gov.vote` at its first vote or count.
- `sign_law { bill }` and `veto_law { bill }` need `gov.sign`.
- **Automatic enactment.** A bill that passed becomes law without a
  signature when nobody has `gov.sign` in the government (the executive
  office is empty), and when the deadline of the stage `sign` passes.
- A legislative vote is public: each `vote_law` is the event
  `politics.bill_voted` with the member and the vote.

The reasons of a rejection (`why`): `no_quorum`, `voted_down`, `vetoed`
(no override), `no_parameter` (a newer set of modules has no such policy
parameter).

The ways to a law (`how`): `signed`, `deadline`, `vacant`, `override`.

### 8.3 The register

The module keeps for each government:

- the open bills;
- the last `register_max` closed bills;
- the laws in force: for each parameter the newest law, with the value,
  the value before, the bill and the frame.

The register is public. See section 12.

## 9. Appointments

`appoint { post, resident, city }` and `dismiss { post, resident, city }`
are verbs of the executive. They need `gov.appoint`.

- The mayor fills `police_chief`, `police_officer` and `judge` of its
  city. The president fills `central_banker`.
- `city` is the city of a city post. Without it, the module uses the city
  with the lowest id in which the resident has `gov.appoint`.
- The executive cannot appoint itself.
- The resident is in the world. The place does not matter.
- A resident that holds another role in the same organization cannot be
  appointed: the appointment would take that office away without a word.
- A post has a number of seats (section 3). A full post takes nobody.
  Dismiss a holder first.
- An appointment has no end. The next executive can dismiss the holder.
- `dismiss` takes a post from a resident that holds it. It cannot remove
  an elected official.

The appointed resident has the powers of the post in its zone in the next
frame. A dismissed resident loses them in the next frame.

## 10. Salaries

At the start of each game day every government pays its officials for the
day that ended.

- The salary of an office or a post is a policy parameter, in cents for
  one day (`salary_*`). A city law changes the pay in that city.
- A city pays its mayor, council, police and judges from the city
  treasury. The nation pays the president, the legislature and the
  central banker from the national treasury.
- The order is: the elected offices, then the posts, in the order of the
  table of section 3. Inside an office the order is the resident id.
- **No funds, no pay.** A salary is paid in full or not at all. When the
  treasury has less than the salary, the official gets nothing for that
  day. The government does not owe the money later. A later official
  with a smaller salary can still be paid.
- The money goes by wire to the cash of the official, in whatever zone it
  is. It arrives in the next frame.
- Each official gets the notice `politics.salary_paid` or
  `politics.salary_unpaid`. Each government that has an official emits
  one `politics.salaries_paid`.

## 11. Recall

A recall petition asks for an early election of one office.

- `start_recall { office }` starts a petition. `mayor` and `council` mean
  the offices of the city in which the resident is. `president` and
  `legislature` are the offices of the nation. The starter is the first
  signer.
- `sign_recall { recall }` adds a signature. For a city office the signer
  is in that city. A resident signs a petition one time.
- The starter and each signer lived `voter_min_age` game minutes or more.
- The petition needs

```
needed = max(recall_signatures_min, ceil(residents * recall_permille / 1000))
```

  signatures. `residents` is the number of residents in the city when the
  petition starts, or in the world for a national office.
- A petition is open for `recall_days` days. Without enough signatures it
  ends with `politics.recall_failed`.
- An office has one open petition at a time. A petition cannot start
  while an election for the office is open.
- A petition cannot start when the regular election is near: when the
  next regular voting day is not after the voting day of an early
  election that a petition of today could still cause.
- A petition can ask for an election for an empty office.

With enough signatures the petition succeeds at once:

- The module emits `politics.recall_succeeded` and opens an early election
  for the office (`kind: recall`).
- The registration of the early election opens at once. Its voting day is
  `today + registration_days + campaign_days + 1`. So the registration
  has the rest of today and `registration_days` days more.
- The holders stay in office until the count. Then the winners take the
  office and the holders before lose it, as in every election.
- The winners serve until the next regular election. The regular calendar
  does not move.

## 12. Public state for viewers

The module publishes three series as `_published` events (see "State for
viewers" in `docs/design/game-design.md`, section 9). The data of each has
the field `key`.

| Kind | Key | Data |
| --- | --- | --- |
| `politics.government_published` | the city number, or `nation` | `gov`, `city`, `org`, `treasury`, `next_vote` (`year`, `day`), `offices`, `elections`, `recalls` |
| `politics.election_published` | the election id | `gov`, `city`, `election`, `office`, `kind`, `phase`, `seats`, `vote_day`, `opens`, `campaign_from`, `polls_open`, `polls_close`, `candidates`, `turnout`; after the count also `winners` and `tie` |
| `politics.bills_published` | the city number, or `nation` | `gov`, `city`, `open`, `closed`, `laws` |

- `offices` has one entry for each office and post: `office`, `org`,
  `role`, `elected`, `seats`, `salary`, `holders` (`resident`, `name`).
- `elections` has the open elections of the government: `election`,
  `office`, `kind`, `phase`, `vote_day`.
- `recalls` has the open petitions: `recall`, `office`, `starter`,
  `started` (a frame number), `signatures`, `needed`, `deadline`
  (`year`, `day`, `hour`, `minute`).
- `candidates` has `candidate`, `name`, `platform` (untrusted), `raised`,
  `spent`, `fund` and `votes`. The candidate with the most votes is first.
  On the voting day the votes are the live count.
- `phase` is `registration`, `campaign`, `voting` or `decided`.
- A bill in `open` and `closed` has `bill`, `param`, `value`, `old`,
  `sponsor`, `reason` (untrusted or null), `proposed`, `stage`, `deadline`,
  `yes`, `no`, `passed` (the count of the first vote), `vetoed_by`, `how`
  and `why`. The newest closed bill is first. `yes` and `no` are all votes
  of the current stage, also those of members that left (section 8.2).
- A law in `laws` has `param`, `value`, `old`, `bill`, `enacted` and `how`.
- The frames of an election and of a bill (`opens`, `polls_open`,
  `deadline`, `enacted` and so on) are frame numbers.

When the module publishes:

- A series is published when it changed, at most one time in
  `publish_minutes` game minutes for all series together.
- The count publishes the result of its election in the same frame.
- At the start of each game day every series is published again. A viewer
  service that started late then has all of it, and no series becomes the
  oldest one in the store of the spectator service.

On the voting day the module also emits `politics.lead_changed` when
another candidate has more votes than every other candidate. It checks
this when it publishes the election.

## 13. Events and notices

### 13.1 Public events

The drama score of each event is a parameter `drama_*`. Each event has the
fields of its government (`gov`, `city`). An election event also has
`election`, `office` and `kind` (`regular` or `recall`). A bill event also
has `bill`, `param` and `value`. A petition event also has `recall`,
`office`, `signatures` and `needed`.

| Kind | More data | Drama |
| --- | --- | --- |
| `politics.election_called` | `seats`, `vote_day`, `registration_until`, `polls_open`, `polls_close` | 15 |
| `politics.candidate_registered` | `candidate`, `platform` (untrusted), `candidates`, `fee` | 20 |
| `politics.donation_made` | `candidate`, `donor`, `amount` | 5 |
| `politics.rally_held` | `candidate`, `platform` (untrusted), `spend`, `radius`, `heard`. Audience `Nearby` | 25 |
| `politics.polls_opened` | `candidates`, `seats`, `polls_close` | 30 |
| `politics.lead_changed` | `leader`, `votes`, `second`, `previous` | 45 |
| `politics.election_decided` | `seats`, `winner`, `winners`, `previous`, `tally` (`candidate`, `votes`, `won`), `turnout`, `tie`, `margin`, `funds_to_treasury` | 85, 95 or 30 |
| `politics.bill_proposed` | `current`, `sponsor`, `reason` (untrusted or null), `deadline` | 20 |
| `politics.bill_voted` | `member`, `vote`, `stage`, `yes`, `no` | 5 |
| `politics.bill_passed` | `yes`, `no`, `members`, `deadline` | 30 |
| `politics.bill_rejected` | `why`, `yes`, `no`, `members`, `sponsor` | 25 |
| `politics.law_vetoed` | `by`, `sponsor`, `deadline` | 50 |
| `politics.law_enacted` | `old`, `how`, `sponsor`, `signed_by` | 60, or 75 after an override |
| `politics.official_appointed`, `politics.official_dismissed` | `post`, `resident`, `by`, `org` | 30, 35 |
| `politics.recall_started` | `starter`, `holders`, `deadline` | 50 |
| `politics.recall_succeeded` | `election` (or null), `vote_day` | 70 |
| `politics.recall_failed` | `starter` | 15 |
| `politics.salaries_paid` | `paid`, `unpaid`, `total`, `treasury` | 2, or 40 when an official was not paid |

- `winner` is the first winner, or null. With it a contract clause of the
  society module can wait for the result of an election, for example
  `politics.election_decided` with `office` = `mayor`, `city` = 0 and
  `winner` = 42. The data of `politics.law_enacted` and of
  `politics.bill_voted` are flat for the same reason.
- `vote_day` is `{ year, day }`. `registration_until`, `polls_open`,
  `polls_close` and `deadline` are `{ year, day, hour, minute }`.
- `margin` is the number of votes between the last winner and the best
  loser, or null when nobody lost or nobody won.

### 13.2 Notices

| Kind | To | Data |
| --- | --- | --- |
| `politics.rally_held` | Residents within the reach of a rally | As the event |
| `politics.election_won`, `politics.election_lost` | Each candidate | The election, `votes` |
| `politics.bill_proposed` | Each other member | As the event |
| `politics.bill_passed` | Each executive | As the event |
| `politics.law_vetoed` | Each member | As the event |
| `politics.bill_rejected`, `politics.law_enacted` | The sponsor | As the event |
| `politics.appointed`, `politics.dismissed` | The resident | As the event |
| `politics.recall_started`, `politics.recall_succeeded` | Each holder of the office | As the event |
| `politics.salary_paid`, `politics.salary_unpaid` | The official | The government, `office`, `amount` |
| `politics.request_still_handled` | The actor | `verb`, `action_id`, `reason` |

The society module also sends `society.org_role_changed` to a resident
that gets an office and `society.org_removed` to a resident that loses
one.

### 13.3 A request that is interrupted

`register_candidate`, `donate`, `campaign`, `vote`, `start_recall` and
`sign_recall` send a request to the global realm when the action starts.
When the action stops before the answer comes (the resident cancels it,
another module interrupts it, it times out), the global realm still does
the work, and the kernel can drop the late answer. The report says
`interrupted`. The resident then gets the notice
`politics.request_still_handled`. It must not send the request again. Its
observation has the outcome.

## 14. The observation section `politics`

The section is small. A list that is empty is absent.

| Field | Content |
| --- | --- |
| `next_vote` | `city` and `nation`: the next regular voting day of the city in which the resident is and of the nation, as `{ year, day }` |
| `elections` | The open elections of that city and of the nation: `election`, `office`, `city` (absent for the nation), `phase`, `minutes_left` of the phase, `candidates` (a number), and `voted` while the polls are open |
| `recalls` | The open petitions of that city and of the nation: `recall`, `office`, `city`, `signatures`, `needed`, `signed`, `minutes_left` |
| `offices` | The offices and posts of the resident: `office`, `city` |
| `candidacies` | The elections in which the resident is a candidate, in every city: `election`, `office`, `city`, `phase`, `fund` |
| `bills` | The bills that wait for the resident: `bill`, `param`, `value`, `now` (the value when the bill was proposed), `city`, `stage`, `minutes_left`, `use` (the verb to use) |

- A member sees a bill in `bills` while it can vote on it and did not
  vote. An executive sees a bill while it waits for a signature.
- With brief detail, `elections`, `recalls` and `bills` have at most
  `brief_list_max` entries each. `more_elections`, `more_recalls` and
  `more_bills` count the rest.
- With full detail the lists have every entry, and each election has
  `ballot`: the candidates with `candidate`, `platform` (untrusted) and
  `votes`.
- The global realm learns of a new resident and of a move to another zone
  one frame later. Until then the section of a new resident has the
  calendar of the nation only.

## 15. Ledger labels and parameters

### 15.1 Ledger labels

| Label | Movement |
| --- | --- |
| `politics.hold` | Cash of a resident to the holding escrow of a request |
| `politics.return` | The rest of a holding escrow back to the resident |
| `politics.candidate_fee` | The fee of a candidate to the treasury of the government |
| `politics.donation` | A donation to a campaign fund |
| `politics.campaign` | The cost of a rally, from the fund to the treasury of the city of the rally |
| `politics.fund_closed` | The rest of a campaign fund to the treasury of the government |
| `politics.salary` | A salary from a treasury to an official |

### 15.2 Parameters

All are in `data/balance.toml` with the prefix `politics.`. (P) marks a
policy parameter: a law can change it.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `city_term_days` | 14 | Days between two regular elections of a city |
| `nation_term_days` | 28 | Days between two regular elections of the nation |
| `city_first_vote_day` | 3 | Day of the world of the first voting day of every city |
| `nation_first_vote_day` | 5 | Day of the world of the first voting day of the nation |
| `registration_days` | 1 | Days of the registration window |
| `campaign_days` | 1 | Days between the end of the registration and the voting day |
| `polls_open_minute` | 360 | Minute of the voting day at which the polls open |
| `polls_close_minute` | 1200 | Minute of the voting day at which the polls close |
| `council_seats` (P) | 5 | Seats of a city council |
| `legislature_seats` (P) | 7 | Seats of the legislature |
| `police_officers_max` (P) | 6 | Police officers of one city |
| `judges_max` (P) | 2 | Judges of one city |
| `candidate_fee` (P) | 5000 | Cents that `register_candidate` costs |
| `platform_max` | 280 | Bytes of a platform |
| `candidates_max` | 12 | Candidates of one election |
| `win_votes_min` | 1 | Votes that a candidate needs to win a seat |
| `voter_min_age` (P) | 1440 | Game minutes that a resident must have lived to vote, to start a recall or to sign one |
| `close_margin` | 1 | Votes between the last winner and the best loser at which an election is close |
| `campaign_minutes` | 30 | Game minutes of one rally |
| `campaign_cost_min` | 100 | Least cost of a rally in cents |
| `campaign_radius_base` | 6 | Reach in tiles of a rally of the least cost |
| `campaign_cents_per_tile` | 100 | Cents that add one tile of reach |
| `campaign_radius_max` | 24 | Greatest reach of a rally in tiles |
| `bill_voting_minutes` | 720 | Game minutes of the stage `vote` |
| `bill_sign_minutes` | 360 | Game minutes of the stage `sign` |
| `bill_override_minutes` | 360 | Game minutes of the stage `override` |
| `quorum_permille` | 500 | Part of the members whose votes a decision needs |
| `override_num`, `override_den` | 2, 3 | Part of the votes cast that overrides a veto |
| `bills_open_max` | 5 | Open bills of one government |
| `bill_text_max` | 280 | Bytes of the reason of a bill |
| `register_max` | 20 | Closed bills that the register keeps for each government |
| `salary_mayor` (P) | 8000 | Cents for one day |
| `salary_councillor` (P) | 3000 | Cents for one day |
| `salary_president` (P) | 12000 | Cents for one day |
| `salary_legislator` (P) | 4000 | Cents for one day |
| `salary_police_chief` (P) | 6000 | Cents for one day |
| `salary_police_officer` (P) | 5000 | Cents for one day |
| `salary_judge` (P) | 6000 | Cents for one day |
| `salary_central_banker` (P) | 8000 | Cents for one day |
| `recall_days` | 2 | Days that a petition is open |
| `recall_signatures_min` | 3 | Least number of signatures of a recall |
| `recall_permille` | 100 | Part of the residents whose signatures a recall needs |
| `publish_minutes` | 5 | Least number of game minutes between two publications |
| `brief_list_max` | 4 | Entries in each list of the brief section |
| `drama_*` | 2 to 95 | Drama scores of the events. Section 13.1 has them |

The module also reads `kernel.global_timeout`: the time after which a
verb that waits for the global realm fails.

## 16. Verbs, results and refusals

The verbs are those of the row `politics` of the table in
`docs/design/game-design.md`, section 5. The module adds no verb.

| Verb | Arguments | Result | Refusals |
| --- | --- | --- | --- |
| `register_candidate` | `election`, `platform` | the election, `fee`, `candidates` | `invalid_args`: bad platform. `insufficient`: no cash for the fee. `not_found`: no such open election. `conflict`: not in the registration window; a candidate already, here or for another office of the government; too many candidates. `not_here`: another city |
| `donate` | `election`, `candidate`, `amount` | `election`, `candidate`, `amount`, `fund` | `invalid_args`: amount not above zero. `insufficient`: not enough cash. `not_found`: no such open election; not a candidate there. `conflict`: the polls are open |
| `campaign` | `election`, `spend` | `election`, `spend`, `radius`, `heard` | `invalid_args`: below the least cost. `not_found`: no such open election. `conflict`: not in the campaign window. `not_here`: another city. `not_allowed`: not a candidate. `insufficient`: the fund has less |
| `vote` | `election`, `candidate` | `election`, `office`, `candidate`, `turnout` | `not_allowed`: lived too short. `not_found`: no such open election; not a candidate there. `conflict`: the polls are not open; voted already; voted for this office in another open election. `not_here`: another city |
| `propose_law` | `param`, `value`, `city` (optional), `reason` (optional) | the bill, `stage`, `deadline` | `not_found`: no such city; no such parameter. `not_allowed`: no `gov.propose`; not a policy parameter. `invalid_args`: value outside the bounds (details `min`, `max`); bad reason. `conflict`: too many open bills; an open bill for the parameter (details `bill`) |
| `vote_law` | `bill`, `vote` | `bill`, `vote`, `yes`, `no`, `stage` (after the vote) | `not_found`: no such open bill. `conflict`: the bill takes no votes now; voted already in this stage. `not_allowed`: no `gov.vote`; not in the electorate of the stage (section 8.2) |
| `sign_law` | `bill` | `bill`, `stage` (`enacted`) | `not_found`. `conflict`: the bill does not wait for the executive. `not_allowed`: no `gov.sign` |
| `veto_law` | `bill` | `bill`, `stage`, `deadline` | As `sign_law` |
| `appoint` | `post`, `resident`, `city` (optional) | the government, `post`, `resident`, `by`, `org` | `not_found`: no such city; no such resident. `not_allowed`: no `gov.appoint` there. `invalid_args`: yourself. `conflict`: holds the post already; holds another role in the organization; the post is full (details `seats`, `holders`) |
| `dismiss` | `post`, `resident`, `city` (optional) | As `appoint` | `not_found`: no such city; the resident does not hold the post. `not_allowed`: no `gov.appoint` there |
| `start_recall` | `office` | the petition, `deadline`, `succeeded`, `election` | `not_allowed`: lived too short. `conflict`: a petition is open (details `recall`); an election for the office is open; the regular election is near (details `regular_vote_day`) |
| `sign_recall` | `recall` | the petition, `succeeded`, `election` | `not_allowed`: lived too short. `not_found`: no such open petition. `not_here`: another city. `conflict`: signed already |

"The election" in a result is `gov`, `city`, `election`, `office` and
`kind`. "The bill" is `gov`, `city`, `bill`, `param` and `value`. "The
petition" is `gov`, `city`, `recall`, `office`, `signatures` and `needed`.
`deadline` in a result is a frame number.

Every verb also answers `invalid_args` when the arguments do not match
its schema, for example an office or a post that does not exist.

## 17. Limits of the design

- **The police and the court are roles of the city government.** The game
  brief asks for a police organization and a court organization for each
  city, made at world generation, with the `law.*` powers as grants of the
  city. The society module has no call with which another module can make
  an organization or give a grant: `org_create` and `org_grant` need a
  resident that acts. `unsup_society::offices` fills only the government
  organizations. So `police_chief`, `police_officer` and `judge` are
  roles of `OrgId::city(c)`, which holds `law.*` by itself. Zone code that
  checks `law.arrest@<city organization>` works with both designs. One
  function of `offices.rs` (`post_org`) says where a post is. One effect
  of this design: an official of a city cannot also hold a post of that
  city.
- **The fee of a candidate is the fee of the city in which it stands**,
  also for a national office. The zone takes the fee before the global
  realm knows the election.
- **A city law has no end.** A city that made its own value for a
  parameter keeps it until another city law changes it. No verb takes the
  value of a city away so that the national value applies again.
- **A city law has an effect only where code reads the parameter in the
  city.** A zone reads the value of its city. Code of the global realm
  that reads only the national value does not see a city law.
- **A rally that stops is paid.** The cost of a rally leaves the fund when
  the rally starts. A candidate that stops the action gets no money back.
- **An empty office stays empty** until the next regular election or an
  early election after a petition. The module appoints nobody in between.
- **One vote for each resident, not for each owner.** An owner with many
  residents has many votes. `voter_min_age` only stops residents that
  were made on the voting day.
- **Votes are in the state.** The state has the choice of each voter until
  the count, so that the voters of a candidate that left the world can
  vote again. No event, observation or published state shows it.
- **The residents of a city** are the residents that are in the city at
  that moment. The game has no registered home city. A visitor votes and
  signs where it is.
- **Names in published state** are the display names of the directory of
  the kernel, as plain text.
- **The list of open elections in the brief section** can be longer than
  `brief_list_max` on a day when a city votes and the nation registers.
  `more_elections` then counts the rest; full detail has them all.
