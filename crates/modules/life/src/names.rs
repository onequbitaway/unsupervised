//! The names that the life module shares with other modules and with the
//! viewer: site kinds, tile codes, district types, attributes, conditions,
//! skills and tags. They come from `docs/design/game-design.md`.

/// Kinds of site.
pub mod site {
    /// A house for one household.
    pub const HOME: &str = "home";
    /// A building with several flats.
    pub const APARTMENT: &str = "apartment";
    /// A public place to sleep.
    pub const SHELTER: &str = "shelter";
    /// A shop.
    pub const SHOP: &str = "shop";
    /// A restaurant.
    pub const RESTAURANT: &str = "restaurant";
    /// An office.
    pub const OFFICE: &str = "office";
    /// A factory.
    pub const FACTORY: &str = "factory";
    /// A workshop.
    pub const WORKSHOP: &str = "workshop";
    /// A farm. It is open land.
    pub const FARM: &str = "farm";
    /// A mine. It is open land.
    pub const MINE: &str = "mine";
    /// A forest. It is open land.
    pub const FOREST: &str = "forest";
    /// A warehouse.
    pub const WAREHOUSE: &str = "warehouse";
    /// A bank.
    pub const BANK: &str = "bank";
    /// The exchange of a city.
    pub const EXCHANGE: &str = "exchange";
    /// The city hall.
    pub const CITY_HALL: &str = "city_hall";
    /// A police station.
    pub const POLICE_STATION: &str = "police_station";
    /// The courthouse.
    pub const COURTHOUSE: &str = "courthouse";
    /// The jail.
    pub const JAIL: &str = "jail";
    /// A hospital.
    pub const HOSPITAL: &str = "hospital";
    /// A school.
    pub const SCHOOL: &str = "school";
    /// A station for journeys between cities.
    pub const STATION: &str = "station";
    /// A park. It is open land.
    pub const PARK: &str = "park";
    /// A newsroom.
    pub const NEWSROOM: &str = "newsroom";
    /// The depot: the system buyer and seller of last resort.
    pub const DEPOT: &str = "depot";

    /// Every kind of site.
    pub const ALL: [&str; 24] = [
        HOME,
        APARTMENT,
        SHELTER,
        SHOP,
        RESTAURANT,
        OFFICE,
        FACTORY,
        WORKSHOP,
        FARM,
        MINE,
        FOREST,
        WAREHOUSE,
        BANK,
        EXCHANGE,
        CITY_HALL,
        POLICE_STATION,
        COURTHOUSE,
        JAIL,
        HOSPITAL,
        SCHOOL,
        STATION,
        PARK,
        NEWSROOM,
        DEPOT,
    ];
}

/// Tile codes. The life module writes them and the viewer draws them.
pub mod tile {
    /// Grass. Walkable.
    pub const GRASS: u8 = 0;
    /// Road. Walkable and fast.
    pub const ROAD: u8 = 1;
    /// Pavement. Walkable.
    pub const PAVEMENT: u8 = 2;
    /// Floor inside a site. Walkable. A door is a floor tile in a wall.
    pub const FLOOR: u8 = 3;
    /// Wall. Not walkable.
    pub const WALL: u8 = 4;
    /// Water. Not walkable.
    pub const WATER: u8 = 5;
    /// Trees. Walkable and slow.
    pub const TREES: u8 = 6;
    /// Field. Walkable.
    pub const FIELD: u8 = 7;
    /// Rock. Not walkable.
    pub const ROCK: u8 = 8;

    /// Returns the name of a tile code, or `"unknown"`.
    #[must_use]
    pub fn name(code: u8) -> &'static str {
        match code {
            GRASS => "grass",
            ROAD => "road",
            PAVEMENT => "pavement",
            FLOOR => "floor",
            WALL => "wall",
            WATER => "water",
            TREES => "trees",
            FIELD => "field",
            ROCK => "rock",
            _ => "unknown",
        }
    }

    /// Returns true when a resident can stand on a tile with this code.
    #[must_use]
    pub fn is_walkable(code: u8) -> bool {
        matches!(code, GRASS | ROAD | PAVEMENT | FLOOR | TREES | FIELD)
    }
}

/// Types of district.
pub mod district {
    /// The centre: government, banks, offices.
    pub const DOWNTOWN: &str = "downtown";
    /// Homes, schools, parks.
    pub const RESIDENTIAL: &str = "residential";
    /// Shops, restaurants, offices.
    pub const COMMERCIAL: &str = "commercial";
    /// Factories, workshops, warehouses.
    pub const INDUSTRIAL: &str = "industrial";
    /// Farms, forests, mines.
    pub const RURAL: &str = "rural";
    /// The only district of a city with one zone. It has a bit of each.
    pub const MIXED: &str = "mixed";
}

/// Attributes of a resident.
pub mod attr {
    /// Food. 0 means starving.
    pub const FOOD: &str = "need.food";
    /// Energy. 0 means exhausted.
    pub const ENERGY: &str = "need.energy";
    /// Health. 0 means death.
    pub const HEALTH: &str = "need.health";
    /// Mood. It changes work output and learning.
    pub const MOOD: &str = "need.mood";
    /// Age in years.
    pub const AGE: &str = "age.years";
    /// Prefix of the skill attributes: `skill.<name>`.
    pub const SKILL_PREFIX: &str = "skill.";
    /// Greatest value of a need.
    pub const NEED_MAX: i64 = 1_000;
}

/// Conditions of a resident.
pub mod cond {
    /// The resident sleeps.
    pub const ASLEEP: &str = "asleep";
    /// The resident is in hospital until healed. Blocking.
    pub const HOSPITALIZED: &str = "hospitalized";
}

/// Tags of sites and items that the life module reads or writes.
pub mod tag {
    /// A site that is never sold.
    pub const PUBLIC: &str = "public";
    /// A site where new residents appear.
    pub const SPAWN: &str = "spawn";
    /// An item that can be eaten. Its attribute `nutrition` says how much
    /// food one unit gives.
    pub const FOOD: &str = "food";
    /// An item that heals. Its attribute `healing` says how much health one
    /// unit gives.
    pub const MEDICINE: &str = "medicine";
}

/// Named numbers of sites and items that the life module reads or writes.
pub mod key {
    /// Site: price in cents.
    pub const VALUE: &str = "value";
    /// Site: number of beds.
    pub const BEDS: &str = "beds";
    /// Item: food points of one unit.
    pub const NUTRITION: &str = "nutrition";
    /// Item: health points of one unit.
    pub const HEALING: &str = "healing";
}

/// The skills, in the order of `docs/design/game-design.md`.
pub const SKILLS: [&str; 13] = [
    "labor",
    "farming",
    "mining",
    "crafting",
    "cooking",
    "trading",
    "management",
    "medicine",
    "law",
    "investigation",
    "stealth",
    "combat",
    "rhetoric",
];

/// Ticks in one game hour.
pub const TICKS_PER_HOUR: u32 = 60;

/// Ticks in one game day.
pub const TICKS_PER_DAY: u32 = 1_440;
