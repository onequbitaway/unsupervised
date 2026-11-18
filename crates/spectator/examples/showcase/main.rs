//! A small world for work on the viewer.
//!
//! The game modules are built in parallel with the viewer. This example
//! gives the viewer something to draw before they exist: a town with
//! streets, buildings and sites, residents that walk, work and speak, and
//! events with every level of drama. It serves the spectator routes and the
//! built viewer:
//!
//! ```sh
//! (cd web && npm ci && npm run build)
//! cargo run --release -p unsup-spectator --example showcase -- --residents 5000
//! ```
//!
//! Then open `http://127.0.0.1:8090/`, or `/?broadcast=1` for the show. The
//! driver also plays stories for the director: crimes with an
//! investigation and a trial, feuds, bankruptcies, elections with
//! candidates. It publishes boards and numbers of the economy. `--delay`
//! delays the stream, and `--data-dir` keeps the chronicle and the pinned
//! clips on the disk.
//!
//! The world is not the game: it has no
//! money, no needs and no rules. Some residents say text that tries to
//! break a viewer (markup, very long text, control characters); the viewer
//! must show it as plain text.

mod map;
mod numbers;
mod plots;
mod town;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use serde_json::{Value, json};
use tower_http::services::ServeDir;
use unsup_kernel::{CityConfig, Pos, World, WorldConfig};
use unsup_protocol::{ActionRequest, OwnerId, ResidentId, SimError, SpawnSpec};
use unsup_runtime::{RuntimeConfig, SimHandle};

use crate::numbers::{Driver, Numbers};
use crate::town::Town;

/// A small world for work on the viewer.
#[derive(Parser)]
#[command(name = "showcase")]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8090")]
    bind: SocketAddr,
    /// Real milliseconds for one game minute.
    #[arg(long, default_value_t = 250)]
    tick_ms: u64,
    /// Number of residents.
    #[arg(long, default_value_t = 400)]
    residents: u32,
    /// Number of cities.
    #[arg(long, default_value_t = 2)]
    cities: u32,
    /// Districts on one side of a city.
    #[arg(long, default_value_t = 2)]
    districts: u32,
    /// Put all residents in the first city.
    #[arg(long)]
    one_city: bool,
    /// Seed of the world and of the driver.
    #[arg(long, default_value_t = 7)]
    seed: u64,
    /// Directory with the built viewer.
    #[arg(long, default_value = "web/dist")]
    web_dir: PathBuf,
    /// Frames that viewers are behind the simulation.
    #[arg(long, default_value_t = 0)]
    delay: u64,
    /// Directory for the chronicle and the pinned clips. Without it they
    /// live in memory.
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

const CITIES: [&str; 6] =
    ["Latent City", "Tokentown", "Gradient Falls", "Port Epoch", "New Tensor", "Kernel Bay"];

const FIRST_NAMES: [&str; 16] = [
    "Ada", "Bo", "Cleo", "Dev", "Esme", "Finn", "Gus", "Hana", "Ines", "Jun", "Kofi", "Lena",
    "Milo", "Nia", "Omar", "Pia",
];

const MODELS: [Option<&str>; 4] = [Some("atlas-2"), Some("quill-mini"), Some("nova-pro"), None];

/// What residents say. Some lines try to break a viewer.
const LINES: [&str; 14] = [
    "Good morning!",
    "Anyone hiring today?",
    "The bread is fresh at the corner.",
    "I heard the mayor wants a new tax.",
    "Meet me at the park at noon.",
    "Rent is due again.",
    "Did you see who won the election?",
    "I need a break.",
    "<img src=x onerror=alert(1)><b>not bold</b>",
    "</canvas><script>document.body.innerHTML='broken'</script>",
    "A very long speech that goes on and on about the price of flour, the state of the roads, \
     the mayor, the weather, the neighbours and everything else that a resident could find to \
     complain about on a slow afternoon in a small town, without a single pause for breath.",
    "left\u{202e}thgir \u{200b}hidden\u{0007} bell",
    "line one\nline two\n\n\nline five",
    "¡Hola! 你好 مرحبا 👋",
];

/// A small random number generator for the driver. The driver is outside
/// the simulation, so it does not use the generator of the kernel.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, below: u64) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) % below.max(1)
    }
}

fn spec(number: u64, city: Option<u16>) -> SpawnSpec {
    let name = format!(
        "{} {}.",
        FIRST_NAMES[(number % 16) as usize],
        char::from(b'A' + (number / 16 % 26) as u8)
    );
    SpawnSpec {
        owner: OwnerId(1 + number % 20),
        name,
        model: MODELS[(number % 4) as usize].map(str::to_string),
        city: city.map(unsup_protocol::CityId),
    }
}

/// Chooses one request by the weights of the driver.
fn request(driver: &Driver, dice: &mut Dice, other: ResidentId) -> (&'static str, Value) {
    let ticks = driver.min_ticks + dice.roll(driver.max_ticks.saturating_sub(driver.min_ticks) + 1);
    let choices = [
        ("work", driver.work),
        ("eat", driver.eat),
        ("rest", driver.rest),
        ("sleep", driver.sleep),
        ("train", driver.train),
        ("say", driver.say),
        ("rob", driver.rob),
        ("marry", driver.marry),
        ("open_business", driver.open_business),
        ("die", driver.die),
    ];
    let mut left = dice.roll(choices.iter().map(|(_, weight)| weight).sum());
    let verb = choices
        .iter()
        .find(|(_, weight)| {
            let hit = left < *weight;
            left = left.saturating_sub(*weight);
            hit
        })
        .map_or("rest", |(verb, _)| *verb);
    let args = match verb {
        "say" => json!({ "text": LINES[dice.roll(LINES.len() as u64) as usize] }),
        "rob" | "marry" => json!({ "other": other }),
        "open_business" | "die" => json!({}),
        _ => json!({ "ticks": ticks }),
    };
    (verb, args)
}

/// Sends requests for residents, like agents would, until the simulation
/// stops.
async fn drive(sim: SimHandle, mut residents: Vec<ResidentId>, numbers: Numbers, args: &Args) {
    let driver = numbers.driver.clone();
    let mut dice = Dice(args.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut next_name = residents.len() as u64;
    let mut ticker = tokio::time::interval(Duration::from_millis(args.tick_ms.max(1)));
    let mut frame: u64 = 0;
    // The events of the stories that wait for their frame.
    let mut waiting: Vec<plots::Step> = Vec::new();
    let election_every = driver.election_every.max(1);
    loop {
        ticker.tick().await;
        frame += 1;
        if residents.is_empty() {
            continue;
        }
        // The stories: what starts now, the campaign of the next election,
        // and the published state.
        let pick = |n: u64| residents[(n % residents.len() as u64) as usize];
        waiting.extend(plots::start(frame, &numbers, &pick));
        if (frame + numbers.plots.campaign_frames).is_multiple_of(election_every) {
            let decided = frame + numbers.plots.campaign_frames;
            waiting.extend(plots::campaign(decided, decided / election_every % 2, &numbers, &pick));
        }
        if frame % numbers.plots.stats_every.max(1) == 1 {
            let data = plots::stats(frame, residents.len() as u64);
            waiting.push(plots::Step {
                due: frame,
                actor: pick(0),
                verb: "declare",
                args: json!({ "kind": "finance.stats_published", "drama": 0, "data": data }),
            });
        }
        if frame % numbers.plots.boards_every.max(1) == 2 {
            for data in plots::boards(frame, &numbers, &pick) {
                waiting.push(plots::Step {
                    due: frame,
                    actor: pick(0),
                    verb: "declare",
                    args: json!({ "kind": "missions.leaderboard_published", "drama": 0, "data": data }),
                });
            }
        }
        let (due, later): (Vec<plots::Step>, Vec<plots::Step>) =
            std::mem::take(&mut waiting).into_iter().partition(|step| step.due <= frame);
        waiting = later;
        for step in due {
            // A resident of a story can be dead by now. The event is then
            // refused, and the story goes on without it.
            if let Err(SimError::Shutdown) =
                sim.act(step.actor, ActionRequest::new(step.verb, step.args)).await
            {
                return;
            }
        }
        let count = (residents.len() as u64 / driver.one_in.max(1)).max(1);
        for _ in 0..count {
            let at = dice.roll(residents.len() as u64) as usize;
            let other = residents[dice.roll(residents.len() as u64) as usize];
            let (verb, body) = request(&driver, &mut dice, other);
            match sim.act(residents[at], ActionRequest::new(verb, body)).await {
                Ok(_) if verb == "die" => {
                    // A new resident takes the place of one that died.
                    residents.swap_remove(at);
                    let city = args.one_city.then_some(0);
                    if let Ok(id) = sim.spawn(spec(next_name, city)).await {
                        residents.push(id);
                    }
                    next_name += 1;
                }
                Ok(_) | Err(SimError::Overloaded | SimError::UnknownResident) => {}
                Err(_) => return,
            }
        }
        if frame.is_multiple_of(election_every) {
            // The first candidate of the campaign wins.
            let winner = residents[((frame * 3) % residents.len() as u64) as usize];
            let city = frame / election_every % 2;
            let _ =
                sim.act(winner, ActionRequest::new("win_election", json!({ "city": city }))).await;
        }
    }
}

fn world(args: &Args, numbers: &Numbers) -> World {
    let mut config = WorldConfig::with_seed(args.seed);
    let side = (args.districts * config.zone_size) as i32;
    config.cities = (0..args.cities.max(1))
        .map(|i| {
            let name = CITIES[i as usize % CITIES.len()];
            let name = if (i as usize) < CITIES.len() {
                name.to_string()
            } else {
                format!("{name} {}", i as usize / CITIES.len() + 1)
            };
            // Cities stand in a row with a gap of one district.
            let x = i as i32 * (side + config.zone_size as i32);
            CityConfig::new(name, Pos::new(x, 0), args.districts.max(1))
        })
        .collect();
    World::new(&[&Town { numbers: numbers.clone() }], config)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let numbers = Numbers::built_in()?;
    let runtime = RuntimeConfig::real_time(Duration::from_millis(args.tick_ms));
    let (sim, join) = unsup_runtime::start(world(&args, &numbers), runtime);

    let mut residents = Vec::new();
    for number in 0..u64::from(args.residents) {
        let city = args.one_city.then_some(0);
        residents.push(sim.spawn(spec(number, city)).await?);
    }

    let mut config = unsup_spectator::Config::default();
    config.stream.delay_frames = args.delay;
    let app = match &args.data_dir {
        Some(dir) => {
            let storage = unsup_runtime::storage::FsStorage::open(dir)?;
            unsup_spectator::router_with_storage(sim.clone(), config, std::sync::Arc::new(storage))?
        }
        None => unsup_spectator::router_with(sim.clone(), config)?,
    };
    let mut app = app;
    if args.web_dir.is_dir() {
        app = app.fallback_service(ServeDir::new(&args.web_dir));
    } else {
        println!("no viewer at {}: build it with `npm run build` in web/", args.web_dir.display());
    }
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    println!(
        "showcase: {} residents, http://{} (debug view at /debug)",
        residents.len(),
        listener.local_addr()?
    );

    tokio::select! {
        result = axum::serve(listener, app) => result?,
        () = drive(sim.clone(), residents, numbers, &args) => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    drop(sim);
    join.shutdown().map(|_world| ()).map_err(Into::into)
}
