//! The governments, the offices and the posts of the game.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{CityId, OrgId, Value};

/// A government: the nation or one city.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub(crate) enum Gov {
    /// The national government, `OrgId::NATION`.
    Nation,
    /// The government of a city, `OrgId::city(c)`.
    City(CityId),
}

impl Gov {
    /// Returns the government organization.
    pub(crate) fn org(self) -> OrgId {
        match self {
            Gov::Nation => OrgId::NATION,
            Gov::City(city) => OrgId::city(city),
        }
    }

    /// Returns the government that an organization is, if it is one that
    /// has elections.
    pub(crate) fn of_org(org: OrgId) -> Option<Gov> {
        if org == OrgId::NATION { Some(Gov::Nation) } else { org.as_city().map(Gov::City) }
    }

    /// Returns the city of a city government.
    pub(crate) fn city(self) -> Option<CityId> {
        match self {
            Gov::Nation => None,
            Gov::City(city) => Some(city),
        }
    }

    /// Returns the key of the published state of this government: the
    /// number of the city, or the text `nation`.
    pub(crate) fn key(self) -> Value {
        match self {
            Gov::Nation => json!("nation"),
            Gov::City(city) => json!(city),
        }
    }

    /// Returns the fields that name this government in an event, a notice
    /// or an observation: `gov` (`nation` or `city`) and, for a city,
    /// `city`.
    pub(crate) fn describe(self) -> Map<String, Value> {
        let mut out = Map::new();
        match self {
            Gov::Nation => {
                out.insert("gov".into(), json!("nation"));
            }
            Gov::City(city) => {
                out.insert("gov".into(), json!("city"));
                out.insert("city".into(), json!(city));
            }
        }
        out
    }

    /// Like [`Gov::describe`], with more fields.
    pub(crate) fn describe_with(self, more: Value) -> Value {
        let mut out = self.describe();
        if let Value::Object(more) = more {
            out.extend(more);
        }
        Value::Object(out)
    }
}

/// An office that residents fill in an election.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, JsonSchema,
)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub(crate) enum Office {
    /// The executive of a city.
    Mayor,
    /// The members of a city council.
    Council,
    /// The executive of the nation.
    President,
    /// The members of the national legislature.
    Legislature,
}

impl Office {
    /// The offices of a city, in the order in which their elections open.
    pub(crate) const CITY: [Office; 2] = [Office::Mayor, Office::Council];
    /// The offices of the nation, in the order in which their elections
    /// open.
    pub(crate) const NATION: [Office; 2] = [Office::President, Office::Legislature];

    /// Returns the offices of a government.
    pub(crate) fn of(gov: Gov) -> [Office; 2] {
        match gov {
            Gov::Nation => Office::NATION,
            Gov::City(_) => Office::CITY,
        }
    }

    /// Returns the name of the office in arguments, events and published
    /// state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Office::Mayor => "mayor",
            Office::Council => "council",
            Office::President => "president",
            Office::Legislature => "legislature",
        }
    }

    /// Returns true for an office of the nation.
    pub(crate) const fn is_national(self) -> bool {
        matches!(self, Office::President | Office::Legislature)
    }
}

/// A post that the executive fills with `appoint`.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, JsonSchema,
)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub(crate) enum Post {
    /// The head of the police of a city.
    PoliceChief,
    /// A police officer of a city.
    PoliceOfficer,
    /// A judge of the court of a city.
    Judge,
    /// The head of the central bank.
    CentralBanker,
}

impl Post {
    /// The posts of a city, in the order of the published state.
    pub(crate) const CITY: [Post; 3] = [Post::PoliceChief, Post::PoliceOfficer, Post::Judge];

    /// Returns the name of the post in arguments, events and published
    /// state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Post::PoliceChief => "police_chief",
            Post::PoliceOfficer => "police_officer",
            Post::Judge => "judge",
            Post::CentralBanker => "central_banker",
        }
    }

    /// Returns true for a post of the nation. The president fills it.
    pub(crate) const fn is_national(self) -> bool {
        matches!(self, Post::CentralBanker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_government_maps_to_its_organization_and_back() {
        assert_eq!(Gov::Nation.org(), OrgId::NATION);
        assert_eq!(Gov::City(CityId(3)).org(), OrgId(103));
        assert_eq!(Gov::of_org(OrgId::NATION), Some(Gov::Nation));
        assert_eq!(Gov::of_org(OrgId(103)), Some(Gov::City(CityId(3))));
        assert_eq!(Gov::of_org(OrgId::CENTRAL_BANK), None);
        assert_eq!(Gov::of_org(OrgId(5_000_000)), None);
    }

    #[test]
    fn a_government_has_a_key_and_a_description() {
        assert_eq!(Gov::Nation.key(), json!("nation"));
        assert_eq!(Gov::City(CityId(2)).key(), json!(2));
        assert_eq!(Value::Object(Gov::Nation.describe()), json!({ "gov": "nation" }));
        assert_eq!(
            Gov::City(CityId(2)).describe_with(json!({ "office": "mayor" })),
            json!({ "gov": "city", "city": 2, "office": "mayor" })
        );
    }

    #[test]
    fn names_are_the_wire_form() {
        for office in [Office::Mayor, Office::Council, Office::President, Office::Legislature] {
            assert_eq!(json!(office), json!(office.name()));
            assert_eq!(office.is_national(), Office::NATION.contains(&office));
        }
        for post in [Post::PoliceChief, Post::PoliceOfficer, Post::Judge, Post::CentralBanker] {
            assert_eq!(json!(post), json!(post.name()));
            assert_eq!(post.is_national(), !Post::CITY.contains(&post));
        }
    }
}
