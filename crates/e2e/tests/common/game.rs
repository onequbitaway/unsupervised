//! What an outside party can read about the game: the catalogue and the
//! map. The tests take prices, wages and times from here, so that they
//! stay true when the numbers of the game change.

use serde_json::Value;

use super::{World, id, int};

/// The public catalogue: verbs, parameters, items.
pub struct Catalogue(pub Value);

impl Catalogue {
    /// Reads `GET /v1/catalogue`.
    pub async fn read(world: &World) -> Catalogue {
        Catalogue(world.public("/v1/catalogue").await)
    }

    /// Returns the value of a parameter.
    ///
    /// # Panics
    ///
    /// Panics when the catalogue has no parameter with this name.
    pub fn param(&self, name: &str) -> i64 {
        let params = self.0["params"].as_array().expect("the catalogue has parameters");
        let param = params
            .iter()
            .find(|param| param["name"] == name)
            .unwrap_or_else(|| panic!("the catalogue has the parameter `{name}`"));
        int(&param["value"], name)
    }

    /// Returns the catalogue entry of a verb, when the game has the verb.
    pub fn verb(&self, name: &str) -> Option<&Value> {
        self.0["verbs"]
            .as_array()
            .expect("the catalogue has verbs")
            .iter()
            .find(|v| v["verb"] == name)
    }

    /// Returns the names of all verbs.
    pub fn verbs(&self) -> Vec<&str> {
        let verbs = self.0["verbs"].as_array().expect("the catalogue has verbs");
        verbs.iter().map(|verb| verb["verb"].as_str().expect("a verb name")).collect()
    }

    /// Returns the nutrition of an item, when the item is food: it has
    /// the tag `food` and the attribute `nutrition`.
    pub fn nutrition(&self, item: &str) -> Option<i64> {
        let items = self.0["items"].as_array().expect("the catalogue has items");
        let item = items.iter().find(|entry| entry["name"] == item)?;
        let food = item["tags"].as_array()?.iter().any(|tag| tag == "food");
        item["attrs"]["nutrition"].as_i64().filter(|_| food)
    }

    /// Returns the default of an attribute: the value of a new resident.
    pub fn attr_default(&self, name: &str) -> i64 {
        let attrs = self.0["attrs"].as_array().expect("the catalogue has attributes");
        let attr = attrs
            .iter()
            .find(|attr| attr["name"] == name)
            .unwrap_or_else(|| panic!("the catalogue has the attribute `{name}`"));
        int(&attr["default"], name)
    }
}

/// One site of the public map.
#[derive(Clone, Debug)]
pub struct Site {
    /// The id of the site.
    pub id: u64,
    /// The kind, for example `depot`.
    pub kind: String,
    /// The name, for example `Depot`.
    pub name: String,
    /// The zone of the site.
    pub zone: u64,
    /// The middle of the rectangle of the site.
    pub middle: (i64, i64),
}

/// The sites of one city, from the public view of its zones.
pub struct Map {
    /// Every site of the city.
    pub sites: Vec<Site>,
}

impl Map {
    /// Reads the map of a city: `GET /v1/world`, then
    /// `GET /v1/world/zones/{id}` for each zone of the city.
    pub async fn read(world: &World, city: u64) -> Map {
        let layout = world.public("/v1/world").await;
        assert_eq!(layout["type"], "layout");
        let cities = layout["cities"].as_array().expect("the layout has cities");
        let entry = cities
            .iter()
            .find(|entry| entry["id"] == city)
            .unwrap_or_else(|| panic!("the layout has the city {city}"));
        let mut sites = Vec::new();
        for zone in entry["zones"].as_array().expect("a city has zones") {
            let zone = id(zone, "a zone id");
            let view = world.public(&format!("/v1/world/zones/{zone}?residents=false")).await;
            assert_eq!(view["type"], "zone");
            for site in view["sites"].as_array().expect("a zone view has sites") {
                let rect = &site["rect"];
                let middle =
                    |low: &str, high: &str| (int(&rect[low], low) + int(&rect[high], high)) / 2;
                sites.push(Site {
                    id: id(&site["id"], "a site id"),
                    kind: site["kind"].as_str().expect("a site kind").to_string(),
                    name: site["name"].as_str().expect("a site name").to_string(),
                    zone,
                    middle: (middle("x0", "x1"), middle("y0", "y1")),
                });
            }
        }
        Map { sites }
    }

    /// Returns the site of one of the kinds that is nearest to a place.
    ///
    /// # Panics
    ///
    /// Panics when the city has no site of these kinds.
    pub fn nearest(&self, kinds: &[&str], from: (i64, i64)) -> &Site {
        let distance = |site: &&Site| {
            let (dx, dy) = (site.middle.0 - from.0, site.middle.1 - from.1);
            (dx.abs().max(dy.abs()), site.id)
        };
        self.sites
            .iter()
            .filter(|site| kinds.contains(&site.kind.as_str()))
            .min_by_key(distance)
            .unwrap_or_else(|| panic!("the city has a site of the kinds {kinds:?}"))
    }
}

/// Reads the position in the section `self` of an observation.
pub fn pos(myself: &Value) -> (i64, i64) {
    (int(&myself["pos"]["x"], "x"), int(&myself["pos"]["y"], "y"))
}

/// Reads an attribute in the section `self` of an observation.
pub fn attr(myself: &Value, name: &str) -> i64 {
    int(&myself["attrs"][name], name)
}

/// Reads how many units of an item the section `self` shows.
pub fn carried(myself: &Value, item: &str) -> i64 {
    myself["items"].get(item).map_or(0, |qty| int(qty, item))
}
