//! The zone reports of the leaderboards and a resident that changes zone.
//!
//! The kernel moves a resident to its new zone after every zone ticked, so
//! in one frame a resident is in the report of one zone only. The first
//! test shows that with the kernel as it is. The other tests give the
//! global realm two reports of one frame that name the same resident, as
//! another process could send them, and check the rule for that case: the
//! report of the zone where the resident stays wins.

mod common;

use common::{FAR, events, ok, rich, world};
use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::protocol::FrameOutput;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Dest, Envelope, Input, Message, Money, OrgId, Output, OwnerId, Pos, RealmId, ResidentId, ZoneId,
};

/// A position in zone 0.
const NEAR: Pos = Pos::new(12, 12);

/// One row of the message `missions.zone_report`, as it crosses a process
/// border. The fields are those of the module.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Row {
    id: ResidentId,
    owner: OwnerId,
    model: Option<String>,
    born: u64,
    cash: i64,
    standing: i64,
    notoriety: i64,
    orgs: Vec<OrgId>,
    leaving: bool,
}

/// The message `missions.zone_report`, as it crosses a process border.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Report {
    frame: u64,
    rows: Vec<Row>,
    sites: Vec<(ResidentId, i64)>,
}

impl Message for Report {
    const KIND: &'static str = "missions.zone_report";
}

/// Makes the report of a zone with one resident.
fn report(frame: u64, resident: ResidentId, cash: i64, leaving: bool) -> Report {
    let row = Row { id: resident, owner: OwnerId(2), cash, leaving, ..Row::default() };
    Report { frame, rows: vec![row], sites: Vec::new() }
}

/// Gives the global realm the reports of zones, in this order, as the
/// coordinator of a cluster does: one `Input::Deliver`.
fn deliver(w: &mut TestWorld<'_>, reports: &[(u32, Report)]) {
    let envelopes = reports
        .iter()
        .map(|(zone, report)| {
            let envelope = Envelope::module(RealmId::Zone(ZoneId(*zone)), Dest::Global, report);
            Envelope::from_bytes(&envelope.to_bytes()).expect("the envelope reads back")
        })
        .collect();
    w.world.deliver_external(envelopes);
    assert_eq!(w.world.dropped_messages(), 0, "the module took every report");
}

/// Returns the wealth of a resident on the board that a frame published.
fn published_wealth(frames: &[FrameOutput], frame: u64, resident: ResidentId) -> Option<i64> {
    let boards: Vec<_> = events(frames, "missions.leaderboard_published")
        .into_iter()
        .filter(|event| event.frame == frame && event.data["key"] == json!("wealth"))
        .collect();
    assert_eq!(boards.len(), 1, "frame {frame} published one wealth board");
    let top = boards[0].data["top"].as_array().expect("the top list");
    top.iter()
        .find(|entry| entry["resident"] == json!(resident))
        .map(|entry| entry["score"].as_i64().expect("a score"))
}

/// Returns the wealth of a resident as the verb `leaderboard` gives it.
fn wealth(w: &mut TestWorld<'_>, resident: ResidentId) -> i64 {
    let read = ok(w, resident, "leaderboard", json!({ "board": "wealth" }));
    read["you"]["score"].as_i64().expect("the resident is on the board")
}

/// Runs one frame and returns the wealth of a resident on the board of
/// that frame. The resident gets 500 cents first, so that each frame has
/// its own amount.
fn frame_with_more_cash(w: &mut TestWorld<'_>, resident: ResidentId) -> (i64, Option<i64>) {
    w.give_money(resident, Money(500));
    let cash = w.cash(resident).cents();
    let frames = w.step(1);
    (cash, published_wealth(&frames, w.world.frame(), resident))
}

#[test]
fn a_resident_that_changes_zone_is_in_one_report_of_each_frame() {
    let mut w = world();
    // Every zone reports in every frame, and the boards are published in
    // every frame: the board of a frame has the reports of that frame.
    w.set_param("missions.report_period", 1);
    w.set_param("missions.board_period", 1);
    let bob = rich(&mut w, "Bob", 2, 1_000);
    w.set_pos(bob, NEAR);
    w.step(2);
    assert_eq!(w.zone_of(bob), ZoneId(0));

    // Bob stays in zone 0.
    let (cash, on_board) = frame_with_more_cash(&mut w, bob);
    assert_eq!((cash, on_board), (1_500, Some(1_500)));

    // Bob walks to zone 3 in the tick of a frame. Zone 0 still has him
    // when it reports, with the cash of this frame. Zone 3 gets him after
    // its tick, so it does not report him in this frame.
    w.act_ok(bob, "walk_test", json!({ "x": FAR.x, "y": FAR.y }));
    let (cash, on_board) = frame_with_more_cash(&mut w, bob);
    assert_eq!(w.zone_of(bob), ZoneId(3));
    assert_eq!((cash, on_board), (2_000, Some(2_000)));
    // A snapshot keeps what the boards know about a resident that left
    // its zone. The next frame has the report of zone 3.
    w.snapshot_restore();
    let (cash, on_board) = frame_with_more_cash(&mut w, bob);
    assert_eq!((cash, on_board), (2_500, Some(2_500)));

    // Back to zone 0: the zone with the lower number gets him now. The
    // reports of the zones reach the global realm in zone order, and the
    // board is right in both directions.
    w.act_ok(bob, "walk_test", json!({ "x": NEAR.x, "y": NEAR.y }));
    let (cash, on_board) = frame_with_more_cash(&mut w, bob);
    assert_eq!(w.zone_of(bob), ZoneId(0));
    assert_eq!((cash, on_board), (3_000, Some(3_000)));
    let (cash, on_board) = frame_with_more_cash(&mut w, bob);
    assert_eq!((cash, on_board), (3_500, Some(3_500)));

    // A move between two frames: Bob is in zone 3 before the frame starts.
    w.set_pos(bob, FAR);
    let (cash, on_board) = frame_with_more_cash(&mut w, bob);
    assert_eq!(w.zone_of(bob), ZoneId(3));
    assert_eq!((cash, on_board), (4_000, Some(4_000)));

    // Zone 0 skips a frame (time dilation) in which Bob comes back between
    // two frames. No zone reports him in that frame: the board keeps what
    // zone 3 said last. Zone 0 reports him in its next tick.
    w.set_pos(bob, NEAR);
    w.give_money(bob, Money(500));
    let Output::Frame(skipped) = w.world.apply(Input::Step { skip: vec![ZoneId(0)] }) else {
        panic!("a step gives a frame");
    };
    assert_eq!(published_wealth(&[skipped], w.world.frame(), bob), Some(4_000));
    let frames = w.step(1);
    assert_eq!(published_wealth(&frames, w.world.frame(), bob), Some(4_500));
    w.assert_conserved();
}

#[test]
fn of_two_reports_of_one_frame_the_zone_where_the_resident_stays_wins() {
    // Bob went from zone 3 to zone 0 in frame 10. Zone 3 names him as a
    // resident that leaves, zone 0 as one of its residents. The reports
    // come in both orders.
    for stays_first in [true, false] {
        let mut w = world();
        let bob = rich(&mut w, "Bob", 2, 0);
        w.step(2);
        let stays = (0, report(10, bob, 900, false));
        let leaves = (3, report(10, bob, 100, true));
        let order = if stays_first { [stays, leaves] } else { [leaves, stays] };
        deliver(&mut w, &order);
        assert_eq!(wealth(&mut w, bob), 900, "stays first: {stays_first}");
    }
}

#[test]
fn a_report_of_a_newer_frame_wins_and_an_older_one_changes_nothing() {
    let mut w = world();
    let bob = rich(&mut w, "Bob", 2, 0);
    w.step(2);
    // Zone 0 is slow and reports frame 10 after zone 3 reported frame 11.
    deliver(&mut w, &[(3, report(11, bob, 700, false)), (0, report(10, bob, 100, false))]);
    assert_eq!(wealth(&mut w, bob), 700);
    // A resident that leaves in a newer frame is newer than a resident
    // that stayed in an older frame.
    deliver(&mut w, &[(3, report(12, bob, 800, true))]);
    assert_eq!(wealth(&mut w, bob), 800);
    // In the next frame the zone where it arrived reports it.
    deliver(&mut w, &[(0, report(13, bob, 850, false))]);
    assert_eq!(wealth(&mut w, bob), 850);
}
