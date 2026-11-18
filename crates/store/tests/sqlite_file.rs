//! The SQLite engine on a real file: persistence, the schema version and
//! what the file holds.

use std::path::PathBuf;

use unsup_protocol::ResidentId;
use unsup_store::{
    NewOwner, NewResident, Principal, Secret, SqliteStore, Store, StoreError, Token, TokenKind,
    random_hex,
};

/// A directory in the temporary directory of the system. It is removed
/// when the value is dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let name = format!("unsup-store-test-{}", random_hex(8).expect("random bytes"));
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir(&path).expect("a new directory");
        TempDir(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ada() -> NewOwner {
    NewOwner { name: "Ada".into(), provider: "local".into(), subject: "ada".into() }
}

#[test]
fn the_file_keeps_the_data_and_never_a_token() {
    let dir = TempDir::new();
    let path = dir.file("store.sqlite3");
    let token = Token::generate(TokenKind::Owner).expect("random bytes");
    let key = Token::generate(TokenKind::Resident).expect("random bytes");
    let secret = Secret::generate_webhook().expect("random bytes");
    {
        let store = SqliteStore::open(&path).expect("a new file");
        assert_eq!(store.schema_version(), Ok(1));
        let owner = store.create_owner(&ada(), &token.hash(), &token.hint(), 10).expect("owner");
        let new = NewResident {
            resident: ResidentId(1),
            owner: owner.id,
            name: "One".into(),
            model: Some("model-x".into()),
        };
        store.create_resident(&new, &key.hash(), &key.hint(), 11).expect("resident");
        store.set_webhook(ResidentId(1), "https://example.org/hook", &secret, 12).expect("hook");
        store.set_setting("motd", "hello").expect("setting");
    }

    // A second open finds everything and does not run the migration again.
    let store = SqliteStore::open(&path).expect("the same file");
    assert_eq!(store.schema_version(), Ok(1));
    let owner = store.owner_by_identity("local", "ada").expect("read").expect("ada exists");
    assert_eq!((owner.name.as_str(), owner.created_at), ("Ada", 10));
    assert_eq!(store.authenticate(&token.hash()).expect("auth"), Some(Principal::Owner(owner.id)));
    assert_eq!(
        store.authenticate(&key.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(1), owner: owner.id })
    );
    let record = store.resident(ResidentId(1)).expect("read").expect("one exists");
    assert_eq!((record.name.as_str(), record.model.as_deref()), ("One", Some("model-x")));
    let hook = store.webhook(ResidentId(1)).expect("read").expect("a webhook");
    assert_eq!(hook.secret.reveal(), secret.reveal());
    assert_eq!(store.setting("motd"), Ok(Some("hello".into())));
    drop(store);

    // The bytes on disk hold the hashes and the hints, never a token.
    let mut bytes = std::fs::read(&path).expect("the database file");
    for extra in ["store.sqlite3-wal", "store.sqlite3-shm"] {
        bytes.extend(std::fs::read(dir.file(extra)).unwrap_or_default());
    }
    let holds = |needle: &[u8]| bytes.windows(needle.len()).any(|window| window == needle);
    assert!(holds(token.hash().as_bytes()), "the hash of the owner token is in the file");
    assert!(holds(key.hash().as_bytes()), "the hash of the key is in the file");
    assert!(holds(key.hint().as_bytes()), "the hint of the key is in the file");
    for secret_text in [token.reveal(), key.reveal()] {
        assert!(!holds(secret_text.as_bytes()), "a token is in the file");
        // Not even the random part alone.
        let random_part = &secret_text.as_bytes()["unsup_xxx_".len()..];
        assert_eq!(random_part.len(), 64);
        assert!(!holds(random_part), "the random part of a token is in the file");
    }
}

#[test]
fn a_file_of_a_newer_build_is_refused() {
    let dir = TempDir::new();
    let path = dir.file("store.sqlite3");
    drop(SqliteStore::open(&path).expect("a new file"));
    let conn = rusqlite::Connection::open(&path).expect("the file");
    conn.pragma_update(None, "user_version", 99).expect("a later version");
    drop(conn);

    let error = SqliteStore::open(&path).expect_err("a newer schema");
    let StoreError::Engine(message) = error else { panic!("an engine error, not {error:?}") };
    assert!(message.contains("schema version 99"), "{message}");
}

#[test]
fn a_path_that_cannot_be_a_file_is_an_error() {
    let dir = TempDir::new();
    let error = SqliteStore::open(&dir.file("missing/store.sqlite3")).expect_err("no directory");
    assert!(matches!(error, StoreError::Engine(_)), "{error:?}");
}
