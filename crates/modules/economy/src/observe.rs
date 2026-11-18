//! The observation sections of the module: `job`, `business` and
//! `property`. Each stays small; a section with nothing to say is absent.

use serde_json::{Map, Value, json};
use unsup_kernel::{Holder, Idx, ItemId, ObserveOpts, ParamId, Realm, Site};

use crate::ids::Econ;
use crate::public_works;
use crate::site::{self, ATTR_PRICE, ATTR_TAX_DUE};
use crate::state::jobs::{Job, JobBook};
use crate::state::leases::Leases;
use crate::state::nodes::Nodes;
use crate::state::prices::PriceBook;
use crate::state::ties::Ties;

/// The section `job`: the job of the resident.
pub(crate) fn job(econ: &Econ, realm: &Realm, idx: Idx, _opts: &ObserveOpts) -> Option<Value> {
    let ties = realm.col::<Ties>();
    let tie = ties[idx].job()?;
    Some(json!({
        "job": tie.job,
        "site": tie.site,
        "zone": tie.zone,
        "kind": tie.kind,
        "wage": tie.wage,
        "shift_minutes": realm.param(econ.ids.shift_minutes),
    }))
}

/// Returns the first `max` entries and how many were left out.
fn cut<T>(mut list: Vec<T>, max: usize) -> (Vec<T>, usize) {
    let more = list.len().saturating_sub(max);
    list.truncate(max);
    (list, more)
}

/// Returns a limit of a brief observation from its parameter. An
/// observation with full detail has no limit.
fn limit(realm: &Realm, opts: &ObserveOpts, param: ParamId) -> usize {
    if opts.is_full() { usize::MAX } else { usize::try_from(realm.param(param)).unwrap_or(0) }
}

/// Returns the price list of a shop: each good with its prices and the
/// units in the stock.
fn price_list(realm: &Realm, here: &Site) -> Vec<(ItemId, Value)> {
    let prices = realm.res::<PriceBook>();
    let ledger = realm.ledger();
    let defs = realm.defs();
    prices
        .sites
        .get(&here.id)
        .into_iter()
        .flatten()
        .map(|(&item, price)| {
            let mut entry = json!({
                "item": defs.item_name(item),
                "qty": ledger.qty(Holder::Site(here.id), item),
            });
            if let Some(sell) = price.sell {
                entry["price"] = json!(sell);
            }
            if let Some(buy) = price.buy {
                entry["buy_price"] = json!(buy);
            }
            (item, entry)
        })
        .collect()
}

fn depot_list(econ: &Econ, realm: &Realm) -> Vec<Value> {
    let defs = realm.defs();
    econ.rules
        .depot
        .iter()
        .map(|(&item, prices)| {
            json!({
                "item": defs.item_name(item),
                "price": realm.param(prices.ceiling),
                "buy_price": realm.param(prices.floor),
            })
        })
        .collect()
}

/// Returns the jobs of a site that take workers, the best wage first.
fn open_jobs(realm: &Realm, here: &Site) -> Vec<Value> {
    let book = realm.res::<JobBook>();
    let mut jobs: Vec<&Job> = book.at_site(here.id).filter(|job| job.free_slots() > 0).collect();
    jobs.sort_by_key(|job| (std::cmp::Reverse(job.wage), job.id));
    jobs.into_iter()
        .map(|job| {
            let mut entry = json!({
                "job": job.id,
                "wage": job.wage,
                "free_slots": job.free_slots(),
                "min_level": job.min_level,
            });
            if let Some(task) = &job.task {
                entry["task"] = json!(task);
            }
            entry
        })
        .collect()
}

/// Returns the goods in the stock of a site, without the goods in `shown`:
/// the price list has their units already.
fn stock_list(realm: &Realm, here: &Site, shown: &[ItemId]) -> Vec<Value> {
    let defs = realm.defs();
    realm
        .ledger()
        .items_of(Holder::Site(here.id))
        .into_iter()
        .filter(|(item, _)| !shown.contains(item))
        .map(|(item, qty)| json!({ "item": defs.item_name(item), "qty": qty }))
        .collect()
}

/// Puts a list into a section, with the count of the entries that were
/// left out. An empty list is absent.
fn insert_list(out: &mut Map<String, Value>, name: &str, list: Vec<Value>, more: usize) {
    if !list.is_empty() {
        out.insert(name.into(), json!(list));
    }
    if more > 0 {
        out.insert(format!("more_{name}"), json!(more));
    }
}

/// The section `business`: what the site where the resident is offers:
/// prices, jobs, goods to gather, starter shifts, and whether it is for
/// sale or to let. Whoever controls the site also sees the till. It and
/// the workers of the site see the stock.
pub(crate) fn business(econ: &Econ, realm: &Realm, idx: Idx, opts: &ObserveOpts) -> Option<Value> {
    let sites = realm.sites();
    let here = sites.at(realm.pos(idx))?;
    let rule = econ.rules.kind(&here.kind);
    let resident = realm.id_of(idx);
    let max = limit(realm, opts, econ.ids.observe_prices_max);
    let mut out = Map::new();

    // The goods whose units the price list shows.
    let mut shown: Vec<ItemId> = Vec::new();
    if rule.depot {
        out.insert("depot".into(), json!(true));
        if opts.is_full() {
            out.insert("prices".into(), json!(depot_list(econ, realm)));
        }
    } else {
        let (prices, more) = cut(price_list(realm, here), max);
        shown = prices.iter().map(|(item, _)| *item).collect();
        let prices = prices.into_iter().map(|(_, entry)| entry).collect();
        insert_list(&mut out, "prices", prices, more);
    }

    let (jobs, more) = cut(open_jobs(realm, here), limit(realm, opts, econ.ids.observe_jobs_max));
    insert_list(&mut out, "jobs", jobs, more);

    if !rule.gather.is_empty() {
        let defs = realm.defs();
        let goods: Vec<&str> = rule.gather.iter().map(|y| defs.item_name(y.item)).collect();
        let stock = realm.res::<Nodes>().stock(here.id, realm.tick().0, rule);
        out.insert("gather".into(), json!({ "goods": goods, "left": stock }));
    }
    if let Some(wage) = public_works::wage_at(econ, realm, here).filter(|wage| *wage > 0) {
        let minutes = realm.param(econ.ids.shift_minutes);
        out.insert("public_works".into(), json!({ "wage": wage, "minutes": minutes }));
    }
    if let Some(price) = site::asking_price(here) {
        out.insert("for_sale".into(), json!(price));
    }
    let free_units = site::units(here, rule).saturating_sub(here.tenants.len());
    if let Some(rent) =
        site::asking_rent(here, rule, realm.param(econ.ids.city_rent_bp)).filter(|_| free_units > 0)
    {
        out.insert("for_rent".into(), json!(rent));
    }

    let controls = site::controls(realm, idx, here);
    if controls {
        out.insert("till".into(), json!(realm.ledger().money(Holder::Site(here.id))));
    }
    if controls || realm.res::<JobBook>().job_of(here.id, resident).is_some() {
        let (goods, more) = cut(stock_list(realm, here, &shown), max);
        insert_list(&mut out, "stock", goods, more);
    }

    if out.is_empty() {
        return None;
    }
    out.insert("site".into(), json!(here.id));
    out.insert("kind".into(), json!(here.kind));
    if rule.business {
        out.insert("open".into(), json!(site::is_open(here)));
    }
    Some(Value::Object(out))
}

/// The section `property`: the sites that the resident owns and the homes
/// that it rents. A site in the zone of the resident has more detail. A
/// brief observation shows the oldest entries and counts the rest.
pub(crate) fn property(econ: &Econ, realm: &Realm, idx: Idx, opts: &ObserveOpts) -> Option<Value> {
    let ties = realm.col::<Ties>();
    let mine = &ties[idx];
    if mine.owned().is_empty() && mine.leases().is_empty() {
        return None;
    }
    let max = limit(realm, opts, econ.ids.observe_sites_max);
    let sites = realm.sites();
    let ledger = realm.ledger();
    let leases = realm.res::<Leases>();
    let resident = realm.id_of(idx);
    let mut out = Map::new();
    let owned: Vec<Value> = mine
        .owned()
        .iter()
        .map(|tie| {
            let mut entry = json!({ "site": tie.site, "zone": tie.zone, "kind": tie.kind });
            if let Some(here) = sites.get(tie.site) {
                entry["kind"] = json!(here.kind);
                entry["till"] = json!(ledger.money(Holder::Site(tie.site)));
                if !here.tenants.is_empty() {
                    entry["tenants"] = json!(here.tenants.len());
                }
                for (name, attr) in [("tax_due", ATTR_TAX_DUE), ("price", ATTR_PRICE)] {
                    if here.attr(attr) > 0 {
                        entry[name] = json!(here.attr(attr));
                    }
                }
            }
            entry
        })
        .collect();
    let (owned, more) = cut(owned, max);
    insert_list(&mut out, "owned", owned, more);
    let rented: Vec<Value> = mine
        .leases()
        .iter()
        .map(|tie| {
            let mut entry = json!({ "site": tie.site, "zone": tie.zone, "rent": tie.rent });
            if let Some(lease) = leases.get(tie.site, resident).filter(|lease| lease.overdue > 0) {
                entry["owed"] = json!(lease.owed);
                entry["days_missed"] = json!(lease.overdue);
            }
            entry
        })
        .collect();
    let (rented, more) = cut(rented, max);
    insert_list(&mut out, "leases", rented, more);
    Some(Value::Object(out))
}
