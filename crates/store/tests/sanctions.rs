//! Sanctions and the list of live residents, checked for every engine,
//! and across a new open of a SQLite file.

use std::path::PathBuf;

use unsup_protocol::{OwnerId, ResidentId};
use unsup_store::{
    MemoryStore, NewOwner, NewResident, Sanction, SanctionRecord, SqliteStore, Store, StoreError,
    Token, TokenKind, random_hex,
};

fn owner(store: &dyn Store, subject: &str) -> OwnerId {
    let token = Token::generate(TokenKind::Owner).expect("random bytes");
    let new = NewOwner { name: subject.into(), provider: "local".into(), subject: subject.into() };
    store.create_owner(&new, &token.hash(), &token.hint(), 100).expect("owner").id
}

fn resident(store: &dyn Store, owner: OwnerId, id: u64) {
    let key = Token::generate(TokenKind::Resident).expect("random bytes");
    let new = NewResident { resident: ResidentId(id), owner, name: format!("R{id}"), model: None };
    store.create_resident(&new, &key.hash(), &key.hint(), 200).expect("resident");
}

fn record(sanction: Sanction, reason: &str, created_at: u64) -> SanctionRecord {
    SanctionRecord { sanction, reason: reason.into(), created_at }
}

fn imposing_and_lifting(store: &dyn Store) {
    let ada = owner(store, "ada");
    let bob = owner(store, "bob");
    resident(store, ada, 3);
    resident(store, bob, 7);
    assert_eq!(store.sanctions(), Ok(Vec::new()));

    // A sanction needs its subject.
    for missing in [Sanction::Mute(ResidentId(9)), Sanction::Freeze(ResidentId(9))] {
        let refused = store.impose(missing, "no such resident", 300);
        assert_eq!(refused, Err(StoreError::NotFound("resident 9".into())));
    }
    let refused = store.impose(Sanction::Ban(OwnerId(9)), "no such owner", 300);
    assert_eq!(refused, Err(StoreError::NotFound("owner 9".into())));
    assert_eq!(store.sanctions(), Ok(Vec::new()));

    assert_eq!(store.impose(Sanction::Ban(bob), "sold accounts", 301), Ok(true));
    assert_eq!(store.impose(Sanction::Freeze(ResidentId(7)), "", 302), Ok(true));
    assert_eq!(store.impose(Sanction::Mute(ResidentId(7)), "spam", 303), Ok(true));
    let long_reason = "spam: 3 posts\nwith a link \u{2014} \u{00e9}t\u{00e9}";
    assert_eq!(store.impose(Sanction::Mute(ResidentId(3)), long_reason, 304), Ok(true));
    // A second time changes nothing: the first reason and time stay.
    assert_eq!(store.impose(Sanction::Mute(ResidentId(7)), "another reason", 999), Ok(false));

    // Mutes, then freezes, then bans, each by ascending id.
    assert_eq!(
        store.sanctions(),
        Ok(vec![
            record(Sanction::Mute(ResidentId(3)), long_reason, 304),
            record(Sanction::Mute(ResidentId(7)), "spam", 303),
            record(Sanction::Freeze(ResidentId(7)), "", 302),
            record(Sanction::Ban(bob), "sold accounts", 301),
        ])
    );

    // A mute of a resident is not a mute of the owner with the same number.
    assert_eq!(store.lift(Sanction::Ban(OwnerId(3))), Ok(false));
    assert_eq!(store.lift(Sanction::Mute(ResidentId(3))), Ok(true));
    assert_eq!(store.lift(Sanction::Mute(ResidentId(3))), Ok(false));
    assert_eq!(store.lift(Sanction::Ban(bob)), Ok(true));
    assert_eq!(
        store.sanctions(),
        Ok(vec![
            record(Sanction::Mute(ResidentId(7)), "spam", 303),
            record(Sanction::Freeze(ResidentId(7)), "", 302),
        ])
    );
    // The settings of a caller are not touched, and do not show them.
    store.set_setting("motd", "hello").expect("set");
    assert_eq!(store.setting("motd"), Ok(Some("hello".into())));
    assert_eq!(store.sanctions().map(|list| list.len()), Ok(2));
}

fn the_end_of_a_record_ends_its_mute_and_its_freeze(store: &dyn Store) {
    let ada = owner(store, "ada");
    for id in [1, 2, 3] {
        resident(store, ada, id);
    }
    assert_eq!(store.resident_ids(), Ok(vec![ResidentId(1), ResidentId(2), ResidentId(3)]));
    for sanction in [
        Sanction::Mute(ResidentId(1)),
        Sanction::Freeze(ResidentId(1)),
        Sanction::Mute(ResidentId(2)),
        Sanction::Freeze(ResidentId(3)),
        Sanction::Ban(ada),
    ] {
        assert_eq!(store.impose(sanction, "why", 300), Ok(true));
    }

    assert_eq!(store.retire_resident(ResidentId(1), "died", 400), Ok(true));
    assert_eq!(store.resident_ids(), Ok(vec![ResidentId(2), ResidentId(3)]));
    let kept = |store: &dyn Store| -> Vec<Sanction> {
        store.sanctions().expect("the list").into_iter().map(|r| r.sanction).collect()
    };
    assert_eq!(
        kept(store),
        [Sanction::Mute(ResidentId(2)), Sanction::Freeze(ResidentId(3)), Sanction::Ban(ada)]
    );
    // A later resident with the same id starts without a sanction.
    resident(store, ada, 1);
    assert_eq!(store.lift(Sanction::Mute(ResidentId(1))), Ok(false));

    // A new world ends every mute and every freeze. The ban stays.
    assert_eq!(store.retire_all_residents("world_reset", 500), Ok(3));
    assert_eq!(store.resident_ids(), Ok(Vec::new()));
    assert_eq!(kept(store), [Sanction::Ban(ada)]);
}

macro_rules! for_each_engine {
    ($($scenario:ident),+ $(,)?) => {
        mod memory {
            $(
                #[test]
                fn $scenario() {
                    super::$scenario(&super::MemoryStore::new());
                }
            )+
        }
        mod sqlite {
            $(
                #[test]
                fn $scenario() {
                    let store = super::SqliteStore::open_in_memory().expect("an empty database");
                    super::$scenario(&store);
                }
            )+
        }
    };
}

for_each_engine!(imposing_and_lifting, the_end_of_a_record_ends_its_mute_and_its_freeze);

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
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_sqlite_file_keeps_the_sanctions_without_a_new_schema() {
    let dir = TempDir::new();
    let path = dir.0.join("store.sqlite3");
    let ada = {
        let store = SqliteStore::open(&path).expect("a new file");
        let ada = owner(&store, "ada");
        resident(&store, ada, 5);
        assert_eq!(store.impose(Sanction::Mute(ResidentId(5)), "spam", 10), Ok(true));
        assert_eq!(store.impose(Sanction::Freeze(ResidentId(5)), "", 11), Ok(true));
        assert_eq!(store.impose(Sanction::Ban(ada), "many accounts", 12), Ok(true));
        ada
    };

    let store = SqliteStore::open(&path).expect("the same file");
    // The sanctions are rows of the settings table: the schema is the
    // one that every earlier build wrote.
    assert_eq!(store.schema_version(), Ok(1));
    assert_eq!(
        store.sanctions(),
        Ok(vec![
            record(Sanction::Mute(ResidentId(5)), "spam", 10),
            record(Sanction::Freeze(ResidentId(5)), "", 11),
            record(Sanction::Ban(ada), "many accounts", 12),
        ])
    );
    assert_eq!(store.resident_ids(), Ok(vec![ResidentId(5)]));
}

#[test]
fn a_row_that_is_not_a_sanction_is_an_error_and_not_a_silent_gap() {
    let store = SqliteStore::open_in_memory().expect("an empty database");
    // A caller that writes into the reserved keys breaks the list.
    store.set_setting("sanction:mute:not-a-number", "1 x").expect("set");
    let error = store.sanctions().expect_err("the row cannot be read");
    assert_eq!(
        error,
        StoreError::Engine(
            "the setting `sanction:mute:not-a-number` does not hold a sanction".into()
        )
    );
}
