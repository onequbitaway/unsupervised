//! What the world generator knows about each kind of site and each type of
//! district. These tables say what a thing is. The numbers (values, beds,
//! counts, weights) are parameters in `data/balance.toml`.

use crate::names::{district, site};

/// The type of a district.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum District {
    /// The centre: government, banks, offices.
    Downtown,
    /// Shops, restaurants, offices.
    Commercial,
    /// Homes, schools, parks.
    Residential,
    /// Factories, workshops, warehouses.
    Industrial,
    /// Farms, forests, mines.
    Rural,
    /// The only district of a city with one zone.
    Mixed,
}

impl District {
    /// The five types of a city with several zones, from the centre of the
    /// city to its edge.
    pub const RINGS: [District; 5] = [
        District::Downtown,
        District::Commercial,
        District::Residential,
        District::Industrial,
        District::Rural,
    ];

    /// Every type.
    pub const ALL: [District; 6] = [
        District::Downtown,
        District::Commercial,
        District::Residential,
        District::Industrial,
        District::Rural,
        District::Mixed,
    ];

    /// Returns the name of the type, as in `docs/design/game-design.md`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            District::Downtown => district::DOWNTOWN,
            District::Commercial => district::COMMERCIAL,
            District::Residential => district::RESIDENTIAL,
            District::Industrial => district::INDUSTRIAL,
            District::Rural => district::RURAL,
            District::Mixed => district::MIXED,
        }
    }

    /// Returns the type with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<District> {
        District::ALL.into_iter().find(|d| d.name() == name)
    }

    /// Returns the position of the type in [`District::ALL`].
    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// How much room a site takes in a city block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Size {
    /// A quarter of a block.
    Small,
    /// Half a block.
    Medium,
    /// A whole block.
    Large,
}

impl Size {
    /// Returns how often a block is cut in two to make a lot of this size.
    pub(crate) fn level(self) -> u8 {
        match self {
            Size::Large => 0,
            Size::Medium => 1,
            Size::Small => 2,
        }
    }

    /// Returns the room in quarters of a block.
    pub(crate) fn quarters(self) -> u32 {
        match self {
            Size::Large => 4,
            Size::Medium => 2,
            Size::Small => 1,
        }
    }
}

/// How many sites of a kind a city has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// Exactly one in each city.
    Unique,
    /// One for a number of zones (parameter `life.gen.zones_per.<kind>`),
    /// and at least one.
    Service,
    /// At least one in each city. More come from the mix of each district
    /// (parameters `life.gen.mix.<district>.<kind>`).
    Filler,
}

/// What a site looks like on the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Shape {
    /// Walls with a door, and floor inside.
    Building,
    /// Open land with fields.
    Field,
    /// Open land with trees.
    Woods,
    /// Open land around a mass of rock.
    Quarry,
    /// Open land with grass, trees and a pond.
    Garden,
}

/// What the generator knows about one kind of site.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Kind {
    /// Name of the kind, as in `docs/design/game-design.md`.
    pub(crate) name: &'static str,
    /// Start of the display name of a site of this kind.
    pub(crate) title: &'static str,
    pub(crate) size: Size,
    pub(crate) role: Role,
    pub(crate) shape: Shape,
    /// True for a public building. It gets the tag `public`.
    pub(crate) public: bool,
    /// True for a site where new residents appear. It gets the tag `spawn`.
    pub(crate) spawn: bool,
    /// The district types where the kind belongs, best first.
    pub(crate) prefers: &'static [District],
}

use District::{Commercial, Downtown, Industrial, Residential, Rural};

/// Makes a kind that is a private building.
const fn building(
    name: &'static str,
    title: &'static str,
    size: Size,
    role: Role,
    prefers: &'static [District],
) -> Kind {
    Kind { name, title, size, role, shape: Shape::Building, public: false, spawn: false, prefers }
}

/// Makes a kind that is a public building.
const fn public(
    name: &'static str,
    title: &'static str,
    size: Size,
    role: Role,
    prefers: &'static [District],
) -> Kind {
    Kind { public: true, ..building(name, title, size, role, prefers) }
}

/// Makes a kind that is open land.
const fn land(
    name: &'static str,
    title: &'static str,
    size: Size,
    shape: Shape,
    prefers: &'static [District],
) -> Kind {
    Kind { shape, ..building(name, title, size, Role::Filler, prefers) }
}

/// Every kind of site. The order is the order in which a city places the
/// sites that it must have, so the kinds that a city needs most come first.
pub(crate) const KINDS: [Kind; 24] = [
    Kind {
        spawn: true,
        ..public(
            site::SHELTER,
            "Shelter",
            Size::Large,
            Role::Service,
            &[Residential, Downtown, Commercial, Industrial],
        )
    },
    Kind {
        spawn: true,
        ..public(
            site::STATION,
            "Station",
            Size::Large,
            Role::Service,
            &[Downtown, Commercial, Industrial, Residential],
        )
    },
    public(
        site::CITY_HALL,
        "City Hall",
        Size::Medium,
        Role::Unique,
        &[Downtown, Commercial, Residential],
    ),
    public(
        site::HOSPITAL,
        "Hospital",
        Size::Medium,
        Role::Service,
        &[Residential, Downtown, Commercial],
    ),
    public(
        site::SCHOOL,
        "School",
        Size::Medium,
        Role::Service,
        &[Residential, Downtown, Commercial],
    ),
    public(
        site::POLICE_STATION,
        "Police Station",
        Size::Small,
        Role::Service,
        &[Downtown, Residential, Commercial, Industrial],
    ),
    public(
        site::COURTHOUSE,
        "Courthouse",
        Size::Medium,
        Role::Unique,
        &[Downtown, Commercial, Residential],
    ),
    public(site::JAIL, "Jail", Size::Medium, Role::Unique, &[Industrial, Rural, Downtown]),
    public(site::BANK, "Bank", Size::Small, Role::Service, &[Downtown, Commercial, Residential]),
    public(site::EXCHANGE, "Exchange", Size::Medium, Role::Unique, &[Downtown, Commercial]),
    public(site::DEPOT, "Depot", Size::Medium, Role::Unique, &[Industrial, Commercial, Rural]),
    building(site::NEWSROOM, "Newsroom", Size::Small, Role::Unique, &[Downtown, Commercial]),
    land(site::FARM, "Farm", Size::Large, Shape::Field, &[Rural, Residential, Industrial]),
    building(site::SHOP, "Shop", Size::Small, Role::Filler, &[Commercial, Downtown, Residential]),
    building(site::HOME, "Home", Size::Small, Role::Filler, &[Residential, Rural, Commercial]),
    building(
        site::APARTMENT,
        "Apartments",
        Size::Medium,
        Role::Filler,
        &[Residential, Downtown, Commercial],
    ),
    building(
        site::RESTAURANT,
        "Restaurant",
        Size::Small,
        Role::Filler,
        &[Commercial, Downtown, Residential],
    ),
    building(site::OFFICE, "Office", Size::Small, Role::Filler, &[Downtown, Commercial]),
    building(site::FACTORY, "Factory", Size::Large, Role::Filler, &[Industrial, Rural]),
    building(
        site::WORKSHOP,
        "Workshop",
        Size::Small,
        Role::Filler,
        &[Industrial, Commercial, Residential],
    ),
    building(site::WAREHOUSE, "Warehouse", Size::Medium, Role::Filler, &[Industrial, Commercial]),
    land(site::MINE, "Mine", Size::Medium, Shape::Quarry, &[Rural, Industrial]),
    land(site::FOREST, "Forest", Size::Large, Shape::Woods, &[Rural, Residential]),
    Kind {
        public: true,
        ..land(
            site::PARK,
            "Park",
            Size::Medium,
            Shape::Garden,
            &[Residential, Downtown, Commercial],
        )
    },
];
