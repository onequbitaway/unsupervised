//! What a resident knows when it chooses an action.
//!
//! An argument generator of a behaviour profile asks the scene for facts:
//! where the resident is, what it carries, who is near, which sites are
//! near, and which tiles of its city are walkable. The commands that run
//! in one process read the facts from the world. The swarm reads them from
//! the last observation that the server sent.

use std::collections::BTreeMap;
use std::sync::Arc;

use unsup_protocol::{CityId, ItemId, Pos, Rect, ResidentId, SiteId};

use crate::map::{CityMap, WorldMap};

/// A resident near the one that acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Neighbour {
    /// Id of the resident.
    pub id: ResidentId,
    /// Where it is.
    pub pos: Pos,
}

/// A site near the resident that acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NearSite {
    /// Id of the site.
    pub id: SiteId,
    /// Kind of the site, for example `shop`.
    pub kind: String,
    /// The tile of the site that is nearest to the resident.
    pub pos: Pos,
}

/// Goods that the resident carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Carried {
    /// The item.
    pub item: ItemId,
    /// Number of units. It is greater than zero.
    pub qty: i64,
}

/// The facts that argument generators and conditions can ask for.
pub trait Scene {
    /// Returns the resident that acts.
    fn me(&self) -> ResidentId;

    /// Returns the position of the resident.
    fn pos(&self) -> Pos;

    /// Returns the map of the city of the resident: its tiles and its
    /// sites. `None` when the scene does not know them.
    fn map(&self) -> Option<&CityMap>;

    /// Returns the tiles in which a generated position must stay: the city
    /// of the resident. `None` when the scene does not know them.
    fn bounds(&self) -> Option<Rect> {
        self.map().map(CityMap::bounds)
    }

    /// Returns the kind of the site where the resident stands.
    fn site_kind(&mut self) -> Option<&str>;

    /// Returns the cash of the resident in cents. `None` when the scene
    /// does not know it.
    fn cash(&self) -> Option<i64>;

    /// Returns the goods that the resident carries.
    fn carried(&mut self) -> &[Carried];

    /// Returns the value of an attribute of the resident, for example
    /// `need.food`. `None` when the scene does not know the attribute.
    fn attr(&self, name: &str) -> Option<i64>;

    /// Returns the other residents within `radius` tiles, in ascending id
    /// order. The distance is `Pos::dist`: a diagonal step counts as one.
    fn residents(&mut self, radius: u32) -> &[Neighbour];

    /// Returns the sites within `radius` tiles.
    fn sites(&mut self, radius: u32) -> &[NearSite];
}

/// A scene that holds its facts in lists. The swarm fills it from an
/// observation; tests fill it by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListScene {
    /// The resident that acts.
    pub me: ResidentId,
    /// Where it is.
    pub pos: Pos,
    /// The tiles of its city, when they are known and the map is not.
    pub bounds: Option<Rect>,
    /// The map of the world, when it is known.
    pub map: Option<Arc<WorldMap>>,
    /// The city of the resident. With `map`, it gives the map of the city.
    pub city: Option<CityId>,
    /// Kind of the site where it stands.
    pub site_kind: Option<String>,
    /// Its cash in cents, when it is known.
    pub cash: Option<i64>,
    /// The goods that it carries.
    pub carried: Vec<Carried>,
    /// Its attributes, by name.
    pub attrs: BTreeMap<String, i64>,
    /// Every other resident that it can see.
    pub residents: Vec<Neighbour>,
    /// Every site that it can see.
    pub sites: Vec<NearSite>,
    near_residents: Vec<Neighbour>,
    near_sites: Vec<NearSite>,
}

impl ListScene {
    /// Makes a scene with no resident and no site in sight, for a resident
    /// that carries nothing.
    #[must_use]
    pub fn new(me: ResidentId, pos: Pos, bounds: Option<Rect>) -> ListScene {
        ListScene { me, pos, bounds, ..ListScene::default() }
    }
}

impl Scene for ListScene {
    fn me(&self) -> ResidentId {
        self.me
    }

    fn pos(&self) -> Pos {
        self.pos
    }

    fn map(&self) -> Option<&CityMap> {
        self.map.as_ref()?.city(self.city?)
    }

    fn bounds(&self) -> Option<Rect> {
        self.map().map(CityMap::bounds).or(self.bounds)
    }

    fn site_kind(&mut self) -> Option<&str> {
        self.site_kind.as_deref()
    }

    fn cash(&self) -> Option<i64> {
        self.cash
    }

    fn carried(&mut self) -> &[Carried] {
        &self.carried
    }

    fn attr(&self, name: &str) -> Option<i64> {
        self.attrs.get(name).copied()
    }

    fn residents(&mut self, radius: u32) -> &[Neighbour] {
        let (me, pos) = (self.me, self.pos);
        self.near_residents.clear();
        self.near_residents.extend(
            self.residents.iter().filter(|n| n.id != me && n.pos.dist(pos) <= radius).copied(),
        );
        self.near_residents.sort_unstable_by_key(|n| n.id);
        &self.near_residents
    }

    fn sites(&mut self, radius: u32) -> &[NearSite] {
        let pos = self.pos;
        self.near_sites.clear();
        self.near_sites.extend(self.sites.iter().filter(|s| s.pos.dist(pos) <= radius).cloned());
        &self.near_sites
    }
}
