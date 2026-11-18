//! The configuration of the services that the server wires: the viewer
//! service (`[spectator]`) and the missions of sponsors (`[sponsors]`).

mod common;

use std::time::Duration;

use common::TempDir;
use unsup_server::{Overrides, ServerConfig, SponsorConfig};

/// Loads a configuration file with this text.
fn load(text: &str) -> anyhow::Result<ServerConfig> {
    let dir = TempDir::new();
    let file = dir.write("server.toml", text);
    ServerConfig::load(Some(&file), Overrides::default())
}

#[test]
fn the_defaults_take_sponsor_missions_and_change_nothing_for_viewers() {
    let config = ServerConfig::default();
    assert_eq!(
        config.sponsors,
        SponsorConfig {
            enabled: true,
            missions_per_day: 3,
            max_mission_cents: 500_000,
            answer_timeout_ms: 10_000,
        }
    );
    assert_eq!(config.sponsors.answer_timeout(), Duration::from_secs(10));
    assert_eq!(config.spectator, unsup_spectator::Config::default());
    assert_eq!(config.spectator.stream.delay_frames, 0);
    // The development mode changes neither.
    let dev = ServerConfig::dev();
    assert_eq!((&dev.sponsors, &dev.spectator), (&config.sponsors, &config.spectator));
}

#[test]
fn a_file_changes_the_values_that_it_names() {
    let config = load(
        "[sponsors]\nenabled = false\nmissions_per_day = 1\n\n\
         [spectator.stream]\ndelay_frames = 300\n\n\
         [spectator.director]\nmin_hold_frames = 20\n\n\
         [spectator.clips]\npublic_pin = false\n",
    )
    .expect("a valid configuration");
    let defaults = ServerConfig::default();
    assert_eq!(
        config.sponsors,
        SponsorConfig { enabled: false, missions_per_day: 1, ..defaults.sponsors.clone() }
    );
    let mut viewers = defaults.spectator.clone();
    viewers.stream.delay_frames = 300;
    viewers.director.min_hold_frames = 20;
    viewers.clips.public_pin = false;
    assert_eq!(config.spectator, viewers);
    // The rest of the configuration is as without the file.
    assert_eq!((config.bind, &config.gateway), (defaults.bind, &defaults.gateway));
}

#[test]
fn values_that_are_not_valid_are_refused_with_their_reason() {
    let cases = [
        ("[sponsors]\nper_day = 2\n", "unknown field `per_day`"),
        ("[sponsors]\nenabled = \"no\"\n", "invalid type: string \"no\", expected a boolean"),
        ("[sponsors]\nmissions_per_day = -1\n", "invalid value: integer `-1`"),
        ("[sponsors]\nanswer_timeout_ms = 0\n", "`sponsors.answer_timeout_ms` must be at least 1"),
        // The viewer service names what is wrong in its table.
        ("[spectator.stream]\nspeed = 2\n", "the `[spectator]` table: unknown field `speed`"),
        ("[spectator.weather]\nrain = 1\n", "the `[spectator]` table: unknown field `weather`"),
        (
            "[spectator.director]\nmin_hold_frames = 61\n",
            "the `[spectator]` table: director.min_hold_frames is 61, it must not be above \
             director.max_hold_frames (60)",
        ),
        ("spectator = 5\n", "invalid type: integer `5`, expected a map"),
    ];
    for (text, reason) in cases {
        let error = load(text).expect_err("the configuration is refused");
        let message = format!("{error:#}");
        assert!(message.contains(reason), "{text:?} gave: {message}");
    }
}
