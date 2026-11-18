//! Argument generators: how a profile makes the arguments of an action.
//!
//! In a profile, `args` is a TOML value. A table with the key `gen` is a
//! generator. Every other value is taken as it is written, and generators
//! can stand anywhere inside tables and lists.
//!
//! ```toml
//! args = { x = { gen = "near_x", radius = 12 }, y = { gen = "near_y", radius = 12 } }
//! args = { gen = "near_tile", radius = 12 }          # the whole object is { x, y }
//! args = { to = { gen = "nearby_resident", radius = 8 }, text = { gen = "text", words = 5 } }
//! ```

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use unsup_kernel::Rng;
use unsup_protocol::{ItemInfo, Pos};

use crate::random;
use crate::scene::Scene;

/// The key that marks a table as a generator.
const GEN_KEY: &str = "gen";

/// Words for generated text. They mean nothing; the text is only load.
const WORDS: [&str; 16] = [
    "hello", "market", "today", "bread", "work", "river", "price", "friend", "street", "news",
    "morning", "trade", "bridge", "garden", "vote", "story",
];

/// How a generator names an item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemAs {
    /// The name of the item, for example `bread`.
    #[default]
    Name,
    /// The numeric id of the item.
    Id,
}

/// One argument generator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "gen", rename_all = "snake_case", deny_unknown_fields)]
pub enum Generator {
    /// A uniform integer from `min` to `max`, both included.
    Int {
        /// Least value.
        min: i64,
        /// Greatest value.
        max: i64,
    },
    /// One of the listed values, each with the same chance.
    Choice {
        /// The values.
        of: Vec<Value>,
    },
    /// A sentence of `words` random words.
    Text {
        /// Number of words.
        words: u32,
    },
    /// The id of the resident that acts.
    #[serde(rename = "self")]
    Me,
    /// A random tile within `radius` of the resident, as `{ x, y }`. The
    /// tile stays inside the city of the resident.
    NearTile {
        /// Greatest distance in tiles on each axis.
        radius: u32,
    },
    /// The column of a random tile within `radius` of the resident.
    NearX {
        /// Greatest distance in tiles.
        radius: u32,
    },
    /// The row of a random tile within `radius` of the resident.
    NearY {
        /// Greatest distance in tiles.
        radius: u32,
    },
    /// The id of a random other resident within `radius` tiles. The action
    /// is not possible when nobody is there.
    NearbyResident {
        /// Greatest distance in tiles.
        radius: u32,
    },
    /// The id of a random site within `radius` tiles. The action is not
    /// possible when no such site is there.
    NearbySite {
        /// Greatest distance in tiles.
        radius: u32,
        /// Only sites of this kind, for example `shop`.
        #[serde(default)]
        kind: Option<String>,
    },
    /// The nearest tile of a random site within `radius` tiles, as
    /// `{ x, y }`. Use it to walk to a site.
    SiteTile {
        /// Greatest distance in tiles.
        radius: u32,
        /// Only sites of this kind.
        #[serde(default)]
        kind: Option<String>,
    },
    /// A random item of the catalogue. The action is not possible when the
    /// catalogue has no such item.
    Item {
        /// Only items with this tag, for example `food`.
        #[serde(default)]
        tag: Option<String>,
        /// Whether to give the name or the id.
        #[serde(default, rename = "as")]
        form: ItemAs,
    },
}

/// The arguments of an action as the profile wrote them: fixed values with
/// generators inside.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum ArgSpec {
    /// No arguments. The request carries none.
    #[default]
    Absent,
    /// A fixed value.
    Literal(Value),
    /// A generator.
    Gen(Generator),
    /// A table. Each value is made on its own.
    Object(Vec<(String, ArgSpec)>),
    /// A list. Each element is made on its own.
    List(Vec<ArgSpec>),
}

impl ArgSpec {
    /// Reads a specification from a parsed TOML or JSON value.
    pub fn from_value(value: Value) -> Result<ArgSpec, String> {
        match value {
            Value::Object(map) if map.contains_key(GEN_KEY) => {
                let name = map.get(GEN_KEY).cloned().unwrap_or(Value::Null);
                serde_json::from_value::<Generator>(Value::Object(map))
                    .map(ArgSpec::Gen)
                    .map_err(|error| format!("generator {name}: {error}"))
            }
            Value::Object(map) => {
                let mut fields = Vec::with_capacity(map.len());
                for (key, value) in map {
                    let spec =
                        ArgSpec::from_value(value).map_err(|error| format!("`{key}`: {error}"))?;
                    fields.push((key, spec));
                }
                Ok(ArgSpec::Object(fields).folded())
            }
            Value::Array(list) => {
                let specs = list.into_iter().map(ArgSpec::from_value).collect::<Result<_, _>>()?;
                Ok(ArgSpec::List(specs).folded())
            }
            other => Ok(ArgSpec::Literal(other)),
        }
    }

    /// Turns a table or a list without a generator into one fixed value.
    fn folded(self) -> ArgSpec {
        match self.fixed() {
            Some(value) => ArgSpec::Literal(value),
            None => self,
        }
    }

    /// Returns the value when the specification has no generator.
    fn fixed(&self) -> Option<Value> {
        match self {
            ArgSpec::Absent | ArgSpec::Gen(_) => None,
            ArgSpec::Literal(value) => Some(value.clone()),
            ArgSpec::Object(fields) => fields
                .iter()
                .map(|(key, spec)| spec.fixed().map(|value| (key.clone(), value)))
                .collect::<Option<Map<String, Value>>>()
                .map(Value::Object),
            ArgSpec::List(specs) => {
                specs.iter().map(ArgSpec::fixed).collect::<Option<Vec<Value>>>().map(Value::Array)
            }
        }
    }

    /// Returns the names of the top-level arguments, or `None` when a
    /// generator makes the whole object.
    #[must_use]
    pub fn keys(&self) -> Option<Vec<&str>> {
        match self {
            ArgSpec::Absent => Some(Vec::new()),
            ArgSpec::Object(fields) => Some(fields.iter().map(|(key, _)| key.as_str()).collect()),
            ArgSpec::Literal(Value::Object(map)) => Some(map.keys().map(String::as_str).collect()),
            _ => None,
        }
    }

    /// Makes the arguments of one action. Returns `None` when a generator
    /// cannot make its value now, for example when no resident is near.
    /// Absent arguments give `Value::Null`.
    pub fn generate(
        &self,
        rng: &mut Rng,
        scene: &mut dyn Scene,
        items: &[ItemInfo],
    ) -> Option<Value> {
        match self {
            ArgSpec::Absent => Some(Value::Null),
            ArgSpec::Literal(value) => Some(value.clone()),
            ArgSpec::Gen(generator) => generator.generate(rng, scene, items),
            ArgSpec::Object(fields) => {
                let mut map = Map::with_capacity(fields.len());
                for (key, spec) in fields {
                    map.insert(key.clone(), spec.generate(rng, scene, items)?);
                }
                Some(Value::Object(map))
            }
            ArgSpec::List(specs) => specs
                .iter()
                .map(|spec| spec.generate(rng, scene, items))
                .collect::<Option<Vec<Value>>>()
                .map(Value::Array),
        }
    }
}

impl<'de> Deserialize<'de> for ArgSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ArgSpec, D::Error> {
        let value = Value::deserialize(deserializer)?;
        ArgSpec::from_value(value).map_err(serde::de::Error::custom)
    }
}

/// Returns a uniform coordinate within `radius` of `at` that stays inside
/// `lo..hi` when the range is known. The range is cut first, so that a
/// resident at the edge of its city does not pile up on the edge.
fn near(rng: &mut Rng, at: i32, radius: u32, range: Option<(i32, i32)>) -> i32 {
    let radius = i64::from(radius);
    let mut lo = i64::from(at) - radius;
    let mut hi = i64::from(at) + radius;
    if let Some((first, past)) = range {
        let (first, last) = (i64::from(first), i64::from(past) - 1);
        if first <= last {
            lo = lo.clamp(first, last);
            hi = hi.clamp(first, last);
        }
    }
    let value = random::int_between(rng, lo, hi);
    i32::try_from(value).unwrap_or(at)
}

fn tile(pos: Pos) -> Value {
    json!({ "x": pos.x, "y": pos.y })
}

impl Generator {
    /// Makes one value. Returns `None` when the scene has nothing to choose
    /// from.
    pub fn generate(
        &self,
        rng: &mut Rng,
        scene: &mut dyn Scene,
        items: &[ItemInfo],
    ) -> Option<Value> {
        match self {
            Generator::Int { min, max } => Some(json!(random::int_between(rng, *min, *max))),
            Generator::Choice { of } => rng.pick(of).cloned(),
            Generator::Text { words } => {
                let mut text = String::new();
                for n in 0..*words {
                    if n > 0 {
                        text.push(' ');
                    }
                    text.push_str(rng.pick(&WORDS).copied().unwrap_or("hello"));
                }
                Some(Value::String(text))
            }
            Generator::Me => Some(json!(scene.me())),
            Generator::NearTile { radius } => {
                let (pos, bounds) = (scene.pos(), scene.bounds());
                let x = near(rng, pos.x, *radius, bounds.map(|b| (b.x0, b.x1)));
                let y = near(rng, pos.y, *radius, bounds.map(|b| (b.y0, b.y1)));
                Some(tile(Pos::new(x, y)))
            }
            Generator::NearX { radius } => {
                let range = scene.bounds().map(|b| (b.x0, b.x1));
                Some(json!(near(rng, scene.pos().x, *radius, range)))
            }
            Generator::NearY { radius } => {
                let range = scene.bounds().map(|b| (b.y0, b.y1));
                Some(json!(near(rng, scene.pos().y, *radius, range)))
            }
            Generator::NearbyResident { radius } => {
                rng.pick(scene.residents(*radius)).map(|neighbour| json!(neighbour.id))
            }
            Generator::NearbySite { radius, kind } => {
                pick_site(rng, scene, *radius, kind.as_deref()).map(|(id, _)| id)
            }
            Generator::SiteTile { radius, kind } => {
                pick_site(rng, scene, *radius, kind.as_deref()).map(|(_, pos)| tile(pos))
            }
            Generator::Item { tag, form } => {
                let fits = |item: &&ItemInfo| {
                    tag.as_ref().is_none_or(|tag| item.tags.iter().any(|t| t == tag))
                };
                let count = items.iter().filter(fits).count() as u64;
                if count == 0 {
                    return None;
                }
                let item = items.iter().filter(fits).nth(rng.below(count) as usize)?;
                Some(match form {
                    ItemAs::Name => json!(item.name),
                    ItemAs::Id => json!(item.id),
                })
            }
        }
    }
}

/// Chooses a random site within `radius`, of the given kind when there is
/// one. Returns its id and its nearest tile.
fn pick_site(
    rng: &mut Rng,
    scene: &mut dyn Scene,
    radius: u32,
    kind: Option<&str>,
) -> Option<(Value, Pos)> {
    let sites = scene.sites(radius);
    let fits = |kind_of_site: &str| kind.is_none_or(|kind| kind == kind_of_site);
    let count = sites.iter().filter(|site| fits(&site.kind)).count() as u64;
    if count == 0 {
        return None;
    }
    let site = sites.iter().filter(|site| fits(&site.kind)).nth(rng.below(count) as usize)?;
    Some((json!(site.id), site.pos))
}
