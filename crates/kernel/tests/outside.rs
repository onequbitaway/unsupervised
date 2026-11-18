//! Messages from outside the simulation: `Envelope::module`,
//! `Blueprint::handles_message` and what `Input::Deliver` does with a
//! message that no handler can take (architecture.md, section 5.6).

mod common;

use common::{Demo, GlobalLog, Hello, HelloBack, Poke, ZoneLog, world};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Builder, Dest, Envelope, Message, NoArgs, Plugin, RealmId, ResidentId, Resource, Scope,
    VerbSpec, ZoneId,
};

/// A message with the kind of `Hello` and another shape.
#[derive(Serialize, Deserialize)]
struct WrongHello {
    n: String,
}

impl Message for WrongHello {
    const KIND: &'static str = "demo.hello";
}

/// A message with the kind of `Poke` and another shape.
#[derive(Serialize, Deserialize)]
struct WrongPoke {
    n: Vec<String>,
}

impl Message for WrongPoke {
    const KIND: &'static str = "demo.poke";
}

/// A message that no module takes.
#[derive(Serialize, Deserialize)]
struct Unheard {
    text: String,
}

impl Message for Unheard {
    const KIND: &'static str = "demo.unheard";
}

fn hellos(w: &TestWorld<'_>) -> Vec<(ZoneId, u32)> {
    w.global().res::<GlobalLog>().hellos.clone()
}

#[test]
fn an_envelope_from_outside_reaches_the_handler() {
    let mut w = world();
    w.step(2);
    let before = hellos(&w);
    assert_eq!(before.len(), 4, "the hellos of world generation");

    let envelope = Envelope::module(RealmId::Zone(ZoneId(2)), Dest::Global, &Hello { n: 9 });
    assert_eq!(
        (envelope.kind(), envelope.from, envelope.to, envelope.seq),
        ("demo.hello", RealmId::Zone(ZoneId(2)), Dest::Global, 0)
    );
    // It is an envelope like any other: it survives the wire.
    let envelope = Envelope::from_bytes(&envelope.to_bytes()).expect("an envelope decodes");
    assert_eq!(envelope.kind(), "demo.hello");

    w.world.deliver_external(vec![envelope]);
    assert_eq!(hellos(&w), before, "a message waits for the next tick of its target");
    w.step(1);
    // The handler of the global realm got the typed message with the
    // sender that the envelope names, and answered that zone.
    assert_eq!(hellos(&w)[4..], [(ZoneId(2), 9)]);
    w.step(1);
    assert_eq!(w.zone(ZoneId(2)).res::<ZoneLog>().hello_back, [102, 109]);

    // To one zone, to every zone and to a resident.
    let ada = w.spawn("Ada");
    w.world.deliver_external(vec![
        Envelope::module(RealmId::Global, Dest::Zone(ZoneId(1)), &HelloBack { n: 7 }),
        Envelope::module(RealmId::Global, Dest::AllZones, &HelloBack { n: 8 }),
        Envelope::module(RealmId::Global, Dest::Resident(ada), &Poke { n: 5 }),
    ]);
    w.step(1);
    assert_eq!(w.zone(ZoneId(1)).res::<ZoneLog>().hello_back, [101, 7, 8]);
    assert_eq!(w.zone(ZoneId(3)).res::<ZoneLog>().hello_back, [103, 8]);
    assert_eq!(w.zone(ZoneId(0)).res::<ZoneLog>().resident_messages, [(ada, 5)]);
    assert_eq!(w.world.dropped_messages(), 0);
}

#[test]
fn the_blueprint_tells_which_messages_have_a_handler() {
    let w = world();
    let bp = w.world.blueprint();
    // `Hello` goes from the zones to the global realm, `HelloBack` the
    // other way, and `Poke` goes to a resident.
    assert!(bp.handles_message(Hello::KIND, Scope::Global));
    assert!(!bp.handles_message(Hello::KIND, Scope::Zone));
    assert!(bp.handles_message(HelloBack::KIND, Scope::Zone));
    assert!(!bp.handles_message(HelloBack::KIND, Scope::Global));
    assert!(bp.handles_resident_message(Poke::KIND));
    assert!(
        !bp.handles_message(Poke::KIND, Scope::Zone),
        "a resident handler takes no zone message"
    );
    assert!(!bp.handles_resident_message(Hello::KIND));
    for scope in [Scope::Zone, Scope::Global] {
        assert!(!bp.handles_message(Unheard::KIND, scope));
    }
    assert!(!bp.handles_resident_message(Unheard::KIND));
}

#[test]
fn a_delivered_message_that_no_handler_can_take_is_dropped_and_counted() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.step(2);
    let before = hellos(&w);
    let hash_of_twin = {
        // A world that got no message at all, after the same frames.
        let mut twin = world();
        twin.spawn("Ada");
        twin.step(5);
        twin.hash()
    };

    // No handler takes the kind in the target realm.
    w.world.deliver_external(vec![
        Envelope::module(RealmId::Global, Dest::Global, &Unheard { text: "anybody?".into() }),
        Envelope::module(RealmId::Global, Dest::Zone(ZoneId(1)), &Hello { n: 1 }),
        Envelope::module(RealmId::Global, Dest::Resident(ada), &Hello { n: 2 }),
    ]);
    assert_eq!(w.world.dropped_messages(), 0, "a message is dropped when it is delivered");
    w.step(1);
    assert_eq!(w.world.dropped_messages(), 3);

    // The payload does not decode into the type of the handler.
    w.world.deliver_external(vec![
        Envelope::module(RealmId::Zone(ZoneId(0)), Dest::Global, &WrongHello { n: "nine".into() }),
        Envelope::module(RealmId::Global, Dest::Resident(ada), &WrongPoke { n: vec!["x".into()] }),
    ]);
    w.step(1);
    assert_eq!(w.world.dropped_messages(), 5);
    // A message for a resident or a zone that does not exist was dropped
    // before this change too.
    w.world.deliver_external(vec![
        Envelope::module(RealmId::Global, Dest::Resident(ResidentId(99)), &Poke { n: 1 }),
        Envelope::module(RealmId::Global, Dest::Zone(ZoneId(99)), &HelloBack { n: 1 }),
    ]);
    assert_eq!(w.world.dropped_messages(), 7);

    // No handler ran, and the world is the world that got no message.
    w.step(1);
    assert_eq!(hellos(&w), before);
    assert!(w.zone(ZoneId(0)).res::<ZoneLog>().resident_messages.is_empty());
    assert_eq!(w.hash(), hash_of_twin);
    w.assert_conserved();
}

#[test]
fn a_delivered_message_is_still_from_outside_after_a_snapshot() {
    let mut w = world();
    w.step(2);
    w.world.deliver_external(vec![Envelope::module(
        RealmId::Zone(ZoneId(0)),
        Dest::Global,
        &WrongHello { n: "nine".into() },
    )]);
    // The message waits in the inbox of the global realm. A world that is
    // restored now must drop it too, not stop.
    w.snapshot_restore();
    assert_eq!(w.world.dropped_messages(), 0, "the count is not simulation state");
    w.step(1);
    assert_eq!(w.world.dropped_messages(), 1);
    assert_eq!(hellos(&w).len(), 4);
}

// ---- A module that sends what its own handler cannot read ---------------------------

fn as_text<S: Serializer>(n: &u32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&n.to_string())
}

/// A message that is written as text and read as a number: it never reads
/// back.
#[derive(Serialize, Deserialize)]
struct OneWay {
    #[serde(serialize_with = "as_text")]
    n: u32,
}

impl Message for OneWay {
    const KIND: &'static str = "trap.one_way";
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Caught {
    n: u32,
}

impl Resource for Caught {
    const NAME: &'static str = "trap.caught";
}

struct Trap;

impl Plugin for Trap {
    fn name(&self) -> &'static str {
        "trap"
    }

    fn build(&self, b: &mut Builder) {
        if b.scope() == Scope::Global {
            b.resource::<Caught>();
            b.on_message::<OneWay>(|realm, _from, message| {
                realm.res_mut::<Caught>().n += message.n;
            });
        }
        b.instant_verb::<NoArgs>(
            VerbSpec {
                verb: "trap",
                module: "trap",
                summary: "Send a message that cannot be read.",
                free: false,
                allowed_in: &[],
            },
            |realm, _ctx, _args| {
                realm.send(Dest::Global, &OneWay { n: 1 });
                Ok(Value::Null)
            },
        );
    }
}

#[test]
#[should_panic(expected = "message `trap.one_way` from zone 0: cannot decode trap.one_way")]
fn a_message_of_a_module_that_its_handler_cannot_read_is_a_bug() {
    let mut w = TestWorld::new(&[&Demo, &Trap]);
    let ada = w.spawn("Ada");
    w.act_ok(ada, "trap", json!({}));
    w.step(1);
}

#[test]
fn the_same_message_from_outside_is_dropped() {
    let mut w = TestWorld::new(&[&Demo, &Trap]);
    assert!(w.world.blueprint().handles_message(OneWay::KIND, Scope::Global));
    w.world.deliver_external(vec![Envelope::module(
        RealmId::Global,
        Dest::Global,
        &OneWay { n: 1 },
    )]);
    w.step(2);
    assert_eq!(w.world.dropped_messages(), 1);
    assert_eq!(w.global().res::<Caught>().n, 0);
}
