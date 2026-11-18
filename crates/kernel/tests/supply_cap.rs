//! The supply cap: no realm creates more than `SUPPLY_CAP`, so a world
//! where every realm is at the cap still has room in one balance.

mod common;

use common::Demo;
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Builder, CityConfig, Dest, GRANT_FAUCET, Holder, ItemDef, LedgerError, MAX_ZONES, Money,
    Plugin, Pos, RealmId, SUPPLY_CAP, UNCLAIMED, World, WorldConfig, ZoneId,
};

/// Where each realm keeps what it created.
const VAULT: Holder = Holder::Account(1);
/// The one balance that gets everything.
const HOARD: Holder = Holder::Account(9);

/// A module with one item and nothing else.
struct Goods;

impl Plugin for Goods {
    fn name(&self) -> &'static str {
        "goods"
    }

    fn build(&self, b: &mut Builder) {
        b.item(ItemDef::new("grain"));
    }
}

/// The realms of a world: the global realm, then the zones.
fn realm_ids(world: &World) -> Vec<RealmId> {
    let zones = world.layout().zones().iter().map(|zone| RealmId::Zone(zone.id));
    std::iter::once(RealmId::Global).chain(zones).collect()
}

#[test]
fn the_largest_world_at_the_cap_fits_in_one_balance() {
    // Three cities of 90 x 90, 9 x 9 and 3 x 3 zones: exactly `MAX_ZONES`.
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("Large", Pos::new(0, 0), 90),
            CityConfig::new("Middle", Pos::new(10_000, 0), 9),
            CityConfig::new("Small", Pos::new(20_000, 0), 3),
        ],
        zone_size: 8,
        ..WorldConfig::default()
    };
    let mut world = World::new(&[&Goods], config);
    let realms = realm_ids(&world);
    assert_eq!(realms.len(), MAX_ZONES + 1);
    let faucet = world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet");
    let grain = world.blueprint().item_id("grain").expect("the item of the module");

    // Every realm creates as much as it can, of money and of the item, and
    // sends all of it to one balance in the global realm.
    for &id in &realms {
        let realm = world.realm(id).expect("the world hosts every realm");
        {
            let mut ledger = realm.ledger_mut();
            assert_eq!(ledger.try_mint(faucet, VAULT, Money(SUPPLY_CAP)), Ok(()));
            assert_eq!(ledger.try_mint(faucet, VAULT, Money(1)), Err(LedgerError::Overflow));
            assert_eq!(ledger.try_produce(faucet, VAULT, grain, SUPPLY_CAP), Ok(()));
            assert_eq!(ledger.try_produce(faucet, VAULT, grain, 1), Err(LedgerError::Overflow));
        }
        if id == RealmId::Global {
            let mut ledger = realm.ledger_mut();
            ledger.transfer_money(VAULT, HOARD, Money(SUPPLY_CAP), "sweep").unwrap();
            ledger.transfer_item(VAULT, HOARD, grain, SUPPLY_CAP, "sweep").unwrap();
        } else {
            realm.wire_money(VAULT, Dest::Global, HOARD, Money(SUPPLY_CAP), "sweep").unwrap();
            realm.wire_item(VAULT, Dest::Global, HOARD, grain, SUPPLY_CAP, "sweep").unwrap();
        }
    }
    let on_the_way = MAX_ZONES as i64 * SUPPLY_CAP;
    let everything = (MAX_ZONES as i64 + 1) * SUPPLY_CAP;
    assert_eq!(everything, 2_305_561_534_236_983_296);
    let totals = world.audit().expect("the ledgers are balanced");
    assert_eq!((totals.money.held, totals.money.in_flight), (SUPPLY_CAP, on_the_way));
    assert_eq!(
        (totals.items[&grain].held, totals.items[&grain].in_flight),
        (SUPPLY_CAP, on_the_way)
    );

    // The wires arrive. A balance without room would stop the world here.
    world.step();
    let global = world.realm(RealmId::Global).expect("the world hosts the global realm");
    assert_eq!(global.ledger().money(HOARD), Money(everything));
    assert_eq!(global.ledger().qty(HOARD, grain), everything);
    let totals = world.audit().expect("the ledgers are balanced");
    assert_eq!(
        (totals.money.held, totals.money.minted, totals.money.in_flight),
        (everything, everything, 0)
    );
    assert_eq!(
        (totals.items[&grain].held, totals.items[&grain].produced, totals.items[&grain].in_flight),
        (everything, everything, 0)
    );

    // The one balance moves as a whole to another holder, and no realm can
    // add to it.
    {
        let mut ledger = global.ledger_mut();
        assert_eq!(ledger.transfer_money(HOARD, UNCLAIMED, Money(everything), "sweep"), Ok(()));
        assert_eq!(ledger.transfer_item(HOARD, UNCLAIMED, grain, everything, "sweep"), Ok(()));
        assert_eq!(ledger.money(UNCLAIMED), Money(everything));
        assert_eq!(ledger.try_mint(faucet, UNCLAIMED, Money(1)), Err(LedgerError::Overflow));
    }
    for &id in &realms {
        let realm = world.realm(id).expect("the world hosts every realm");
        realm.ledger_mut().mint(faucet, VAULT, Money(1));
        assert_eq!(realm.ledger().money(VAULT), Money(0), "{id} is at the cap");
    }
    world.audit().expect("the ledgers are balanced");
}

#[test]
fn a_resident_can_carry_inherit_and_receive_the_whole_supply() {
    let mut w = TestWorld::new(&[&Demo]);
    let ann = w.spawn_at("Ann", Pos::new(10, 10));
    let bob = w.spawn_at("Bob", Pos::new(12, 10));
    let cat = w.spawn_at("Cat", Pos::new(100, 100));
    let start = w.assert_conserved().money.minted;
    assert_eq!(start, 3_000, "each resident starts with 10.00");
    let faucet = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet");
    let coal = w.world.blueprint().item_id("coal").expect("an item of the demo module");
    let realms = realm_ids(&w.world);
    assert_eq!(realms.len(), 5);

    // Every realm creates all that it still can and sends it to Ann.
    let mut created = start;
    for &id in &realms {
        let realm = w.world.realm(id).expect("the world hosts every realm");
        let room = SUPPLY_CAP - realm.ledger().totals().money.minted;
        created += room;
        {
            let mut ledger = realm.ledger_mut();
            assert_eq!(ledger.try_mint(faucet, VAULT, Money(room + 1)), Err(LedgerError::Overflow));
            assert_eq!(ledger.try_mint(faucet, VAULT, Money(room)), Ok(()));
            assert_eq!(ledger.try_produce(faucet, VAULT, coal, SUPPLY_CAP), Ok(()));
        }
        let to = Holder::Resident(ann);
        if id == RealmId::Zone(ZoneId(0)) {
            let mut ledger = realm.ledger_mut();
            ledger.transfer_money(VAULT, to, Money(room), "sweep").unwrap();
            ledger.transfer_item(VAULT, to, coal, SUPPLY_CAP, "sweep").unwrap();
        } else {
            realm.wire_money(VAULT, Dest::Resident(ann), to, Money(room), "sweep").unwrap();
            realm.wire_item(VAULT, Dest::Resident(ann), to, coal, SUPPLY_CAP, "sweep").unwrap();
        }
    }
    assert_eq!(created, 5 * SUPPLY_CAP);
    w.step(2);
    let everything = created - 2_000;
    assert_eq!(w.cash(ann), Money(everything), "all but the 10.00 of Bob and of Cat");
    assert_eq!(w.qty(Holder::Resident(ann), "coal"), 5 * SUPPLY_CAP);

    // Ann leaves the world; Bob in the same zone inherits (`move_holdings`).
    w.with_resident(ann, |realm, idx| realm.despawn(idx, Holder::Resident(bob), "test"));
    w.world.settle();
    assert_eq!(w.cash(bob), Money(everything + 1_000));
    assert_eq!(w.qty(Holder::Resident(bob), "coal"), 5 * SUPPLY_CAP);
    w.assert_conserved();

    // Bob carries it to another zone (the holdings arrive with him), and
    // pays all of it to Cat there.
    w.act_ok(bob, "jump", json!({ "zone": 3, "x": 101, "y": 100 }));
    w.step(1);
    assert_eq!(w.zone_of(bob), ZoneId(3));
    assert_eq!(w.cash(bob), Money(everything + 1_000));
    w.act_ok(bob, "pay", json!({ "to": cat, "amount": everything + 1_000 }));
    assert_eq!((w.cash(bob), w.cash(cat)), (Money(0), Money(created)));

    // A wire of everything to a resident that no longer exists ends in the
    // unclaimed account of the global realm.
    w.with_resident(cat, |realm, _| {
        realm
            .wire_money(
                Holder::Resident(cat),
                Dest::Resident(ann),
                Holder::Resident(ann),
                Money(created),
                "lost",
            )
            .unwrap();
    });
    w.step(2);
    assert_eq!(w.global().ledger().money(UNCLAIMED), Money(created));
    let totals = w.assert_conserved();
    assert_eq!(
        (totals.money.held, totals.money.minted, totals.money.in_flight),
        (created, created, 0)
    );
    assert_eq!(totals.items[&coal].held, 5 * SUPPLY_CAP);
}
