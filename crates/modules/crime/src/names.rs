//! The names that the crime module shares with other modules and with
//! clients: conditions, attributes, permissions, tags, sinks, events and
//! notices. Most come from `docs/design/game-design.md`.

/// Conditions of a resident.
pub mod cond {
    /// The resident has an open warrant. Police can arrest it on sight.
    pub const WANTED: &str = "wanted";
    /// The resident is under arrest and waits for a decision. Blocking.
    pub const DETAINED: &str = "detained";
    /// The resident serves a sentence in jail. Blocking.
    pub const JAILED: &str = "jailed";
}

/// Attributes of a resident.
pub mod attr {
    /// Criminal fame, 0 and up. A conviction raises it.
    pub const NOTORIETY: &str = "rep.notoriety";
    /// Public reputation, -1000 to 1000. The society module owns it.
    pub const STANDING: &str = "rep.standing";
}

/// Permissions that the module checks. They are powers of the state: a
/// resident holds one in a city when its organization has it from the
/// government of that city or from the nation.
pub mod perm {
    /// Lets a resident arrest, charge and release a suspect.
    pub const ARREST: &str = "law.arrest";
    /// Lets a resident search a resident and investigate a scene.
    pub const SEARCH: &str = "law.search";
    /// Lets a resident rule a case.
    pub const JUDGE: &str = "law.judge";
    /// Lets a resident release an inmate before the end of its sentence.
    pub const JAIL: &str = "law.jail";
}

/// Tags of items and sites that the module reads.
pub mod tag {
    /// An item that is illegal to carry while the parameter
    /// `crime.contraband_illegal` is 1.
    pub const CONTRABAND: &str = "contraband";
    /// An item that helps a burglar.
    pub const TOOL: &str = "tool";
    /// A public building. The city watch looks for wanted residents there.
    pub const PUBLIC: &str = "public";
}

/// Named numbers of sites and items that the module reads or writes.
pub mod key {
    /// Site: the value in cents. Vandalism lowers it and `repair` gives it
    /// back.
    pub const VALUE: &str = "value";
    /// Site: the value in cents that vandalism took and that `repair` can
    /// give back.
    pub const DAMAGE: &str = "damage";
    /// Item: the normal price of one unit in cents. The economy module
    /// writes it. The module uses it to rank loot and to price a good that
    /// the depot does not trade.
    pub const BASE_PRICE: &str = "base_price";
}

/// Faucets and sinks that the module uses.
pub mod flow {
    /// Sink of goods that vandalism destroys.
    pub const DESTRUCTION: &str = "destruction";
    /// Sink of the money that `repair` costs.
    pub const REPAIR: &str = "repair.materials";
    /// Sink of the goods and faucet of the money of `fence`. The economy
    /// module uses the same names for a sale to the depot.
    pub const EXPORT: &str = "export.sale";
    /// Prefix of the parameters with the price that the depot pays for one
    /// unit of a good. The economy module defines them.
    pub const DEPOT_FLOOR: &str = "economy.depot_floor.";
}

/// Kinds of the public events of the module.
pub mod event {
    /// A theft, a burglary or a robbery. The data never names the thief.
    pub const THEFT: &str = "crime.theft_committed";
    /// An attack on a resident. The data never names the attacker.
    pub const ASSAULT: &str = "crime.assault_committed";
    /// Damage to a site. The data never names the vandal.
    pub const VANDALISM: &str = "crime.vandalism_committed";
    /// Contraband was taken from a carrier. The data never names the
    /// carrier.
    pub const CONTRABAND: &str = "crime.contraband_seized";
    /// A warrant was written. The data never names the suspect.
    pub const WARRANT: &str = "crime.warrant_issued";
    /// A search of a resident. The data never names the resident.
    pub const SEARCH: &str = "crime.search_made";
    /// An arrest. The data has `officer`, `target` and `case`.
    pub const ARREST: &str = "crime.arrest_made";
    /// A suspect in custody was charged. A trial starts.
    pub const CHARGED: &str = "crime.suspect_charged";
    /// A suspect or an inmate left custody without a sentence to serve.
    pub const RELEASED: &str = "crime.suspect_released";
    /// A jury was called for a trial.
    pub const JURY: &str = "crime.jury_summoned";
    /// The end of a trial.
    pub const VERDICT: &str = "crime.verdict_given";
    /// An inmate reached the end of its sentence.
    pub const SERVED: &str = "crime.sentence_served";
    /// A convict paid a fine or a restitution.
    pub const FINE_PAID: &str = "crime.fine_paid";
    /// A fine that was not paid in time became jail time.
    pub const FINE_DEFAULTED: &str = "crime.fine_defaulted";
    /// Stolen goods were sold. The data never names the seller.
    pub const FENCED: &str = "crime.goods_fenced";
    /// A damaged site was repaired.
    pub const REPAIRED: &str = "crime.site_repaired";
    /// A bounty was posted.
    pub const BOUNTY_POSTED: &str = "crime.bounty_posted";
    /// A bounty was paid to the officer that made the arrest.
    pub const BOUNTY_PAID: &str = "crime.bounty_paid";
    /// A bounty ended without an arrest. The money went back.
    pub const BOUNTY_EXPIRED: &str = "crime.bounty_expired";
}

/// Kinds of the notices of the module.
pub mod notice {
    /// To a victim or a witness of a crime.
    pub const WITNESSED: &str = "crime.witnessed";
    /// To the receiver of a bribe.
    pub const BRIBE: &str = "crime.bribe_received";
    /// To a resident that a warrant names, and when the warrant ends.
    pub const WANTED: &str = "crime.wanted";
    /// To a resident that was searched.
    pub const SEARCHED: &str = "crime.searched";
    /// To a resident that was arrested.
    pub const ARRESTED: &str = "crime.arrested";
    /// To a suspect that was charged.
    pub const CHARGED: &str = "crime.charged";
    /// To a resident that left custody.
    pub const RELEASED: &str = "crime.released";
    /// To a resident that was called for a jury, and when its duty ends.
    pub const JURY: &str = "crime.jury_duty";
    /// To the defendant and the victim at the end of a trial.
    pub const VERDICT: &str = "crime.verdict";
    /// To a convict whose fine became jail time.
    pub const FINE_DEFAULTED: &str = "crime.fine_defaulted";
    /// To a victim that got a restitution.
    pub const RESTITUTION: &str = "crime.restitution";
    /// To the poster of a bounty and to the officer that earned it.
    pub const BOUNTY: &str = "crime.bounty";
    /// To an officer on patrol that saw a crime.
    pub const PATROL: &str = "crime.patrol_alert";
}

/// Labels of the ledger movements of the module.
pub mod label {
    /// Stolen cash and goods.
    pub const LOOT: &str = "crime.loot";
    /// Cash of a bribe.
    pub const BRIBE: &str = "crime.bribe";
    /// Hot goods, hot cash and contraband that a search took.
    pub const SEIZED: &str = "crime.seized";
    /// A fine to the city treasury.
    pub const FINE: &str = "crime.fine";
    /// A restitution to the victim.
    pub const RESTITUTION: &str = "crime.restitution";
    /// Money of a bounty: into the escrow, to the officer or back.
    pub const BOUNTY: &str = "crime.bounty";
}
