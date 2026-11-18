//! The events of the module: kinds, drama scores and payloads.
//!
//! The kinds in the first group have fixed data
//! (`docs/design/game-design.md`, section 9). Other modules read them.

use serde_json::json;
use unsup_kernel::{Builder, CityId, EventMeta, Holder, Pos, Realm, ResidentId, SiteId};

/// A shift ended and the wage was paid.
pub const SHIFT_COMPLETED: &str = "economy.shift_completed";
/// Goods were sold for money: at a shop, to a shop or at the depot.
pub const GOODS_SOLD: &str = "economy.goods_sold";
/// Goods were made by a recipe or gathered from a node.
pub const GOODS_PRODUCED: &str = "economy.goods_produced";
/// A site has a new owner after a sale.
pub const PROPERTY_SOLD: &str = "economy.property_sold";
/// A tenant moved in.
pub const LEASE_STARTED: &str = "economy.lease_started";
/// A tenant was evicted because it did not pay.
pub const TENANT_EVICTED: &str = "economy.tenant_evicted";
/// A business got its permit.
pub const BUSINESS_OPENED: &str = "economy.business_opened";

/// A business gave up its permit.
pub const BUSINESS_CLOSED: &str = "economy.business_closed";
/// A job was posted.
pub const JOB_POSTED: &str = "economy.job_posted";
/// A resident took a job.
pub const WORKER_HIRED: &str = "economy.worker_hired";
/// A resident left a job.
pub const WORKER_LEFT: &str = "economy.worker_left";
/// A shift ended and the till could not pay the wage.
pub const SHIFT_UNPAID: &str = "economy.shift_unpaid";
/// A lease ended without an eviction.
pub const LEASE_ENDED: &str = "economy.lease_ended";
/// The city took a site: a loan default or unpaid property tax.
pub const PROPERTY_FORECLOSED: &str = "economy.property_foreclosed";
/// A site went to the heir of its dead owner.
pub const PROPERTY_INHERITED: &str = "economy.property_inherited";

/// The event kind for the drama score of a worker that is fired. The event
/// itself is `economy.worker_left`.
const WORKER_FIRED: &str = "economy.worker_fired";

/// The parameter with the drama score of each event kind
/// (`docs/design/game-design.md`, section 6).
const DRAMA: [(&str, &str); 16] = [
    (SHIFT_COMPLETED, "economy.drama.shift_completed"),
    (GOODS_SOLD, "economy.drama.goods_sold"),
    (GOODS_PRODUCED, "economy.drama.goods_produced"),
    (PROPERTY_SOLD, "economy.drama.property_sold"),
    (LEASE_STARTED, "economy.drama.lease_started"),
    (TENANT_EVICTED, "economy.drama.tenant_evicted"),
    (BUSINESS_OPENED, "economy.drama.business_opened"),
    (BUSINESS_CLOSED, "economy.drama.business_closed"),
    (JOB_POSTED, "economy.drama.job_posted"),
    (WORKER_HIRED, "economy.drama.worker_hired"),
    (WORKER_LEFT, "economy.drama.worker_left"),
    (WORKER_FIRED, "economy.drama.worker_fired"),
    (SHIFT_UNPAID, "economy.drama.shift_unpaid"),
    (LEASE_ENDED, "economy.drama.lease_ended"),
    (PROPERTY_FORECLOSED, "economy.drama.property_foreclosed"),
    (PROPERTY_INHERITED, "economy.drama.property_inherited"),
];

/// Checks that the data file has the drama parameter of every event kind.
/// A missing parameter is a mistake in the plugin and panics when the world
/// is made, so that no event gets a score of 0 by accident.
pub(crate) fn check_drama_params(b: &Builder) {
    for (_, param) in DRAMA {
        let _ = b.param_id(param);
    }
}

/// Returns the drama score of an event kind from its parameter.
fn drama(realm: &Realm, scored_as: &str) -> u8 {
    let score = DRAMA
        .iter()
        .find(|(kind, _)| *kind == scored_as)
        .and_then(|(_, param)| realm.defs().param_id(param))
        .map_or(0, |param| realm.param(param));
    u8::try_from(score.clamp(0, 100)).unwrap_or(0)
}

/// Emits a public event. `scored_as` is the kind whose drama score the
/// event gets; two events can share one kind.
fn emit_as(
    realm: &Realm,
    kind: &'static str,
    scored_as: &str,
    actor: Option<ResidentId>,
    pos: Option<Pos>,
    data: &serde_json::Value,
) {
    let mut meta = EventMeta::public(kind, drama(realm, scored_as));
    meta.actor = actor;
    meta.pos = pos;
    realm.emit(meta, data);
}

/// Emits a public event with the drama score of its own kind.
fn emit(
    realm: &Realm,
    kind: &'static str,
    actor: Option<ResidentId>,
    pos: Option<Pos>,
    data: &serde_json::Value,
) {
    emit_as(realm, kind, kind, actor, pos, data);
}

/// `economy.shift_completed`. `wage` is the wage before tax, `tax` the
/// income tax that was withheld, `skill` the name of the skill that the
/// shift trained.
pub(crate) fn shift_completed(
    realm: &Realm,
    resident: ResidentId,
    site: SiteId,
    pos: Pos,
    wage: i64,
    tax: i64,
    skill: &str,
) {
    emit(
        realm,
        SHIFT_COMPLETED,
        Some(resident),
        Some(pos),
        &json!({ "resident": resident, "site": site, "wage": wage, "tax": tax, "skill": skill }),
    );
}

/// The facts of one sale of goods.
pub(crate) struct Sale<'a> {
    /// Who gave the goods.
    pub(crate) seller: Holder,
    /// The resident that got the goods. `None` when a shop or the depot
    /// bought.
    pub(crate) buyer: Option<ResidentId>,
    /// Where.
    pub(crate) site: SiteId,
    /// The city of the site.
    pub(crate) city: CityId,
    /// Name of the good.
    pub(crate) item: &'a str,
    /// Units.
    pub(crate) qty: i64,
    /// Cents for one unit.
    pub(crate) unit_price: i64,
    /// Sales tax in cents.
    pub(crate) tax: i64,
}

/// `economy.goods_sold`.
pub(crate) fn goods_sold(realm: &Realm, actor: ResidentId, pos: Pos, sale: &Sale<'_>) {
    emit(
        realm,
        GOODS_SOLD,
        Some(actor),
        Some(pos),
        &json!({
            "seller": sale.seller,
            "buyer": sale.buyer,
            "site": sale.site,
            "city": sale.city,
            "item": sale.item,
            "qty": sale.qty,
            "unit_price": sale.unit_price,
            "tax": sale.tax,
        }),
    );
}

/// `economy.goods_produced`.
pub(crate) fn goods_produced(
    realm: &Realm,
    resident: ResidentId,
    site: SiteId,
    pos: Pos,
    item: &str,
    qty: i64,
) {
    emit(
        realm,
        GOODS_PRODUCED,
        Some(resident),
        Some(pos),
        &json!({ "resident": resident, "site": site, "item": item, "qty": qty }),
    );
}

/// `economy.property_sold`.
pub(crate) fn property_sold(
    realm: &Realm,
    actor: ResidentId,
    pos: Pos,
    site: SiteId,
    from: Option<Holder>,
    to: Holder,
    price: i64,
) {
    let city = crate::site::city(realm);
    emit(
        realm,
        PROPERTY_SOLD,
        Some(actor),
        Some(pos),
        &json!({ "site": site, "city": city, "from": from, "to": to, "price": price }),
    );
}

/// `economy.lease_started`.
pub(crate) fn lease_started(realm: &Realm, site: SiteId, pos: Pos, tenant: ResidentId, rent: i64) {
    emit(
        realm,
        LEASE_STARTED,
        Some(tenant),
        Some(pos),
        &json!({ "site": site, "tenant": tenant, "rent": rent }),
    );
}

/// `economy.tenant_evicted`.
pub(crate) fn tenant_evicted(realm: &Realm, site: SiteId, pos: Pos, tenant: ResidentId) {
    emit(
        realm,
        TENANT_EVICTED,
        Some(tenant),
        Some(pos),
        &json!({ "site": site, "tenant": tenant }),
    );
}

/// `economy.business_opened`.
pub(crate) fn business_opened(
    realm: &Realm,
    actor: ResidentId,
    site: SiteId,
    pos: Pos,
    owner: Option<Holder>,
    kind: &str,
) {
    emit(
        realm,
        BUSINESS_OPENED,
        Some(actor),
        Some(pos),
        &json!({ "site": site, "owner": owner, "kind": kind }),
    );
}

/// `economy.business_closed`. `reason` is `owner`, `converted`,
/// `foreclosure` or `estate`.
pub(crate) fn business_closed(
    realm: &Realm,
    site: SiteId,
    pos: Pos,
    owner: Option<Holder>,
    kind: &str,
    reason: &str,
) {
    emit(
        realm,
        BUSINESS_CLOSED,
        None,
        Some(pos),
        &json!({ "site": site, "owner": owner, "kind": kind, "reason": reason }),
    );
}

/// `economy.job_posted`.
pub(crate) fn job_posted(
    realm: &Realm,
    actor: ResidentId,
    pos: Pos,
    job: u64,
    site: SiteId,
    wage: i64,
    slots: u32,
) {
    let city = crate::site::city(realm);
    emit(
        realm,
        JOB_POSTED,
        Some(actor),
        Some(pos),
        &json!({ "job": job, "site": site, "city": city, "wage": wage, "slots": slots }),
    );
}

/// `economy.worker_hired`.
pub(crate) fn worker_hired(
    realm: &Realm,
    resident: ResidentId,
    pos: Pos,
    site: SiteId,
    job: u64,
    wage: i64,
) {
    emit(
        realm,
        WORKER_HIRED,
        Some(resident),
        Some(pos),
        &json!({ "resident": resident, "site": site, "job": job, "wage": wage }),
    );
}

/// `economy.worker_left`. `reason` is `quit`, `fired`, `closed` or `gone`.
pub(crate) fn worker_left(
    realm: &Realm,
    resident: ResidentId,
    pos: Pos,
    site: SiteId,
    job: u64,
    reason: &str,
) {
    // A worker that is fired is more news than one that quits.
    let scored_as = if reason == "fired" { WORKER_FIRED } else { WORKER_LEFT };
    emit_as(
        realm,
        WORKER_LEFT,
        scored_as,
        Some(resident),
        Some(pos),
        &json!({ "resident": resident, "site": site, "job": job, "reason": reason }),
    );
}

/// `economy.shift_unpaid`.
pub(crate) fn shift_unpaid(realm: &Realm, resident: ResidentId, site: SiteId, pos: Pos, wage: i64) {
    emit(
        realm,
        SHIFT_UNPAID,
        Some(resident),
        Some(pos),
        &json!({ "resident": resident, "site": site, "wage": wage }),
    );
}

/// `economy.lease_ended`. `by` is `tenant`, `owner` or `gone`.
pub(crate) fn lease_ended(realm: &Realm, site: SiteId, pos: Pos, tenant: ResidentId, by: &str) {
    emit(
        realm,
        LEASE_ENDED,
        Some(tenant),
        Some(pos),
        &json!({ "site": site, "tenant": tenant, "by": by }),
    );
}

/// `economy.property_foreclosed`. `reason` is `loan_default` or
/// `property_tax`.
pub(crate) fn property_foreclosed(
    realm: &Realm,
    site: SiteId,
    pos: Pos,
    from: Option<Holder>,
    reason: &str,
) {
    let city = crate::site::city(realm);
    emit(
        realm,
        PROPERTY_FORECLOSED,
        None,
        Some(pos),
        &json!({ "site": site, "city": city, "from": from, "reason": reason }),
    );
}

/// `economy.property_inherited`.
pub(crate) fn property_inherited(
    realm: &Realm,
    site: SiteId,
    pos: Pos,
    from: ResidentId,
    to: Holder,
) {
    let city = crate::site::city(realm);
    emit(
        realm,
        PROPERTY_INHERITED,
        None,
        Some(pos),
        &json!({ "site": site, "city": city, "from": from, "to": to }),
    );
}
