//! Sites: buildings and lots in a zone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_protocol::{Pos, Rect, ResidentId, SiteId, SiteView};

use crate::ledger::Holder;

/// A building or a lot in a zone.
///
/// A site holds money and goods through `Holder::Site(id)` in the ledger of
/// its zone. Make the id with `SiteId(realm.alloc_id())`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    /// Id of the site.
    pub id: SiteId,
    /// Kind of the site, for example `shop` or `home`.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Tiles that the site covers.
    pub rect: Rect,
    /// Who owns the site, if anyone.
    pub owner: Option<Holder>,
    /// Residents that rent or live in the site.
    pub tenants: Vec<ResidentId>,
    /// Tags. The kernel knows one tag: a site with the tag `spawn` is a
    /// place where new residents of the city appear.
    pub tags: Vec<String>,
    /// Named numbers, for example `rent` or `capacity`.
    pub attrs: BTreeMap<String, i64>,
}

impl Site {
    /// The tag of a site where new residents appear.
    pub const SPAWN_TAG: &'static str = "spawn";

    /// Makes a site without an owner, tenants, tags or attributes.
    #[must_use]
    pub fn new(id: SiteId, kind: impl Into<String>, name: impl Into<String>, rect: Rect) -> Site {
        Site {
            id,
            kind: kind.into(),
            name: name.into(),
            rect,
            owner: None,
            tenants: Vec::new(),
            tags: Vec::new(),
            attrs: BTreeMap::new(),
        }
    }

    /// Adds a tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Site {
        self.add_tag(tag);
        self
    }

    /// Sets the owner.
    #[must_use]
    pub fn with_owner(mut self, owner: Holder) -> Site {
        self.owner = Some(owner);
        self
    }

    /// Sets a named number.
    #[must_use]
    pub fn with_attr(mut self, name: impl Into<String>, value: i64) -> Site {
        self.attrs.insert(name.into(), value);
        self
    }

    /// Returns true when the site has the tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Adds a tag unless the site has it.
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !self.has_tag(&tag) {
            self.tags.push(tag);
        }
    }

    /// Returns a named number, or 0 when the site does not have it.
    #[must_use]
    pub fn attr(&self, name: &str) -> i64 {
        self.attrs.get(name).copied().unwrap_or(0)
    }

    /// Sets a named number.
    pub fn set_attr(&mut self, name: impl Into<String>, value: i64) {
        self.attrs.insert(name.into(), value);
    }

    /// Returns true when the resident is a tenant.
    #[must_use]
    pub fn is_tenant(&self, resident: ResidentId) -> bool {
        self.tenants.contains(&resident)
    }

    /// Adds a tenant unless the resident is one already.
    pub fn add_tenant(&mut self, resident: ResidentId) {
        if !self.is_tenant(resident) {
            self.tenants.push(resident);
        }
    }

    /// Removes a tenant. Returns true when the resident was one.
    pub fn remove_tenant(&mut self, resident: ResidentId) -> bool {
        let before = self.tenants.len();
        self.tenants.retain(|&t| t != resident);
        self.tenants.len() != before
    }

    /// Returns the public view of the site.
    #[must_use]
    pub fn view(&self) -> SiteView {
        SiteView {
            id: self.id,
            kind: self.kind.clone(),
            name: self.name.clone(),
            rect: self.rect,
            tags: self.tags.clone(),
        }
    }
}

/// The sites of one zone, ordered by id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sites {
    sites: BTreeMap<SiteId, Site>,
}

impl Sites {
    /// Adds a site, or replaces the site with the same id. Returns the id.
    pub fn insert(&mut self, site: Site) -> SiteId {
        let id = site.id;
        self.sites.insert(id, site);
        id
    }

    /// Removes a site. Its ledger balance, if any, stays in the ledger; move
    /// it first.
    pub fn remove(&mut self, id: SiteId) -> Option<Site> {
        self.sites.remove(&id)
    }

    /// Returns one site.
    #[must_use]
    pub fn get(&self, id: SiteId) -> Option<&Site> {
        self.sites.get(&id)
    }

    /// Returns one site for a change.
    pub fn get_mut(&mut self, id: SiteId) -> Option<&mut Site> {
        self.sites.get_mut(&id)
    }

    /// Returns the number of sites.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sites.len()
    }

    /// Returns true when the zone has no site.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// Returns every site, ordered by id.
    pub fn iter(&self) -> impl Iterator<Item = &Site> + '_ {
        self.sites.values()
    }

    /// Returns every site for a change, ordered by id.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Site> + '_ {
        self.sites.values_mut()
    }

    /// Returns the site that covers `pos`. When sites overlap, the one with
    /// the lowest id wins.
    #[must_use]
    pub fn at(&self, pos: Pos) -> Option<&Site> {
        self.sites.values().find(|s| s.rect.contains(pos))
    }

    /// Returns the sites with a tag, ordered by id.
    pub fn with_tag<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Site> + 'a {
        self.sites.values().filter(move |s| s.has_tag(tag))
    }

    /// Returns the sites of a kind, ordered by id.
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Site> + 'a {
        self.sites.values().filter(move |s| s.kind == kind)
    }

    /// Returns the sites of an owner, ordered by id.
    pub fn owned_by(&self, owner: Holder) -> impl Iterator<Item = &Site> + '_ {
        self.sites.values().filter(move |s| s.owner == Some(owner))
    }

    /// Returns the sites whose nearest tile is at most `radius` tiles from
    /// `pos`, ordered by distance and then by id.
    #[must_use]
    pub fn within(&self, pos: Pos, radius: u32) -> Vec<&Site> {
        let mut found: Vec<(u32, &Site)> = self
            .sites
            .values()
            .filter_map(|s| {
                let d = s.rect.dist_to(pos);
                (d <= radius).then_some((d, s))
            })
            .collect();
        found.sort_by_key(|(d, s)| (*d, s.id));
        found.into_iter().map(|(_, s)| s).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sites {
        let mut sites = Sites::default();
        sites.insert(
            Site::new(SiteId(5), "shop", "Bakery", Rect::sized(10, 10, 4, 4)).with_tag("food"),
        );
        sites.insert(
            Site::new(SiteId(2), "home", "Flat 1", Rect::sized(30, 10, 2, 2))
                .with_owner(Holder::Resident(ResidentId(7)))
                .with_attr("rent", 500),
        );
        sites.insert(
            Site::new(SiteId(9), "plaza", "Square", Rect::sized(8, 8, 10, 10)).with_tag("spawn"),
        );
        sites
    }

    #[test]
    fn lookup_by_position_tag_kind_and_owner() {
        let sites = sample();
        assert_eq!(sites.len(), 3);
        assert!(!sites.is_empty());
        // The bakery and the square overlap; the lower id wins.
        assert_eq!(sites.at(Pos::new(11, 11)).map(|s| s.id), Some(SiteId(5)));
        assert_eq!(sites.at(Pos::new(8, 8)).map(|s| s.id), Some(SiteId(9)));
        assert_eq!(sites.at(Pos::new(0, 0)).map(|s| s.id), None);
        assert_eq!(sites.with_tag(Site::SPAWN_TAG).map(|s| s.id).collect::<Vec<_>>(), [SiteId(9)]);
        assert_eq!(sites.of_kind("home").map(|s| s.id).collect::<Vec<_>>(), [SiteId(2)]);
        assert_eq!(
            sites.owned_by(Holder::Resident(ResidentId(7))).map(|s| s.id).collect::<Vec<_>>(),
            [SiteId(2)]
        );
        assert_eq!(
            sites.iter().map(|s| s.id).collect::<Vec<_>>(),
            [SiteId(2), SiteId(5), SiteId(9)]
        );
    }

    #[test]
    fn within_orders_by_distance_then_id() {
        let sites = sample();
        let near: Vec<_> = sites.within(Pos::new(12, 12), 20).iter().map(|s| s.id).collect();
        assert_eq!(near, [SiteId(5), SiteId(9), SiteId(2)]);
        let near: Vec<_> = sites.within(Pos::new(12, 12), 17).iter().map(|s| s.id).collect();
        assert_eq!(near, [SiteId(5), SiteId(9)]);
        assert!(sites.within(Pos::new(200, 200), 5).is_empty());
    }

    #[test]
    fn tenants_tags_and_attributes() {
        let mut sites = sample();
        let flat = sites.get_mut(SiteId(2)).unwrap();
        flat.add_tenant(ResidentId(3));
        flat.add_tenant(ResidentId(3));
        assert_eq!(flat.tenants, [ResidentId(3)]);
        assert!(flat.is_tenant(ResidentId(3)));
        assert!(flat.remove_tenant(ResidentId(3)));
        assert!(!flat.remove_tenant(ResidentId(3)));
        flat.add_tag("quiet");
        flat.add_tag("quiet");
        assert_eq!(flat.tags, ["quiet"]);
        assert_eq!((flat.attr("rent"), flat.attr("none")), (500, 0));
        flat.set_attr("rent", 650);
        assert_eq!(sites.get(SiteId(2)).unwrap().attr("rent"), 650);
        for site in sites.iter_mut() {
            site.set_attr("seen", 1);
        }
        assert_eq!(sites.get(SiteId(9)).unwrap().attr("seen"), 1);
        let view = sites.get(SiteId(5)).unwrap().view();
        assert_eq!((view.id, view.kind.as_str(), view.tags.len()), (SiteId(5), "shop", 1));
        assert_eq!(sites.remove(SiteId(5)).map(|s| s.name), Some("Bakery".to_string()));
        assert_eq!(sites.get(SiteId(5)), None);
    }
}
