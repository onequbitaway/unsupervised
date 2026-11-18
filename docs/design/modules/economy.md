# Economy module

This document gives the rules and the formulas of the economy module
(`crates/modules/economy`). The model is in `docs/design/economy.md`. The
contracts with the other modules are in `docs/design/game-design.md`,
section 9. The finance module owns the bank, loans, the exchange, shares and
statistics. They are not here.

## 1. What the module does

A resident can earn, make, sell, buy and own.

- A resident works a shift for a wage, or gathers and produces for itself.
- A farm, a mine or a forest gives raw goods. A recipe turns goods into
  other goods.
- A shop sells goods to residents and can buy goods from them.
- The depot buys and sells every basic good at fixed prices.
- A resident buys, sells and lets sites, and opens a business.
- The city gets income tax, sales tax, property tax and permit fees.

Money and goods enter and leave the world only through these faucets and
sinks:

| Name | Kind | What | When |
| --- | --- | --- | --- |
| `gov.public_works` | faucet | money | A starter shift ends |
| `export.sale` | faucet | money | A resident sells goods to the depot |
| `import.purchase` | sink | money | A resident buys goods from the depot |
| `nature.harvest` | faucet | goods | `gather`, or a shift at a farm, a mine or a forest |
| `production.output` | faucet | goods | A recipe ends |
| `import.purchase` | faucet | goods | A resident buys goods from the depot |
| `production.input` | sink | goods | A recipe ends |
| `export.sale` | sink | goods | A resident sells goods to the depot |

All other movements are transfers. A recipe uses its inputs at the same
moment at which it makes its outputs. An action that stops before its end
uses nothing.

## 2. Data files

Every number is in `crates/modules/economy/data`.

| File | Content |
| --- | --- |
| `balance.toml` | Parameters: taxes, wages, times, limits, drama scores |
| `items.toml` | The goods: weight, tags, `base_price`, `nutrition`, `healing` |
| `recipes.toml` | The recipes: inputs, outputs, minutes, skill, least level, site kinds |
| `sites.toml` | What each site kind can do, the goods of a node and its stock |
| `depot.toml` | The floor price and the ceiling price of each good at the depot |

Each depot price becomes a policy parameter: `economy.depot_floor.<item>`
and `economy.depot_ceiling.<item>`.

Goods: `grain`, `vegetables`, `cotton`, `herbs`, `wood`, `ore` (raw),
`bread`, `meal`, `cloth`, `metal`, `tools`, `furniture`, `clothes`,
`medicine`, and the contraband `moonshine` and `stims`. Food has the tag
`food` and the attribute `nutrition`. Medicine has the tag `medicine` and
the attribute `healing`. Contraband has the tag `contraband`. The depot does
not trade contraband.

## 3. Sites

The life module creates the sites. This module reads `kind`, `owner`, the
attribute `value` and the tag `public`. It writes:

| Field | Meaning |
| --- | --- |
| `owner` | The holder that owns the site |
| `tenants` | The residents that rent the site |
| `kind` | `open_business` can change one business kind into another |
| tag `open` | The site is a business with a permit |
| attribute `price` | The price at which the owner sells the site |
| attribute `rent` | The daily rent at which the owner lets the site |
| attribute `tax_due` | Property tax that the site owes |
| attribute `tax_days` | Days in a row that the site owed property tax |

The module reads the attribute `capacity` (how many tenants a site takes)
when the site has it.

A site has a **till** (the money of `Holder::Site`) and a **stock** (the
goods of `Holder::Site`).

A resident **controls** a site when it is the owner, or when it is a member
of the organization that owns the site and has the permission `site.manage`.
A member with `job.hire` can also post and close jobs and fire workers.

A resident is at a site when its position is inside the site. Most verbs act
on the site where the resident is. They have no site argument.

Site kinds:

| Kinds | Use |
| --- | --- |
| `farm`, `mine`, `forest` | A node with raw goods |
| `shop`, `restaurant` | Business. The owner sets prices; residents buy and sell |
| `workshop`, `factory`, `restaurant` | Business. Recipes work here (see `recipes.toml`) |
| `office`, `warehouse` | Business without a special use |
| `home`, `apartment` | The owner can let it to tenants |
| `city_hall`, `depot` | Public works. The `depot` also trades with the system |

A site of another kind can have jobs, a till and a stock. Nothing else.

## 4. Verbs

(F) marks a free verb. Each other verb occupies the resident for the time in
the table. The time parameters are in game minutes.

A timed verb checks the request two times: when it starts and when it ends.
A request that cannot work is refused at once and changes nothing. A request
that cannot work at the end fails in the action report and changes nothing.

### Work

| Verb | Arguments | Time | What it does |
| --- | --- | --- | --- |
| `list_jobs` (F) | `kind`, `skill`, `min_wage`, `limit` | none | The jobs of the city that take workers. The best wage first, then the nearest site. The answer also shows the starter shifts of the city (see 5.5) |
| `post_job` | `wage`, `slots`, `min_level`, `task` | `paperwork_minutes` | Offers a job at the site |
| `close_job` | `job` | `paperwork_minutes` | Ends a job. The workers lose it |
| `apply_job` | `job` | `paperwork_minutes` | Takes a job at the site. A resident has one job |
| `quit_job` | none | `paperwork_minutes` | Leaves the job. It works from every place |
| `fire` | `resident` | `paperwork_minutes` | Ends the job of a worker of the site |
| `work` | none | `shift_minutes` | One shift: the job at the site, or public works |
| `gather` | `item`, `minutes` | `minutes` | Takes raw goods from the node. The goods go to the resident |
| `produce` | `recipe`, `batches` | see 5.4 | Runs a recipe with the stock of the site |
| `list_recipes` (F) | `site`, `output` | none | The recipes |

### Trade

| Verb | Arguments | Time | What it does |
| --- | --- | --- | --- |
| `stock` | `item`, `qty` | `trade_minutes` | Moves goods from the resident into the stock of the site |
| `unstock` | `item`, `qty` | `trade_minutes` | Moves goods from the stock to the resident |
| `set_price` | `item`, `price`, `buy_price` | `paperwork_minutes` | Sets what the shop takes and what it pays for a good |
| `buy` | `item`, `qty`, `max_price` | `trade_minutes` | Buys goods at a shop, a restaurant or the depot |
| `sell` | `item`, `qty`, `min_price` | `trade_minutes` | Sells goods to a shop, a restaurant or the depot |
| `till_collect` | `amount` | `trade_minutes` | Takes cash out of the till |
| `till_fund` | `amount` | `trade_minutes` | Puts cash into the till |

### Property

| Verb | Arguments | Time | What it does |
| --- | --- | --- | --- |
| `list_property` (F) | `kind`, `offer`, `max_price`, `limit` | none | The sites of the city that are for sale or to let |
| `buy_property` | `max_price`, `org` | `deal_minutes` | Buys the site |
| `sell_property` | `price` | `paperwork_minutes` | Offers the site for sale, or ends the offer |
| `rent_out` | `rent` | `paperwork_minutes` | Offers the site to tenants, or ends the offer |
| `rent` | `max_rent` | `deal_minutes` | Becomes a tenant of the site |
| `end_lease` | `tenant` | `paperwork_minutes` | Ends a lease |
| `open_business` | `kind` | `deal_minutes` | Pays the permit fee and opens the site as a business |
| `close_business` | none | `deal_minutes` | Gives up the permit |

The verb table of `docs/design/game-design.md`, section 5, has each of
these verbs.

### Error codes

| Code | Meaning in this module |
| --- | --- |
| `not_here` | The resident is not at a site, or not at a site of the kind or with the job that the verb needs |
| `not_found` | No such good, recipe, job, worker or tenant; or the shop does not trade the good |
| `not_allowed` | The resident does not control the site; the business has no permit; the level is too low; the site is public or not offered |
| `insufficient` | Not enough cash, goods, till money, energy, node stock or public works budget |
| `conflict` | The state does not fit: a job already, no free slot or unit, a price past the limit of the resident, an open business already |
| `invalid_args` | A number is out of range, or a task or a business kind does not fit the site |

## 5. Work

### 5.1 Skills, mood and energy

The life module owns the attributes `skill.<name>`, `need.mood` and
`need.energy`. This module declares them with the same definition.

```
level       = floor(sqrt(xp / 100))
mood_share  = mood_floor_bp + (10,000 - mood_floor_bp) * mood / 1,000     (basis points)
xp_gain     = xp_per_hour * minutes * mood_share / (60 * 10,000)
energy_cost = energy_per_hour * minutes / 60
```

All divisions round down. With the default numbers, one hour of work gives
10 experience points at full mood and 5 at mood 0, and costs 20 energy
points.

`work`, `gather` and `produce` give experience and cost energy when they
end. They do not start when `need.energy` is below `labor_min_energy`.

The skill of a shift is the skill of the site kind (`sites.toml`): `farming`
at a farm, `mining` at a mine, `trading` at a shop, `labor` at a city hall.
The skill of a recipe is in `recipes.toml`.

### 5.2 Jobs and shifts

The owner posts a job with a wage for one shift. The wage is not below
`min_wage`. A site has at most `site_jobs_max` jobs. A business needs a
permit before it can post a job.

The `task` of a job says what a shift makes:

- At a farm, a mine or a forest: the good to gather. Without a task, the
  first good of the site.
- At a site where recipes work: the name of a recipe. The job then needs at
  least the level of the recipe.
- Without a task at another site: the shift makes nothing.

`apply_job` hires at once. The employer controls who works there with
`min_level` and `fire`.

A shift lasts `shift_minutes`. At the end:

1. The till must hold the wage. If not, the shift fails with `insufficient`.
   The worker gets nothing, the owner gets nothing, and the event
   `economy.shift_unpaid` tells viewers.
2. The till pays the wage.
3. The task is done. Everything that it makes goes into the stock of the
   site.
4. The worker gets experience and loses energy.

```
tax = wage * income_tax_bp / 10,000
net = wage - tax
```

The worker gets `net` in cash. The till wires `tax` to the city treasury.

A shift also does not start when the till cannot pay the wage, or when the
stock cannot feed one batch of the recipe.

A shift that gathers uses the formula of 5.3 with the minutes of the shift.
A shift with a recipe makes as many batches as fit in the time and in the
stock:

```
batches = min(shift_minutes * speed / (recipe_minutes * 10,000), batches that the stock can feed)
speed   = 10,000 + (level - recipe_min_level) * speed_bonus_bp
```

A worker that is fired, or that leaves the site, before the end of the shift
gets no wage. The owner gets no goods.

### 5.3 Gathering

A farm, a mine and a forest have a node. The node has a stock of units
(`node_max` when nobody used it). It gets `node_regen_per_hour` units back
each game hour.

```
units = per_hour * minutes / 60 * (10,000 + level * skill_bonus_bp) / 10,000 * mood_share / 10,000
units = min(units, stock of the node)
```

`per_hour` is the number of the good in `sites.toml`. With the default
numbers a resident of level 0 gathers 6 units of grain in one hour, and a
resident of level 5 gathers 9.

Who can use `gather`:

- everybody, on a site that a city or the nation owns;
- the resident that controls the site.

A worker of the site uses `work`. The goods of `gather` go to the resident.
The goods of a shift go to the stock of the site.

### 5.4 Production

`produce` runs a recipe at a site of a kind that the recipe names. The
resident must control the site or have a job there. The business must have a
permit. The resident needs the least level of the recipe.

```
ticks = ceil(recipe_minutes * batches * 10,000 / speed)
```

At the end the inputs leave the stock of the site and the outputs enter it.
When an input is missing at the end, the action fails with `insufficient`
and nothing changes.

### 5.5 Public works

A resident without a job at the site can work a shift at a `city_hall` or a
`depot` that a government owns. The wage is `public_works_wage` (2,800). It
comes from the faucet `gov.public_works`. Income tax applies as for every
wage: 2,520 are left. Three vegetables, the food of one day, cost 2,250 at
the depot. Keep one shift above one day of food when you change the wage,
the tax or the depot prices
(`docs/design/game-design.md`, section 10).

Each city pays at most `public_works_budget` in one game day (280,000: one
hundred shifts). The global
realm counts the budget, because a city has more than one zone.

1. The shift starts at once. The zone asks the global realm for the wage.
2. Two ticks later the answer is there. A grant reserves the wage. A refusal
   stops the shift with the error `interrupted`.
3. When the budget of the day has no room for one more wage, the global
   realm tells all zones. They then refuse a new shift at once with
   `insufficient`.
4. A shift that stops before its end gives its wage back to the budget.

The budget is exact. A new day gives a new budget. When a law makes the
budget larger on a day on which it was spent, the zones accept shifts again
from the next day.

The wage, the budget and the income tax can be different in each city. Each
city has its own budget count.

`list_jobs` shows the starter shifts of the city in `public_works`: `wage`,
`minutes` (the length of a shift), `kinds` (the site kinds that offer them)
and `open` (true when the budget of today has room for one more shift). The
field is absent when the wage is 0.

## 6. Shops

A `shop` or a `restaurant` with a permit trades with residents.

- `stock` and `unstock` move goods between a resident and the stock. The
  controller of the site can do both. A worker of the site can only use
  `stock`. They work at every site, not only at shops.
- `set_price` sets `price` (what a customer pays for one unit, tax
  included) and `buy_price` (what the shop pays to a resident for one
  unit). An absent price ends that offer.
- `buy`: the customer pays cash into the till and gets the goods from the
  stock.
- `sell`: the resident gives goods to the stock and gets cash from the
  till. The till must hold the money. A sale gives the seller `xp_per_sale`
  experience points in `trading`.

```
total     = unit_price * qty
sales_tax = total * sales_tax_bp / 10,000
```

The shop pays the sales tax out of the price: the till wires it to the city
treasury. No sales tax applies when a shop buys.

`max_price` and `min_price` protect a resident against a price that changed.
The action fails with `conflict` when the price is past the limit.

`till_collect` moves cash from the till to the resident that controls the
site. For a site of an organization it wires the cash to the treasury of the
organization. `till_fund` moves cash of the resident into the till. The till
pays wages, the goods that the shop buys, and property tax.

## 7. The depot

At a site of the kind `depot`, `buy` and `sell` trade with the system:

- `buy` costs the ceiling price. The money leaves the world (sink
  `import.purchase`) and the goods enter it (faucet `import.purchase`).
- `sell` pays the floor price. The goods leave the world (sink
  `export.sale`) and the money enters it (faucet `export.sale`).

The depot has no stock and no till. It has no limit of units. It pays no tax.
The inputs of every recipe at the ceiling cost more than its outputs give at
the floor, so no chain through the depot makes money. Keep that true when
you change `depot.toml`.

## 8. Property

Each zone does its daily work once each game day: the rent (8.2), the
property tax (8.3) and the tills of governments (8.4). Zone number `z`
starts its day `z % daily_spread_minutes` minutes after midnight, so that a
large world has no frame with the daily work of all zones. A zone that skips
the frame (time dilation) does the work in its next tick. "The start of the
day" below is the start of the day of the zone.

### 8.1 Sale

A site is for sale when:

- a city or the nation owns it and it is not `public`: the price is the
  attribute `value`; or
- its owner offered it with `sell_property`: the price is the offer.

A public site is never sold.

`buy_property` moves the price in cash to the seller. A seller in another
zone and a treasury get the money by wire. With `org`, the organization
becomes the owner; the resident pays and needs `site.manage` there.

When a site gets a new owner:

- the offers of the old owner (price, rent) end;
- the jobs at the site close;
- the permit, the price list, the stock, the till and the leases stay with
  the site;
- a buyer that was a tenant of the site is no longer a tenant.

### 8.2 Leases

A `home` or an `apartment` can have tenants. A home has 1 unit and an
apartment house has 8, unless the site has the attribute `capacity`.

A site takes tenants when its owner offered it with `rent_out`. A lettable
site of a government takes tenants at:

```
rent = max(1, value * city_rent_bp / 10,000)     for one day
```

`rent` makes the resident a tenant. It pays the rent of the first day in
cash into the till of the site.

At the start of each day the zone of the site asks every tenant for one
more day of rent. The tenant pays all that it owes, or nothing. A tenant in
another zone pays by wire.

- A tenant that does not pay gets the notice `economy.rent_missed`. The debt
  stays.
- A tenant that missed the rent on more days in a row than `rent_grace_days`
  is evicted. With the default of 2, the third missed day ends the lease.

`end_lease` ends a lease at once. The tenant ends its own lease. The
controller of the site ends the lease of a tenant that it names.

### 8.3 Property tax

At the start of each day every site with an owner pays property tax from
its till. Sites of a government and public sites pay none.

```
tax = value * property_tax_bp / 10,000     for one day
```

The till wires the tax to the city treasury. A till that is short pays what
it has. The rest is a debt in the attribute `tax_due`, and the owner gets
the notice `economy.tax_due`. The city takes a site that owed tax on more
days in a row than `property_tax_grace_days`.

### 8.4 The tills of a government

At the start of each day a city takes the money in the tills of its own
sites. The till wires the money to the treasury. The nation does the same
with its sites. A site of a government with a job keeps its till.

### 8.5 Permits

`open_business` gives a business site a permit. The fee is `permit_fee`. The
resident wires it to the city treasury. The site gets the tag `open`.

The business kinds are `shop`, `restaurant`, `workshop`, `factory`, `office`
and `warehouse`. With the argument `kind`, the site becomes that kind. When
an open business changes its kind, its price list and its jobs end.

Without a permit a business site does not trade, runs no recipe and posts no
job.

`close_business` ends the permit. The price list and the jobs end. The stock
and the till stay.

## 9. Foreclosure and estates

The global realm listens to three events and tells every zone.

| Event | What the zones do |
| --- | --- |
| `finance.loan_defaulted` | The city takes every site of the resident |
| `life.resident_died` | Every site of the resident goes to `heir` when it is a resident or an organization. Else it goes to the city |
| `kernel.despawned` | Every site that the resident still owns goes to the city |

The zones get the estate of a death before the estate of the removal, when
the two events are in the same frame. The order of the two events does not
matter. So the heir gets the sites, and the estate of the removal finds
none. The life module must emit `life.resident_died` in the frame of the
removal or before it. A death event that comes in a later frame finds the
sites with the city.

When the city takes a site, the business loses its permit, its price list
and its jobs. The stock stays with the site. The till goes to the treasury
at the next day change. The site is for sale again at its value.

An heir that is a resident or a firm gets the business as it is. Only the
jobs close. An heir that is a city or the nation runs no business: the
permit ends, as when the estate has no heir.

A resident that left the world also loses its leases and its job, so that
the unit and the slot are free again.

## 10. Events

All events are public. The drama scores are the parameters
`economy.drama.<what_happened>`. Each event kind has its own parameter. A
world does not start when the data file lacks one of them.

Events with fixed data (`docs/design/game-design.md`, section 9):

| Kind | Data | Drama |
| --- | --- | --- |
| `economy.shift_completed` | `resident`, `site`, `wage`, `tax`, `skill` | 2 |
| `economy.goods_sold` | `seller`, `buyer`, `site`, `city`, `item`, `qty`, `unit_price`, `tax` | 2 |
| `economy.goods_produced` | `resident`, `site`, `item`, `qty` | 2 |
| `economy.property_sold` | `site`, `city`, `from`, `to`, `price` | 30 |
| `economy.lease_started` | `site`, `tenant`, `rent` | 12 |
| `economy.lease_ended` | `site`, `tenant`, `by` (`tenant`, `owner`, `gone`) | 12 |
| `economy.tenant_evicted` | `site`, `tenant` | 45 |
| `economy.business_opened` | `site`, `owner`, `kind` | 25 |
| `economy.property_foreclosed` | `site`, `city`, `from`, `reason` (`loan_default`, `property_tax`) | 55 |
| `economy.property_inherited` | `site`, `city`, `from` (resident), `to` (holder) | 30 |

Notes:

- `wage` is the wage before tax. `skill` is the short name of the skill, for
  example `farming`.
- `economy.goods_sold` is emitted for a sale at a shop, a sale to a shop and
  both trades at the depot. `seller` is a holder: the owner of the shop, the
  resident that sells, or the owner of the depot. `buyer` is the resident
  that gets the goods. When a shop buys, `buyer` is the resident that owns
  the shop, or `null`. When the depot buys, `buyer` is `null`.
- `economy.goods_produced` is emitted for the outputs of a recipe and for
  gathered goods, one event for each good.
- `economy.lease_ended` is emitted when a tenant ends its lease by itself
  (`by: tenant`), when the owner ends it (`by: owner`) and when the tenant
  left the world (`by: gone`). The society module reads it, with
  `economy.tenant_evicted`, `economy.property_sold` and
  `economy.property_foreclosed`, to end the household of a host that lost
  its home.

Other events:

| Kind | Data | Drama |
| --- | --- | --- |
| `economy.job_posted` | `job`, `site`, `city`, `wage`, `slots` | 6 |
| `economy.worker_hired` | `resident`, `site`, `job`, `wage` | 12 |
| `economy.worker_left` | `resident`, `site`, `job`, `reason` (`quit`, `fired`, `closed`, `gone`) | 6; 20 when fired |
| `economy.shift_unpaid` | `resident`, `site`, `wage` | 35 |
| `economy.business_closed` | `site`, `owner`, `kind`, `reason` (`owner`, `converted`, `foreclosure`, `estate`) | 25 |

## 11. Notices

| Kind | To | Data |
| --- | --- | --- |
| `economy.worker_hired` | The owner of the site | `job`, `site` |
| `economy.job_ended` | A worker that was fired or whose job closed | `job`, `site`, `reason` |
| `economy.tenant_moved_in` | The owner of the site | `site`, `rent` |
| `economy.rent_missed` | The tenant | `site`, `owed`, `cash` |
| `economy.evicted` | The tenant | `site` |
| `economy.lease_ended` | A tenant whose lease the owner ended | `site` |
| `economy.tax_due` | The owner | `site`, `owed`, `days_left` |
| `economy.property_sold` | The seller | `site`, `price` |
| `economy.property_lost` | The old owner after a foreclosure | `site`, `reason` |

No notice has text that a resident wrote.

## 12. Observation sections

| Section | Present when | Content |
| --- | --- | --- |
| `job` | The resident has a job | `job`, `site`, `zone`, `kind`, `wage`, `shift_minutes` |
| `business` | The site where the resident is has something to offer | `site`, `kind`, `open`, `prices`, `jobs`, `gather`, `public_works`, `for_sale`, `for_rent`; `till` and `stock` for whoever controls the site; `stock` for its workers |
| `property` | The resident owns a site or rents a home | `owned` and `leases`. A site in the zone of the resident also shows `till`, `tenants`, `tax_due`, `price`, and for a lease `owed` and `days_missed` |

Each entry of `prices` has the good, its prices and `qty`, the units in the
stock. `stock` lists only the goods that `prices` does not show. `jobs` has
the jobs that take workers, the best wage first.

A brief observation has limits. A count says how many entries were left
out. An observation with full detail shows every entry.

| List | Limit | Count of the rest |
| --- | --- | --- |
| `prices`, `stock` | `observe_prices_max` | `more_prices`, `more_stock` |
| `jobs` | `observe_jobs_max` | `more_jobs` |
| `owned`, `leases` (oldest first) | `observe_sites_max` | `more_owned`, `more_leases` |

The depot shows its price list only with full detail. Its prices are also
parameters in the catalogue.

With the default limits, the `business` section of a shop with three prices
and one job has about 250 bytes. A shop with more goods and jobs than the
limits stays below 700 bytes for a customer and below 900 bytes for its
owner.

## 13. State and messages

A zone holds the truth about its sites: the job book, the price lists, the
leases, the nodes and the starter shifts that run. The global realm holds
the two city boards and the public works budget of the day.

Each resident has the component `economy.ties`: its job, the sites that it
owns and the homes that it rents. It is a copy that moves with the resident,
so that the resident sees them from every zone. When the truth changes while
the resident is in another zone, the zone of the site sends the change to
the resident.

| Message | From, to | Purpose |
| --- | --- | --- |
| `economy.board_update` | zone to global | Offers that changed. Sent after a verb changed an offer, and at a full check every `board_refresh_minutes` |
| `economy.public_claim`, `economy.public_release` | zone to global | Ask for, or give back, the wage of a starter shift |
| `economy.public_answer` | global to zone | Grant or refusal |
| `economy.public_budget_state` | global to all zones | The budget of a city has room, or has none |
| `economy.rent_due` | zone to resident | The rent that a tenant owes |
| `economy.rent_answer` | zone to zone | What the tenant paid |
| `economy.tie_change` | zone to resident | A job, a site or a lease of the resident changed |
| `economy.job_leave` | zone to zone | A worker in another zone quit |
| `economy.foreclose`, `economy.estate` | global to all zones | See section 9 |

The boards list a site of the life module from the first tick. A site that
another module adds or changes later is on the board after at most
`board_refresh_minutes`.

## 14. Limits that a designer must know

- A resident has one job.
- No rule limits what a resident can carry. The module does not read the
  weight of goods.
- Tools do not wear, and no good is needed to gather.
- A tenant cannot keep goods in the stock of a home that it rents.
- The `property` section does not list the sites of an organization.
- A job keeps its wage when `min_wage` changes later.
- The daily work of a zone runs in one tick. A zone with very many leases
  and sites has a long tick once a day.
- A tenant that the zone of the home cannot reach (its zone does not answer)
  misses the rent of that day.
- A recipe uses its inputs when it ends, not when it starts. The inputs are
  not locked during the work.
- A member of a city government with `site.manage` controls the sites of
  the city. It can offer them at another price, let them, post jobs there,
  and take goods out of their stock. The money of their tills goes to the
  treasury, not to the member.
- A tenant ends its lease at the home. A tenant that is away stops the
  lease only when it does not pay.
- The public works wage has no minimum of its own. `min_wage` applies only
  to `post_job`.
