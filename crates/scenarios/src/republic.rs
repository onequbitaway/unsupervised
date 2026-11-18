//! The founding of a republic: the first days of a new world, played
//! through verbs.
//!
//! Twelve newcomers earn their first money with public works, gather their
//! food, elect a mayor, a council and a president, and fill the posts of
//! the police, the court and the central bank. The central banker issues
//! the money of the first year into the national treasury.
//!
//! Stories that need officials or capital start from this world: the
//! president pays a grant from the national treasury with `org_pay`. So
//! all their money comes from the faucets of the game.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, OrgId, ResidentId, Site};

use crate::game::{Game, Setup, plugins};

/// The money that the central banker issues in the founding: the limit of
/// one game year (`finance.issue_limit_per_year`).
pub const ISSUED: i64 = 10_000_000;

/// The residents of the republic and their offices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Republic {
    /// Mara, the mayor of the first city. She won the election.
    pub mayor: ResidentId,
    /// Milo. He lost the election for mayor.
    pub rival: ResidentId,
    /// Cleo and Cato, the two members of the city council.
    pub council: [ResidentId; 2],
    /// Petra, the president.
    pub president: ResidentId,
    /// Bram, the central banker. The president appointed him.
    pub banker: ResidentId,
    /// Otto, a police officer of the first city. The mayor appointed him.
    pub officer: ResidentId,
    /// Judy, a judge of the first city. The mayor appointed her.
    pub judge: ResidentId,
    /// Ann, Ben, Cal and Dee: residents without an office.
    pub citizens: [ResidentId; 4],
}

impl Republic {
    /// Returns every resident of the republic, in the order of their ids.
    #[must_use]
    pub fn everyone(&self) -> Vec<ResidentId> {
        let mut all = vec![self.mayor, self.rival];
        all.extend(self.council);
        all.extend([self.president, self.banker, self.officer, self.judge]);
        all.extend(self.citizens);
        all
    }
}

/// Returns the id of the open election for an office, as a resident reads
/// it in the `politics` section of its observation.
///
/// # Panics
///
/// Panics when no election for the office is open.
#[must_use]
pub fn open_election(g: &Game, r: ResidentId, office: &str) -> u64 {
    let section = g.section(r, "politics").unwrap_or_default();
    let found = section["elections"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|election| election["office"] == office)
        .and_then(|election| election["election"].as_u64());
    match found {
        Some(id) => id,
        None => panic!("no election for `{office}` is open: {section}"),
    }
}

fn go(site: &Site) -> Value {
    json!({ "site": site.id })
}

/// Adds a walk to a site and a number of actions there to a plan.
fn at(
    plan: &mut Vec<(ResidentId, &'static str, Value)>,
    r: ResidentId,
    site: &Site,
    actions: &[(&'static str, Value)],
) {
    plan.push((r, "move_to", go(site)));
    for (verb, args) in actions {
        plan.push((r, verb, args.clone()));
    }
}

/// Lets every resident eat the vegetables that it carries until it has no
/// room for one more.
pub fn feed(g: &mut Game, residents: &[ResidentId]) {
    let nutrition = 120;
    let mut plan = Vec::new();
    for &r in residents {
        let room = (1_000 - g.attr(r, "need.food")) / nutrition;
        for _ in 0..room.min(g.carried(r, "vegetables")) {
            plan.push((r, "eat", json!({ "item": "vegetables" })));
        }
    }
    g.act_all(&plan);
}

/// Sends every resident to the shelter. Each one eats and then sleeps
/// until it is rested.
pub fn night(g: &mut Game, residents: &[ResidentId]) {
    let shelter = g.site("shelter");
    let mut plan = Vec::new();
    for &r in residents {
        at(&mut plan, r, &shelter, &[]);
    }
    g.act_all(&plan);
    feed(g, residents);
    g.each(residents, "sleep", json!({}));
}

/// Plays the founding of the republic in a new world, from the first
/// frame to 08:00 of day 6. See the module documentation.
///
/// # Panics
///
/// Panics when a step of the story fails.
pub fn found(g: &mut Game) -> Republic {
    assert_eq!(g.frame(), 0, "the founding starts in a new world");
    let names = [
        "Mara", "Milo", "Cleo", "Cato", "Petra", "Bram", "Otto", "Judy", "Ann", "Ben", "Cal", "Dee",
    ];
    let ids: Vec<ResidentId> = names.iter().map(|name| g.spawn(name)).collect();
    let republic = Republic {
        mayor: ids[0],
        rival: ids[1],
        council: [ids[2], ids[3]],
        president: ids[4],
        banker: ids[5],
        officer: ids[6],
        judge: ids[7],
        citizens: [ids[8], ids[9], ids[10], ids[11]],
    };
    let Republic { mayor, rival, council, president, banker, officer, judge, citizens } =
        republic.clone();
    let everyone = republic.everyone();
    let candidates = [mayor, rival, council[0], council[1]];
    let others: Vec<ResidentId> =
        everyone.iter().copied().filter(|r| !candidates.contains(r)).collect();

    // Their owners drive these residents directly, so they give up the
    // default routines: a resident that sleeps by itself would miss the
    // hours of the story. They eat and sleep when the story says so.
    g.each(&everyone, "set_routines", json!({ "routines": [] }));
    // The global realm opens the first city elections in the first frame.
    g.step(2);
    let hall = g.site("city_hall");
    let farm = g.site("farm");
    let work = ("work", json!({}));
    let gather = ("gather", json!({ "item": "vegetables", "minutes": 240 }));

    // Day 1. Registration for the city offices is open for this day only.
    // A candidate earns the fee with two shifts of public works, then it
    // gathers food for a week. The others gather first and work then.
    let mayor_election = open_election(g, mayor, "mayor");
    let council_election = open_election(g, mayor, "council");
    let mut plan = Vec::new();
    for (index, &candidate) in candidates.iter().enumerate() {
        let election = if index < 2 { mayor_election } else { council_election };
        let platform = format!("{} for the city", names[index]);
        let register =
            ("register_candidate", json!({ "election": election, "platform": platform }));
        at(&mut plan, candidate, &hall, &[work.clone(), work.clone(), register]);
        at(&mut plan, candidate, &farm, std::slice::from_ref(&gather));
    }
    for &r in &others {
        at(&mut plan, r, &farm, std::slice::from_ref(&gather));
        at(&mut plan, r, &hall, &[work.clone(), work.clone()]);
    }
    g.act_all(&plan);
    night(g, &everyone);

    // Day 2. Everybody works two shifts.
    feed(g, &everyone);
    let mut plan = Vec::new();
    for &r in &everyone {
        at(&mut plan, r, &hall, &[work.clone(), work.clone()]);
    }
    g.act_all(&plan);
    night(g, &everyone);

    // Day 3. The nation opens its elections at midnight. The polls of the
    // city are open from 06:00 to 20:00.
    g.run_to(3, 10, 0);
    feed(g, &everyone);
    let president_election = open_election(g, president, "president");
    g.act(
        president,
        "register_candidate",
        json!({ "election": president_election, "platform": "One country, one bank" }),
    );
    // Milo has his own vote and the votes of Cal and Dee. Everybody else
    // votes for Mara. Seven vote for Cleo and five for Cato.
    let for_rival = [rival, citizens[2], citizens[3]];
    let mut plan = Vec::new();
    for (index, &voter) in everyone.iter().enumerate() {
        let choice = if for_rival.contains(&voter) { rival } else { mayor };
        plan.push((voter, "vote", json!({ "election": mayor_election, "candidate": choice })));
        let choice = if index < 7 { council[0] } else { council[1] };
        plan.push((voter, "vote", json!({ "election": council_election, "candidate": choice })));
    }
    g.act_all(&plan);
    // The count is at 20:00. The winners have their powers one frame later.
    g.run_to(3, 20, 2);
    g.act(mayor, "appoint", json!({ "post": "police_officer", "resident": officer, "city": 0 }));
    g.act(mayor, "appoint", json!({ "post": "judge", "resident": judge, "city": 0 }));
    night(g, &everyone);

    // Day 4. Everybody gathers food for one more week and works a shift.
    feed(g, &everyone);
    let mut plan = Vec::new();
    for &r in &everyone {
        at(&mut plan, r, &farm, std::slice::from_ref(&gather));
        at(&mut plan, r, &hall, std::slice::from_ref(&work));
    }
    g.act_all(&plan);
    night(g, &everyone);

    // Day 5. The polls of the nation. The president takes office at 20:00
    // and appoints the central banker, who issues the money of the year.
    g.run_to(5, 10, 0);
    feed(g, &everyone);
    g.each(&everyone, "vote", json!({ "election": president_election, "candidate": president }));
    g.run_to(5, 20, 2);
    g.act(president, "appoint", json!({ "post": "central_banker", "resident": banker }));
    g.step(1);
    g.act(banker, "central_bank_issue", json!({ "amount": ISSUED }));
    night(g, &everyone);
    g.run_to(6, 8, 0);
    feed(g, &everyone);
    republic
}

/// The key of a setup in the cache: the seed, the cities and the districts.
type Shape = (u64, u32, u32);

/// Returns the republic of a setup at 08:00 of day 6, ready for a story.
///
/// The founding is played once for each shape of world in a process. Each
/// call restores the world from the snapshot of that founding, so a story
/// also proves that a world goes on from a snapshot.
///
/// # Panics
///
/// Panics when the setup changes parameters: the founding is played with
/// the data of the game.
#[must_use]
pub fn republic(setup: &Setup) -> (Game, Republic) {
    static FOUNDED: Mutex<BTreeMap<Shape, (Vec<u8>, Republic)>> = Mutex::new(BTreeMap::new());
    assert!(setup.params.is_empty(), "the republic is founded with the data of the game");
    let key = (setup.shape.seed, setup.shape.cities, setup.shape.districts);
    // A test that failed while it held the lock does not stop the others.
    let mut founded = FOUNDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (bytes, cast) = founded.entry(key).or_insert_with(|| {
        let plain = Setup { restore_every: None, ..setup.clone() };
        let mut g = Game::new(&plain);
        let cast = found(&mut g);
        (g.world.world.snapshot(), cast)
    });
    let world = TestWorld::restore(plugins(), bytes);
    (Game::from_world(world, setup.restore_every), cast.clone())
}

/// Pays a grant from the national treasury to a resident: the president
/// uses `org_pay`. Returns after the money arrived.
pub fn grant(g: &mut Game, republic: &Republic, to: ResidentId, amount: i64) {
    let before = g.cash(to);
    let nation = OrgId::NATION;
    g.act(
        republic.president,
        "org_pay",
        json!({ "org": nation, "amount": amount, "to_resident": to }),
    );
    // The payment goes by wire from the global realm to the zone.
    g.step(1);
    assert_eq!(g.cash(to), before + amount, "the grant did not arrive");
    assert!(g.money(Holder::Org(nation)) >= 0);
}
