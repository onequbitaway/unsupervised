//! The show reads the events that the game modules really emit.
//!
//! The world of this test has every module of the server. Its residents
//! use the real verbs: a theft that the city watch sees, an arrest, a
//! charge, a candidate for mayor, a mission of a sponsor. The test then
//! reads the stories, a life story and the published state, and checks
//! that the editorial data (`data/director.toml`) fits the data of those
//! events: the right residents, one story for each case, election, bill
//! and mission, and words without a gap.

mod common;

use common::Harness;
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Builder, CityId, Money, OrgId, Plugin, Pos, ResidentId, Scope, Stage, WorldConfig,
};
use unsup_missions::{MissionSpec, SponsorMission};
use unsup_runtime::RuntimeConfig;
use unsup_runtime::storage::MemStorage;
use unsup_spectator::Config;

/// A tile on a street of the map of seed 0. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// Parameters of the world: every crime works and the city watch sees it,
/// no witness names the thief, and the boards are published often.
const PARAMS: [(&str, i64); 8] = [
    ("crime.chance_min_bp", 10_000),
    ("crime.watch_detect_bp", 10_000),
    ("crime.identify_base_bp", 0),
    ("crime.identify_fail_bonus_bp", 0),
    ("crime.identify_force_bonus_bp", 0),
    ("crime.patrol_detect_bp", 0),
    ("missions.report_period", 5),
    ("missions.board_period", 5),
];

/// The residents of the test.
struct Cast {
    bo: ResidentId,
    cy: ResidentId,
    eve: ResidentId,
    ann: ResidentId,
}

/// The id of Eve: the third resident of the world.
const EVE: ResidentId = ResidentId(3);

/// Gives Eve an office of the city with the power to arrest, in frame 2:
/// the global realm knows the residents by then. The politics module gives
/// its offices with the same two functions of the society module.
struct Deputy;

impl Plugin for Deputy {
    fn name(&self) -> &'static str {
        "deputy"
    }

    fn build(&self, b: &mut Builder) {
        if b.scope() != Scope::Global {
            return;
        }
        b.system(Stage::Update, 1_000, "deputy.appoint", |realm| {
            if realm.frame() != 2 {
                return;
            }
            let city = OrgId::city(CityId(0));
            unsup_society::offices::define_role(realm, city, "police", &["law.arrest"])
                .expect("a valid role");
            unsup_society::offices::set_holders(realm, city, "police", &[EVE]).expect("Eve lives");
        });
    }
}

/// Makes the world with every module of the server and puts the residents
/// in place. No frame has run: the service sees the world from its first
/// frame, in which the politics module calls the first elections.
fn world() -> (unsup_kernel::World, Cast) {
    static LIFE: unsup_life::LifePlugin = unsup_life::LifePlugin;
    static ECONOMY: unsup_economy::EconomyPlugin = unsup_economy::EconomyPlugin;
    static FINANCE: unsup_finance::FinancePlugin = unsup_finance::FinancePlugin;
    static SOCIETY: unsup_society::SocietyPlugin = unsup_society::SocietyPlugin;
    static CRIME: unsup_crime::CrimePlugin = unsup_crime::CrimePlugin;
    static POLITICS: unsup_politics::PoliticsPlugin = unsup_politics::PoliticsPlugin;
    static MISSIONS: unsup_missions::MissionsPlugin = unsup_missions::MissionsPlugin;
    static DEPUTY: Deputy = Deputy;
    let plugins: [&dyn Plugin; 8] =
        [&LIFE, &ECONOMY, &FINANCE, &SOCIETY, &CRIME, &POLITICS, &MISSIONS, &DEPUTY];
    let data: String = PARAMS
        .iter()
        .map(|(name, value)| format!("[params.\"{name}\"]\nvalue = {value}\n"))
        .collect();
    let config = WorldConfig { data: vec![data], ..WorldConfig::with_seed(0) };
    let mut w = TestWorld::with_config(&plugins, config);
    let resident = |w: &mut TestWorld<'_>, name: &str, cash: i64, pos: Pos| {
        let id = w.spawn(name);
        w.give_money(id, Money(cash));
        w.set_pos(id, pos);
        id
    };
    // Bo is resident 1. A count of 1 in the data of an event must not put
    // him into a story.
    let bo = resident(&mut w, "Bo", 2_000, STREET);
    let cy = resident(&mut w, "Cy", 0, STREET.offset(1, 0));
    let eve = resident(&mut w, "Eve", 0, STREET.offset(0, 1));
    let ann = resident(&mut w, "Ann", 20_000, STREET.offset(5, 0));
    assert_eq!((bo, eve), (ResidentId(1), EVE));
    assert_eq!(w.world.frame(), 0);
    (w.world, Cast { bo, cy, eve, ann })
}

/// Runs frames until the feed has an event of a kind, and returns the data
/// of the newest one. The modules decide how many frames a verb takes.
async fn until(harness: &mut Harness, kind: &str) -> Value {
    for _ in 0..200 {
        let feed = harness.get_ok(&format!("/v1/feed?kind={kind}&limit=500")).await;
        if let Some(event) = feed["events"].as_array().and_then(|events| events.last()) {
            return event["data"].clone();
        }
        harness.step().await;
    }
    panic!("no `{kind}` in 200 frames");
}

/// Returns the one story of a kind whose timeline has an event of
/// `event_kind`.
fn story_with<'a>(stories: &'a Value, kind: &str, event_kind: &str) -> &'a Value {
    let found: Vec<&Value> = stories["stories"]
        .as_array()
        .expect("stories")
        .iter()
        .filter(|story| {
            story["kind"] == kind
                && story["timeline"]
                    .as_array()
                    .is_some_and(|beats| beats.iter().any(|beat| beat["kind"] == event_kind))
        })
        .collect();
    assert_eq!(found.len(), 1, "one `{kind}` story with `{event_kind}`: {stories}");
    found[0]
}

fn lines(story: &Value) -> Vec<&str> {
    let beats = story["timeline"].as_array().expect("a timeline");
    beats.iter().map(|beat| beat["line"].as_str().expect("a line")).collect()
}

fn residents(story: &Value) -> Vec<(u64, &str)> {
    let list = story["residents"].as_array().expect("residents");
    list.iter()
        .map(|r| (r["id"].as_u64().expect("an id"), r["name"].as_str().expect("a name")))
        .collect()
}

/// Returns the shot that the director has on air.
async fn on_air(harness: &Harness) -> Value {
    harness.get_ok("/v1/director").await["shot"].clone()
}

#[tokio::test]
async fn the_director_leaves_the_wide_view_for_an_arrest_of_the_real_modules() {
    let (world, Cast { bo, cy, eve, .. }) = world();
    let storage = std::sync::Arc::new(MemStorage::new());
    let mut harness =
        Harness::start_world(world, Config::default(), RuntimeConfig::manual(), storage).await;
    // Frame 1 calls the elections. Frame 2 gives Eve her office, and the
    // zones know it one frame later.
    harness.steps(3).await;
    let layout = harness.get_ok("/v1/world").await;
    let city = layout["cities"][0]["name"].as_str().expect("a city name").to_string();

    // The elections of frame 1 name no resident, no site and no district,
    // and they are not breaking news. The director has nothing to show:
    // a wide view of the city with its residents.
    let quiet = on_air(&harness).await;
    assert_eq!(
        (&quiet["focus"]["type"], &quiet["zoom"], &quiet["drama"], &quiet["breaking"]),
        (&json!("none"), &json!("city"), &json!(0), &json!(false))
    );
    assert_eq!(quiet["caption"], json!(format!("{city} · 4 residents")));

    // Cy takes 500 cents from Bo. The city watch sees it, and Eve arrests
    // Cy: `crime.arrest_made` with the officer as the actor and the
    // suspect in `target`, drama 55.
    harness.act(cy, "pickpocket", json!({ "target": bo })).await;
    until(&mut harness, "crime.warrant_issued").await;
    harness.step().await;
    harness.act(eve, "arrest", json!({ "target": cy })).await;
    let arrest = until(&mut harness, "crime.arrest_made").await;
    assert_eq!((&arrest["officer"], &arrest["target"]), (&json!(eve), &json!(cy)));

    // A shot stays for a least time, so the arrest is on air some frames
    // later. The director then shows the officer from near, with the line
    // of the event and the words of the crime story.
    let mut shots = Vec::new();
    for _ in 0..30 {
        shots.push(on_air(&harness).await);
        harness.step().await;
    }
    let shot = shots
        .iter()
        .find(|shot| shot["kind"] == "crime.arrest_made")
        .unwrap_or_else(|| panic!("the arrest got no shot in 30 frames: {shots:?}"));
    assert_eq!(
        (&shot["zoom"], &shot["breaking"], &shot["drama"], &shot["caption"]),
        (&json!("close"), &json!(false), &json!(55), &json!("Eve arrested Cy"))
    );
    assert_eq!(shot["focus"], json!({ "type": "resident", "id": eve, "name": "Eve" }));
    assert_eq!(shot["label"], "Crime");
    assert!(shot["story"].is_u64(), "the shot names the story of the case: {shot}");
    // The camera is on a resident of the event, not on the city.
    let wide = shots.iter().filter(|shot| shot["focus"]["type"] == "none").count();
    assert!(wide < shots.len(), "every one of the 30 frames had a wide view");
    harness.stop().await;
}

#[tokio::test]
async fn the_show_reads_the_events_of_the_real_modules() {
    let (world, Cast { bo, cy, eve, ann }) = world();
    let storage = std::sync::Arc::new(MemStorage::new());
    let mut harness =
        Harness::start_world(world, Config::default(), RuntimeConfig::manual(), storage).await;
    // Frame 1 calls the elections. Frame 2 gives Eve her office, and the
    // zones know it one frame later.
    harness.steps(3).await;

    // The names of the map, as the layout gives them.
    let layout = harness.get_ok("/v1/world").await;
    let city = layout["cities"][0]["name"].as_str().expect("a city name").to_string();
    let district = layout["zones"]
        .as_array()
        .expect("zones")
        .iter()
        .find(|zone| {
            let b = &zone["bounds"];
            let inside = |low: &Value, high: &Value, at: i32| {
                low.as_i64().is_some_and(|low| low <= i64::from(at))
                    && high.as_i64().is_some_and(|high| i64::from(at) < high)
            };
            inside(&b["x0"], &b["x1"], STREET.x) && inside(&b["y0"], &b["y1"], STREET.y)
        })
        .expect("the zone of the street")["name"]
        .as_str()
        .expect("a zone name")
        .to_string();

    // A sponsor publishes a mission: make one arrest. Eve takes it.
    let mission: MissionSpec = serde_json::from_value(json!({
        "title": "Catch a thief",
        "text": "The town needs order.",
        "objectives": [{ "event": "crime.arrest_made", "count": 1 }],
        "reward": 900,
        "deadline_in": 600,
    }))
    .expect("a mission definition");
    let message = SponsorMission { request: 7, sponsor: "Acme Mills".into(), mission };
    harness.sim.send_global(&message).await.expect("the world takes the mission");
    let posted = until(&mut harness, "missions.mission_posted").await;
    let mission = posted["mission"].as_u64().expect("the id of the mission");
    harness.act(eve, "mission_accept", json!({ "mission": mission })).await;
    until(&mut harness, "missions.mission_accepted").await;

    // Cy takes 500 cents from Bo. The city watch sees it: a warrant.
    harness.act(cy, "pickpocket", json!({ "target": bo })).await;
    let theft = until(&mut harness, "crime.theft_committed").await;
    let case = theft["case"].as_u64().expect("the case");
    assert_eq!(
        theft,
        json!({ "case": case, "kind": "theft", "city": 0, "site": null, "value": 500, "attempt": false })
    );
    until(&mut harness, "crime.warrant_issued").await;
    // The warrant reaches the zone of Cy one frame later. Eve arrests Cy
    // and charges it. The city has no judge: a jury.
    harness.step().await;
    harness.act(eve, "arrest", json!({ "target": cy })).await;
    until(&mut harness, "crime.arrest_made").await;
    // The mission of Eve counts the arrest and pays her.
    until(&mut harness, "missions.mission_closed").await;
    harness.steps(8).await;
    harness.act(eve, "charge", json!({ "target": cy, "case": case })).await;
    let jury = until(&mut harness, "crime.jury_summoned").await;
    let jurors = jury["jurors"].as_u64().expect("jurors");

    // Ann runs for mayor in the first election of the city.
    let called = harness.get_ok("/v1/feed?kind=politics.election_called&limit=500").await;
    let mayor = called["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event["data"]["office"] == "mayor")
        .expect("an election for mayor")["data"]["election"]
        .as_u64()
        .expect("the id of the election");
    harness
        .act(ann, "register_candidate", json!({ "election": mayor, "platform": "More parks." }))
        .await;
    let registered = until(&mut harness, "politics.candidate_registered").await;
    // The boards of the missions module follow in at most 5 frames, with
    // the cash of each resident after all of this.
    harness.steps(6).await;
    // The politics module counts the candidates. The count is 1, which is
    // also the id of Bo.
    assert_eq!((&registered["candidate"], &registered["candidates"]), (&json!(ann), &json!(1)));

    let stories = harness.get_ok("/v1/stories?limit=128").await;

    // The crime: one story for the case, from the theft to the jury. The
    // theft names nobody. The arrest names the officer and the suspect.
    let crime = story_with(&stories, "crime", "crime.theft_committed");
    let theft_line = format!("A theft took $5.00 in {district}");
    let warrant_line = format!("A warrant was issued for a theft in {city}");
    let jury_line = format!("A jury of {jurors} residents was called for a theft trial");
    assert_eq!(
        lines(crime),
        // The city watch saw the theft, so the warrant is in the frame of
        // the theft. The registry is in the global realm, whose events
        // come first in a frame.
        [
            warrant_line.as_str(),
            theft_line.as_str(),
            "Eve arrested Cy",
            "Cy was charged with theft",
            jury_line.as_str(),
        ]
    );
    assert_eq!(residents(crime), [(eve.0, "Eve"), (cy.0, "Cy")]);
    assert_eq!(crime["label"], "Crime");

    // The election for mayor: the call and the candidate, in one story
    // with the number of the election. Bo is not in it.
    let election = story_with(&stories, "election", "politics.candidate_registered");
    let called_line = format!("An election for mayor was called in {city}");
    let runs_line = format!("Ann runs for mayor in {city}");
    assert_eq!(lines(election), [called_line.as_str(), runs_line.as_str()]);
    assert_eq!(residents(election), [(ann.0, "Ann")]);
    // The election for the council is another story.
    let all = stories["stories"].as_array().expect("stories");
    assert_eq!(all.iter().filter(|story| story["kind"] == "election").count(), 2);

    // The mission: posted by the sponsor, taken, completed and closed.
    let mission_story = story_with(&stories, "mission", "missions.mission_completed");
    assert_eq!(
        lines(mission_story),
        [
            "A mission was posted: “Catch a thief” for $9.00",
            "Eve took a mission",
            "Eve completed a mission and earned $9.00",
            "A mission closed (completed)",
        ]
    );
    assert_eq!(residents(mission_story), [(eve.0, "Eve")]);

    // The life story of Cy has the arrest and the charge.
    let profile = harness.get_ok(&format!("/v1/residents/{cy}/profile")).await;
    let crimes: Vec<&str> = profile["life"]["crimes"]
        .as_array()
        .expect("the chapter of the crimes")
        .iter()
        .map(|entry| entry["line"].as_str().expect("a line"))
        .collect();
    assert_eq!(crimes, ["Eve arrested Cy", "Cy was charged with theft"]);
    // The life story of Eve has her mission with her work.
    let profile = harness.get_ok(&format!("/v1/residents/{eve}/profile")).await;
    let jobs: Vec<&str> = profile["life"]["jobs"]
        .as_array()
        .expect("the chapter of the jobs")
        .iter()
        .map(|entry| entry["line"].as_str().expect("a line"))
        .collect();
    assert_eq!(jobs, ["Eve took a mission", "Eve completed a mission and earned $9.00"]);

    // The board of the missions module, as the leaderboard panel gets it:
    // the list `top`, each entry with a rank, a resident, a name and a
    // score.
    let board = harness.get_ok("/v1/leaderboards/wealth").await;
    assert_eq!(
        (&board["kind"], &board["key"]),
        (&json!("missions.leaderboard_published"), &json!("wealth"))
    );
    let data = &board["data"];
    assert_eq!((&data["unit"], &data["model_source"]), (&json!("cents"), &json!("self_reported")));
    let top = data["top"].as_array().expect("the top list");
    // Ann paid the fee of a candidate. Then Bo, who lost 500 cents, Eve
    // with the reward, and Cy with the loot.
    let fee = registered["fee"].as_i64().expect("the fee");
    let entry = |rank: u64, id: ResidentId, name: &str, score: i64| json!({ "rank": rank, "resident": id, "name": name, "owner": 1, "model": null, "score": score });
    assert_eq!(
        top,
        &[
            entry(1, ann, "Ann", 20_000 - fee),
            entry(2, bo, "Bo", 1_500),
            entry(3, eve, "Eve", 900),
            entry(4, cy, "Cy", 500),
        ]
    );
    for list in ["by_owner", "by_org", "by_model"] {
        assert!(data[list].is_array(), "{list}: {data}");
    }

    // The election of the politics module, as the election panel gets it:
    // the office, the phase and the candidates with their names.
    let published =
        harness.get_ok(&format!("/v1/published/politics.election_published/{mayor}")).await;
    let data = &published["data"];
    assert_eq!(
        (&data["office"], &data["phase"], &data["election"], &data["city"]),
        (&json!("mayor"), &json!("registration"), &json!(mayor), &json!(0))
    );
    let candidates = data["candidates"].as_array().expect("the candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        (&candidates[0]["candidate"], &candidates[0]["name"], &candidates[0]["votes"]),
        (&json!(ann), &json!("Ann"), &json!(0))
    );
    harness.stop().await;
}
