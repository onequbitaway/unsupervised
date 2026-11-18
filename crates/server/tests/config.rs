//! The configuration: the defaults, a file, the development mode and the
//! values of the environment and the command line.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::TempDir;
use unsup_server::{Overrides, ServerConfig, StoreConfig, StoreKind, WorldShape};

fn load(file: Option<&Path>, overrides: Overrides) -> ServerConfig {
    ServerConfig::load(file, overrides).expect("a valid configuration")
}

/// Returns the text of the error for a configuration that is refused.
fn refusal(file: Option<&Path>, overrides: Overrides) -> String {
    let error = ServerConfig::load(file, overrides).expect_err("the configuration is refused");
    format!("{error:#}")
}

#[test]
fn the_defaults_are_a_server_with_a_sqlite_store() {
    let config = ServerConfig::default();
    assert_eq!(config.bind, "127.0.0.1:8080".parse().expect("an address"));
    assert_eq!(config.tick_interval, Duration::from_secs(1));
    assert_eq!(config.world, WorldShape { seed: 1, cities: 2, districts: 3 });
    assert_eq!(config.world, WorldShape::default());
    assert_eq!(config.web_dir, Some(PathBuf::from("web/dist")));
    assert_eq!(config.store, StoreConfig::Sqlite(PathBuf::from("var/store.sqlite3")));
    assert_eq!(config.shutdown_grace, Duration::from_secs(5));
    assert!(!config.dev);
    assert_eq!(config.gateway.join_file, Some(PathBuf::from("docs/join.md")));
    assert_eq!(config.gateway.admin_token, None);
    assert_eq!(config.gateway.cors_origins, Vec::<String>::new());
    assert!(!config.gateway.webhooks.allow_private);
    assert_eq!(config.gateway.limits.residents_per_owner, 3);
    assert_eq!(config, load(None, Overrides::default()));
}

#[test]
fn the_development_mode_has_fast_ticks_and_a_store_in_memory() {
    let config = ServerConfig::dev();
    assert!(config.dev);
    assert_eq!(config.tick_interval, Duration::from_millis(100));
    assert_eq!(config.store, StoreConfig::Memory);
    assert_eq!(config.gateway.cors_origins, ["*"]);
    assert!(config.gateway.webhooks.allow_private);
    // Everything else is as without the mode.
    let plain = ServerConfig::default();
    assert_eq!((config.bind, &config.world), (plain.bind, &plain.world));
    assert_eq!(config.gateway.limits, plain.gateway.limits);
}

#[test]
fn a_file_replaces_the_defaults_and_keeps_the_rest() {
    let dir = TempDir::new();
    let file = dir.write(
        "server.toml",
        r#"
        bind = "0.0.0.0:9000"
        tick_ms = 250
        data_dir = "/srv/unsupervised"
        join_file = "/srv/join.md"
        shutdown_grace_ms = 1500

        [world]
        seed = 42
        districts = 4

        [gateway]
        admin_token = "an-admin-token-from-the-file"
        cors_origins = ["https://viewer.example"]

        [gateway.limits]
        residents_per_owner = 5

        [gateway.webhooks]
        attempts = 2
        "#,
    );
    let config = load(Some(&file), Overrides::default());
    assert_eq!(config.bind, "0.0.0.0:9000".parse().expect("an address"));
    assert_eq!(config.tick_interval, Duration::from_millis(250));
    assert_eq!(config.store, StoreConfig::Sqlite(PathBuf::from("/srv/unsupervised/store.sqlite3")));
    assert_eq!(config.shutdown_grace, Duration::from_millis(1500));
    // `cities` is not in the file: the default stays.
    assert_eq!(config.world, WorldShape { seed: 42, cities: 2, districts: 4 });
    assert_eq!(config.gateway.join_file, Some(PathBuf::from("/srv/join.md")));
    assert_eq!(config.gateway.cors_origins, ["https://viewer.example"]);
    assert_eq!(config.gateway.limits.residents_per_owner, 5);
    assert_eq!(config.gateway.webhooks.attempts, 2);
    // Values of the same tables that the file does not name stay.
    let plain = ServerConfig::default();
    assert_eq!(config.gateway.limits.max_plan, plain.gateway.limits.max_plan);
    assert_eq!(config.gateway.webhooks.timeout_ms, plain.gateway.webhooks.timeout_ms);
    // The token is kept as a hash that takes only the same token.
    let admin = config.gateway.admin_token.expect("an admin token");
    assert!(admin.matches("an-admin-token-from-the-file"));
    assert!(!admin.matches("an-admin-token-from-the-fild"));
    assert!(!format!("{admin:?}").contains("an-admin-token"));
}

#[test]
fn the_order_is_defaults_file_development_mode_then_overrides() {
    let dir = TempDir::new();
    let file = dir.write(
        "server.toml",
        "tick_ms = 250\nstore = \"sqlite\"\n[world]\nseed = 5\n[dev]\ntick_ms = 20\n",
    );
    let dev = || Overrides { dev: true, ..Overrides::default() };

    // Without the mode, the `[dev]` table of the file does nothing.
    let config = load(Some(&file), Overrides::default());
    assert_eq!(config.tick_interval, Duration::from_millis(250));
    assert!(matches!(config.store, StoreConfig::Sqlite(_)));

    // With the mode, `[dev]` replaces the file: its own value for the
    // tick, the built-in value for the store.
    let config = load(Some(&file), dev());
    assert_eq!(config.tick_interval, Duration::from_millis(20));
    assert_eq!(config.store, StoreConfig::Memory);
    assert_eq!(config.world.seed, 5);

    // The environment and the command line replace everything.
    let overrides = Overrides {
        bind: Some("127.0.0.1:7001".parse().expect("an address")),
        tick_ms: Some(7),
        seed: Some(99),
        cities: Some(1),
        districts: Some(2),
        web_dir: Some(PathBuf::from("/srv/web")),
        data_dir: Some(PathBuf::from("/srv/data")),
        store: Some(StoreKind::Sqlite),
        join_file: Some(PathBuf::from("/srv/join.md")),
        admin_token: Some("an-admin-token-from-a-flag".into()),
        ..dev()
    };
    let config = load(Some(&file), overrides);
    assert!(config.dev);
    assert_eq!(config.bind, "127.0.0.1:7001".parse().expect("an address"));
    assert_eq!(config.tick_interval, Duration::from_millis(7));
    assert_eq!(config.world, WorldShape { seed: 99, cities: 1, districts: 2 });
    assert_eq!(config.web_dir, Some(PathBuf::from("/srv/web")));
    assert_eq!(config.store, StoreConfig::Sqlite(PathBuf::from("/srv/data/store.sqlite3")));
    assert_eq!(config.gateway.join_file, Some(PathBuf::from("/srv/join.md")));
    assert!(config.gateway.admin_token.is_some_and(|t| t.matches("an-admin-token-from-a-flag")));
}

#[test]
fn the_join_file_of_the_gateway_table_wins_over_the_one_at_the_top() {
    let dir = TempDir::new();
    let file =
        dir.write("server.toml", "join_file = \"top.md\"\n[gateway]\njoin_file = \"inner.md\"\n");
    let config = load(Some(&file), Overrides::default());
    assert_eq!(config.gateway.join_file, Some(PathBuf::from("inner.md")));
}

#[test]
fn a_configuration_that_is_not_valid_is_refused_with_its_reason() {
    let dir = TempDir::new();
    let cases = [
        ("tick = 5\n", "unknown field `tick`"),
        ("[world]\nsize = 3\n", "unknown field `size`"),
        ("[gateway]\ncolour = \"red\"\n", "unknown field `colour`"),
        ("[gateway.limits]\nmax_plans = 3\n", "unknown field `max_plans`"),
        ("tick_ms = \"fast\"\n", "invalid type: string \"fast\""),
        ("bind = \"nowhere\"\n", "invalid socket address syntax"),
        ("store = \"postgres\"\n", "unknown variant `postgres`"),
        ("[world]\ncities = 0\n", "`world.cities` must be at least 1"),
        ("[world]\ndistricts = 0\n", "`world.districts` must be at least 1"),
        ("[gateway]\nadmin_token = \"short\"\n", "the admin token must have at least 16 bytes"),
    ];
    for (text, reason) in cases {
        let file = dir.write("server.toml", text);
        let message = refusal(Some(&file), Overrides::default());
        assert!(message.contains(reason), "{text:?} gave: {message}");
    }

    let file = dir.write("server.toml", "bind = [\n");
    let message = refusal(Some(&file), Overrides::default());
    assert!(message.contains("server.toml is not valid TOML"), "{message}");

    let missing = dir.path().join("no-such-file.toml");
    let message = refusal(Some(&missing), Overrides::default());
    assert!(message.contains("cannot read the configuration file"), "{message}");
    assert!(message.contains("no-such-file.toml"), "{message}");

    // A value of the command line is checked in the same way.
    let short = Overrides { admin_token: Some("short".into()), ..Overrides::default() };
    let message = refusal(None, short);
    assert!(message.contains("the admin token must have at least 16 bytes"), "{message}");
    assert!(!message.contains("short\""), "the message holds the token: {message}");
}
