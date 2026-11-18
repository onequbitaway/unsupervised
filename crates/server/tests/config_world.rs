//! The configuration of the durable world: where it is kept, how it is
//! written, and the parameter files.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::TempDir;
use unsup_runtime::{DurableConfig, SyncPolicy};
use unsup_server::{Overrides, ServerConfig, StoreConfig, StoreKind, WorldStore, WorldStoreKind};

fn load(file: Option<&Path>, overrides: Overrides) -> ServerConfig {
    ServerConfig::load(file, overrides).expect("a valid configuration")
}

#[test]
fn the_defaults_keep_the_world_in_the_data_directory_with_the_numbers_of_the_runtime() {
    let config = ServerConfig::default();
    assert_eq!(config.world_store, WorldStore::Dir(PathBuf::from("var/world")));
    assert_eq!(config.params, Vec::<PathBuf>::new());
    let durability = &config.durability;
    assert_eq!(
        (durability.sync, durability.snapshot_every, durability.keep_snapshots),
        (None, None, None)
    );
    assert_eq!((durability.segment_bytes, durability.drop_from), (None, None));
    assert!(durability.dilation);
    assert_eq!(durability.audit_every, 600);
    // Without a value of its own, the server writes as the runtime does.
    assert_eq!(durability.to_runtime(), DurableConfig::default());
    // No sign-in with GitHub.
    assert!(!config.gateway.github.is_on());
}

#[test]
fn accounts_in_memory_keep_the_world_in_memory_too() {
    let dev = ServerConfig::dev();
    assert_eq!((dev.store, dev.world_store), (StoreConfig::Memory, WorldStore::Memory));

    let dir = TempDir::new();
    let file = dir.write("server.toml", "store = \"memory\"\nworld_store = \"disk\"\n");
    assert_eq!(load(Some(&file), Overrides::default()).world_store, WorldStore::Memory);

    // Accounts in a file and the world in memory: a new world each start.
    let file = dir.write("server.toml", "data_dir = \"/srv/u\"\nworld_store = \"memory\"\n");
    let config = load(Some(&file), Overrides::default());
    assert_eq!(config.store, StoreConfig::Sqlite(PathBuf::from("/srv/u/store.sqlite3")));
    assert_eq!(config.world_store, WorldStore::Memory);

    // The command line wins, also in the development mode.
    let overrides = Overrides {
        dev: true,
        store: Some(StoreKind::Sqlite),
        world_store: Some(WorldStoreKind::Disk),
        data_dir: Some(PathBuf::from("/srv/data")),
        ..Overrides::default()
    };
    let config = load(Some(&file), overrides);
    assert_eq!(config.world_store, WorldStore::Dir(PathBuf::from("/srv/data/world")));
}

#[test]
fn a_file_sets_how_the_world_is_written() {
    let dir = TempDir::new();
    let file = dir.write(
        "server.toml",
        r#"
        params = ["/etc/unsupervised/cheap.toml", "night.toml"]

        [durability]
        sync = "batch"
        snapshot_every_frames = 120
        keep_snapshots = 9
        segment_bytes = 4096
        dilation = false
        audit_every_frames = 0
        drop_from = 77
        "#,
    );
    let config = load(Some(&file), Overrides::default());
    assert_eq!(
        config.params,
        [PathBuf::from("/etc/unsupervised/cheap.toml"), PathBuf::from("night.toml")]
    );
    let durability = &config.durability;
    assert!(!durability.dilation);
    assert_eq!(durability.audit_every, 0);
    let runtime = durability.to_runtime();
    assert_eq!(runtime.sync, SyncPolicy::EveryBatch);
    assert_eq!((runtime.snapshot_every, runtime.keep_snapshots), (120, 9));
    assert_eq!((runtime.segment_bytes, runtime.drop_from), (4096, Some(77)));
    // What the file does not name stays as the runtime has it.
    assert_eq!(runtime.max_record_bytes, DurableConfig::default().max_record_bytes);

    // The three ways to force the log to the disk.
    let sync = |text: &str| {
        let file = dir.write("sync.toml", &format!("[durability]\n{text}\n"));
        load(Some(&file), Overrides::default()).durability.to_runtime().sync
    };
    assert_eq!(sync("sync = \"record\""), SyncPolicy::EveryRecord);
    assert_eq!(
        sync("sync = \"interval\"\nsync_interval_ms = 250"),
        SyncPolicy::Interval(Duration::from_millis(250))
    );
    // An interval alone means the interval policy.
    assert_eq!(sync("sync_interval_ms = 20"), SyncPolicy::Interval(Duration::from_millis(20)));
    assert_eq!(sync("sync = \"interval\""), DurableConfig::default().sync);

    // The command line: parameter files replace those of the file, and
    // the sequence number to drop from.
    let overrides = Overrides {
        params: vec![PathBuf::from("flag.toml")],
        drop_from: Some(5),
        ..Overrides::default()
    };
    let config = load(Some(&file), overrides);
    assert_eq!(config.params, [PathBuf::from("flag.toml")]);
    assert_eq!(config.durability.drop_from, Some(5));
}

#[test]
fn the_github_sign_in_takes_its_id_and_secret_from_the_overrides() {
    let overrides = Overrides {
        github_client_id: Some("an-id".into()),
        github_client_secret: Some("a-secret-from-the-environment".into()),
        ..Overrides::default()
    };
    let config = load(None, overrides);
    assert!(config.gateway.github.is_on());
    assert_eq!(config.gateway.github.client_id, "an-id");
    // A print of the whole configuration does not show the secret.
    let shown = format!("{config:?}");
    assert!(!shown.contains("a-secret-from-the-environment"), "{shown}");
    // The id alone does not turn the sign-in on.
    let only_id = Overrides { github_client_id: Some("an-id".into()), ..Overrides::default() };
    assert!(!load(None, only_id).gateway.github.is_on());
}

#[test]
fn values_that_are_not_valid_are_refused_with_their_reason() {
    let dir = TempDir::new();
    let cases = [
        ("world_store = \"cloud\"\n", "unknown variant `cloud`"),
        ("params = \"one.toml\"\n", "invalid type: string"),
        ("[durability]\nsync = \"often\"\n", "unknown variant `often`"),
        ("[durability]\nsnapshots = 5\n", "unknown field `snapshots`"),
        ("[durability]\nkeep_snapshots = 0\n", "`durability.keep_snapshots` must be at least 1"),
        ("[gateway.moderation]\npatterns = [\"**\"]\n", "matches every text"),
        ("[gateway.moderation]\nlinks = \"sometimes\"\n", "unknown variant `sometimes`"),
        ("[gateway.abuse.funnel]\nmin_giver = 2\n", "unknown field `min_giver`"),
        (
            "[gateway.github]\nclient_id = \"i\"\nclient_secret = \"s\"\ntoken_url = \"nowhere\"\n",
            "`github.token_url` is not an address",
        ),
    ];
    for (text, reason) in cases {
        let file = dir.write("server.toml", text);
        let error = ServerConfig::load(Some(&file), Overrides::default()).expect_err("a refusal");
        let message = format!("{error:#}");
        assert!(message.contains(reason), "{text:?} gave: {message}");
    }
}
