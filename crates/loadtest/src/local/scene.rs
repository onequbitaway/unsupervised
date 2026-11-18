//! The scene of a resident, read from a world in this process.

use unsup_kernel::{Idx, Realm, World};
use unsup_protocol::{Pos, RealmId, Rect, ResidentId};

use crate::scene::{NearSite, Neighbour, Scene};

/// Buffers that many scenes share one after the other, so that a scene does
/// not allocate for each resident.
#[derive(Debug, Default)]
pub struct Scratch {
    indices: Vec<Idx>,
    residents: Vec<Neighbour>,
    sites: Vec<NearSite>,
}

/// What one resident of a world in this process knows. The lists of
/// neighbours and sites are read from the zone only when a generator asks.
pub struct LocalScene<'a> {
    realm: &'a Realm,
    idx: Idx,
    me: ResidentId,
    pos: Pos,
    bounds: Option<Rect>,
    scratch: &'a mut Scratch,
}

impl<'a> LocalScene<'a> {
    /// Makes the scene of a resident. Returns `None` when the resident is
    /// not in the world.
    pub fn of(
        world: &'a World,
        resident: ResidentId,
        scratch: &'a mut Scratch,
    ) -> Option<LocalScene<'a>> {
        let zone = world.locate(resident)?;
        let realm = world.realm(RealmId::Zone(zone))?;
        let idx = realm.idx_of(resident)?;
        Some(LocalScene::at(world, realm, idx, scratch))
    }

    /// Makes the scene of the resident at `idx` of a zone realm of `world`.
    pub(crate) fn at(
        world: &World,
        realm: &'a Realm,
        idx: Idx,
        scratch: &'a mut Scratch,
    ) -> LocalScene<'a> {
        let layout = world.layout();
        let bounds =
            realm.zone_info().and_then(|zone| layout.city(zone.city)).map(|city| city.bounds);
        LocalScene { realm, idx, me: realm.id_of(idx), pos: realm.pos(idx), bounds, scratch }
    }
}

impl Scene for LocalScene<'_> {
    fn me(&self) -> ResidentId {
        self.me
    }

    fn pos(&self) -> Pos {
        self.pos
    }

    fn bounds(&self) -> Option<Rect> {
        self.bounds
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
        let sites = self.realm.sites();
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
