//! The offices and posts as roles of the government organizations.
//!
//! The society module owns organizations and memberships. This file is the
//! only place of the politics module that changes them, and it does so only
//! through `unsup_society::offices`.
//!
//! Every office and every post is a role in a government organization:
//!
//! | Role | Organization | Permissions |
//! | --- | --- | --- |
//! | `mayor` | city | `gov.sign`, `gov.appoint`, `treasury.spend` |
//! | `councillor` | city | `gov.propose`, `gov.vote` |
//! | `police_chief` | city | `law.arrest`, `law.search`, `law.jail` |
//! | `police_officer` | city | `law.arrest`, `law.search` |
//! | `judge` | city | `law.judge`, `law.jail` |
//! | `president` | nation | `gov.sign`, `gov.appoint`, `treasury.spend` |
//! | `legislator` | nation | `gov.propose`, `gov.vote` |
//! | `central_banker` | central bank | `bank.central` |

use unsup_kernel::{ActionError, CityId, OrgId, ParamId, Realm, ResidentId};
use unsup_society::permissions::{GOV_APPOINT, TREASURY_SPEND};
use unsup_society::{Orgs, offices};

use crate::model::{Gov, Office, Post};
use crate::params::{self, Params};

/// Role of the executive of a city.
pub const MAYOR: &str = "mayor";
/// Role of a member of a city council.
pub const COUNCILLOR: &str = "councillor";
/// Role of the executive of the nation.
pub const PRESIDENT: &str = "president";
/// Role of a member of the national legislature.
pub const LEGISLATOR: &str = "legislator";
/// Role of the head of the police of a city.
pub const POLICE_CHIEF: &str = "police_chief";
/// Role of a police officer of a city.
pub const POLICE_OFFICER: &str = "police_officer";
/// Role of a judge of a city.
pub const JUDGE: &str = "judge";
/// Role of the head of the central bank.
pub const CENTRAL_BANKER: &str = "central_banker";

/// Lets a member propose a bill.
pub(crate) const GOV_PROPOSE: &str = "gov.propose";
/// Lets a member vote on a bill.
pub(crate) const GOV_VOTE: &str = "gov.vote";
/// Lets a member sign or veto a bill.
pub(crate) const GOV_SIGN: &str = "gov.sign";

/// The permissions of an executive.
const EXECUTIVE: &[&str] = &[GOV_SIGN, GOV_APPOINT, TREASURY_SPEND];
/// The permissions of a member of a council or of the legislature.
const MEMBER: &[&str] = &[GOV_PROPOSE, GOV_VOTE];

/// The roles of a city government with their permissions.
const CITY_ROLES: [(&str, &[&str]); 5] = [
    (MAYOR, EXECUTIVE),
    (COUNCILLOR, MEMBER),
    (POLICE_CHIEF, &["law.arrest", "law.search", "law.jail"]),
    (POLICE_OFFICER, &["law.arrest", "law.search"]),
    (JUDGE, &["law.judge", "law.jail"]),
];

/// The roles of the national government with their permissions.
const NATION_ROLES: [(&str, &[&str]); 2] = [(PRESIDENT, EXECUTIVE), (LEGISLATOR, MEMBER)];

/// The roles of the central bank with their permissions.
const BANK_ROLES: [(&str, &[&str]); 1] = [(CENTRAL_BANKER, &["bank.central"])];

/// Returns the role of an office.
pub(crate) const fn office_role(office: Office) -> &'static str {
    match office {
        Office::Mayor => MAYOR,
        Office::Council => COUNCILLOR,
        Office::President => PRESIDENT,
        Office::Legislature => LEGISLATOR,
    }
}

/// Returns the role of a post.
pub(crate) const fn post_role(post: Post) -> &'static str {
    match post {
        Post::PoliceChief => POLICE_CHIEF,
        Post::PoliceOfficer => POLICE_OFFICER,
        Post::Judge => JUDGE,
        Post::CentralBanker => CENTRAL_BANKER,
    }
}

/// Returns the organization in which a post is a role. `city` is the city
/// of a city post; a national post ignores it.
pub(crate) fn post_org(post: Post, city: CityId) -> OrgId {
    if post.is_national() { OrgId::CENTRAL_BANK } else { OrgId::city(city) }
}

/// Returns the government whose executive fills a post and whose treasury
/// pays it.
pub(crate) fn post_gov(post: Post, city: CityId) -> Gov {
    if post.is_national() { Gov::Nation } else { Gov::City(city) }
}

/// Returns every government of the world: the nation, then the cities in
/// id order.
pub(crate) fn governments(realm: &Realm) -> Vec<Gov> {
    let cities = realm.layout().cities().iter().map(|city| Gov::City(city.id));
    std::iter::once(Gov::Nation).chain(cities).collect()
}

/// Defines one role, or stops the world: a role that cannot be defined is a
/// mistake in this module or a world without the society module.
fn define(realm: &Realm, org: OrgId, role: &str, perms: &[&str]) {
    if let Err(error) = offices::define_role(realm, org, role, perms) {
        panic!(
            "the politics module cannot define the role `{role}` in organization {org}: {error}"
        );
    }
}

/// Defines the roles of every government organization. It runs in the
/// global realm when the world is made. A second call changes nothing.
pub(crate) fn define_roles(realm: &Realm) {
    for (role, perms) in NATION_ROLES {
        define(realm, OrgId::NATION, role, perms);
    }
    for (role, perms) in BANK_ROLES {
        define(realm, OrgId::CENTRAL_BANK, role, perms);
    }
    for city in realm.layout().cities() {
        for (role, perms) in CITY_ROLES {
            define(realm, OrgId::city(city.id), role, perms);
        }
    }
}

/// Returns the residents that hold a role, ordered by id.
pub(crate) fn holders(realm: &Realm, org: OrgId, role: &str) -> Vec<ResidentId> {
    offices::holders(realm, org, role)
}

/// Returns the role of a resident in an organization.
pub(crate) fn role_of(realm: &Realm, org: OrgId, resident: ResidentId) -> Option<String> {
    realm.res::<Orgs>().get(org).and_then(|found| found.role_of(resident)).map(str::to_string)
}

/// Returns true when a resident has a permission in an organization. The
/// registry of the society module decides.
pub(crate) fn can(realm: &Realm, org: OrgId, resident: ResidentId, perm: &str) -> bool {
    realm.res::<Orgs>().get(org).is_some_and(|found| found.can(resident, perm))
}

/// Returns the members of an organization that have a permission, ordered
/// by id.
pub(crate) fn members_with(realm: &Realm, org: OrgId, perm: &str) -> Vec<ResidentId> {
    let orgs = realm.res::<Orgs>();
    let Some(found) = orgs.get(org) else {
        return Vec::new();
    };
    found.members().map(|(member, _)| member).filter(|&member| found.can(member, perm)).collect()
}

/// Checks that a resident has a permission in a government.
pub(crate) fn require(
    realm: &Realm,
    gov: Gov,
    resident: ResidentId,
    perm: &str,
) -> Result<(), ActionError> {
    if can(realm, gov.org(), resident, perm) {
        return Ok(());
    }
    let place = match gov {
        Gov::Nation => "the national government".to_string(),
        Gov::City(city) => format!("the government of city {city}"),
    };
    Err(ActionError::not_allowed(format!("You need the permission `{perm}` in {place}.")))
}

/// Sets who holds a role. After the call the residents of `list` that are
/// in the world hold it and nobody else does. Returns the holders as the
/// registry has them.
pub(crate) fn install(
    realm: &Realm,
    org: OrgId,
    role: &str,
    list: &[ResidentId],
) -> Vec<ResidentId> {
    let alive: Vec<ResidentId> = {
        let directory = realm.directory();
        list.iter().copied().filter(|&resident| directory.is_alive(resident)).collect()
    };
    // The set-up defined the role and every resident of the list is alive,
    // so the registry does not refuse the call. The answer is read back
    // from the registry in every case.
    let _ = offices::set_holders(realm, org, role, &alive);
    holders(realm, org, role)
}

/// One office or post of a government: where the role is, how many seats
/// it has and what it pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seat {
    /// Name of the office or the post, for example `council`.
    pub(crate) name: &'static str,
    /// The organization that has the role.
    pub(crate) org: OrgId,
    /// Name of the role.
    pub(crate) role: &'static str,
    /// Number of seats.
    pub(crate) seats: u64,
    /// Cents for one game day.
    pub(crate) salary: i64,
    /// True for an office that an election fills.
    pub(crate) elected: bool,
}

/// Returns the number of seats of an office of a government.
pub(crate) fn office_seats(realm: &Realm, p: &Params, gov: Gov, office: Office) -> u64 {
    match office {
        Office::Mayor | Office::President => 1,
        Office::Council => params::amount(realm, p.council_seats, gov),
        Office::Legislature => params::amount(realm, p.legislature_seats, gov),
    }
}

/// Returns the number of seats of a post of a government.
pub(crate) fn post_seats(realm: &Realm, p: &Params, gov: Gov, post: Post) -> u64 {
    match post {
        Post::PoliceChief | Post::CentralBanker => 1,
        Post::PoliceOfficer => params::amount(realm, p.police_officers_max, gov),
        Post::Judge => params::amount(realm, p.judges_max, gov),
    }
}

/// Returns the parameter with the daily salary of an office.
const fn office_salary(p: &Params, office: Office) -> ParamId {
    match office {
        Office::Mayor => p.salary_mayor,
        Office::Council => p.salary_councillor,
        Office::President => p.salary_president,
        Office::Legislature => p.salary_legislator,
    }
}

/// Returns the parameter with the daily salary of a post.
const fn post_salary(p: &Params, post: Post) -> ParamId {
    match post {
        Post::PoliceChief => p.salary_police_chief,
        Post::PoliceOfficer => p.salary_police_officer,
        Post::Judge => p.salary_judge,
        Post::CentralBanker => p.salary_central_banker,
    }
}

/// Returns every office and post of a government, in the order in which
/// the government pays them: the elected offices, then the posts.
pub(crate) fn seats(realm: &Realm, p: &Params, gov: Gov) -> Vec<Seat> {
    let office = |office: Office| Seat {
        name: office.name(),
        org: gov.org(),
        role: office_role(office),
        seats: office_seats(realm, p, gov, office),
        salary: params::value(realm, office_salary(p, office), gov),
        elected: true,
    };
    let post = |post: Post, city: CityId| Seat {
        name: post.name(),
        org: post_org(post, city),
        role: post_role(post),
        seats: post_seats(realm, p, gov, post),
        salary: params::value(realm, post_salary(p, post), gov),
        elected: false,
    };
    let mut out: Vec<Seat> = Office::of(gov).into_iter().map(office).collect();
    match gov {
        // The city of a national post does not count.
        Gov::Nation => out.push(post(Post::CentralBanker, CityId(0))),
        Gov::City(city) => out.extend(Post::CITY.into_iter().map(|kind| post(kind, city))),
    }
    out
}

/// Returns the name of the office or the post that a role of a government
/// organization is.
pub(crate) fn seat_name(role: &str) -> Option<&'static str> {
    let offices = [Office::Mayor, Office::Council, Office::President, Office::Legislature]
        .into_iter()
        .map(|office| (office_role(office), office.name()));
    let posts = [Post::PoliceChief, Post::PoliceOfficer, Post::Judge, Post::CentralBanker]
        .into_iter()
        .map(|post| (post_role(post), post.name()));
    offices.chain(posts).find(|(name, _)| *name == role).map(|(_, seat)| seat)
}
