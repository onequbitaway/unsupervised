//! Reads: what a resident has, is and sees.

use serde_json::Value;
use unsup_kernel::protocol::{Detail, Notice, ObserveOpts};
use unsup_kernel::{CityId, Holder, OrgId, Pos, ResidentId, ZoneId};

use crate::game::Game;

impl Game {
    /// Returns the cash of a resident in cents.
    #[must_use]
    pub fn cash(&self, r: ResidentId) -> i64 {
        self.world.cash(r).cents()
    }

    /// Returns the money of a holder in cents, summed over all realms.
    #[must_use]
    pub fn money(&self, holder: Holder) -> i64 {
        self.world.money(holder).cents()
    }

    /// Returns how many units of an item a resident carries.
    #[must_use]
    pub fn carried(&self, r: ResidentId, item: &str) -> i64 {
        self.world.qty(Holder::Resident(r), item)
    }

    /// Returns how many units of an item a holder has.
    #[must_use]
    pub fn qty(&self, holder: Holder, item: &str) -> i64 {
        self.world.qty(holder, item)
    }

    /// Returns an attribute of a resident.
    #[must_use]
    pub fn attr(&self, r: ResidentId, name: &str) -> i64 {
        self.world.attr(r, name)
    }

    /// Returns true when a resident has a condition.
    #[must_use]
    pub fn has_condition(&self, r: ResidentId, name: &str) -> bool {
        self.world.has_condition(r, name)
    }

    /// Returns true when the resident is in the world.
    #[must_use]
    pub fn is_alive(&self, r: ResidentId) -> bool {
        self.world.is_alive(r)
    }

    /// Returns the position of a resident.
    #[must_use]
    pub fn pos(&self, r: ResidentId) -> Pos {
        self.world.pos(r)
    }

    /// Returns the zone of a resident.
    #[must_use]
    pub fn zone_of(&self, r: ResidentId) -> ZoneId {
        self.world.zone_of(r)
    }

    /// Returns the current value of a parameter in the global realm.
    #[must_use]
    pub fn param(&self, name: &str) -> i64 {
        self.world.param(name)
    }

    /// Returns the value of a parameter in a city: the value that a city
    /// law made, else the national value.
    #[must_use]
    pub fn param_in(&self, name: &str, city: CityId) -> i64 {
        match self.world.world.blueprint().param_id(name) {
            Some(param) => self.world.global().param_in(param, city),
            None => panic!("no parameter is named `{name}`"),
        }
    }

    /// Returns the money in the treasury of a city in cents.
    #[must_use]
    pub fn city_treasury(&self, city: CityId) -> i64 {
        self.money(Holder::Org(OrgId::city(city)))
    }

    /// Returns the money in the national treasury in cents.
    #[must_use]
    pub fn national_treasury(&self) -> i64 {
        self.money(Holder::Org(OrgId::NATION))
    }

    /// Returns one section of the brief observation of a resident.
    #[must_use]
    pub fn section(&self, r: ResidentId, section: &str) -> Option<Value> {
        self.world.observe(r).sections.get(section).cloned()
    }

    /// Returns one section of the observation of a resident with full
    /// detail.
    #[must_use]
    pub fn section_full(&self, r: ResidentId, section: &str) -> Option<Value> {
        let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
        self.world.observe_with(r, &opts).sections.get(section).cloned()
    }

    /// Returns the notices of one kind that the inbox of a resident still
    /// has, oldest first.
    #[must_use]
    pub fn notices(&self, r: ResidentId, kind: &str) -> Vec<Notice> {
        self.world.notices(r).into_iter().filter(|notice| notice.kind == kind).collect()
    }

    /// Returns the newest notice of one kind in the inbox of a resident.
    ///
    /// # Panics
    ///
    /// Panics when the inbox has no such notice.
    #[must_use]
    pub fn notice(&self, r: ResidentId, kind: &str) -> Notice {
        match self.notices(r, kind).pop() {
            Some(notice) => notice,
            None => panic!("resident {r} has no notice `{kind}`"),
        }
    }
}
