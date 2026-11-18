//! The scene of a resident, read from a world in this process.

use unsup_kernel::{Holder, Idx, Realm, World};
use unsup_protocol::{Pos, RealmId, ResidentId};

use crate::map::{CityMap, WorldMap};
use crate::scene::{Carried, NearSite, Neighbour, Scene};

/// Buffers that many scenes share one after the other, so that a scene does
/// not allocate for each resident.
#[derive(Debug, Default)]
pub struct Scratch {
    indices: Vec<Idx>,
    residents: Vec<Neighbour>,
    sites: Vec<NearSite>,
    carried: Vec<Carried>,
    site_kind: String,
}

/// What one resident of a world in this process knows. The facts are read
/// from the zone only when a generator or a condition asks.
pub struct LocalScene<'a> {
    realm: &'a Realm,
    idx: Idx,
    me: ResidentId,
    pos: Pos,
    map: Option<&'a CityMap>,
    scratch: &'a mut Scratch,
}

impl<'a> LocalScene<'a> {
    /// Makes the scene of a resident. Returns `None` when the resident is
    /// not in the world.
    pub fn of(
        world: &'a World,
        map: &'a WorldMap,
        resident: ResidentId,
        scratch: &'a mut Scratch,
    ) -> Option<LocalScene<'a>> {
        let zone = world.locate(resident)?;
        let realm = world.realm(RealmId::Zone(zone))?;
        let idx = realm.idx_of(resident)?;
        Some(LocalScene::at(map, realm, idx, scratch))
    }

    /// Makes the scene of the resident at `idx` of a zone realm.
    pub(crate) fn at(
        map: &'a WorldMap,
        realm: &'a Realm,
        idx: Idx,
        scratch: &'a mut Scratch,
    ) -> LocalScene<'a> {
        let map = realm.zone_info().and_then(|zone| map.city(zone.city));
        LocalScene { realm, idx, me: realm.id_of(idx), pos: realm.pos(idx), map, scratch }
    }
}

impl Scene for LocalScene<'_> {
    fn me(&self) -> ResidentId {
        self.me
    }

    fn pos(&self) -> Pos {
        self.pos
    }

    fn map(&self) -> Option<&CityMap> {
        self.map
    }

    fn site_kind(&mut self) -> Option<&str> {
        // The zone knows the site as it is now: a business can change its
        // kind after the map was read.
        let realm = self.realm;
        let sites = realm.sites();
        let site = sites.at(self.pos)?;
        self.scratch.site_kind.clear();
        self.scratch.site_kind.push_str(&site.kind);
        Some(&self.scratch.site_kind)
    }

    fn cash(&self) -> Option<i64> {
        Some(self.realm.ledger().money(Holder::Resident(self.me)).cents())
    }

    fn carried(&mut self) -> &[Carried] {
        self.scratch.carried.clear();
        for (item, qty) in self.realm.ledger().items_of(Holder::Resident(self.me)) {
            if qty > 0 {
                self.scratch.carried.push(Carried { item, qty });
            }
        }
        &self.scratch.carried
    }

    fn attr(&self, name: &str) -> Option<i64> {
        let attr = self.realm.defs().attr_id(name)?;
        Some(self.realm.attr(self.idx, attr))
    }

    fn residents(&mut self, radius: u32) -> &[Neighbour] {
        // The kernel returns the residents of this zone in id order.
        self.realm.nearby_into(self.pos, radius, &mut self.scratch.indices);
        self.scratch.residents.clear();
        for &idx in &self.scratch.indices {
            if idx != self.idx {
                self.scratch
                    .residents
                    .push(Neighbour { id: self.realm.id_of(idx), pos: self.realm.pos(idx) });
            }
        }
        &self.scratch.residents
    }

    fn sites(&mut self, radius: u32) -> &[NearSite] {
        let realm = self.realm;
        let sites = realm.sites();
        self.scratch.sites.clear();
        for site in sites.within(self.pos, radius) {
            self.scratch.sites.push(NearSite {
                id: site.id,
                kind: site.kind.clone(),
                pos: site.rect.nearest(self.pos),
            });
        }
        &self.scratch.sites
    }
}
