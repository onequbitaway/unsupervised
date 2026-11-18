# The finance module

This document gives the rules and the formulas of the finance module
(`crates/modules/finance`). The module has the bank, the loans, the exchange,
the shares, the central bank and the public statistics. The numbers are in
`crates/modules/finance/data/balance.toml`. This document names each
parameter without the prefix `finance.`.

Read `docs/design/architecture.md` and `docs/design/economy.md` first.

## 1. Rules for the whole module

- Money is in cents. Rates are in basis points (bp). 10,000 bp is 100 percent.
- All arithmetic uses integers. A division rounds down unless this document
  says otherwise.
- A yearly rate becomes a daily rate when you divide it by the days of the
  game year (`WorldConfig::days_per_year`, 28 by default).
- A game day is 1,440 frames. Day 0 is the first day. Midnight is the first
  frame of a day. The module counts days by the frame number, not by the tick
  of a zone.
- The module creates money only through the faucets `bank.loan_issue` and
  `central_bank.issue`. It destroys money only through the sinks
  `bank.loan_repay` and `central_bank.retire`. All other movements are
  transfers.
- The bank treasury is `Holder::Org(OrgId::CENTRAL_BANK)`. It gets the fees
  of the bank and the interest of loans. It pays the interest of deposits.
- A refused verb changes nothing. When a refusal comes from the global realm
  after cash left the zone, the cash comes back by wire together with the
  answer.

### Where the state is

| State | Realm | Resource or holder |
| --- | --- | --- |
| Account balance | global | `Holder::Account(resident id)` |
| Accounts, loans, credit records | global | `finance.bank` |
| Share register and share order books | global | `finance.shares` |
| Cash that a share bid locks | global | `Holder::Escrow(order id)` |
| Yearly counters of the central bank | global | `finance.central_bank` |
| Statistics | global | `finance.stats` |
| Heirs of residents that died | global | `finance.estates` |
| Orders and order books of the exchange | zone of the exchange | `finance.exchange` |
| Cash and goods of an exchange order | zone of the exchange | `Holder::Escrow(order id)` |

### Verbs

The module owns these verbs. `docs/design/game-design.md`, section 5, lists
them in the four rows of `finance`. The economy module must not register
them. (F) is a free verb. The last column says if the verb was in the first
game design (`yes`) or if this module added it (`new`).

| Verb | Kind | Place | First game design |
| --- | --- | --- | --- |
| `open_account` | zone, waits for the global realm | at a `bank` | yes |
| `deposit` | zone, waits for the global realm | at a `bank` | yes |
| `withdraw` | zone, waits for the global realm | at a `bank` | yes |
| `transfer` (F) | global | anywhere | yes |
| `loan_request` | global | anywhere | yes |
| `loan_repay` | global | anywhere | yes |
| `order_place` | zone, takes time | at an `exchange` | yes |
| `order_cancel` | zone, takes time | at an `exchange` | yes |
| `market_claim` | zone, takes time | at an `exchange` | yes |
| `market_book` (F) | zone | in the district of an `exchange` | yes |
| `issue_shares` | zone, waits for the global realm | anywhere | yes |
| `buy_shares` | global | anywhere | yes |
| `sell_shares` | global | anywhere | yes |
| `pay_dividend` | zone, waits for the global realm | anywhere | yes |
| `cancel_shares` | global | anywhere | **new** |
| `shares_book` (F) | global | anywhere | **new** |
| `central_bank_set_rate` | zone, waits for the global realm | anywhere | **new** |
| `central_bank_issue` | zone, waits for the global realm | anywhere | **new** |
| `central_bank_retire` | zone, waits for the global realm | anywhere | **new** |

`transfer`, `loan_repay`, `cancel_shares` and `shares_book` also work for a
resident with the condition `hospitalized`, `jailed` or `detained`. They
only use the bank account. So a patient and an inmate can pay what they
owe.

## 2. Bank accounts

A resident has at most one account. The balance is the money of
`Holder::Account(resident id)` in the ledger of the global realm.

| Verb | Arguments | Effect |
| --- | --- | --- |
| `open_account` | none | The resident pays `account_open_fee` in cash to the bank treasury and gets an account |
| `deposit` | `amount` | Cash goes into the account by wire |
| `withdraw` | `amount` | Money of the account goes to the resident as cash by wire. The cash arrives together with the answer |
| `transfer` | `to`, `amount` | Money goes from the account of the sender to the account of `to`. The receiver gets the notice `finance.transfer_received` |

Fees:

```text
withdraw fee = amount * withdraw_fee_bp / 10,000
transfer fee = amount * transfer_fee_bp / 10,000
```

The account pays the fee on top of the amount. The fee goes to the bank
treasury. The balance must be at least the amount plus the fee.

Refusals:

| Verb | Code | When |
| --- | --- | --- |
| `open_account`, `deposit`, `withdraw` | `not_here` | The resident is not at a `bank` site |
| `open_account` | `insufficient` | The cash is less than the fee |
| `open_account` | `conflict` | The resident has an account. The fee comes back |
| `deposit`, `withdraw`, `transfer` | `invalid_args` | The amount is zero or less |
| `deposit` | `insufficient` | The cash is less than the amount |
| `deposit`, `withdraw`, `transfer` | `not_found` | The resident has no account. For `transfer`: or the receiver has none |
| `withdraw`, `transfer` | `insufficient` | The free balance is less than the amount plus the fee. The collateral of a loan is not free |
| `transfer` | `invalid_args` | The receiver is the sender |

### Deposit interest

The bank pays interest at each midnight.

```text
deposit rate = max(0, base_rate_bp - deposit_spread_bp)     (per year)
unit         = 10,000 * days per year
sum          = lowest balance of the day * deposit rate + carry
interest     = sum / unit                                   (whole cents)
carry        = sum mod unit                                 (kept for later)
```

- The lowest balance of the day is the lowest balance between the last
  midnight and this one. Money that came in during the day earns nothing for
  that day. On the day that an account opens, the lowest balance is 0.
- A deposit that reaches the global realm in the frame of a midnight is in
  the account before the bank does its daily work.
- The carry keeps the part of the interest that is less than one cent.
- The interest comes from the bank treasury. It is a transfer.
- When the treasury has less than the interest of all accounts, each account
  gets `interest * treasury / total interest`. The bank does not pay the rest
  later.

## 3. Loans

A resident has at most one loan. `loan_request` has the arguments `amount`
and `term_days`. The bank puts the amount in the account. The money comes
from the faucet `bank.loan_issue`.

### Credit limit

```text
age      = whole days since the account opened
seasoned = min(lowest balance of yesterday, lowest balance of today),
           and not below 0
raw      = loan_base_limit
         + age * loan_limit_per_account_day
         + loans repaid in full * loan_limit_per_repaid_loan
         + seasoned * loan_deposit_factor_bp / 10,000
keep     = max(0, 10,000 - defaults * loan_default_penalty_bp)
limit    = min(loan_limit_max, raw * keep / 10,000)
```

Money that arrives today does not raise the limit before tomorrow. Money
that leaves today lowers the limit at once.

With the numbers of the data file, `loan_base_limit`,
`loan_limit_per_account_day` and `loan_limit_per_repaid_loan` are all 0. So
the limit is the seasoned deposit, and a resident without a deposit gets no
credit. The part "Collateral" below says why.

### Collateral

```text
held = principal * loan_collateral_bp / 10,000
free = max(0, balance - held)
```

While a loan is open, the bank holds a part of the account as collateral.

- `withdraw`, `transfer` and `buy_shares` take only free money. The refusal
  is `insufficient`; its details have `held`.
- The daily collection takes an instalment only from free money. A
  repayment of principal lowers `held`, so it can use the collateral that
  it frees: the balance must be at least the payment plus the collateral
  of the principal that stays open. Interest must come from free money.
- At a default the bank takes the whole balance, the collateral with it.
- `held` is in the result of `loan_request`, in the `loan` part of the
  `bank` section and in the notice `finance.instalment_missed`.

`loan_collateral_bp` is 10,000 in the data file: the collateral is the
whole principal. Then a default costs the bank no principal, and a loan
makes no money that stays in the world.

**Why the defaults are safe.** A resident that owns nothing loses nothing
in a default: the bank cannot take cash, goods or sites, and a credit
record does not hurt a resident that its owner made only to borrow. So
each cent of credit without collateral is a cent that an owner can take
with a throwaway resident. Before this rule there were three ways:

1. Wait: each day of account age gave 500 of credit.
2. Borrow the least amount and repay it at once, again and again: each
   repaid loan gave 20,000 of credit.
3. Deposit, wait one day, borrow the same amount, take the deposit and the
   loan out, give the cash away and default.

The first two are closed by the two parameters at 0, the third by the
collateral.

**What the defaults cost.** A loan against a deposit that the bank holds
gives the borrower no new money to spend. It is useful only to a server
that accepts some risk: lower `loan_collateral_bp`, or raise
`loan_base_limit`, `loan_limit_per_account_day` or
`loan_limit_per_repaid_loan`. Then a part of each loan is unsecured, and a
default writes that part off. Credit that is secured by sites is not
built: see section 13.

### Reserve limit

```text
deposits     = sum of the balances of all accounts
lendable     = deposits * (10,000 - reserve_ratio_bp) / 10,000
lending room = max(0, lendable - principal of all loans)
```

The bank refuses a loan that is larger than the lending room.

### Terms of a loan

```text
rate             = base_rate_bp + loan_spread_bp     (per year)
instalment       = amount / term_days, rounded up
first collection = midnight number loan_first_due_days after the grant
```

The rate is fixed for the life of the loan.

### Daily collection

At each midnight from the first collection, the bank does these steps for
each loan:

```text
unit          = 10,000 * days per year
sum           = principal * rate + carry
due interest  = due interest + sum / unit
carry         = sum mod unit
due principal = min(principal, due principal + instalment)
              = principal, on the last day of the term and after it
```

- When the free balance covers `due interest + due principal`, the bank
  takes both. The interest goes to the bank treasury. The principal is
  destroyed through the sink `bank.loan_repay`. "Covers" is the rule of
  "Collateral" above.
- When it does not, the bank takes nothing. The count of missed
  instalments grows by one. The resident gets the notice
  `finance.instalment_missed`. The amounts stay due, so the next collection
  asks for more.
- A payment that leaves nothing due sets the count of missed instalments to
  zero.
- Interest that is due does not earn interest.
- The loan ends when the principal and the due interest are zero. The record
  of the resident gets one more repaid loan. The module emits
  `finance.loan_repaid`.

### Early repayment

`loan_repay` has the argument `amount`. It pays `min(amount, principal + due
interest)` from the account: the due interest first, then principal. The
payment must leave the collateral of the principal that stays open. An early
payment of principal lowers the interest of the days that follow and can end
the loan before its term.

### Default

A loan defaults when the count of missed instalments reaches
`loan_missed_max`. Then:

1. The bank seizes the balance of the account, with the collateral: first
   up to the principal (destroyed through `bank.loan_repay`), then up to the
   due interest (to the bank treasury).
2. The bank gives up the rest of the principal. That money stays in the
   world. The statistics show the sum as `loans.written_off`.
3. The record of the resident gets one default. The resident cannot borrow
   for `bankruptcy_days` days.
4. The module emits `finance.loan_defaulted { resident, outstanding }`.
   `outstanding` is the principal and the due interest that nobody paid. The
   economy module forecloses the sites of the resident on this event.

A default seizes only the bank account. It does not take cash on the person,
goods, shares, or cash that an open order locks.

Refusals:

| Verb | Code | When |
| --- | --- | --- |
| `loan_request`, `loan_repay` | `invalid_args` | The amount is zero or less |
| `loan_request` | `invalid_args` | The amount is below `loan_min`, the term is outside `loan_term_min_days` to `loan_term_max_days`, or the account cannot hold the amount |
| `loan_request`, `loan_repay` | `not_found` | The resident has no account. For `loan_repay`: or no loan |
| `loan_request` | `conflict` | The resident has a loan |
| `loan_request` | `not_allowed` | The resident is bankrupt, or the amount is above the credit limit |
| `loan_request` | `insufficient` | The amount is above the lending room |
| `loan_repay` | `insufficient` | The balance is less than the payment and the collateral that stays |

## 4. The exchange

Each zone with an `exchange` site has one order book for each item. The
orders and the escrows are state of that zone. A trader must stand in the
`exchange` site for `order_place`, `order_cancel` and `market_claim`. Each of
these verbs takes `exchange_ticks` ticks. The verb checks its conditions at
the start and again at the end. It moves value at the end.

### Orders

`order_place` has the arguments `item`, `side` (`buy` or `sell`), `qty` and
`price`. The price is a limit: the most that a buy order pays for one unit,
the least that a sell order takes.

- A buy order locks `qty * price` cents of cash in `Holder::Escrow(order id)`.
- A sell order locks `qty` units of the item in `Holder::Escrow(order id)`.
- The escrow also holds what the order has earned: the goods of a buy order,
  the cash of a sell order.
- The exchange does not trade an item with the tag `contraband`.
- A trader cannot place an order that would trade with its own open order
  for the same item (`conflict`).
- A trader has at most `orders_max_per_trader` orders at one exchange. An
  order counts until the trader has collected all that it holds.

### Matching

A system runs in each tick. For each book that got a new order it does this
while the best bid is at or above the best ask:

```text
best bid  = highest price; among equal prices, the oldest order
best ask  = lowest price; among equal prices, the oldest order
quantity  = min(open quantity of the bid, open quantity of the ask)
price     = limit price of the older of the two orders
value     = quantity * price
fee       = value * exchange_fee_bp / 10,000
```

- The escrow of the buy order pays `value - fee` to the escrow of the sell
  order and `fee` to the city. So the seller pays the fee.
- The fees of one run go to `Holder::Org(OrgId::city(c))` in one wire.
- `exchange_fee_bp` can have another value for each city.
- The escrow of the sell order gives the goods to the escrow of the buy
  order.
- The module emits `finance.order_filled { city, item, qty, price, buyer,
  seller }` for each trade. Each of the two traders gets one notice
  `finance.order_filled` for each order and run.
- An order trades when its trader is away.

A buyer that pays less than its limit keeps the difference. The difference
waits in the escrow with the goods.

### Collection

- `market_claim` moves to the trader all the cash and goods that its orders
  hold, less what open orders still lock. A closed order that is empty is
  then forgotten.
- `order_cancel` takes one order out of the book and moves all that its
  escrow holds to the trader: the locked part and the fills.
- `market_book` has the argument `item`. It shows the best `book_depth` price
  levels of each side, with the quantity and the number of orders at each
  level, and the price of the last trade. It is free. It works in each
  position of the zone of the exchange.

Refusals:

| Verb | Code | When |
| --- | --- | --- |
| `order_place`, `order_cancel`, `market_claim` | `not_here` | The resident is not at an `exchange` site |
| `market_book` | `not_here` | The zone has no `exchange` site |
| `order_place`, `market_book` | `invalid_args` | No item has this name |
| `order_place` | `invalid_args` | The quantity is outside 1 to `order_qty_max`, or the price is outside 1 to `order_price_max` |
| `order_place` | `not_allowed` | The item is contraband |
| `order_place` | `conflict` | The trader has too many orders here, or the order would trade with its own order |
| `order_place` | `insufficient` | The trader does not have the cash or the goods that the order locks |
| `order_cancel` | `not_found` | The trader has no order with this id at this exchange |
| `market_claim` | `not_found` | The orders of the trader hold nothing to collect |

## 5. Shares

A share is an entry in the share register of the global realm. It is not a
good. Each organization has one order book. Money for shares moves between
bank accounts, the escrows of buy orders and the treasury of the
organization.

| Verb | Arguments | Effect |
| --- | --- | --- |
| `issue_shares` | `org`, `shares`, `price` | The organization offers new shares at the limit price. The money of each sale goes to `Holder::Org(org)` |
| `buy_shares` | `org`, `shares`, `price` | A buy order. It locks `shares * price` cents of the account in `Holder::Escrow(order id)` |
| `sell_shares` | `org`, `shares`, `price` | A sell order. It locks the shares |
| `cancel_shares` | `order` | Cancels an open order of the resident and gives back what it locks |
| `shares_book` | `org` | Shows the best `book_depth` levels of each side, the last price, the shares that residents own and the new shares on offer |
| `pay_dividend` | `org`, `amount` | Pays treasury money to the holders |

- `issue_shares` and `pay_dividend` need the permission `treasury.spend` in
  the organization. The zone checks it with `realm.can`.
- `buy_shares` and `sell_shares` need a bank account.
- An organization has at most `shares_max_per_org` shares: the shares that
  residents own plus the new shares on offer.
- A resident has at most `share_orders_max` open share orders. An offer of
  new shares counts for the treasurer that placed it.
- A resident cannot place an order that would trade with its own order. An
  offer of new shares is an order of the organization, so its treasurer can
  buy from it.
- Share trades have no fee.

### Matching

A new order trades at once against the book.

```text
partner  = best order of the other side: the best price, then the oldest order
price    = limit price of the partner, which waited in the book
quantity = min(open quantity of the two orders)
```

The buyer gets the difference between its limit and the price back in its
account at once. The part of the new order that finds no partner waits in
the book. The module emits `finance.shares_traded { org, shares, price,
buyer, seller }` for each trade. `seller` is null for new shares. The trader
whose order waited gets the notice `finance.share_order_filled`.

### Dividend

```text
total     = shares that residents own (shares in a sell order count)
part of i = amount * shares of i / total                    (whole cents)
left      = amount - sum of the parts
```

The `left` cents go, one each, to the holders with the largest remainders of
the division. When remainders are equal, the lowest resident id comes first.
So the parts add up to `amount` exactly. Each part goes to the bank account
of the holder. A holder without an account (an heir) gets an account without
a fee. Each holder gets the notice `finance.dividend_received`.

Refusals:

| Verb | Code | When |
| --- | --- | --- |
| `issue_shares`, `pay_dividend` | `not_allowed` | The resident does not have `treasury.spend` in the organization |
| `issue_shares`, `buy_shares`, `sell_shares` | `invalid_args` | The number of shares is outside 1 to `shares_max_per_org`, or the price is outside 1 to `share_price_max` |
| `issue_shares` | `not_allowed` | The organization would have more than `shares_max_per_org` shares |
| `issue_shares`, `buy_shares`, `sell_shares` | `conflict` | The resident has `share_orders_max` open orders |
| `buy_shares`, `sell_shares` | `not_found` | The resident has no account, or the organization has no shares |
| `buy_shares`, `sell_shares` | `conflict` | The order would trade with an own order |
| `buy_shares` | `insufficient` | The balance is less than `shares * price` |
| `sell_shares` | `insufficient` | The resident does not own that many free shares |
| `cancel_shares` | `not_found` | The resident has no open order with this id |
| `shares_book` | `not_found` | The organization has no shares |
| `pay_dividend` | `invalid_args` | The amount is zero or less |
| `pay_dividend` | `not_found` | No resident owns a share of the organization |
| `pay_dividend` | `insufficient` | The treasury has less than the amount |

## 6. The central bank

The three verbs need the permission `bank.central` in
`OrgId::CENTRAL_BANK` or in `OrgId::NATION`. The permission in another
organization gives nothing. The zone checks the permission. The global realm
does the work.

| Verb | Arguments | Effect |
| --- | --- | --- |
| `central_bank_set_rate` | `rate_bp` | Sets the parameter `base_rate_bp` for the nation |
| `central_bank_issue` | `amount` | Creates money in `Holder::Org(OrgId::NATION)` through the faucet `central_bank.issue` |
| `central_bank_retire` | `amount` | Destroys money of `Holder::Org(OrgId::NATION)` through the sink `central_bank.retire` |

Limits. A law can change each of them, because each is a policy parameter.

```text
base_rate_floor_bp <= rate_bp <= base_rate_ceiling_bp
money issued in this game year  + amount <= issue_limit_per_year
money retired in this game year + amount <= retire_limit_per_year
```

- The game year is the year of the calendar. The counters start at zero in
  each new year.
- The base rate sets the rate of new loans and the rate of deposits. A loan
  that exists keeps its rate.
- Events: `finance.rate_changed { resident, old_bp, rate_bp }`,
  `finance.money_issued { resident, amount }`,
  `finance.money_retired { resident, amount }`.

Refusals: `not_allowed` without the permission, outside the rate band or
above a yearly limit; `invalid_args` for an amount of zero or less, or an
amount that the treasury cannot hold; `insufficient` when the treasury has
less than the amount to retire.

## 7. Statistics

At each midnight the global realm emits the public event
`finance.stats_published`. There is one series, so the data has no `key`.
The resource `finance.stats` keeps the last publication.

| Field | Meaning |
| --- | --- |
| `day` | Number of game days that have ended |
| `money_supply` | All money: created minus destroyed |
| `money` | Where the money is: `cash`, `deposits`, `treasuries`, `tills`, `escrow`, `other`, `in_transit` |
| `price_index_bp` | The price index. 10,000 is the level of the reference prices |
| `prices` | For each item that was sold: `last`, `reference`, `basket` |
| `gini_bp` | Wealth inequality: 0 when all residents have the same, near 10,000 when one resident has all |
| `residents` | Number of residents that the Gini coefficient counts |
| `faucets`, `sinks` | Money in cents. For each faucet and sink: `total` since the world began, `day` since the last publication |
| `goods_faucets`, `goods_sinks` | Goods in units, all items together. The same two numbers for each faucet and sink |
| `loans` | `count`, `outstanding` principal, `written_off` principal |
| `base_rate_bp` | The base rate |

### Sources

- Each zone sends a report of its ledger to the global realm: the cash of
  residents, the tills of sites, the escrows, the other holders, and the
  totals of each faucet and sink for money and for goods. It reports each
  `stats_report_period` frames and at each midnight. All zones use the same
  frames.
- The report of a midnight also lists the cash of each resident of the zone.
  The global realm uses the list once and then drops it.
- The global realm reads its own ledger at the time of the publication.
- Prices come from the events `economy.goods_sold` (`item`, `qty`,
  `unit_price`) and `finance.order_filled` (`item`, `qty`, `price`).

### Formulas

```text
money_supply = sum of all faucet totals - sum of all sink totals
parts        = cash + deposits + treasuries + tills + escrow + other
in_transit   = money_supply - parts

For each item:
price of the day = cents paid in the day / units sold in the day
reference price  = price of the first day on which the item was sold
last price       = price of the last day on which the item was sold
ratio            = last price * 10,000 / reference price

price_index_bp = sum of the ratios of the basket items / number of basket items

wealth of a resident = cash on the person + balance of the bank account
gini = (2 * sum(i * x_i) - (n + 1) * sum(x_i)) / (n * sum(x_i))
```

In the Gini formula, `x_1 ... x_n` are the wealth values from low to high.
The result is in basis points. The basket has each item with the tag `food`
that has a price. The index is 10,000 when no basket item has a price.

### Approximations

1. **Zone data is as old as the last report.** At midnight each zone that
   runs reports in the same frame, so the data is exact then. A zone that
   skips that frame (time dilation) is read from its older report. Then the
   parts of `money` can count a resident that moved in two zones or in none.
   The error shows in `in_transit`, which can then be negative.
2. **`money_supply` uses the same reports.** It is exact when each zone
   reported in the frame of the publication. It is not exact for a zone that
   skipped the frame and used a faucet or a sink after its last report.
3. **A zone makes its report after its systems.** Value that leaves the
   zone later in the same tick is in the report, and it can be in the global
   ledger too. An action that a routine starts does this, for example a
   `deposit`. Then `in_transit` is negative by that amount, and the wealth of
   that resident counts the amount twice. The next publication is exact
   again.
4. **`in_transit` is a remainder.** It is the money in wires between realms
   plus the errors of items 1 to 3. The module does not measure it directly.
5. **`deposits` is each `Holder::Account` of the global ledger except account
   0.** Account 0 (unclaimed value) is in `other`. If another module uses
   `Holder::Account` for something that is not a deposit, the statistics
   count it as a deposit.
6. **`treasuries` is each `Holder::Org` of the global ledger.** An
   organization holder in a zone ledger is in `other`.
7. **A price is a mean of one day.** It is weighted by units and rounded
   down to one cent. An item that was not sold in a day keeps its last price.
8. **The reference price is the price of the first day with a sale.** So an
   item enters the index at 10,000. Items that enter later pull the index
   towards 10,000.
9. **The index has no weights.** Each basket item counts the same. Each
   ratio and the mean are rounded down.
10. **The basket has only food.** `docs/design/economy.md` asks for food and
    housing. The module gets no price of housing from its two events.
11. **The index is national.** The sales of all cities are in one mean.
12. **A sale in the frame of a midnight counts for the day that ended.** The
    zones run before the global realm in each frame.
13. **Wealth is cash and the bank balance only.** It does not count goods,
    sites, shares or value in escrows. It does not subtract loans.
14. **The Gini coefficient counts the residents of the zones that reported
    in the frame of the publication.** It is rounded down. With `n`
    residents its maximum is `(n - 1) / n`.
15. **The `day` flow of the first publication is the total.**
16. **A faucet or a sink that was never used is absent.** The kernel has one
    list of names for money and for goods. Most names are used for only one
    of the two, so a total of zero is not shown.
17. **Goods flows count units of all items together.** One unit of ore and
    one unit of bread count the same. The flows show activity, not value.

Cost: the report of a midnight has one pair of numbers for each resident.

## 8. Observation

| Section | Observer | Content |
| --- | --- | --- |
| `bank` | global | `balance`; `loan` (`principal`, `rate_bp`, `owed`, `next_instalment`, `next_in_ticks`, `days_left`, `missed`, `held`); `bankrupt_until` (`year`, `day`); `shares` (`holdings` with `org`, `shares`, `locked`; `open_orders`; `more_holdings`) |
| `market` | zone | `orders` (`order`, `item`, `side`, `price`, `qty`, `filled`, `open`); `claimable` (`cash`, `items`); `more_orders` |

- The `bank` section is absent when the resident has no account, no shares
  and no open share order. Each part is absent when it has nothing to say.
- `next_instalment` is what the next collection takes: the due principal and
  the due interest after that midnight.
- The `market` section is present only when the resident stands in an
  `exchange` site and has orders there.
- Brief detail lists at most `observe_list_max` holdings and orders. The
  fields `more_holdings` and `more_orders` count the rest.
- Full detail lists all holdings and orders. It adds `credit` (`limit`,
  `seasoned_balance`, `loans_repaid`, `defaults`) to `bank` and the open
  share orders (`orders`) to `shares`.

## 9. Events and notices

Public events. The drama scores are parameters (`drama_*`).

| Kind | Data |
| --- | --- |
| `finance.account_opened` | `resident` |
| `finance.loan_granted` | `resident`, `amount`, `rate_bp`, `term_days` |
| `finance.loan_repaid` | `resident` |
| `finance.loan_defaulted` | `resident`, `outstanding` |
| `finance.order_filled` | `city`, `item`, `qty`, `price`, `buyer`, `seller` |
| `finance.shares_issued` | `org`, `shares`, `price` |
| `finance.shares_traded` | `org`, `shares`, `price`, `buyer`, `seller` (null for new shares) |
| `finance.dividend_paid` | `org`, `amount`, `shares`, `holders` |
| `finance.rate_changed` | `resident`, `old_bp`, `rate_bp` |
| `finance.money_issued` | `resident`, `amount` |
| `finance.money_retired` | `resident`, `amount` |
| `finance.estate_settled` | `resident`, `heir` (holder), `balance`, `loan_unpaid`, `shares` |
| `finance.stats_published` | See section 7 |

Notices to one resident:

| Kind | Data |
| --- | --- |
| `finance.transfer_received` | `from`, `amount` |
| `finance.instalment_missed` | `due`, `balance`, `held`, `missed`, `default_after` |
| `finance.loan_defaulted` | `outstanding`, `seized`, `bankrupt_until` |
| `finance.order_filled` | `order`, `item`, `side`, `qty`, `value`, `unfilled` |
| `finance.share_order_filled` | `order`, `org`, `side`, `shares`, `price`, `unfilled` |
| `finance.dividend_received` | `org`, `amount` |
| `finance.request_still_handled` | `verb`, `action_id`, `reason` |

The module reads these events of other modules: `economy.goods_sold`,
`life.resident_died`, `kernel.despawned`, `kernel.action_interrupted`.

## 10. A request that is interrupted

The verbs that wait for the global realm send their request when they start.
The answer comes one frame later. When the action stops before the answer
(the resident cancels it, another module interrupts it, it times out), the
request has left. The global realm does the work anyway. The kernel drops
the late answer, so the action report says `interrupted`.

The resident then gets the notice `finance.request_still_handled { verb,
action_id, reason }`. The resident must not send the request again. It finds
the result in its observation. No money is lost: the wire arrives, and a
refusal sends the cash back.

The verbs of the exchange move value only when they end. An interrupted
exchange verb did nothing.

## 11. The estate of a resident that leaves the world

The kernel gives the cash and the goods on the person to the heir. The
module does the same for what it holds, at the end of the tick in which the
resident left.

The heir is the `heir` of the event `life.resident_died`:

- another resident or an organization, as the event says;
- else the treasury of the city where the resident was;
- `UNCLAIMED` when the resident left without that event (an administrator
  removed it).

Steps:

1. The open share orders of the resident end. The cash of a bid goes back to
   the account.
2. The loan ends. The bank takes from the account up to the principal
   (destroyed through `bank.loan_repay`) and then up to the due interest. It
   gives up the rest. This is not a default, and the module emits no
   `finance.loan_defaulted`.
3. The shares go to an heir that is a resident. Shares of another heir are
   cancelled.
4. The balance goes to the heir: to its account, or as cash by wire when it
   is a resident without an account, or to the holder. The account closes.
5. Each zone that ever had an exchange order closes the orders of the
   resident there. It sends all that their escrows hold to the heir.

The module emits `finance.estate_settled` when the resident had an account
or shares.

## 12. Parameters

All names have the prefix `finance.`. "Policy" means that a law can change
the parameter.

| Parameter | Default | Policy | Meaning |
| --- | --- | --- | --- |
| `account_open_fee` | 100 | no | Cents that `open_account` costs |
| `withdraw_fee_bp` | 0 | no | Fee of `withdraw` |
| `transfer_fee_bp` | 10 | no | Fee of `transfer` |
| `deposit_spread_bp` | 300 | no | Base rate minus this is the deposit rate |
| `base_rate_bp` | 500 | no | Base rate. The central banker sets it |
| `loan_spread_bp` | 300 | no | Base rate plus this is the rate of a new loan |
| `reserve_ratio_bp` | 1,000 | yes | Part of the deposits that the bank cannot lend |
| `loan_min` | 1,000 | no | Least amount of a loan |
| `loan_limit_max` | 5,000,000 | yes | Greatest credit limit |
| `loan_base_limit` | 0 | no | Credit limit without history and deposits |
| `loan_limit_per_account_day` | 0 | no | Credit for each day of account age |
| `loan_limit_per_repaid_loan` | 0 | no | Credit for each repaid loan |
| `loan_deposit_factor_bp` | 10,000 | no | Part of the seasoned balance that counts |
| `loan_collateral_bp` | 10,000 | no | Part of the open principal that the bank holds in the account |
| `loan_default_penalty_bp` | 5,000 | no | Part of the limit that each default removes |
| `loan_term_min_days` | 1 | no | Shortest term |
| `loan_term_max_days` | 28 | no | Longest term |
| `loan_first_due_days` | 2 | no | Midnights from the grant to the first collection |
| `loan_missed_max` | 3 | no | Missed instalments in a row that make a default |
| `bankruptcy_days` | 28 | yes | Days without credit after a default |
| `exchange_fee_bp` | 50 | yes | Fee of a trade at the exchange |
| `exchange_ticks` | 1 | no | Ticks of `order_place`, `order_cancel`, `market_claim` |
| `orders_max_per_trader` | 8 | no | Orders of one trader at one exchange |
| `order_qty_max` | 100,000 | no | Greatest quantity of an order |
| `order_price_max` | 100,000,000 | no | Greatest unit price of an order |
| `book_depth` | 5 | no | Price levels that `market_book` and `shares_book` show |
| `shares_max_per_org` | 1,000,000 | no | Greatest number of shares of an organization |
| `share_price_max` | 100,000,000 | no | Greatest price of a share |
| `share_orders_max` | 8 | no | Open share orders of one resident |
| `base_rate_floor_bp` | 0 | yes | Lowest base rate |
| `base_rate_ceiling_bp` | 3,000 | yes | Highest base rate |
| `issue_limit_per_year` | 10,000,000 | yes | Cents that the central bank can issue in a game year |
| `retire_limit_per_year` | 10,000,000 | yes | Cents that the central bank can retire in a game year |
| `stats_report_period` | 60 | no | Frames between two zone reports |
| `observe_list_max` | 3 | no | Entries of a list in a brief observation |
| `drama_routine` | 3 | no | Drama of routine events |
| `drama_loan_granted` | 12 | no | Drama of `finance.loan_granted` |
| `drama_loan_defaulted` | 55 | no | Drama of `finance.loan_defaulted` |
| `drama_shares` | 20 | no | Drama of `finance.shares_issued` and `finance.dividend_paid` |
| `drama_central_bank` | 35 | no | Drama of the central bank events |

A zone reads a parameter with the value of its city. The global realm reads
the national value. So a city can have its own `exchange_fee_bp`. The bank,
the shares and the central bank use national values.

## 13. Limits of the design

- One zone has one book for each item. A city with `exchange` sites in two
  zones has two books for each item. A zone with two `exchange` sites has
  one.
- The bank is one national bank. It cannot fail: it lends created money, and
  it pays deposit interest only from its treasury.
- Money that a default writes off stays in the world. The faucet
  `bank.loan_issue` is then larger than the sink `bank.loan_repay`. With
  the collateral of the data file nothing is written off.
- **A loan of the data file gives no new money to spend** (section 3,
  "Collateral"). Credit that is useful and safe needs a pledge that the
  borrower cannot move: a site. That needs three things that are not
  built: the bank must know the value of the sites of a resident (the
  economy module owns it), the economy module must refuse the sale of a
  site while its owner has a loan, and the limit must follow the sites.
  The foreclosure at a default exists already.
- The account of a resident uses the resident id as the account number. No
  other module must use `Holder::Account` with a number that can be a
  resident id.
- The module does not know when an organization ends. Its shares and its
  order book stay.
