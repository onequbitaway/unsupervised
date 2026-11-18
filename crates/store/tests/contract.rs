//! The contract of [`Store`], checked for every engine.

use unsup_protocol::{OwnerId, ResidentId};
use unsup_store::{
    MemoryStore, NewOwner, NewResident, Principal, Secret, SqliteStore, Store, StoreCounts,
    StoreError, Token, TokenKind,
};

fn new_owner(subject: &str) -> NewOwner {
    NewOwner { name: format!("Owner {subject}"), provider: "local".into(), subject: subject.into() }
}

/// Makes an owner and returns its id and its token.
fn owner(store: &dyn Store, subject: &str) -> (OwnerId, Token) {
    let token = Token::generate(TokenKind::Owner).expect("random bytes");
    let owner =
        store.create_owner(&new_owner(subject), &token.hash(), &token.hint(), 100).expect("owner");
    (owner.id, token)
}

/// Makes a resident record and returns its key.
fn resident(store: &dyn Store, owner: OwnerId, id: u64, name: &str) -> Token {
    let key = Token::generate(TokenKind::Resident).expect("random bytes");
    let new = NewResident { resident: ResidentId(id), owner, name: name.into(), model: None };
    store.create_resident(&new, &key.hash(), &key.hint(), 200).expect("resident");
    key
}

fn owners(store: &dyn Store) {
    let (ada, ada_token) = owner(store, "ada");
    let (bob, bob_token) = owner(store, "bob");
    assert_eq!((ada, bob), (OwnerId(1), OwnerId(2)));

    let read = store.owner(ada).expect("read").expect("ada exists");
    assert_eq!(
        (read.name.as_str(), read.provider.as_str(), read.subject.as_str(), read.created_at),
        ("Owner ada", "local", "ada", 100)
    );
    assert_eq!(store.owner(OwnerId(9)).expect("read"), None);
    assert_eq!(store.owner_by_identity("local", "bob").expect("read").map(|o| o.id), Some(bob));
    assert_eq!(store.owner_by_identity("github", "bob").expect("read"), None);

    assert_eq!(store.authenticate(&ada_token.hash()).expect("auth"), Some(Principal::Owner(ada)));
    assert_eq!(store.authenticate(&bob_token.hash()).expect("auth"), Some(Principal::Owner(bob)));
    let stranger = Token::generate(TokenKind::Owner).expect("random bytes");
    assert_eq!(store.authenticate(&stranger.hash()).expect("auth"), None);

    // The same identity cannot sign up as a second owner.
    let again = store.create_owner(&new_owner("ada"), &stranger.hash(), &stranger.hint(), 101);
    assert!(matches!(again, Err(StoreError::Conflict(_))), "{again:?}");
    assert_eq!(store.authenticate(&stranger.hash()).expect("auth"), None);
    // A token cannot belong to two owners.
    let reused = store.create_owner(&new_owner("eve"), &ada_token.hash(), &ada_token.hint(), 101);
    assert!(matches!(reused, Err(StoreError::Conflict(_))), "{reused:?}");
    assert_eq!(store.owner_by_identity("local", "eve").expect("read"), None);

    // A second token for the same owner; the first keeps working.
    let second = Token::generate(TokenKind::Owner).expect("random bytes");
    store.add_owner_token(ada, &second.hash(), &second.hint(), 102).expect("second token");
    assert_eq!(store.authenticate(&second.hash()).expect("auth"), Some(Principal::Owner(ada)));
    assert_eq!(store.authenticate(&ada_token.hash()).expect("auth"), Some(Principal::Owner(ada)));
    let nobody = store.add_owner_token(OwnerId(9), &stranger.hash(), &stranger.hint(), 102);
    assert_eq!(nobody, Err(StoreError::NotFound("owner 9".into())));
    let twice = store.add_owner_token(bob, &second.hash(), &second.hint(), 102);
    assert!(matches!(twice, Err(StoreError::Conflict(_))), "{twice:?}");
}

fn residents(store: &dyn Store) {
    let (ada, _) = owner(store, "ada");
    let (bob, _) = owner(store, "bob");
    let key_7 = resident(store, ada, 7, "Seven");
    let key_3 = resident(store, ada, 3, "Three");
    let key_5 = resident(store, bob, 5, "Five");

    let listed = store.residents_of(ada).expect("list");
    assert_eq!(
        listed.iter().map(|r| (r.resident.0, r.name.as_str())).collect::<Vec<_>>(),
        [(3, "Three"), (7, "Seven")]
    );
    assert_eq!(listed[1].key_hint, key_7.hint());
    assert_eq!((listed[0].owner, listed[0].created_at, listed[0].model.clone()), (ada, 200, None));
    assert_eq!(store.count_residents(ada).expect("count"), 2);
    assert_eq!(store.count_residents(bob).expect("count"), 1);
    assert_eq!(store.count_residents(OwnerId(9)).expect("count"), 0);
    assert_eq!(store.resident(ResidentId(5)).expect("read").map(|r| r.owner), Some(bob));
    assert_eq!(store.resident(ResidentId(6)).expect("read"), None);

    assert_eq!(
        store.authenticate(&key_3.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(3), owner: ada })
    );
    assert_eq!(
        store.authenticate(&key_5.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(5), owner: bob })
    );

    // A live resident id is unique. The refused call leaves no key behind.
    let key = Token::generate(TokenKind::Resident).expect("random bytes");
    let twin =
        NewResident { resident: ResidentId(3), owner: bob, name: "Twin".into(), model: None };
    let refused = store.create_resident(&twin, &key.hash(), &key.hint(), 201);
    assert_eq!(refused, Err(StoreError::Conflict("resident 3".into())));
    assert_eq!(store.authenticate(&key.hash()).expect("auth"), None);
    // The owner must exist.
    let orphan =
        NewResident { resident: ResidentId(8), owner: OwnerId(9), name: "X".into(), model: None };
    let refused = store.create_resident(&orphan, &key.hash(), &key.hint(), 201);
    assert_eq!(refused, Err(StoreError::NotFound("owner 9".into())));
    // A key cannot open two residents.
    let copy = NewResident { resident: ResidentId(8), owner: ada, name: "X".into(), model: None };
    let refused = store.create_resident(&copy, &key_5.hash(), &key_5.hint(), 201);
    assert!(matches!(refused, Err(StoreError::Conflict(_))), "{refused:?}");
    assert_eq!(store.resident(ResidentId(8)).expect("read"), None);

    // The model is kept.
    let with_model = NewResident {
        resident: ResidentId(9),
        owner: bob,
        name: "Nine".into(),
        model: Some("model-x".into()),
    };
    let record = store.create_resident(&with_model, &key.hash(), &key.hint(), 202).expect("nine");
    assert_eq!(record.model.as_deref(), Some("model-x"));
    let read = store.resident(ResidentId(9)).expect("read").expect("nine exists");
    assert_eq!(read, record);
}

fn rotation(store: &dyn Store) {
    let (ada, _) = owner(store, "ada");
    let old = resident(store, ada, 4, "Four");
    let other = resident(store, ada, 6, "Six");

    let new = Token::generate(TokenKind::Resident).expect("random bytes");
    store.rotate_resident_key(ResidentId(4), &new.hash(), &new.hint(), 300).expect("rotate");
    assert_eq!(store.authenticate(&old.hash()).expect("auth"), None);
    assert_eq!(
        store.authenticate(&new.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(4), owner: ada })
    );
    let record = store.resident(ResidentId(4)).expect("read").expect("four exists");
    assert_eq!(record.key_hint, new.hint());
    // The key of another resident is not touched.
    assert_eq!(
        store.authenticate(&other.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(6), owner: ada })
    );

    let fresh = Token::generate(TokenKind::Resident).expect("random bytes");
    let missing = store.rotate_resident_key(ResidentId(5), &fresh.hash(), &fresh.hint(), 301);
    assert_eq!(missing, Err(StoreError::NotFound("resident 5".into())));
    // A rotation to a key that exists changes nothing.
    let taken = store.rotate_resident_key(ResidentId(4), &other.hash(), &other.hint(), 301);
    assert!(matches!(taken, Err(StoreError::Conflict(_))), "{taken:?}");
    assert_eq!(
        store.authenticate(&new.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(4), owner: ada })
    );
}

fn webhooks(store: &dyn Store) {
    let (ada, _) = owner(store, "ada");
    resident(store, ada, 2, "Two");
    resident(store, ada, 1, "One");
    let secret = Secret::generate_webhook().expect("random bytes");
    let other = Secret::generate_webhook().expect("random bytes");

    assert_eq!(store.webhook(ResidentId(1)).expect("read"), None);
    store.set_webhook(ResidentId(2), "https://two.example/hook", &secret, 400).expect("set");
    store.set_webhook(ResidentId(1), "https://one.example/hook", &secret, 401).expect("set");
    let read = store.webhook(ResidentId(2)).expect("read").expect("a webhook");
    assert_eq!(
        (read.resident, read.url.as_str(), read.secret.reveal(), read.created_at),
        (ResidentId(2), "https://two.example/hook", secret.reveal(), 400)
    );
    // A second call replaces the address and the secret.
    store.set_webhook(ResidentId(2), "https://two.example/new", &other, 402).expect("replace");
    let all = store.webhooks().expect("list");
    assert_eq!(
        all.iter().map(|w| (w.resident.0, w.url.as_str(), w.created_at)).collect::<Vec<_>>(),
        [(1, "https://one.example/hook", 401), (2, "https://two.example/new", 402)]
    );
    assert_eq!(all[1].secret.reveal(), other.reveal());

    assert_eq!(store.clear_webhook(ResidentId(1)), Ok(true));
    assert_eq!(store.clear_webhook(ResidentId(1)), Ok(false));
    assert_eq!(store.webhook(ResidentId(1)).expect("read"), None);
    assert_eq!(
        store.set_webhook(ResidentId(3), "https://x.example", &secret, 403),
        Err(StoreError::NotFound("resident 3".into()))
    );
    assert_eq!(store.clear_webhook(ResidentId(3)), Ok(false));
}

fn retirement(store: &dyn Store) {
    let (ada, ada_token) = owner(store, "ada");
    let (bob, _) = owner(store, "bob");
    let key_1 = resident(store, ada, 1, "One");
    let key_2 = resident(store, ada, 2, "Two");
    let key_3 = resident(store, bob, 3, "Three");
    let secret = Secret::generate_webhook().expect("random bytes");
    for id in [1, 2, 3] {
        store.set_webhook(ResidentId(id), "https://example.org/hook", &secret, 500).expect("set");
    }

    assert_eq!(store.retire_resident(ResidentId(1), "died", 501), Ok(true));
    assert_eq!(store.retire_resident(ResidentId(1), "died", 502), Ok(false));
    assert_eq!(store.retire_resident(ResidentId(9), "died", 502), Ok(false));
    assert_eq!(store.authenticate(&key_1.hash()).expect("auth"), None);
    assert_eq!(store.resident(ResidentId(1)).expect("read"), None);
    assert_eq!(store.webhook(ResidentId(1)).expect("read"), None);
    assert_eq!(store.count_residents(ada).expect("count"), 1);
    // The other records are as before.
    assert_eq!(
        store.authenticate(&key_2.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(2), owner: ada })
    );
    assert_eq!(
        store.webhooks().expect("list").iter().map(|w| w.resident.0).collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        store.counts().expect("counts"),
        StoreCounts { owners: 2, residents: 2, retired: 1, webhooks: 2 }
    );

    // A new world can give the id of a retired resident to a new one.
    let key_1b = resident(store, bob, 1, "One again");
    assert_eq!(
        store.authenticate(&key_1b.hash()).expect("auth"),
        Some(Principal::Resident { resident: ResidentId(1), owner: bob })
    );
    assert_eq!(store.authenticate(&key_1.hash()).expect("auth"), None);

    assert_eq!(store.retire_all_residents("world_reset", 503), Ok(3));
    assert_eq!(store.retire_all_residents("world_reset", 504), Ok(0));
    for key in [&key_1b, &key_2, &key_3] {
        assert_eq!(store.authenticate(&key.hash()).expect("auth"), None);
    }
    assert_eq!(store.residents_of(ada).expect("list"), []);
    assert_eq!(store.webhooks().expect("list"), []);
    assert_eq!(
        store.counts().expect("counts"),
        StoreCounts { owners: 2, residents: 0, retired: 4, webhooks: 0 }
    );
    // Owners and their tokens outlive the world.
    assert_eq!(store.authenticate(&ada_token.hash()).expect("auth"), Some(Principal::Owner(ada)));
}

fn settings(store: &dyn Store) {
    assert_eq!(store.setting("motd"), Ok(None));
    store.set_setting("motd", "hello").expect("set");
    store.set_setting("other", "1").expect("set");
    assert_eq!(store.setting("motd"), Ok(Some("hello".into())));
    store.set_setting("motd", "bye").expect("replace");
    assert_eq!(store.setting("motd"), Ok(Some("bye".into())));
    assert_eq!(store.setting("other"), Ok(Some("1".into())));
    assert_eq!(store.clear_setting("motd"), Ok(true));
    assert_eq!(store.clear_setting("motd"), Ok(false));
    assert_eq!(store.setting("motd"), Ok(None));
}

/// Runs every scenario once for each engine, each on an empty store.
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

for_each_engine!(owners, residents, rotation, webhooks, retirement, settings);
