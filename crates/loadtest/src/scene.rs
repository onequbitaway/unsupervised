//! What a resident knows when it chooses an action.
//!
//! An argument generator of a behaviour profile asks the scene for facts:
//! where the resident is, who is near, which sites are near. The commands
//! that run in one process read the facts from the world. The swarm reads
//! them from the last observation that the server sent.

use unsup_protocol::{Pos, Rect, ResidentId, SiteId};

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

/// The facts that argument generators can ask for.
pub trait Scene {
    /// Returns the resident that acts.
    fn me(&self) -> ResidentId;

    /// Returns the position of the resident.
    fn pos(&self) -> Pos;

    /// Returns the tiles in which a generated position must stay: the city
    /// of the resident. `None` when the scene does not know them.
    fn bounds(&self) -> Option<Rect>;

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
    /// The tiles of its city, when they are known.
    pub bounds: Option<Rect>,
    /// Every other resident that it can see.
    pub residents: Vec<Neighbour>,
    /// Every site that it can see.
    pub sites: Vec<NearSite>,
    near_residents: Vec<Neighbour>,
    near_sites: Vec<NearSite>,
}

impl ListScene {
    /// Makes a scene with no resident and no site in sight.
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

    fn bounds(&self) -> Option<Rect> {
        self.bounds
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
