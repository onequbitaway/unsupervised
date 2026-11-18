//! Places: the sites of the map, and how a resident gets to them.

use serde_json::{Value, json};
use unsup_kernel::protocol::{ViewQuery, WorldView};
use unsup_kernel::{CityId, Pos, ResidentId, Site, SiteId, ZoneId};

use crate::game::Game;

/// The tile codes that a resident cannot enter: wall, water and rock
/// (`docs/design/game-design.md`, section 2).
const BLOCKED_TILES: [u8; 3] = [4, 5, 8];

impl Game {
    /// Returns the sites of a kind in all cities, by zone and then by id.
    #[must_use]
    pub fn sites(&self, kind: &str) -> Vec<Site> {
        let mut found = Vec::new();
        for zone in self.world.world.layout().zones() {
            found.extend(self.world.zone(zone.id).sites().of_kind(kind).cloned());
        }
        found
    }

    /// Returns the sites of a kind in one city, by zone and then by id.
    #[must_use]
    pub fn sites_in(&self, kind: &str, city: CityId) -> Vec<Site> {
        let mut found = Vec::new();
        for zone in self.world.world.layout().zones().iter().filter(|zone| zone.city == city) {
            found.extend(self.world.zone(zone.id).sites().of_kind(kind).cloned());
        }
        found
    }

    /// Returns the first site of a kind in the first city.
    ///
    /// # Panics
    ///
    /// Panics when the city has no such site.
    #[must_use]
    pub fn site(&self, kind: &str) -> Site {
        match self.sites_in(kind, CityId(0)).into_iter().next() {
            Some(site) => site,
            None => panic!("the first city has no {kind}"),
        }
    }

    /// Finds the sites of a kind on the public map of a city, as an agent
    /// does it: the view of each zone lists its sites. Returns their ids,
    /// by zone and then by id.
    ///
    /// # Panics
    ///
    /// Panics when the world refuses a view.
    #[must_use]
    pub fn on_map(&self, kind: &str, city: CityId) -> Vec<SiteId> {
        let mut found = Vec::new();
        for zone in self.world.world.layout().zones().iter().filter(|zone| zone.city == city) {
            match self.world.world.view(&ViewQuery::Zone { zone: zone.id }) {
                Ok(WorldView::Zone { sites, .. }) => {
                    found.extend(sites.iter().filter(|site| site.kind == kind).map(|site| site.id));
                }
                Ok(other) => panic!("zone {} gave no zone view: {other:?}", zone.id),
                Err(e) => panic!("zone {} has no view: {e}", zone.id),
            }
        }
        found
    }

    /// Returns the zone of a site.
    ///
    /// # Panics
    ///
    /// Panics when no zone has the site.
    #[must_use]
    pub fn zone_of_site(&self, id: SiteId) -> ZoneId {
        for zone in self.world.world.layout().zones() {
            if self.world.zone(zone.id).sites().get(id).is_some() {
                return zone.id;
            }
        }
        panic!("no zone has site {id}");
    }

    /// Returns a site as it is now.
    #[must_use]
    pub fn site_now(&self, id: SiteId) -> Site {
        let zone = self.zone_of_site(id);
        match self.world.zone(zone).sites().get(id) {
            Some(site) => site.clone(),
            None => panic!("zone {zone} lost site {id}"),
        }
    }

    /// Returns the site where a resident stands.
    #[must_use]
    pub fn site_of(&self, r: ResidentId) -> Option<Site> {
        self.world.with_resident(r, |realm, idx| realm.sites().at(realm.pos(idx)).cloned())
    }

    /// Returns the tile inside a site that a resident can stand on and
    /// that is nearest to the middle of the site.
    ///
    /// # Panics
    ///
    /// Panics when the site has no such tile.
    #[must_use]
    pub fn floor_of(&self, site: &Site) -> Pos {
        let zone = self.zone_of_site(site.id);
        let tiles = self.world.zone(zone).tiles();
        let centre = site.rect.centre();
        let mut best: Option<(u32, Pos)> = None;
        for y in site.rect.y0..site.rect.y1 {
            for x in site.rect.x0..site.rect.x1 {
                let pos = Pos::new(x, y);
                let walkable = tiles.get(pos).is_some_and(|code| !BLOCKED_TILES.contains(&code));
                let dist = pos.dist(centre);
                if walkable && best.is_none_or(|(d, _)| dist < d) {
                    best = Some((dist, pos));
                }
            }
        }
        match best {
            Some((_, pos)) => pos,
            None => panic!("site {} has no tile to stand on", site.id),
        }
    }

    /// Puts a resident at a position without a verb. It is for the setup
    /// of a story: a step of the game uses [`Game::walk`].
    pub fn place(&mut self, r: ResidentId, pos: Pos) {
        self.world.set_pos(r, pos);
    }

    /// Puts a resident inside a site without a verb. It is for the setup
    /// of a story.
    pub fn place_in(&mut self, r: ResidentId, site: &Site) {
        let pos = self.floor_of(site);
        self.place(r, pos);
    }

    /// Walks a resident to a site with the verb `move_to` and returns the
    /// result of the walk.
    ///
    /// # Panics
    ///
    /// Panics when the walk fails or does not end inside the site.
    pub fn walk(&mut self, r: ResidentId, site: &Site) -> Value {
        let result = self.act(r, "move_to", json!({ "site": site.id }));
        let here = self.pos(r);
        assert!(site.rect.contains(here), "the walk to site {} ended at {here}", site.id);
        result
    }

    /// Walks several residents to one site at the same time, each with the
    /// verb `move_to`.
    pub fn walk_all(&mut self, residents: &[ResidentId], site: &Site) {
        self.each(residents, "move_to", json!({ "site": site.id }));
        for &r in residents {
            let here = self.pos(r);
            assert!(
                site.rect.contains(here),
                "the walk of {r} to site {} ended at {here}",
                site.id
            );
        }
    }

    /// Walks a resident to a tile with the verb `move_to`.
    pub fn walk_to(&mut self, r: ResidentId, pos: Pos) -> Value {
        let result = self.act(r, "move_to", json!({ "x": pos.x, "y": pos.y }));
        assert_eq!(self.pos(r), pos, "the walk ended at another tile");
        result
    }
}
