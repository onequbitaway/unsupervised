# Economy model

The economy is a closed loop. Money and goods enter and leave only through
named faucets and sinks. The ledger counts every one of them. The audit runs
every tick.

## 1. Stores of value

| Holder | Where | Holds |
| --- | --- | --- |
| Resident | zone | Cash and goods on the person. They can be stolen |
| Site | zone | Till and stock of a building |
| Organization | global | Treasury |
| Account | global | Bank deposit |
| Escrow | either | Value locked for a contract, an order, a mission or an estate |

Cash is physical. To pay a person or a shop, the payer must be there, or use
a bank transfer. Goods are physical too. To sell goods, somebody must carry
them to the buyer or to the exchange.

## 2. Money faucets and sinks

| Name | Kind | Module | Trigger | Control |
| --- | --- | --- | --- | --- |
| `gov.public_works` | faucet | economy | A resident completes a starter shift at a city hall or a depot | Parameters: wage and daily budget for each city. A law can change them |
| `export.sale` | faucet | economy, crime | The depot buys goods at the floor price. `fence` sells stolen goods for a part of that price | Parameters: floor price for each item, `crime.fence_bp` |
| `bank.loan_issue` | faucet | finance | The bank grants a loan | Parameters: credit limit, reserve limit, collateral |
| `central_bank.issue` | faucet | finance | The central banker issues money to the national treasury | Parameter: yearly limit. A law can change it |
| `sponsor.reward` | faucet | missions | A human sponsor publishes a mission. The reward is new money in the escrow of the mission | Parameter: daily limit for all sponsors |
| `import.purchase` | sink | economy | A resident buys goods from the depot at the ceiling price | Parameters: ceiling price for each item |
| `bank.loan_repay` | sink | finance | A borrower repays principal, or the bank takes its collateral at a default | Automatic |
| `central_bank.retire` | sink | finance | The central banker removes money of the national treasury | Role permission `bank.central`, yearly limit |
| `sponsor.expired` | sink | missions | A sponsor mission closes with money in its escrow | Automatic |
| `season.reset` | sink | missions | The soft reset at the end of a season cuts cash above a floor | Parameters: floor and share |
| `repair.materials` | sink | crime | `repair` gives a damaged site its value back | Parameter: cost for each cent of value |

Taxes, fines, fees, wages, rent, interest, dividends, salaries, grants of a
treasury and estates are transfers. They do not create or destroy money.

- The kernel has one more faucet, `kernel.grant`. No verb uses it. It is
  for tests and for tools of an administrator.
- An estate is never destroyed. It goes to the heir, else to the treasury
  of the city. Value for a holder that no longer exists waits on the
  unclaimed account of the global realm (`Holder::Account(0)`).
- Principal that the bank gives up in a default stays in the world. With
  the default numbers a loan is fully covered by collateral, so the bank
  gives up no principal.

## 3. Goods faucets and sinks

| Name | Kind | Module | Trigger |
| --- | --- | --- | --- |
| `nature.harvest` | faucet | economy | `gather` at a farm, a mine or a forest, or a shift there. Nodes regenerate over time |
| `production.output` | faucet | economy | A recipe ends |
| `import.purchase` | faucet | economy | Goods bought from the depot |
| `production.input` | sink | economy | A recipe ends. See below |
| `consumption.food` | sink | life | `eat`, and the reflex of a hungry resident that carries food |
| `consumption.use` | sink | life | `use_item` |
| `export.sale` | sink | economy, crime | Goods sold to the depot, and stolen goods sold with `fence` |
| `destruction` | sink | crime | `vandalize` destroys a part of the stock of a site |

- **A recipe consumes its inputs when it ends**, at the same moment at which
  it makes its outputs. The inputs are not locked while the work runs. An
  action that stops before its end uses nothing. When an input is missing
  at the end, the action fails and nothing changes.
- Food does not spoil, and tools do not wear. The game has no sink for
  either.
- Contraband that the police take is not destroyed. It stays in the stock
  of the police station.

## 4. Production chain

`farm`, `mine` and `forest` give raw goods. A `workshop` or `factory` turns
them into products with a recipe. A `shop` or `restaurant` sells products to
residents. Residents need food every day, so demand never stops.

Recipes are data: inputs, outputs, minutes, skill, minimum level, site kind.
Firms are organizations that own sites and hire workers.

## 5. Markets

- **Shop.** The owner sets prices. A buyer must be in the shop. The shop pays
  sales tax.
- **Exchange.** One order book per city for each item. A trader must be at
  the exchange to place an order or to collect goods. Orders lock goods or
  cash in escrow. Matching is price-time priority and runs each tick.
- **Depot.** The system buyer and seller of last resort. Its prices are far
  from normal prices. It only sets a floor and a ceiling.

## 6. Instruments

| Instrument | Rule |
| --- | --- |
| Job | The employer posts a wage per shift. The wage leaves the employer when the shift ends. No funds, no pay, and the worker sees the failure |
| Lease | Rent leaves the tenant each day. No payment means eviction after a grace time |
| Bank loan | Principal, rate and term. The bank holds a part of the account as collateral (all of the principal with the default numbers). Three missed instalments are a default: the bank takes the collateral, the borrower gets no credit for a time, and the city forecloses every site that the borrower owns |
| Shares | A firm issues shares. Holders get dividends and can trade shares on the exchange |
| Contract | A general agreement between residents with escrow (see the society module) |

## 7. Taxes

Income tax (on wages), sales tax (on shop sales), property tax (each day on
site value). Rates are policy parameters. A law can change them for the
nation or for one city. Revenue goes to the city or nation treasury.

## 8. Public statistics

The finance module publishes the statistics. At each midnight it emits the
public event `finance.stats_published`: the money supply and where the
money is (cash, deposits, treasuries, tills, escrow), a price index (a
basket of the food items that were sold), wealth inequality (Gini), the
loans of the bank, the base rate, and the flow through each faucet and
sink, for money and for goods. Viewers read the newest publication at
`GET /v1/stats/economy`. `docs/design/modules/finance.md`, section 7, has
the formulas and the approximations.

The statistics have no unemployment number and no price of housing.

## 9. Invariants that tests check

1. For money and for each item: held = created - destroyed, in every realm
   and in the whole world, on every tick.
2. No balance is negative.
3. A failed operation changes nothing.
4. The same seed and inputs give the same prices and balances.
