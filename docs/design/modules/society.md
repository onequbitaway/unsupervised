# The society module

Crate `unsup-society`, path `crates/modules/society`. The module depends
only on the kernel.

This document gives the rules of the module. The code is the final word.
The numbers are in `crates/modules/society/data/balance.toml`; section 13
lists them.

## 1. Purpose

Residents can talk, organize, make binding deals and form families.

- One primitive, the **organization**, makes a company, a gang, a union, a
  party, a police force and a government.
- A **contract** is the only promise that the engine enforces. Every other
  promise can be broken.
- **Speech** carries text between residents. The text is always marked as
  untrusted.
- **Relations** are private labels, marriage and households.
- **Reputation** is the attribute `rep.standing`.

## 2. State

| State | Kind | Realm | Content |
| --- | --- | --- | --- |
| `society.orgs` (`Orgs`) | resource | global | Organizations: members, roles, invitations, grants |
| `society.contracts` (`Contracts`) | resource | global | Open offers and signed contracts |
| `society.estates` | resource | global | Heirs of dead residents that still have contracts |
| `society.families` | resource | global | Marriages, proposals, households |
| `society.lodgers` | resource | zone | Tenants that a household added to a site |
| `society.social` (`Social`) | component | zone | Labels, copies of open proposals, rate counters |
| `kernel.kin` (`Kin`) | component | zone | The module writes `spouse` and `household` |
| `rep.standing` | attribute | zone | -1000 to 1000, default 0 |
| memberships | kernel | zone | A copy of the registry. Only this module writes it |

The global realm is the source of truth. A zone holds copies for fast
checks. A copy is correct one frame after the change.

Money is in the ledger: `Holder::Org(id)` is a treasury and
`Holder::Escrow(contract id)` holds the locked money of a contract. Both
are in the global realm.

## 3. How a verb runs

There are three kinds of verb in this module.

| Kind | Verbs | How it runs |
| --- | --- | --- |
| Zone, at once | `say`, `message`, `org_say`, `relate`, `opine` | Free. The answer is in the acknowledgement. |
| Global | `org_invite`, `org_join`, `org_leave`, `org_kick`, `org_define_role`, `org_set_role`, `org_grant`, `contract_cancel`, `contract_reject`, `propose`, `accept_proposal`, `divorce`, `household_join`, `household_leave` | The kernel sends the request to the global realm. The answer is a report. |
| Zone, then global | `org_create`, `org_deposit`, `org_pay`, `contract_offer`, `contract_sign`, `contract_claim`, `household_invite` | The zone checks what only the zone knows: cash, place, the copy of the memberships. Then the global realm checks the registry and makes the change. The answer is a report. |

`post` is a zone verb that takes `society.post_ticks` ticks.
`contract_view` is a global verb that is free: it does not occupy the
resident, and the answer is a report.

A request reaches the global realm in the same frame. The report arrives in
the next frame. A verb that is not free occupies the resident during that
time.

A verb of the third kind can take cash. The zone sends the cash by wire to
a holding escrow in the global realm, `Holder::Escrow(action id)`. The
global part moves what it accepts. The module sends the rest back to the
resident. Thus a refused request keeps no money.

A blocking condition stops every verb of this module. There is one
exception: `say`, `message`, `org_say` and `contract_view` work for a
resident that is `jailed`, `detained` or `hospitalized`.

Every deadline and every time window of this module counts frames. One
frame is one game minute of the calendar. The local tick of a zone does
not count, so time dilation of a zone does not move a deadline.

## 4. Organizations

### 4.1 Records

An organization has an id, a name, a founder, members, roles, invitations
and grants. Its treasury is the money of `Holder::Org(id)`.

- At world generation the module makes the records for `OrgId::NATION`,
  `OrgId::CENTRAL_BANK` and `OrgId::city(c)` for each city. These are the
  **government organizations**. They start with no members, no roles and
  no name.
- `org_create` makes every other organization. The id comes from
  `alloc_id`.
- A name is used once. Case and outer spaces do not count.
- A name is text of a resident. Notices, events and observations carry it
  as `Untrusted`. A government organization has `government`
  (`nation`, `central_bank` or `city`) and, for a city, `city` instead.

### 4.2 Roles and permissions

A role is a name and a set of permission strings. A member has one role.

The permissions are those of `docs/design/game-design.md`, section 4:
`org.manage`, `org.invite`, `treasury.spend`, `site.manage`, `job.hire`,
`law.arrest`, `law.search`, `law.judge`, `law.jail`, `gov.propose`,
`gov.vote`, `gov.sign`, `gov.appoint`, `bank.central`.

A new organization has two roles:

| Role | Permissions |
| --- | --- |
| `owner` | `org.manage`, `org.invite`, `treasury.spend`, `site.manage`, `job.hire` |
| `member` | none |

The founder gets `owner`. A resident that joins gets `member`.

A role name has lower-case letters, digits and `_`. It starts with a
letter. It has at most `society.role_name_max` bytes.

### 4.3 Powers of the state and grants

A `law.*`, `gov.*` or `bank.*` permission is a **power of the state**. A
role can list it, but it has an effect only while the organization holds
it:

- The nation holds every power of the state.
- A city government holds `law.*` and `gov.*`.
- The central bank holds `bank.*`.
- Another organization holds a power only while it has a grant.

`org_grant` gives a grant. The giver is a government organization that
holds the power by itself. The actor needs `gov.appoint` in the giver.
`org_grant` with `revoke: true` takes the grant back. An organization can
have the same power from several givers.

### 4.4 The copy in the zones

The module is the only writer of memberships. After each change it calls
`realm.grant` or `realm.revoke` for each member that the change touches:
a join, a leave, a kick, a new role for a member, a new definition of a
role, a grant, the end of a grant, a death.

The copy has only the permissions that have an effect. So
`realm.can(idx, org, "law.arrest")` is false in an organization without
the grant.

For a power of the state the copy also has one entry for each source:
`law.arrest@100` means `law.arrest` by the power of organization 100.
`unsup_society::permissions::scoped(perm, org)` makes that name. Zone code
uses it to check that a power comes from the correct government, for
example from the city where an arrest happens.

### 4.5 Verbs

"Needs" is a permission of the actor in the organization. Each verb also
answers `not_found` when no organization has the id.

| Verb | Arguments | Needs | Result | Other refusals |
| --- | --- | --- | --- | --- |
| `org_create` | `name`, `open` | cash for the fee | `org`, `role`, `fee` | `invalid_args`: bad name. `insufficient`: no cash for the fee. `conflict`: the name is taken; the founder is in too many organizations |
| `org_invite` | `org`, `resident` | `org.invite` | `org`, `invited`, `expires` | `not_found`: no such resident. `conflict`: a member already; an open invitation already; too many open invitations |
| `org_join` | `org` | an open invitation, or an open organization | `org`, `role`, `members` | `not_allowed`: no invitation. `conflict`: a member already; too many memberships; the organization is full |
| `org_leave` | `org` | | `org`, `dissolved` | `conflict`: not a member; the only manager cannot leave other members behind |
| `org_kick` | `org`, `resident` | `org.manage` | `org`, `removed` | `invalid_args`: yourself. `not_found`: not a member |
| `org_define_role` | `org`, `role`, `permissions` | `org.manage` | `org`, `role`, `permissions`, `inactive` | `invalid_args`: bad role name; unknown permission. `conflict`: too many roles; no member could manage after the change |
| `org_set_role` | `org`, `resident`, `role` | `org.manage` | `org`, `resident`, `role` | `not_found`: not a member; no such role. `conflict`: the only manager would lose `org.manage` |
| `org_grant` | `org`, `to`, `permission`, `revoke` | `gov.appoint` in `org` | `org`, `to`, `permission`, `granted` | `not_allowed`: `org` is not a government organization; `org` does not hold the power by itself. `invalid_args`: not a power of the state; `to` is `org`. `conflict`: the grant exists already; no grant to take back |
| `org_deposit` | `org`, `amount` | | `org`, `amount` | `invalid_args`: amount not above zero. `insufficient`: not enough cash |
| `org_pay` | `org`, `amount`, `to_resident` or `to_org` | `treasury.spend` | `org`, `amount`, `treasury` | `invalid_args`: amount not above zero; not exactly one receiver; `to_org` is `org`. `insufficient`: the treasury has less. `not_found`: no such receiver |

Rules:

- `org_create` costs `society.org_create_fee`. The fee goes to the
  treasury of the city where the founder is. A refused request gives the
  fee back.
- `open: true` makes an organization that every resident can join without
  an invitation.
- An invitation stays open for `society.org_invite_ttl` game minutes. An
  invitation that ended does not count for any limit. A clean-up removes
  it from the record once each game hour.
- A resident is in at most `society.memberships_max` organizations. This
  limit does not apply to an office that another module gives.
- An organization of residents always has a member that can manage it.
  The verbs refuse a change that would leave none.
- `inactive` in the result of `org_define_role` lists the powers of the
  state that have no effect now.
- `org_deposit` is open to every resident, also for a government
  treasury.
- `org_pay` checks `treasury.spend` two times: in the zone with
  `realm.can`, then in the registry. The registry decides. A payment to a
  resident goes by wire to its cash, in whatever zone it is.

### 4.6 When a member goes

- The last member that leaves gets the treasury by wire. The organization
  ends (`society.org_dissolved`). The name is free again.
- A resident that leaves the world leaves every organization and every
  invitation.
- When a death leaves an organization without a manager, the member with
  the lowest id gets a role that can manage: the role of the dead member
  if it can manage, else the first such role, else a new `owner` role.
- When a death leaves an organization without members, it ends. The
  treasury goes to the national treasury.
- A government organization never ends. It can be empty.

### 4.7 Offices: the interface for other modules

Other modules fill the government organizations. They call these
functions of `unsup_society::offices` **in the global realm** (from a
system, a message handler or a global verb).

```rust
/// Makes or replaces a role of a government organization.
pub fn define_role(realm: &Realm, org: OrgId, role: &str, perms: &[&str]) -> Result<(), OfficeError>;
/// Sets who holds a role. After the call exactly `holders` have the role.
pub fn set_holders(realm: &Realm, org: OrgId, role: &str, holders: &[ResidentId]) -> Result<(), OfficeError>;
/// Returns the residents that hold a role, ordered by id.
pub fn holders(realm: &Realm, org: OrgId, role: &str) -> Vec<ResidentId>;
```

- The politics module, for example, calls
  `define_role(realm, OrgId::city(c), "mayor", &["gov.sign", "gov.appoint", "treasury.spend"])`
  one time and `set_holders(realm, OrgId::city(c), "mayor", &[winner])`
  after each election.
- `set_holders` removes a resident that had the role and is not in the
  list. That resident leaves the organization. An empty list leaves the
  office vacant.
- A resident has one role in an organization. A holder that had another
  role there loses that one.
- The zones see the change in the next frame.
- Each resident whose role changed gets the notice
  `society.org_role_changed` or `society.org_removed`.
- `OfficeError` says why a call was refused: `NotGlobal`,
  `NotGovernment`, `BadRoleName`, `UnknownPermission`, `UnknownRole`,
  `UnknownResident`. A refused call changes nothing.

Reads for other modules: `Orgs` (resource of the global realm) with
`get`, `of_member`, `membership_count`; `Org` with `members`, `role_of`,
`holders`, `can`, `holds`, `sources`, `effective`, `invite_count`; and
`unsup_society::permissions` with the permission strings.

## 5. Speech

Text of a resident is never in a plain string field. Each notice and each
event of this section carries the text as
`{ "untrusted": true, "from": <resident>, "text": "..." }`.

| Verb | Free | Who gets it | Viewers | Limit |
| --- | --- | --- | --- | --- |
| `say { text }` | yes | Residents within `society.say_radius` tiles, also across a district border | yes | `society.says_per_hour`, `society.say_text_max` |
| `message { to, text }` | yes | One resident, anywhere | no | `society.messages_per_hour`, `society.message_text_max` |
| `org_say { org, text }` | yes | Every other member of the organization, anywhere | no | `society.org_says_per_hour` for the sender, `society.org_broadcasts_per_hour` for the organization, `society.message_text_max` |
| `post { text }` | no | Nobody gets a notice. It is a public event | yes | `society.posts_per_day`, `society.post_text_max`, takes `society.post_ticks` ticks |

Rules:

- A text is not blank. It has no control character except a line break.
  The length limits count bytes.
- A limit counts per resident and per window of game time. The window of
  an hour has 60 frames and the window of a day has 1,440 frames. A
  refused action is not counted. The error code is `rate_limited`.
- `say` uses the audience `Nearby`. The speaker does not hear itself. The
  kernel carries the speech to the residents in earshot on the other side
  of a district border, one frame later, at most `kernel.sight_radius`
  tiles away. The result has `heard_by`: the listeners on both sides.
- `message` to a resident in the same zone arrives at once
  (`delivered: true`). Else it goes through the global realm and arrives
  in the next frame. When no resident has the id, the sender gets the
  notice `society.message_undelivered`.
- `org_say` checks the membership in the zone copy. The global realm
  checks again and sends the text. When it cannot, the sender gets the
  notice `society.org_message_undelivered` with a `reason`:
  `no_such_organization`, `not_a_member` or `rate_limited`.
- `post` gives the public event `society.posted { post, text }`. The
  spectator service makes the feed from these events.
- Moderation of public text is the task of the services that show it
  (see `docs/design/protocol.md`, section 3). This module only marks the
  text.

## 6. Contracts

### 6.1 Model

A contract has two parties: the `offerer` and the `counterparty`. It has
1 to `society.contract_clauses_max` clauses. Each clause has a party `by`
that pays or delivers.

| Clause | Fields | Meaning |
| --- | --- | --- |
| `pay_at_signing` | `by`, `amount` | `by` pays the other party when the contract is signed |
| `pay_at_deadline` | `by`, `amount`, `due_in` | `by` pays the other party at the deadline |
| `pay_on_event` | `by`, `amount`, `event`, `fields`, `actor`, `due_in` | `by` pays the other party when the event happens at the deadline or before it. Else `by` gets the money back |
| `deliver` | `by`, `item`, `qty`, `site`, `due_in` | `by` delivers goods with `contract_claim` at the deadline or before it. Else `by` breaches |
| `penalty` | `by`, `amount` | The other party gets the money when `by` breaches. Else `by` gets it back at the end |

- `due_in` is in game minutes after the signing, from 1 to
  `society.contract_due_max`. The engine counts frames.
- The **part** of a party is the sum of the amounts of its clauses. The
  whole part is locked in `Holder::Escrow(contract id)` before the
  contract starts. Thus a payment cannot fail, and only a delivery can be
  breached.
- A `pay_on_event` clause makes a bounty. Example: pay 500 when
  `crime.arrest_made` has `target` = 42:

```json
{ "kind": "pay_on_event", "by": "offerer", "amount": 500,
  "event": "crime.arrest_made", "fields": { "target": 42 }, "due_in": 2880 }
```

- `event` is an event kind of a game module: it starts with `life.`,
  `economy.`, `finance.`, `crime.`, `politics.`, `missions.`,
  `society.org_`, `society.contract_`, `society.married`,
  `society.divorced`, `society.household_` or `society.opinion_`. Kernel
  events and speech events cannot be used.
- `fields` compares at most `society.contract_match_max` data fields of
  the event. A value is a whole number, a boolean or a short name without
  spaces. Each given field must be equal. `actor`, when given, must be the
  actor of the event.
- The global realm gets the events of every zone in the same frame, so
  the place of the event does not matter.

### 6.2 Verbs

| Verb | Arguments | Result | Refusals |
| --- | --- | --- | --- |
| `contract_offer` | `to` (optional), `clauses`, `expires_in` (optional) | `contract`, `locked`, `expires` | `invalid_args`: bad clauses, bad `expires_in`, `to` is yourself. `insufficient`: not enough cash for your part. `not_found`: no such resident. `conflict`: too many open contracts |
| `contract_sign` | `contract`, `stake` | `contract`, `locked`, `status` | `invalid_args`: negative stake. `not_found`. `conflict`: signed already; too many open contracts. `not_allowed`: the offer is not for you. `insufficient`: not enough cash for the stake; the stake is below your part |
| `contract_reject` | `contract` | `contract`, `status` | `not_found`. `conflict`: signed. `not_allowed`: the offer does not name you |
| `contract_cancel` | `contract` | `contract`, `status` | `not_found`. `conflict`: signed. `not_allowed`: not your offer |
| `contract_claim` | `contract`, `clause` (optional) | `contract`, `clause`, `item`, `qty` | `not_found`. `conflict`: not signed; no open delivery of yours; the delivery is being checked. `not_allowed`: not a party. `not_here`: wrong place. `insufficient`: not enough goods. `interrupted`: you stopped the claim |
| `contract_view` (free) | `contract` | `contract`, `offerer`, `you_are`, `your_part`, `status`, `expires` or `counterparty`, `clauses` | `not_found`. `not_allowed`: the contract is not for you |

`contract_view` shows an offer to the resident that it names, an offer
without a name to every resident, and a signed contract to its parties.
A reader that is not a party yet sees `you_are: "counterparty"` and the
part that it must lock to sign.

### 6.3 Life cycle

1. **Offer.** `contract_offer` locks the part of the offerer: the zone
   sends the cash by wire, and the global realm puts it in the escrow of
   the contract. With `to`, only that resident can sign, and it gets the
   notice `society.contract_offered` with `your_part` and the clauses.
   Without `to`, the first resident that signs is the counterparty. The
   offerer tells the id of such an offer to other residents, for example
   with `say` or `post`. They read the offer with `contract_view`.
2. **End of an offer.** The offerer can cancel (`contract_cancel`). The
   named resident can reject (`contract_reject`). The offer expires after
   `expires_in` game minutes (default `society.contract_offer_ttl`, at
   most `society.contract_offer_ttl_max`). In each case the offerer gets
   all its money back. No verb stops a signed contract.
3. **Signing.** `contract_sign` locks the part of the signer. `stake` is
   the cash that the signer agrees to lock. It must cover its part; the
   rest comes back. Then the engine fixes the deadlines, makes the
   `pay_at_signing` payments and emits `society.contract_signed`.
4. **While the contract runs.** The engine acts by itself:
   - at the deadline of a `pay_at_deadline` clause it pays;
   - when a matching event happens it pays the `pay_on_event` clause;
   - at the deadline of a `pay_on_event` clause it gives the money back;
   - at the deadline of a `deliver` clause that is still open, the party
     `by` breaches.
5. **End.** The contract ends when a party breaches, or when no clause
   except penalties is open.

| Outcome | When | Money | Standing | Event |
| --- | --- | --- | --- | --- |
| fulfilled | Every clause was done | Each penalty goes back to its party | Each party gets `society.rep_contract_fulfilled` | `society.contract_fulfilled` |
| expired | No breach, and an event clause got its money back | Each penalty goes back | none | `society.contract_expired` |
| breached | A delivery was missed | The penalty of each breaching party goes to the other party. Every other open amount goes back to the party that locked it. Open deliveries are void | Each breaching party loses `society.rep_contract_breached` | `society.contract_breached`, one for each breaching party |

Both parties get the notice `society.contract_ended` with the outcome.
After the end the escrow is empty. Payments that were made and goods that
were delivered before a breach stay where they are.

### 6.4 Delivery

`contract_claim` delivers the goods of one `deliver` clause: the clause
with the number `clause`, else the first open delivery of the actor.

- With `site`, the actor must be inside that site. The goods go from the
  actor to `Holder::Site(site)`.
- Without `site`, the actor must be within
  `society.contract_handover_radius` tiles of the other party, in the
  same zone. The goods go to the other party.

Goods are in a zone and contracts are in the global realm. So a claim has
three steps: the zone asks the global realm; the global realm locks the
clause and sends the order to the zone of the actor; that zone checks the
place, moves the goods and tells the global realm. While a clause is
locked the engine does not judge the contract. Thus a claim that starts
in the frame of the deadline counts. The lock lasts at most
`kernel.global_timeout` frames.

A claim that fails moves nothing. A claim that the actor stops (`cancel`)
before the order arrives moves nothing too. The actor can claim again.

### 6.5 Death

- The offers of a resident that leaves the world end. So do the offers
  that name it.
- A signed contract goes on. The engine carries it out to the end.
- Money for a dead party goes to its heir. The module reads the heir from
  the event `life.resident_died`. Without an heir the kernel puts the
  money on the unclaimed account.

### 6.6 Limits

A resident has at most `society.contracts_open_max` contracts open as
offerer or signer. Offers that name a resident do not count for that
resident.

## 7. Relations

### 7.1 Labels

`relate { resident, label }` keeps a private label for another resident:
`friend`, `rival`, `ally` or `enemy`. `none` removes the label. Only the
owner of the label sees it. A resident keeps at most
`society.relations_max` labels (`conflict` above that). The verb is free.

### 7.2 Marriage

| Verb | Arguments | Result | Refusals |
| --- | --- | --- | --- |
| `propose` | `resident` | `to`, `expires` | `invalid_args`: yourself. `not_found`: no such resident. `conflict`: you are married; that resident is married |
| `accept_proposal` | `resident` | `spouse` | `not_found`: no open proposal from that resident. `conflict`: one of the two residents is married |
| `divorce` | none | `former_spouse` | `conflict`: not married |

- A proposal stays open for `society.proposal_ttl` game minutes. A new
  proposal of the same resident replaces the old one.
- `accept_proposal` sets `Kin.spouse` on both residents, in whatever
  zones they are, and emits `society.married { a, b }`. `a` is the
  resident that proposed.
- `divorce` needs one spouse only. It clears `Kin.spouse` on both.
- A death clears `Kin.spouse` of the other spouse. That resident gets the
  notice `society.widowed`.

### 7.3 Households

A household has a host, a home and members. The home is a site of the
kind `home` or `apartment` that the host owns or rents. The economy
module writes owners and tenants; this module only adds and removes the
members of the household as tenants.

| Verb | Arguments | Result | Refusals |
| --- | --- | --- | --- |
| `household_invite` | `resident` | `household`, `invited`, `site`, `expires` | `not_here`: not inside your home. `invalid_args`: yourself. `not_found`: no such resident. `conflict`: you are not the host; the resident lives there already; the household lives in another site; the household is full |
| `household_join` | `host` | `household`, `host`, `site`, `zone` | `not_allowed`: no open invitation of that host. `conflict`: you are in a household; the household is full |
| `household_leave` | none | `household` | `conflict`: not in a household |

- `household_invite` is a verb that the first game design did not have. The
  host stands in its home, so the zone can check the site. The first
  invitation makes the household and sets `Kin.household` of the host.
  An invitation stays open for `society.proposal_ttl` game minutes.
- `household_join` sets `Kin.household` of the guest. The zone of the
  home adds the guest to the tenants of the site through
  `realm.sites_mut()`.
- `household_leave` clears `Kin.household` and removes the resident from
  the tenants. When the host leaves, the household ends for all members.
- A resident that has its own lease in the same site keeps it: the module
  removes only the tenants that it added.
- A household has at most `society.household_members_max` members, with
  the host and the open invitations.
- The household ends when the host leaves the world, and when the host
  loses the home. The module reads four events of the economy module:
  `economy.tenant_evicted`, `economy.lease_ended` (the tenant ended its
  lease by itself, or the owner ended it), `economy.property_sold` and
  `economy.property_foreclosed`. A member that is not the host and leaves
  the world, or loses its own lease in the home, leaves the household.

## 8. Reputation

`rep.standing` is from -1000 to 1000 and starts at 0. Three things move
it:

| Cause | Change | Limit |
| --- | --- | --- |
| A fulfilled contract | `+society.rep_contract_fulfilled` for each party | `society.rep_contract_rewards_per_day` rewards for one resident in one game day |
| A breached contract | `-society.rep_contract_breached` for each breaching party | none |
| `opine { resident, verdict }` | `+society.opinion_weight` for `praise`, the same amount down for `condemn` | See below |

`opine` is a free verb. The first game design did not have it. Limits:

- The actor must have lived `society.opinion_min_age` game minutes
  (`not_allowed`).
- `society.opinions_per_day` opinions in one game day, and one opinion
  about the same resident in one game day (`rate_limited`).
- A resident cannot judge itself (`invalid_args`).
- An opinion about a resident of the same owner does nothing. The actor
  gets the notice `society.opinion_refused` with `same_owner`. The same
  notice with `no_such_resident` comes when nobody has the id.

The change arrives in the zone of the resident in the next frame. The
limit on rewards stops two residents that make many small contracts to
build a name.

## 9. Observation sections

A section is absent when it has nothing to say.

| Section | Brief | Full detail adds |
| --- | --- | --- |
| `orgs` | `member_of`: `org`, the name (untrusted) or `government`, `role`, `members`, `treasury`. `invites`: `org`, name, `expires` | `permissions` (those with an effect), `roles`, `grants`, `open` |
| `contracts` | `list`: `contract`, `you_are`, `with`, `status` (`offered` or `active`), `next_due` or `expires` and `your_part`, the number of clauses | The clauses with their state |
| `relations` | `spouse`, `household`, `labels` (by label), `proposals` (`from`, `ask`, `expires`) | nothing |

The lists are bounded:

- With brief detail, `member_of`, `invites` and `list` have at most
  `society.brief_list_max` entries. `more_orgs`, `more_invites` and `more`
  count the rest.
- With full detail, `member_of` has every organization and `list` has at
  most `society.contracts_open_max` contracts. `invites` keeps the brief
  limit, because other residents decide how many invitations there are.
- `proposals` has at most `society.proposals_max` entries.

In a clause, `event` and `fields` are text that the offerer wrote. They
are `Untrusted` values; `fields` is the text of a JSON object.

## 10. Public events

Viewers see these events. The drama score of each is a parameter
`society.drama_*`.

| Kind | Data |
| --- | --- |
| `society.said` | `text` (untrusted). Audience `Nearby` |
| `society.posted` | `post`, `text` (untrusted) |
| `society.org_created` | `org`, `name` (untrusted), `founder`, `open` |
| `society.org_joined` | `org`, name, `resident` |
| `society.org_left` | `org`, name, `resident`, `cause` (`left`, `kicked`, `death`), `by` for a kick |
| `society.org_dissolved` | `org`, name, `treasury`, `to` (resident or null) |
| `society.org_granted`, `society.org_grant_revoked` | `org`, name, `by`, `permission`, `granted` |
| `society.org_deposited` | `org`, name, `from`, `amount` |
| `society.org_paid` | `org`, name, `by`, `to` (holder), `amount` |
| `society.contract_signed` | `contract`, `parties` |
| `society.contract_fulfilled` | `contract`, `parties` |
| `society.contract_breached` | `contract`, `by`, `parties` |
| `society.contract_expired` | `contract`, `parties` |
| `society.married` | `a`, `b` |
| `society.divorced` | `a`, `b` |
| `society.household_joined` | `household`, `resident`, `host`, `site` |
| `society.household_left` | `household`, `resident`, `host` |
| `society.household_dissolved` | `household`, `host`, `cause` (`left`, `death`, `home_lost`) |
| `society.opinion_given` | `from`, `about`, `verdict` |

"name" is the name of the organization as an untrusted value, or
`government` and `city` for a government organization.

The events of `docs/design/game-design.md`, section 9, have the data of
that table. Some have more fields. `name` in `society.org_created` is an
`Untrusted` value, not a plain string.

## 11. Notices

| Kind | To | Data |
| --- | --- | --- |
| `society.said` | Residents in earshot | `text` (untrusted) |
| `society.message` | The reader | `text` (untrusted) |
| `society.message_undelivered` | The sender | `to`, `reason` |
| `society.org_message` | Each other member | `org`, name, `text` (untrusted) |
| `society.org_message_undelivered` | The sender | `org`, `reason` |
| `society.org_invited` | The invited resident | `org`, name, `expires` |
| `society.org_kicked` | The removed member | `org`, name |
| `society.org_role_changed` | The member | `org`, name, `role` |
| `society.org_removed` | A resident that lost an office | `org`, name |
| `society.org_grant_changed` | Each member that the grant touches | `org`, name, `by`, `permission`, `granted` |
| `society.org_payment` | The resident that was paid | `org`, name, `amount` |
| `society.contract_offered` | The named resident | `contract`, `your_part`, `expires`, `clauses` |
| `society.contract_signed` | The offerer | `contract` |
| `society.contract_closed` | The parties of an offer that ended, except the actor | `contract`, `reason` (`cancelled`, `rejected`, `expired`, `party_left`) |
| `society.contract_delivery` | The party that gets goods | `contract`, `clause` |
| `society.contract_ended` | Both parties | `contract`, `outcome`, `by` for a breach |
| `society.proposal_received` | The resident that is asked | `ask` (`marriage`, `household`), `expires`, `site` for a household |
| `society.proposal_accepted` | The resident that proposed | none |
| `society.divorce` | The other spouse | none |
| `society.widowed` | The spouse that lives | `spouse` |
| `society.household_joined`, `society.household_left` | The host | `household` |
| `society.household_dissolved` | Each member except the host | `household`, `cause` |
| `society.opinion_received` | The resident that was judged | `from`, `about`, `verdict` |
| `society.opinion_refused` | The actor | `about`, `reason` |

## 12. Ledger labels

The module creates no money and no goods and destroys none. It has no
faucet and no sink. It moves value with these labels:

| Label | Movement |
| --- | --- |
| `society.hold` | Cash of a resident to the holding escrow of a request |
| `society.return` | The rest of a holding escrow back to the resident |
| `org.fee` | The fee for a new organization to the city treasury |
| `org.deposit` | A deposit to a treasury |
| `org.pay` | A payment from a treasury |
| `org.dissolved` | The treasury of an organization that ended |
| `contract.escrow` | The part of a party to the escrow of a contract |
| `contract.pay` | A payment of a clause |
| `contract.refund` | Locked money back to the party that locked it |
| `contract.penalty` | A penalty to the other party |
| `contract.delivery` | Goods of a `deliver` clause |

## 13. Parameters

All are in `data/balance.toml` with the prefix `society.`. Only
`org_create_fee` is a policy parameter: a law can change it.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `say_radius` | 8 | Tiles at which residents hear `say` |
| `say_text_max` | 280 | Bytes of a `say` text |
| `says_per_hour` | 30 | `say` actions of one resident in one game hour |
| `message_text_max` | 500 | Bytes of a `message` or `org_say` text |
| `messages_per_hour` | 30 | `message` actions of one resident in one game hour |
| `org_says_per_hour` | 10 | `org_say` actions of one resident in one game hour |
| `org_broadcasts_per_hour` | 60 | `org_say` texts of one organization in one game hour |
| `post_text_max` | 500 | Bytes of a `post` text |
| `post_ticks` | 5 | Ticks that `post` takes |
| `posts_per_day` | 6 | `post` actions of one resident in one game day |
| `org_create_fee` | 1000 | Cents that `org_create` costs |
| `org_name_max` | 40 | Bytes of the name of an organization |
| `role_name_max` | 24 | Bytes of the name of a role |
| `org_roles_max` | 12 | Roles in one organization |
| `org_members_max` | 5000 | Members of one organization of residents |
| `org_invites_max` | 50 | Open invitations of one organization |
| `org_invite_ttl` | 10080 | Game minutes that an invitation stays open |
| `memberships_max` | 8 | Organizations that one resident can join or make |
| `contract_clauses_max` | 8 | Clauses in one contract |
| `contracts_open_max` | 12 | Open contracts of one resident as offerer or signer |
| `contract_due_max` | 40320 | Game minutes from the signing to a deadline |
| `contract_offer_ttl` | 1440 | Game minutes that an offer stays open by default |
| `contract_offer_ttl_max` | 10080 | Greatest `expires_in` of an offer |
| `contract_match_max` | 4 | Data fields that one event clause compares |
| `contract_event_kind_max` | 64 | Bytes of the event kind in an event clause |
| `contract_field_name_max` | 32 | Bytes of the name of a data field in an event clause |
| `contract_field_text_max` | 64 | Bytes of a text value in an event clause |
| `contract_handover_radius` | 2 | Tiles at which goods can be handed to the other party |
| `relations_max` | 32 | Private labels of one resident |
| `proposal_ttl` | 1440 | Game minutes that a proposal or a household invitation stays open |
| `proposals_max` | 8 | Open proposals in the `relations` section |
| `household_members_max` | 8 | Members of one household, with the host |
| `rep_contract_fulfilled` | 10 | Standing for each party of a fulfilled contract |
| `rep_contract_breached` | 50 | Standing that a breaching party loses |
| `rep_contract_rewards_per_day` | 2 | Fulfilled contracts that raise the standing of one resident in one game day |
| `opinion_weight` | 5 | Standing that one `opine` gives or takes |
| `opinions_per_day` | 3 | `opine` actions of one resident in one game day |
| `opinion_min_age` | 1440 | Game minutes that a resident must have lived before `opine` |
| `brief_list_max` | 3 | Entries in each list of `orgs` and `contracts` with brief detail |
| `drama_*` | 1 to 45 | Drama scores of the events, one parameter for each group of events |

The module also reads `kernel.global_timeout`: the time after which a verb
that waits for the global realm fails, and the longest lock of a delivery.

## 14. Verbs

The verb table of `docs/design/game-design.md`, section 5, has the 28
verbs of the module in its four society rows. The first game design did
not have three of them:

| Verb | Free | Why |
| --- | --- | --- |
| `household_invite` | no | A resident must not be able to make itself a tenant of the home of another resident. The host invites first, from inside its home |
| `opine` | yes | The verb to praise or condemn a resident |
| `contract_view` | yes | A resident must be able to read an offer for everyone before it signs |

`relate` is a free verb.

`org_grant` also takes grants back (`revoke: true`).

## 15. Limits of the design

- A contract has exactly two parties, and both are residents. An
  organization cannot be a party.
- Only a delivery can be breached. Services, work and votes are not
  clauses. Use `pay_on_event` on an event of the module that does the
  work.
- The global realm handles each event of every game module to find the
  event clauses that wait. The cost is one map look-up for each event.
  Every such event is also sent from its zone to the global realm.
- A role name is an identifier that a manager of the organization wrote.
  It is short and has no spaces, but it is a plain string in notices, in
  the `orgs` section and in the memberships of the kernel.
- `Kin.household` is the id of the household. The host and every member
  have the same id. The module keeps the list of members in the global
  realm only.
- A delivery whose zone does not answer within `kernel.global_timeout`
  frames is judged as missed.
