# The life module

The life module is the physical world and the body of a resident. It makes
the map of every city. It lets a resident walk and travel. It gives a
resident needs, skills, an age, a death and an estate.

The crate is `crates/modules/life` (`unsup-life`). It depends only on the
kernel. The shared names (site kinds, tile codes, attributes, conditions,
verbs, events) are in `docs/design/game-design.md`. The crate has them as
constants in `unsup_life::names`.

Every number in this document is a default. The numbers are parameters in
`crates/modules/life/data/balance.toml`. The name of a parameter is in
backticks, for example `life.walk_ticks_road`.

All divisions in this document round down, unless the text says "rounded
up". One tick is one game minute. One hour is 60 ticks.

## 1. World generation

Each zone builds itself when the world is made. The result depends only on
the seed, the configuration of the world and the parameters `life.gen.*` and
`life.site.*`. The same seed gives the same map.

The generator uses its own random streams. The plan of a city uses the
stream `2^40 + city`. The plan of a zone uses the stream `2 * 2^40 + zone`.
So the map does not change the random numbers of the simulation.

### 1.1 District types

A city with one zone has one district of the type `mixed`. A mixed district
has a part of each other type.

A city with more zones gives each zone one of the five types `downtown`,
`commercial`, `residential`, `industrial`, `rural`:

1. Count the zones for each type. When the city has fewer than five zones,
   the types get one zone each in this order: downtown, residential, rural,
   industrial, commercial. When the city has five zones or more, each type
   gets one zone. The other zones follow the shares
   `life.gen.share.<type>` (10, 15, 35, 15, 25) by the largest remainder.
2. Sort the zones by their distance from the centre of the city. Zones at
   the same distance come in a seeded random order.
3. Give the zones to the types from the centre to the edge, in this order:
   downtown, commercial, residential, industrial, rural.

So downtown is in the centre and the farms are at the edge.

The district type of a zone is in the zone resource `ZoneFacts`. Each zone
also emits the public event `life.district_published` when the world is
made and once in each game day. The data is
`{ key, zone, city, name, district }`. `key` is the zone.

### 1.2 Roads, blocks and lots

- The tiles at the edge of a zone are road. So a resident can cross each
  border between two zones of a city at each tile.
- Streets cut the rest of the zone into square blocks. A block has a side
  of about `life.gen.block` tiles (14). A street is `life.gen.street` tiles
  wide (2). A zone of 64 tiles has 4 by 4 blocks. Each block has a road on
  its four sides.
- A block is one large lot. A large lot can be cut into two medium lots. A
  medium lot can be cut into two small lots. The cut goes across the longer
  side. A lot has a side of 5 tiles or more. Each lot touches a road.

| Size | Lot | Kinds |
| --- | --- | --- |
| Large | A block | `shelter`, `station`, `farm`, `factory`, `forest` |
| Medium | Half a block | `city_hall`, `hospital`, `school`, `courthouse`, `jail`, `exchange`, `depot`, `apartment`, `warehouse`, `mine`, `park` |
| Small | A quarter of a block | `police_station`, `bank`, `newsroom`, `shop`, `home`, `restaurant`, `office`, `workshop` |

### 1.3 Sites that a city must have

Each city has each kind of site. The generator first makes a list of the
sites that the city must have:

| Rule | Kinds | Number in a city |
| --- | --- | --- |
| Unique | `city_hall`, `courthouse`, `jail`, `exchange`, `depot`, `newsroom` | 1 |
| Service | `shelter`, `station`, `hospital`, `school`, `police_station`, `bank` | 1 for each `life.gen.zones_per.<kind>` zones (2, 16, 6, 4, 6, 6), rounded up, and 1 or more |
| Filler | All other kinds | 1 or more. The mix of each district adds more |

Then it gives each of these sites to a zone. The kinds come in a fixed
order, the most necessary first (shelter, station, city hall, hospital,
...). A site goes to the zone that:

1. has room for it,
2. does not have this kind yet,
3. has the district type that the kind prefers most (a hospital prefers
   residential, then downtown, then commercial),
4. has the most room left, then the lowest zone number.

When no zone has room, the site is left out. This occurs only when the
zones are much smaller than 64 tiles.

### 1.4 The plan of a zone

1. The blocks come in a seeded random order.
2. The sites that the zone must have get their lots first, the largest
   first.
3. Each lot that is left gets a site of a random kind. The weights are
   `life.gen.mix.<district>.<kind>`. Only a kind that is not larger than the
   lot can be chosen. When the kind is smaller than the lot, the lot is cut
   and the other half gets a random site too.
4. A lot on which no kind of the district fits stays green.

### 1.5 The tiles of a site

A **building** stands one tile inside its lot.

- The outer ring of the building is wall (tile 4). The tiles inside are
  floor (tile 3).
- The door is a floor tile in the middle of a wall that faces a road.
  Pavement (tile 2) goes from the door to the edge of the lot.
- A large building has a second door in the opposite wall when that wall
  faces a road too.
- The free tiles around the building are grass (tile 0).
  `life.gen.margin_trees_pct` percent of them (8) get trees (tile 6).
- The rectangle of the site is the rectangle of the walls.

**Open land** has no walls. The rectangle of the site is the whole lot.

| Kind | Tiles |
| --- | --- |
| `farm` | Field (7) on all tiles |
| `forest` | Trees (6) on all tiles |
| `mine` | Rock (8) in the middle, with `life.gen.mine_rim` tiles (2) of grass around it |
| `park` | Grass with some trees, and a pond of water (5) in the middle. The pond has the size of the park divided by `life.gen.pond_divisor` (4) |

A resident can reach each walkable tile of a zone on foot.

### 1.6 The data of a site

- `owner` is `Holder::Org(OrgId::city(c))`.
- The attribute `value` is the price in cents. It is
  `life.site.<kind>.value` plus or minus a random part of at most
  `life.gen.value_jitter_pct` percent (15).
- The tag `public` is on `shelter`, `station`, `city_hall`, `hospital`,
  `school`, `police_station`, `courthouse`, `jail`, `bank`, `exchange`,
  `depot` and `park`.
- The tag `spawn` is on `shelter` and `station`. The kernel puts a new
  resident in the middle of such a site.
- The attribute `beds` is on `home` (2), `apartment` (8) and `shelter` (24):
  `life.site.<kind>.beds`.
- The name is the kind and a number, for example `Shop 3`. The number counts
  the sites of the kind in the zone. A unique site has no number.

### 1.7 What other realms learn

- Each zone tells the global realm where its sites are. The global resource
  `SiteIndex` has the zone, the city and a walkable tile of each site.
- Each zone with a station tells all zones. The zone resource `Stations` has
  each station of the world.
- Each zone with a shelter tells all zones. The zone resource `Shelters` has
  each shelter of the world.

These messages arrive in the first frame.

## 2. Movement

### 2.1 `move_to`

Arguments: `x` and `y`, or `site`.

- The target must be in the city of the resident.
- A target tile must be walkable.
- With `site`, the walk ends on the walkable tile of the site that is
  nearest to the middle of its rectangle. For a building that tile is
  inside, so the path goes through a door.
- A site of another district is found through the global realm. The answer
  comes one frame later. Then the walk starts.

The result is `{ x, y, steps, ticks }`, and `site` when the request had it.

### 2.2 The path

The path search is A* over the walkable tiles of one zone.

- A step goes to one of the eight tiles around a tile.
- A step costs the ticks of the tile that the resident enters:

| Tile | Ticks | Parameter |
| --- | --- | --- |
| Road | 1 | `life.walk_ticks_road` |
| Pavement | 1 | `life.walk_ticks_pavement` |
| Floor | 1 | `life.walk_ticks_floor` |
| Grass | 2 | `life.walk_ticks_grass` |
| Field | 2 | `life.walk_ticks_field` |
| Trees | 4 | `life.walk_ticks_trees` |
| Wall, water, rock | Not walkable | |

- A diagonal step is possible only when the two tiles beside it are
  walkable. So a path does not cut the corner of a wall, and it goes
  straight through a door.
- The search looks at `life.path_search_max` tiles at most (8,192). A zone
  of 64 by 64 tiles has 4,096 tiles.
- The search is deterministic.

### 2.3 Steps

The resident moves one tile at a time. A step is complete after the ticks of
the tile that the resident enters. Viewers see each step.

A walk in one zone is a busy action. Its length is the sum of the ticks of
all steps. The acknowledgement has the length.

Only the residents that walk use time in a tick. The zone resource `Walkers`
has their ids.

### 2.4 Walks across zone borders

A walk to another zone is a waiting action (`pending`). The life module
completes it when the resident arrives.

Each zone plans its own part of the path:

1. In the zone of the target, the path goes to the target.
2. In another zone, the path goes to the edge of the zone, on a side that
   faces the zone of the target. The search prefers an edge tile that is
   near the target. One more step crosses the border. This step costs the
   ticks of a road.
3. The kernel moves the resident to the next zone. The walk continues there
   in the next tick.

Because each zone plans its part, the full path is near the best path but
not always the best path.

The walk fails with `timeout` after `life.walk_timeout` ticks (2,880).

### 2.5 Errors of `move_to`

| Code | When |
| --- | --- |
| `invalid_args` | No target, half a target, two targets, or a tile outside the world |
| `not_allowed` | The tile is not walkable (`details.tile` has its name), or the target is in another city |
| `not_found` | No path, the search limit was reached, or no site has the id |
| `conflict` | A tile on the way became not walkable, or something else moved the resident |

A walk in one zone that stops on the way reports the code `interrupted`
with the same message. A module can stop a busy action only in that way.

`cancel` stops a walk. The resident stays where it is.

### 2.6 `travel`

Argument: `city`.

- The resident must be in a `station`. The city must be another city with a
  station.
- The fare is `life.travel_fare` cents (2,500). It leaves the resident when
  the journey starts. It goes to the treasury of the city of departure
  (`wire_money` to `Holder::Org(OrgId::city(c))`).
- The journey takes `life.travel_ticks` ticks (120). Then the resident is in
  a station of the other city. When that city has more than one station, the
  resident id selects one.
- A journey that is stopped does not give the fare back.
- When the resident is not in a station at the end, the journey fails with
  `conflict`.

Event: `life.traveled { resident, from_city, to_city, site }`.

## 3. Needs

The four needs go from 0 to 1,000. A new resident has 1,000 of each.

### 3.1 The hourly change

Each zone runs one pass in each game hour (`Builder::every`). The pass
changes the needs of a resident as follows. `low` is `life.need_low` (250).

```
food   = max(0, food - 14)                    life.food_decay_per_hour
energy = max(0, energy - 40)   if awake       life.energy_decay_per_hour
mood   = mood - 2                             life.mood_decay_per_hour
mood   = mood - 10   if food < low            life.low_need_mood_per_hour
mood   = mood - 10   if awake and energy < low
health = health - 20 if food = 0              life.starve_health_per_hour
health = health - 10 if awake and energy = 0  life.exhaust_health_per_hour
health = health + 5  if food >= low and energy >= low
                                              life.health_regen_per_hour
```

The pass uses the new values of food and energy for the tests. Mood and
health stay in the range from 0 to 1,000.

- A resident with the condition `asleep` does not lose energy, and low
  energy does not count against it.
- A resident with the condition `hospitalized` does not change.
- When health is 0 after the change, the resident dies. The cause is
  `starvation` when food is 0, else `exhaustion`.
- When health is 0 before the change, another module took it. The cause is
  `injury`.

A full resident that does not eat has no food after 72 hours and dies 50
hours later. A full resident that does not sleep and does not doze (section
3.4) has no energy after 25 hours and dies 100 hours later.

### 3.2 One hour for each hour

Each resident has a clock: the tick up to which it paid for its needs.

- The pass charges a resident only when a full hour passed on its clock.
- A resident that arrives in a zone and owes an hour pays that hour at
  once.
- The clock of a new resident starts so that the first pass of its zone is
  its first hour.

So a resident pays one hour for each hour that it lives. A resident that
changes zone often does not pay more. A resident cannot stay away from the
pass of each zone. The clock uses the tick of the zone, so a zone that runs
slowly also slows the needs.

### 3.3 Notices

| Kind | Data | When |
| --- | --- | --- |
| `life.need_low` | `{ need, value }` | Food, energy or health falls below `low` |
| `life.need_empty` | `{ need, value }` | Food or energy reaches 0 |

### 3.4 Residents with no owner online

The design asks for default routines for each new resident. The kernel has
no function with which a module can set routines. Only the verb
`set_routines` can. So the module does two things.

**Reflexes.** After the hourly change, a resident with no action and no
plan:

- eats one unit of the food that it carries when its food is below
  `life.reflex_eat_below` (300);
- dozes when its energy is below `life.reflex_doze_below` (200). Dozing
  gives `life.reflex_doze_energy` energy (60).

Dozing keeps a resident alive, but its energy stays near the low mark and
its mood falls. `life.reflexes = 0` stops the reflexes.

**Default routines.** `unsup_life::default_routines(blueprint)` returns two
routines. The process that creates a resident can send them with
`set_routines`:

1. `eat when hungry`: food below `life.routine_eat_below` (400) and the
   resident carries a food item. Action: `eat`.
2. `sleep when tired`: energy below `life.routine_sleep_below` (250).
   Action: `sleep`.

## 4. Food, medicine, sleep, rest and hospital

The life module defines no goods. It reads tags and attributes of items.

### 4.1 `eat`

Argument: `item` (optional).

- The item must have the tag `food` and the attribute `nutrition`.
- Without `item`, the resident eats the carried food with the most
  nutrition that is not wasted. When all food is too large, it eats the
  smallest.
- A meal takes `life.eat_ticks` ticks (10). Then one unit goes to the sink
  `consumption.food`, food grows by `nutrition`, and mood grows by
  `life.meal_mood` (5).
- Refusals: `conflict` (food is full), `insufficient` (no such food is
  carried), `not_found` (no item has the name), `not_allowed` (the item is
  not food).

Event: `life.meal_eaten { resident, item, nutrition }`.

### 4.2 `use_item`

Argument: `item`.

- The item must have the tag `medicine` and the attribute `healing`.
- It takes `life.use_item_ticks` ticks (5). Then one unit goes to the sink
  `consumption.use` and health grows by `healing`.
- Refusals: `conflict` (health is full), `insufficient`, `not_found`,
  `not_allowed` (the item is not medicine).

Event: `life.item_used { resident, item, healing }`.

### 4.3 `sleep`

Argument: `ticks` (optional, from `life.sleep_min_ticks` to
`life.sleep_max_ticks`: 30 to 720).

| Quality | Place | Energy per hour | Mood per hour |
| --- | --- | --- | --- |
| `good` | A `home` or `apartment` where the resident is the owner or a tenant | 125 | 10 |
| `fair` | A `shelter` | 90 | 4 |
| `poor` | All other places | 60 | 0 |

The parameters are `life.sleep_energy_<quality>_per_hour` and
`life.sleep_mood_<quality>_per_hour`.

- Without `ticks`, the sleep lasts until energy is full:
  `(1000 - energy) * 60 / rate` ticks, rounded up, in the limits.
- The resident has the condition `asleep` during the sleep.
- At the end, energy grows by `rate * ticks / 60`, and mood in the same
  way.
- A sleep that is stopped gives the points of the time that passed.
- `sleep` is refused with `conflict` when energy is full.
- A resident that sleeps can `inspect` only up to
  `life.asleep_sight_radius` tiles (1).

Event: `life.slept { resident, quality, ticks }`.

### 4.4 `rest`

Argument: `ticks` (1 to `life.rest_max_ticks`: 240).

Rest gives `life.rest_energy_per_hour` energy (20) and
`life.rest_mood_per_hour` mood (20) for each hour. In a `park` it gives
`life.rest_mood_park_per_hour` mood (40).

### 4.5 `heal`

Argument: `points` (optional). Without it, the hospital heals all missing
health.

- The resident must be in a `hospital`.
- The fee is `points * life.heal_fee_per_point` cents (20 for each point).
  It goes from the resident to the till of the hospital
  (`Holder::Site(site)`) when the stay starts.
- The stay takes `points * 60 / life.heal_health_per_hour` ticks, rounded
  up (150 points for each hour).
- The resident has the blocking condition `hospitalized` during the stay.
- At the end, health grows by `points`. A stay that is stopped gives the
  health of the time that passed, and no money back.
- A patient that is not in the hospital at the end gets no health.
- Refusals: `not_here`, `conflict` (health is full), `invalid_args`
  (`points` is 0), `insufficient` (not enough cash).

Event: `life.patient_healed { resident, site, health }`.

## 5. Skills

The skills are `labor`, `farming`, `mining`, `crafting`, `cooking`,
`trading`, `management`, `medicine`, `law`, `investigation`, `stealth`,
`combat`, `rhetoric`. Each is the attribute `skill.<name>`. The attribute
counts experience points from 0.

```
level = floor(sqrt(experience / 100))
```

Level 1 needs 100 points, level 2 needs 400, level 3 needs 900. The
functions `unsup_life::skill_level` and `unsup_life::xp_for_level` do this
calculation. Other modules add experience with `attr_add`.

### 5.1 `train`

Arguments: `skill`, `ticks` (optional, from `life.train_min_ticks` to
`life.train_max_ticks`: 60 to 480; the default is the minimum).

- The resident must be in a `school`.
- The fee is `ticks * life.train_fee_per_hour / 60` cents, rounded up
  (1,000 for each hour). It goes to the till of the school when the lesson
  starts.
- At the end, the resident gets experience:

```
base       = ticks * life.train_xp_per_hour / 60                    (60 for each hour)
experience = base * (min * 1000 + (100 - min) * mood) / (100 * 1000)
             with min = life.learn_min_pct                          (50)
```

  So a resident with full mood learns all of `base`, and a resident with no
  mood learns half of it.
- A lesson that is stopped gives nothing, and no money back.
- Refusals: `invalid_args` (unknown skill, wrong length), `not_here`,
  `insufficient`.

Events: `life.skill_trained { resident, skill, xp, level }` and, when the
level grows, `life.level_gained { resident, skill, level }`.

## 6. Age and death

### 6.1 Age

```
age.years = life.spawn_age + floor((frame - birth frame) / frames in a year)
```

A year has `days_per_year * 1440` frames (28 days). A new resident is 18.
Each zone sets the ages in one pass in each game day. A resident that
arrives in a zone gets its age at once. A birthday gives the event
`life.birthday_reached { resident, age }`.

### 6.2 Death of old age

From the age `life.old_age_from` (70), a resident has one chance to die in
each calendar day:

```
chance in one year = 500 + 300 * (age - 70) basis points
                     life.old_age_death_bp, life.old_age_death_rise_bp
chance in one day  = chance in one year / days_per_year
```

The chance in one year is 5 percent at 70, 35 percent at 80 and 65 percent
at 90. The roll uses `realm.rng()`.

The daily pass of the zone gives the chance. A resident that arrives in a
zone and did not have its chance on that day gets it at once. So a resident
has exactly one chance in a day, also when it changes zone.

### 6.3 Death

`unsup_life::die(realm, idx, cause)` lets a resident die. The life module
calls it with the causes `starvation`, `exhaustion`, `injury` and `old_age`.
A module that depends on the life module can call it with its own cause.

1. The zone emits `life.resident_died` with the data
   `{ resident, cause, age, heir, estate }`. `heir` is the holder that gets
   the estate. `estate` is the cash of the resident in cents. Viewers see
   the event. Residents in sight get a notice of the same kind.
2. The heir is `Kin::successor()`: the heir of the will, else the spouse.
   Without both, the heir is the treasury of the city where the resident
   died.
3. The kernel removes the resident. The cash and the goods go to the heir.

A zone cannot know if an heir in another zone is alive. So an estate for a
resident goes through the global realm:

1. The holdings go to an escrow in the global realm.
2. When the heir is alive, the global realm sends the holdings to the heir.
   The heir gets the notice `life.inheritance { from, money, items }`.
3. When the heir is not alive, the holdings go to the treasury of the city.
4. The global realm emits `life.estate_settled
   { resident, heir, money, items, complete }`.

An estate with nothing in it is dropped after `life.estate_wait_ticks`
ticks (2). Money and goods are never made or lost.

### 6.4 `write_will` (free)

Argument: `heir` (optional). It sets `Kin.heir`. Without `heir`, the will
names nobody. A resident cannot name itself. Event:
`life.will_written { resident, heir }`.

## 7. Goods from hand to hand

### 7.1 `give`

Arguments: `to`, and `money`, or `item` and `qty`, or all three.

- The receiver must be at most `life.give_range` tiles away (1): on the same
  tile or on a tile next to it, in the same zone.
- The cash and the goods move at once. A gift is complete or it does not
  occur. The giver is busy for `life.give_ticks` ticks (1).
- Refusals: `invalid_args` (to itself, nothing to give, a negative amount, a
  quantity below 1), `not_found` (unknown item), `not_here`, `insufficient`.

The receiver gets the notice `life.gift_received`. Event:
`life.gift_given { from, to, money, item, qty }`.

### 7.2 `drop` and `pick_up`

Arguments: `item`, `qty` (optional, 1).

- `drop` moves goods from the resident to the stock of the site where it
  stands (`Holder::Site(site)`). Each resident can do this.
- `pick_up` moves goods from the stock to the resident. The resident must be
  the owner or a tenant of the site, or have the permission `site.manage` in
  the organization that owns the site.
- Each takes `life.carry_ticks` ticks (1).

## 8. `inspect` (free)

Arguments: `resident` or `site`, or nothing.

- A resident in sight: name, model, age, position, distance, activity,
  conditions, the levels of its skills above 0, the site where it stands,
  and how healthy it looks: `critical` below `life.need_low`, `hurt` below
  `life.inspect_hurt_below` (500), else `good`.
- A site in sight: kind, name, rectangle, owner, number of tenants, tags,
  attributes, door tiles, the tile where a walk to the site ends, how well
  the resident would sleep there, and `yours`. The owner and the tenants
  also see the money in the till.
- Nothing: the tile, the zone, the city and the district type of the place
  where the resident stands, and the site there.

Sight is `kernel.sight_radius` tiles. Refusals: `not_here` (not in sight),
`not_found` (no such site in this district), `invalid_args` (two targets).

## 9. Observation sections

| Section | Content | Absent when |
| --- | --- | --- |
| `home` | `{ site, kind, name, zone, pos, sleep }`: the place where the resident sleeps. For a new resident it is the shelter where it appeared, else the nearest shelter of its district, else the nearest shelter of its city. Then it is the last home or shelter where the resident slept. `name` is there only in the zone of the site | The resident has no such place |
| `skills` | `{ <skill>: level }` for each skill above level 0. Full detail gives `{ level, xp, next }` | All skills are at level 0 |

## 10. Events

Each event is public. The drama of an event is the parameter
`life.drama.<name>`.

| Event | Drama |
| --- | --- |
| `life.resident_died` | 80 (`died`) |
| `life.estate_settled` | 25 (`estate`) |
| `life.level_gained` | 20 (`level_gained`) |
| `life.patient_healed` | 15 (`healed`) |
| `life.will_written` | 15 (`will`) |
| `life.gift_given` | 12 (`gift`) |
| `life.traveled` | 12 (`traveled`) |
| `life.skill_trained` | 8 (`trained`) |
| `life.birthday_reached` | 5 (`birthday`) |
| `life.item_used` | 4 (`item_used`) |
| `life.meal_eaten` | 2 (`meal`) |
| `life.slept` | 1 (`slept`) |
| `life.district_published` | 0 |

## 11. Conditions and blocked verbs

| Condition | Blocking | Set by |
| --- | --- | --- |
| `asleep` | No | `sleep` |
| `hospitalized` | Yes | `heal` |

The verbs `eat`, `use_item`, `sleep`, `rest`, `write_will` and `inspect`
work in the blocking conditions `hospitalized`, `jailed` and `detained`.
The other verbs of the module do not.

## 12. Protection against moves by other modules

Another module can move a resident during an action, for example at an
arrest. The life module then does not continue from the new place:

- A walk stops when the resident is not on the tile where the walk put it.
- `travel` and `train` fail when the resident is not in the station or the
  school at the end.
- `heal` gives no health when the patient is not in the hospital at the
  end.

A module that moves a resident should stop its action first
(`realm.interrupt`).

## 13. Performance

- The walk system runs each tick and touches only the residents that walk.
- The needs pass runs once in each game hour for each zone. The age pass
  runs once in each game day for each zone. The kernel spreads the passes
  of the zones over the frames.
- A resident that changes zone costs two small checks on arrival.
- A path search uses buffers that the zone keeps. It allocates only the
  path.
- The state of a resident that does not walk and does not sleep is its bed
  and two numbers.

`cargo run --release -p unsup-life --example bench_walk -- 100000` measures
the frame time with that number of walkers in zones of about 2,000
residents. Each walker has a plan of nine walks, so each walker walks in
each frame. That is much more than in a normal world.

| Walkers | Zones | Steps in a frame | Mean frame time | Worst frame time |
| --- | --- | --- | --- | --- |
| 100,000 | 64 | 90,000 | 13 to 18 ms | 27 to 60 ms |
| 1,000,000 | 529 | 900,000 | 176 to 220 ms | 411 to 823 ms |

These numbers are from a computer with 18 cores that did other work at the
same time. The path search uses about 70 percent of the time. Each walk
starts with one search in each zone on its way.

## 14. Functions for other crates

The base modules do not import each other. They read the attributes, the
conditions, the site data and the events of this document. A crate that
depends on `unsup-life` (the server, the crime module) can also use:

| Item | Use |
| --- | --- |
| `die` | Let a resident die with a cause |
| `skill_level`, `xp_for_level`, `skill_attr` | The level formula |
| `sleep_quality`, `sleep_quality_in` | How well a resident sleeps at a place |
| `default_routines` | The routines of section 3.4 |
| `names` | Site kinds, tile codes, district types, attributes, conditions, tags, skills |
| `ZoneFacts`, `SiteIndex`, `Stations`, `Shelters`, `Walkers`, `Estates` | Resources |
| `Body` | The component of a resident: `bed()`, `is_walking()` |

## 15. Limits

- The kernel section `nearby` does not know the condition `asleep`. A
  resident that sleeps still sees its surroundings in the observation.
- A site name is unique in its zone, not in its city.
- A resident that appears in a station before the first frame of the world
  knows only the shelters of its own zone.
- The life module does not change the owner of a site when its owner dies.
  The economy module can react to `life.resident_died`.
- Each zone plans its own part of a long walk. See section 2.4.
