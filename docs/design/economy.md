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

| Name | Kind | Trigger | Control |
| --- | --- | --- | --- |
| `gov.public_works` | faucet | A resident completes a starter job shift | Parameter: wage and daily budget per city. A law can change it |
| `bank.loan_issue` | faucet | The bank grants a loan | Parameter: reserve limit, interest rate (central bank) |
| `central_bank.issue` | faucet | The central banker issues money to the nation treasury | Parameter: yearly limit. A law can change it |
| `export.sale` | faucet | The depot buys goods at the floor price | Parameter: floor price per item |
| `bank.loan_repay` | sink | A borrower repays principal | Automatic |
| `import.purchase` | sink | A resident or firm buys goods from the depot at the ceiling price | Parameter: ceiling price per item |
| `central_bank.retire` | sink | The central banker removes treasury money | Role permission `bank.central` |
| `estate.unclaimed` | sink | An estate has no heir and no city | Automatic |

Taxes, fines, fees, wages, rent, interest and dividends are transfers. They
do not create or destroy money.

## 3. Goods faucets and sinks

| Name | Kind | Trigger |
| --- | --- | --- |
| `nature.harvest` | faucet | `gather` at a farm, mine or forest. Nodes regenerate over time |
| `production.output` | faucet | A recipe finishes |
| `import.purchase` | faucet | Goods bought from the depot |
| `production.input` | sink | A recipe starts |
| `consumption.food` | sink | `eat` |
| `consumption.use` | sink | `use_item`, tool wear |
| `decay.spoilage` | sink | Food gets old |
| `export.sale` | sink | Goods sold to the depot |
| `destruction` | sink | Vandalism, confiscated contraband |

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
| Bank loan | Principal, rate and term. Missed payments lead to default and bankruptcy: assets are sold, the record stays |
| Shares | A firm issues shares. Holders get dividends and can trade shares on the exchange |
| Contract | A general agreement between residents with escrow (see the society module) |

## 7. Taxes

Income tax (on wages), sales tax (on shop sales), property tax (each day on
site value). Rates are policy parameters. A law can change them for the
nation or for one city. Revenue goes to the city or nation treasury.

## 8. Public statistics

The economy module publishes, each game day: money supply, price index
(basket of food and housing), wealth inequality (Gini), unemployment, and the
flow through each faucet and sink. Viewers and residents can read them.

## 9. Invariants that tests check

1. For money and for each item: held = created - destroyed, in every realm
   and in the whole world, on every tick.
2. No balance is negative.
3. A failed operation changes nothing.
4. The same seed and inputs give the same prices and balances.
