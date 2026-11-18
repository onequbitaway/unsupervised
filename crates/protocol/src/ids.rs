//! Ids, coordinates and time.

use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident($inner:ty)) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
            Serialize, Deserialize, JsonSchema,
        )]
        pub struct $name(pub $inner);

        impl $name {
            /// Returns the raw number.
            #[inline]
            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            #[inline]
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            #[inline]
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// Stable id of a resident. It never changes and is never reused.
    ResidentId(u64)
);
id_type!(
    /// Id of an owner: the account that controls one or more residents.
    OwnerId(u64)
);
id_type!(
    /// Id of an organization (firm, party, family, agency).
    OrgId(u64)
);

impl OrgId {
    /// The national government. Its treasury gets national taxes.
    pub const NATION: OrgId = OrgId(1);
    /// The central bank.
    pub const CENTRAL_BANK: OrgId = OrgId(2);
    /// First id of the city governments. See [`OrgId::city`].
    pub const CITY_BASE: u64 = 100;

    /// Returns the id of the government of a city. Its treasury gets city
    /// taxes and fines.
    #[inline]
    #[must_use]
    pub const fn city(city: CityId) -> OrgId {
        OrgId(Self::CITY_BASE + city.0 as u64)
    }

    /// Returns the city when this id is a city government.
    #[inline]
    #[must_use]
    pub const fn as_city(self) -> Option<CityId> {
        if self.0 >= Self::CITY_BASE && self.0 < Self::CITY_BASE + 65_536 {
            Some(CityId((self.0 - Self::CITY_BASE) as u16))
        } else {
            None
        }
    }
}
id_type!(
    /// Id of a site: a building or lot in a zone.
    SiteId(u64)
);
id_type!(
    /// Id of a zone: one district of a city. Zones are numbered from 0.
    ZoneId(u32)
);
id_type!(
    /// Id of a city. Cities are numbered from 0.
    CityId(u16)
);
id_type!(
    /// Id of an entry in the item catalogue.
    ItemId(u16)
);
id_type!(
    /// A tick count. One tick is one game minute. Each zone has its own tick
    /// counter; the calendar follows the frame number.
    Tick(u64)
);
id_type!(
    /// An amount of money in cents. All money is an integer number of cents.
    Money(i64)
);

impl Tick {
    /// Returns the number of ticks from `earlier` to `self`, or zero when
    /// `earlier` is later.
    #[inline]
    #[must_use]
    pub const fn since(self, earlier: Tick) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl Add<u64> for Tick {
    type Output = Tick;
    #[inline]
    fn add(self, rhs: u64) -> Tick {
        Tick(self.0.saturating_add(rhs))
    }
}

impl Money {
    /// No money.
    pub const ZERO: Money = Money(0);

    /// Returns the amount in cents.
    #[inline]
    #[must_use]
    pub const fn cents(self) -> i64 {
        self.0
    }

    /// Returns true when the amount is greater than zero.
    #[inline]
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }
}

impl Add for Money {
    type Output = Money;
    #[inline]
    fn add(self, rhs: Money) -> Money {
        Money(self.0 + rhs.0)
    }
}

impl Sub for Money {
    type Output = Money;
    #[inline]
    fn sub(self, rhs: Money) -> Money {
        Money(self.0 - rhs.0)
    }
}

impl Neg for Money {
    type Output = Money;
    #[inline]
    fn neg(self) -> Money {
        Money(-self.0)
    }
}

impl AddAssign for Money {
    #[inline]
    fn add_assign(&mut self, rhs: Money) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Money {
    #[inline]
    fn sub_assign(&mut self, rhs: Money) {
        self.0 -= rhs.0;
    }
}

impl std::iter::Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        Money(iter.map(|m| m.0).sum())
    }
}

/// A tile position in global tile coordinates.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub struct Pos {
    /// Column. It grows to the east.
    pub x: i32,
    /// Row. It grows to the south.
    pub y: i32,
}

impl Pos {
    /// Makes a position.
    #[inline]
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Pos {
        Pos { x, y }
    }

    /// Returns the position moved by `dx` columns and `dy` rows.
    #[inline]
    #[must_use]
    pub const fn offset(self, dx: i32, dy: i32) -> Pos {
        Pos { x: self.x.saturating_add(dx), y: self.y.saturating_add(dy) }
    }

    /// Returns the distance in tiles when diagonal steps cost one (Chebyshev
    /// distance). The kernel uses this metric for `nearby` and for audiences.
    #[inline]
    #[must_use]
    pub const fn dist(self, other: Pos) -> u32 {
        let dx = self.x.abs_diff(other.x);
        let dy = self.y.abs_diff(other.y);
        if dx > dy { dx } else { dy }
    }

    /// Returns the distance in tiles when only straight steps are allowed
    /// (Manhattan distance).
    #[inline]
    #[must_use]
    pub const fn manhattan(self, other: Pos) -> u32 {
        self.x.abs_diff(other.x).saturating_add(self.y.abs_diff(other.y))
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// A rectangle of tiles. The minimum corner is inclusive and the maximum
/// corner is exclusive.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub struct Rect {
    /// First column inside the rectangle.
    pub x0: i32,
    /// First row inside the rectangle.
    pub y0: i32,
    /// First column past the rectangle.
    pub x1: i32,
    /// First row past the rectangle.
    pub y1: i32,
}

impl Rect {
    /// Makes a rectangle from its corners.
    #[inline]
    #[must_use]
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    /// Makes a rectangle from its minimum corner and its size.
    #[inline]
    #[must_use]
    pub const fn sized(x0: i32, y0: i32, width: u32, height: u32) -> Rect {
        Rect {
            x0,
            y0,
            x1: x0.saturating_add_unsigned(width),
            y1: y0.saturating_add_unsigned(height),
        }
    }

    /// Returns the number of columns.
    #[inline]
    #[must_use]
    pub const fn width(&self) -> u32 {
        if self.x1 > self.x0 { self.x1.abs_diff(self.x0) } else { 0 }
    }

    /// Returns the number of rows.
    #[inline]
    #[must_use]
    pub const fn height(&self) -> u32 {
        if self.y1 > self.y0 { self.y1.abs_diff(self.y0) } else { 0 }
    }

    /// Returns true when the rectangle holds no tile.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// Returns true when `pos` is inside the rectangle.
    #[inline]
    #[must_use]
    pub const fn contains(&self, pos: Pos) -> bool {
        pos.x >= self.x0 && pos.x < self.x1 && pos.y >= self.y0 && pos.y < self.y1
    }

    /// Returns true when the two rectangles share at least one tile.
    #[inline]
    #[must_use]
    pub const fn intersects(&self, other: &Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x0 < other.x1
            && other.x0 < self.x1
            && self.y0 < other.y1
            && other.y0 < self.y1
    }

    /// Returns the tile in the middle of the rectangle.
    #[inline]
    #[must_use]
    pub const fn centre(&self) -> Pos {
        Pos { x: self.x0 + (self.x1 - self.x0) / 2, y: self.y0 + (self.y1 - self.y0) / 2 }
    }

    /// Returns the tile inside the rectangle that is nearest to `pos`. The
    /// rectangle must not be empty.
    #[inline]
    #[must_use]
    pub fn nearest(&self, pos: Pos) -> Pos {
        Pos {
            x: pos.x.clamp(self.x0, (self.x1 - 1).max(self.x0)),
            y: pos.y.clamp(self.y0, (self.y1 - 1).max(self.y0)),
        }
    }

    /// Returns the distance from `pos` to the nearest tile of the rectangle,
    /// with the same metric as [`Pos::dist`]. It is zero inside the rectangle.
    #[inline]
    #[must_use]
    pub fn dist_to(&self, pos: Pos) -> u32 {
        self.nearest(pos).dist(pos)
    }
}

/// The address of a realm: the global realm or one zone.
///
/// It serializes as `"global"` or `{ "zone": 3 }`.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RealmId {
    /// The realm that holds state without a location.
    Global,
    /// The realm of one zone.
    Zone(ZoneId),
}

impl RealmId {
    /// Returns the zone id when the realm is a zone.
    #[inline]
    #[must_use]
    pub const fn zone(self) -> Option<ZoneId> {
        match self {
            RealmId::Global => None,
            RealmId::Zone(z) => Some(z),
        }
    }

    /// Returns true for the global realm.
    #[inline]
    #[must_use]
    pub const fn is_global(self) -> bool {
        matches!(self, RealmId::Global)
    }
}

impl fmt::Display for RealmId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RealmId::Global => f.write_str("global"),
            RealmId::Zone(z) => write!(f, "zone {z}"),
        }
    }
}

/// The calendar time. It follows the frame number: one frame is one game
/// minute.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub struct Clock {
    /// Year, from 1.
    pub year: u32,
    /// Day of the year, from 1.
    pub day: u32,
    /// Hour of the day, 0 to 23.
    pub hour: u8,
    /// Minute of the hour, 0 to 59.
    pub minute: u8,
}

impl Clock {
    /// Game minutes in one day.
    pub const MINUTES_PER_DAY: u64 = 1_440;

    /// Returns the calendar time of a frame. Frame 0 is year 1, day 1, 00:00.
    /// `days_per_year` must be greater than zero.
    #[must_use]
    pub fn at(frame: u64, days_per_year: u32) -> Clock {
        let days_per_year = u64::from(days_per_year.max(1));
        let minute_of_day = frame % Self::MINUTES_PER_DAY;
        let day_index = frame / Self::MINUTES_PER_DAY;
        Clock {
            year: u32::try_from(day_index / days_per_year + 1).unwrap_or(u32::MAX),
            // The remainder is below `days_per_year`, which came from a `u32`.
            day: (day_index % days_per_year) as u32 + 1,
            hour: (minute_of_day / 60) as u8,
            minute: (minute_of_day % 60) as u8,
        }
    }

    /// Returns the minute of the day, 0 to 1,439.
    #[inline]
    #[must_use]
    pub const fn minute_of_day(&self) -> u32 {
        self.hour as u32 * 60 + self.minute as u32
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "year {} day {} {:02}:{:02}", self.year, self.day, self.hour, self.minute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_follows_frames() {
        assert_eq!(Clock::at(0, 28), Clock { year: 1, day: 1, hour: 0, minute: 0 });
        assert_eq!(Clock::at(61, 28), Clock { year: 1, day: 1, hour: 1, minute: 1 });
        assert_eq!(Clock::at(1_440, 28), Clock { year: 1, day: 2, hour: 0, minute: 0 });
        assert_eq!(
            Clock::at(1_440 * 28 + 1_439, 28),
            Clock { year: 2, day: 1, hour: 23, minute: 59 }
        );
        assert_eq!(Clock::at(1_439, 28).minute_of_day(), 1_439);
    }

    #[test]
    fn rect_geometry() {
        let r = Rect::sized(10, 20, 4, 6);
        assert_eq!(r, Rect::new(10, 20, 14, 26));
        assert_eq!((r.width(), r.height()), (4, 6));
        assert!(r.contains(Pos::new(10, 20)));
        assert!(r.contains(Pos::new(13, 25)));
        assert!(!r.contains(Pos::new(14, 25)));
        assert!(!r.contains(Pos::new(13, 26)));
        assert_eq!(r.centre(), Pos::new(12, 23));
        assert_eq!(r.nearest(Pos::new(0, 100)), Pos::new(10, 25));
        assert_eq!(r.dist_to(Pos::new(12, 22)), 0);
        assert_eq!(r.dist_to(Pos::new(17, 22)), 4);
        assert!(r.intersects(&Rect::new(13, 25, 30, 30)));
        assert!(!r.intersects(&Rect::new(14, 20, 30, 30)));
        assert!(Rect::new(3, 3, 3, 9).is_empty());
    }

    #[test]
    fn distances() {
        let a = Pos::new(0, 0);
        let b = Pos::new(3, -4);
        assert_eq!(a.dist(b), 4);
        assert_eq!(a.manhattan(b), 7);
        assert_eq!(a.offset(2, 5), Pos::new(2, 5));
    }

    #[test]
    fn money_arithmetic() {
        let mut m = Money(250) + Money(50) - Money(100);
        assert_eq!(m, Money(200));
        m += Money(1);
        m -= Money(2);
        assert_eq!(m.cents(), 199);
        assert_eq!(-m, Money(-199));
        assert_eq!([Money(1), Money(2)].into_iter().sum::<Money>(), Money(3));
        assert_eq!(Tick(5) + 3, Tick(8));
        assert_eq!(Tick(5).since(Tick(9)), 0);
    }
}
