//! The city boards: jobs that take workers, and sites for sale or to let.
//!
//! The boards live in the global realm, so a resident reads the board of
//! its whole city with one free verb. Each zone tells the global realm what
//! changed: at the end of a tick in which a verb changed an offer, and at a
//! slow regular check that also finds sites that another module made or
//! changed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, CityId, Dest, GlobalCtx, Message, Pos, Realm, Resource, SiteId, ZoneId,
};

use crate::ids::Econ;
use crate::labour::Labour;
use crate::public_works;
use crate::site;
use crate::state::jobs::JobBook;

/// One site for sale or to let.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PropertyAd {
    site: SiteId,
    zone: ZoneId,
    city: CityId,
    kind: String,
    name: String,
    pos: Pos,
    /// Sale price in cents.
    price: Option<i64>,
    /// Rent in cents for one day.
    rent: Option<i64>,
    /// Tenants that the site can still take. 0 when it is not to let.
    units_free: u32,
}

/// One job that takes workers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JobAd {
    job: u64,
    site: SiteId,
    zone: ZoneId,
    city: CityId,
    kind: String,
    name: String,
    pos: Pos,
    wage: i64,
    skill: String,
    min_level: i64,
    task: Option<String>,
    free_slots: u32,
}

/// What a zone has told the global realm, and whether a verb changed an
/// offer since.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct BoardCache {
    dirty: bool,
    property: BTreeMap<SiteId, PropertyAd>,
    jobs: BTreeMap<u64, JobAd>,
}

impl Resource for BoardCache {
    const NAME: &'static str = "economy.board_cache";
}

/// The changes of the offers of one zone.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct BoardUpdate {
    property: Vec<PropertyAd>,
    property_gone: Vec<SiteId>,
    jobs: Vec<JobAd>,
    jobs_gone: Vec<u64>,
}

impl Message for BoardUpdate {
    const KIND: &'static str = "economy.board_update";
}

impl BoardUpdate {
    fn is_empty(&self) -> bool {
        self.property.is_empty()
            && self.property_gone.is_empty()
            && self.jobs.is_empty()
            && self.jobs_gone.is_empty()
    }
}

/// The boards of every city, in the global realm.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Boards {
    property: BTreeMap<SiteId, PropertyAd>,
    jobs: BTreeMap<u64, JobAd>,
}

impl Resource for Boards {
    const NAME: &'static str = "economy.boards";
}

/// Records that an offer of this zone changed. The next board system sends
/// the change.
pub(crate) fn touch(realm: &Realm) {
    realm.res_mut::<BoardCache>().dirty = true;
}

fn property_ads(econ: &Econ, realm: &Realm, zone: ZoneId, city: CityId) -> Vec<PropertyAd> {
    let city_rent_bp = realm.param(econ.ids.city_rent_bp);
    realm
        .sites()
        .iter()
        .filter_map(|s| {
            let rule = econ.rules.kind(&s.kind);
            let price = site::asking_price(s);
            let units = site::units(s, rule).saturating_sub(s.tenants.len());
            let rent = site::asking_rent(s, rule, city_rent_bp).filter(|_| units > 0);
            (price.is_some() || rent.is_some()).then(|| PropertyAd {
                site: s.id,
                zone,
                city,
                kind: s.kind.clone(),
                name: s.name.clone(),
                pos: s.rect.centre(),
                price,
                rent,
                units_free: if rent.is_some() {
                    u32::try_from(units).unwrap_or(u32::MAX)
                } else {
                    0
                },
            })
        })
        .collect()
}

fn job_ads(econ: &Econ, realm: &Realm, zone: ZoneId, city: CityId) -> Vec<JobAd> {
    let sites = realm.sites();
    realm
        .res::<JobBook>()
        .jobs
        .values()
        .filter(|job| job.free_slots() > 0)
        .filter_map(|job| {
            let s = sites.get(job.site)?;
            let rule = econ.rules.kind(&s.kind);
            let skill = match Labour::resolve(econ, realm, &s.kind, job.task.as_deref()) {
                Ok(labour) => labour.skill(rule),
                Err(_) => &rule.skill,
            };
            Some(JobAd {
                job: job.id,
                site: s.id,
                zone,
                city,
                kind: s.kind.clone(),
                name: s.name.clone(),
                pos: s.rect.centre(),
                wage: job.wage,
                skill: skill.name.clone(),
                min_level: job.min_level,
                task: job.task.clone(),
                free_slots: job.free_slots(),
            })
        })
        .collect()
}

/// Finds what changed between the old and the new set of entries. It
/// replaces the old set and returns the new or changed entries and the keys
/// that are gone.
fn diff<K: Ord + Copy, V: Clone + PartialEq>(
    old: &mut BTreeMap<K, V>,
    new: Vec<V>,
    key: impl Fn(&V) -> K,
) -> (Vec<V>, Vec<K>) {
    let new: BTreeMap<K, V> = new.into_iter().map(|v| (key(&v), v)).collect();
    let gone = old.keys().filter(|k| !new.contains_key(k)).copied().collect();
    let changed =
        new.iter().filter(|(k, v)| old.get(k) != Some(v)).map(|(_, v)| v.clone()).collect();
    *old = new;
    (changed, gone)
}

/// The board system of a zone. It runs each tick and does work only when a
/// verb changed an offer, or every `economy.board_refresh_minutes`.
pub(crate) fn sync(econ: &Econ, realm: &Realm) {
    let Some(zone) = realm.zone_info() else {
        return;
    };
    let period = u64::try_from(realm.param(econ.ids.board_refresh_minutes)).unwrap_or(1).max(1);
    if !realm.tick().0.is_multiple_of(period) && !realm.res::<BoardCache>().dirty {
        return;
    }
    let property = property_ads(econ, realm, zone.id, zone.city);
    let jobs = job_ads(econ, realm, zone.id, zone.city);
    let update = {
        let mut cache = realm.res_mut::<BoardCache>();
        cache.dirty = false;
        let (property, property_gone) = diff(&mut cache.property, property, |ad| ad.site);
        let (jobs, jobs_gone) = diff(&mut cache.jobs, jobs, |ad| ad.job);
        BoardUpdate { property, property_gone, jobs, jobs_gone }
    };
    if !update.is_empty() {
        realm.send(Dest::Global, &update);
    }
}

/// Applies the changes of one zone to the boards of the global realm.
pub(crate) fn on_update(realm: &Realm, update: BoardUpdate) {
    let mut boards = realm.res_mut::<Boards>();
    for site in update.property_gone {
        boards.property.remove(&site);
    }
    for ad in update.property {
        boards.property.insert(ad.site, ad);
    }
    for job in update.jobs_gone {
        boards.jobs.remove(&job);
    }
    for ad in update.jobs {
        boards.jobs.insert(ad.job, ad);
    }
}

/// What a property offer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Offer {
    /// The site is for sale.
    Sale,
    /// The site takes tenants.
    Rent,
}

/// Arguments of `list_property`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListPropertyArgs {
    /// Only sites of this kind, for example `home` or `shop`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only sites for sale (`sale`) or only sites that take tenants
    /// (`rent`). Both when absent.
    #[serde(default)]
    pub offer: Option<Offer>,
    /// Greatest sale price, or greatest rent for one day, in cents.
    #[serde(default)]
    pub max_price: Option<i64>,
    /// How many entries to return. The parameter `economy.list_max` limits
    /// it.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Arguments of `list_jobs`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListJobsArgs {
    /// Only jobs at sites of this kind, for example `farm` or `shop`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only jobs that use this skill, for example `farming`.
    #[serde(default)]
    pub skill: Option<String>,
    /// Least wage in cents for one shift.
    #[serde(default)]
    pub min_wage: Option<i64>,
    /// How many entries to return. The parameter `economy.list_max` limits
    /// it.
    #[serde(default)]
    pub limit: Option<u32>,
}

fn city_of(realm: &Realm, ctx: &GlobalCtx) -> Result<CityId, ActionError> {
    realm
        .layout()
        .zone(ctx.zone)
        .map(|zone| zone.city)
        .ok_or_else(|| ActionError::internal("Your zone is not on the map."))
}

fn limit_of(econ: &Econ, realm: &Realm, city: CityId, asked: Option<u32>) -> usize {
    let max = realm.param_in(econ.ids.list_max, city).max(1);
    let default = realm.param_in(econ.ids.list_default, city).clamp(1, max);
    let limit = asked.map_or(default, |n| i64::from(n).clamp(1, max));
    usize::try_from(limit).unwrap_or(1)
}

/// The verb `list_property`: the sites of the city of the resident that are
/// for sale or to let. With an offer, the cheapest come first; without,
/// the nearest.
pub(crate) fn list_property(
    econ: &Econ,
    realm: &Realm,
    ctx: &GlobalCtx,
    args: &ListPropertyArgs,
) -> Result<Value, ActionError> {
    let city = city_of(realm, ctx)?;
    let limit = limit_of(econ, realm, city, args.limit);
    let max = args.max_price.unwrap_or(i64::MAX);
    let boards = realm.res::<Boards>();
    let mut found: Vec<(i64, u32, &PropertyAd)> = boards
        .property
        .values()
        .filter(|ad| ad.city == city)
        .filter(|ad| args.kind.as_ref().is_none_or(|kind| *kind == ad.kind))
        .filter_map(|ad| {
            let sale = ad.price.filter(|p| *p <= max);
            let rent = ad.rent.filter(|r| *r <= max);
            let cost = match args.offer {
                Some(Offer::Sale) => sale?,
                Some(Offer::Rent) => rent?,
                None => {
                    sale.or(rent)?;
                    0
                }
            };
            Some((cost, ad.pos.dist(ctx.pos), ad))
        })
        .collect();
    found.sort_by_key(|(cost, dist, ad)| (*cost, *dist, ad.site));
    let more = found.len().saturating_sub(limit);
    let listings: Vec<Value> = found
        .into_iter()
        .take(limit)
        .map(|(_, _, ad)| {
            let mut entry = json!({
                "site": ad.site,
                "kind": ad.kind,
                "name": ad.name,
                "zone": ad.zone,
                "pos": ad.pos,
            });
            if let Some(price) = ad.price.filter(|_| args.offer != Some(Offer::Rent)) {
                entry["price"] = json!(price);
            }
            if let Some(rent) = ad.rent.filter(|_| args.offer != Some(Offer::Sale)) {
                entry["rent"] = json!(rent);
                entry["units_free"] = json!(ad.units_free);
            }
            entry
        })
        .collect();
    Ok(json!({ "city": city, "listings": listings, "more": more }))
}

/// The verb `list_jobs`: the jobs of the city of the resident that take
/// workers. The best wage comes first, then the nearest site. The answer
/// also says what the city pays for a starter shift.
pub(crate) fn list_jobs(
    econ: &Econ,
    realm: &Realm,
    ctx: &GlobalCtx,
    args: &ListJobsArgs,
) -> Result<Value, ActionError> {
    let city = city_of(realm, ctx)?;
    let limit = limit_of(econ, realm, city, args.limit);
    let min_wage = args.min_wage.unwrap_or(0);
    let boards = realm.res::<Boards>();
    let mut found: Vec<&JobAd> = boards
        .jobs
        .values()
        .filter(|ad| ad.city == city && ad.wage >= min_wage)
        .filter(|ad| args.kind.as_ref().is_none_or(|kind| *kind == ad.kind))
        .filter(|ad| args.skill.as_ref().is_none_or(|skill| *skill == ad.skill))
        .collect();
    found.sort_by_key(|ad| (std::cmp::Reverse(ad.wage), ad.pos.dist(ctx.pos), ad.job));
    let more = found.len().saturating_sub(limit);
    let jobs: Vec<Value> = found
        .into_iter()
        .take(limit)
        .map(|ad| {
            let mut entry = json!({
                "job": ad.job,
                "site": ad.site,
                "kind": ad.kind,
                "name": ad.name,
                "zone": ad.zone,
                "pos": ad.pos,
                "wage": ad.wage,
                "skill": ad.skill,
                "min_level": ad.min_level,
                "free_slots": ad.free_slots,
            });
            if let Some(task) = &ad.task {
                entry["task"] = json!(task);
            }
            entry
        })
        .collect();
    let mut out = json!({ "city": city, "jobs": jobs, "more": more });
    if let Some(offer) = public_works::offer(econ, realm, city) {
        out["public_works"] = offer;
    }
    Ok(out)
}
