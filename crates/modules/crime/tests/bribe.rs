//! `bribe` and `report_crime`: a secret payment, hidden until somebody
//! reports it or an investigation finds it.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Money, Pos};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn a_bribe_is_secret_until_the_receiver_reports_it() {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &SHARP]));
    let bo = resident_at(&mut w, "Bo", 0, STREET);
    let cy = resident_at(&mut w, "Cy", 1_000, STREET.offset(1, 0));
    // Dee stands next to them and sees nothing.
    let dee = resident_at(&mut w, "Dee", 0, STREET.offset(0, 1));

    let note = "Look away tonight.";
    let (report, mut events) =
        run_with_events(&mut w, cy, "bribe", json!({ "to": bo, "amount": 700, "note": note }));
    assert_eq!(report.result, json!({ "to": bo, "amount": 700 }));
    events.extend(step_events(&mut w, 2));
    assert_eq!((w.cash(cy), w.cash(bo)), (Money(300), Money(700)));

    // No viewer sees a thing: the frames have no event of the crime
    // module, and no event of a gift.
    let loud: Vec<&str> = events
        .iter()
        .map(|event| event.kind.as_str())
        .filter(|kind| kind.starts_with("crime.") || kind.starts_with("life.gift"))
        .collect();
    assert_eq!(loud, Vec::<&str>::new());
    assert_eq!(notice(&w, dee, "crime.witnessed"), None);
    assert_eq!(law(&w, dee), None);
    // The watch sees every crime in this world, but not a bribe.
    assert!(!w.has_condition(cy, "wanted"));
    assert_eq!(ok(&mut w, dee, "case_list", json!({}))["cases"], json!([]));

    // The receiver knows who paid. The text of the giver is marked.
    let got = notice(&w, bo, "crime.bribe_received").expect("the receiver is told");
    let case = got.data["case"].as_u64().expect("a case");
    assert_eq!(got.from, Some(cy));
    assert_eq!(
        got.data,
        json!({
            "case": case, "from": cy, "amount": 700,
            "note": { "untrusted": true, "from": cy, "text": note },
        })
    );
    assert_eq!(
        law(&w, bo).expect("the receiver knows the case")["known_cases"],
        json!([{ "case": case, "kind": "bribery", "role": "receiver", "suspect": cy, "reported": false }])
    );
    // The case is not on the docket, and nobody can read it.
    assert_eq!(fail(&mut w, dee, "case_view", json!({ "case": case })), ErrorCode::NotFound);

    // The receiver keeps the money and reports the giver: 40 points.
    assert_eq!(
        ok(&mut w, bo, "report_crime", json!({ "case": case })),
        json!({ "case": case, "reported": true, "named_suspect": true })
    );
    w.step(2);
    assert!(w.has_condition(cy, "wanted"));
    let view = case_view(&mut w, dee, case);
    assert_eq!(view["kind"], json!("bribery"));
    assert_eq!(view["status"], json!("wanted"));
    assert_eq!(view["evidence_by_source"]["testimony"], json!(40));
    assert_eq!(view["suspect"], json!(cy));
    assert_eq!(w.cash(bo), Money(700));
    w.assert_conserved();
}

#[test]
fn an_investigation_finds_a_bribe() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 0, STREET);
    let cy = resident_at(&mut w, "Cy", 1_000, STREET.offset(1, 0));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(3, 0));
    let gus = resident_at(&mut w, "Gus", 0, STREET.offset(4, 0));
    appoint(&mut w, "police", &["law.search"], &[eve, gus]);
    ok(&mut w, cy, "bribe", json!({ "to": bo, "amount": 250 }));
    assert_eq!(notice(&w, bo, "crime.bribe_received").expect("a notice").data.get("note"), None);

    // Two officers read the trace: 29 + 29 points are a warrant.
    let first = ok(&mut w, eve, "investigate", json!({}));
    let case = case_of(&first);
    assert_eq!(first["kind"], json!("bribery"));
    assert_eq!(first["found"], json!(true));
    w.step(1);
    assert!(!w.has_condition(cy, "wanted"));
    // The first evidence puts the case on the docket, without a name.
    let view = case_view(&mut w, bo, case);
    assert_eq!(view["status"], json!("open"));
    assert_eq!(view.get("suspect"), None);
    ok(&mut w, gus, "investigate", json!({ "case": case }));
    w.step(2);
    assert!(w.has_condition(cy, "wanted"));
    assert!(!w.has_condition(bo, "wanted"));
    w.assert_conserved();
}

#[test]
fn bribe_refuses_bad_requests() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 0, STREET);
    let cy = resident_at(&mut w, "Cy", 1_000, STREET.offset(1, 0));
    let far = resident_at(&mut w, "Far", 0, STREET.offset(0, 5));
    let bribe = |to: unsup_kernel::ResidentId, amount: i64| json!({ "to": to, "amount": amount });
    assert_eq!(fail(&mut w, cy, "bribe", bribe(cy, 100)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, cy, "bribe", bribe(bo, 0)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, cy, "bribe", bribe(bo, -5)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, cy, "bribe", json!({ "to": bo })), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, cy, "bribe", bribe(far, 100)), ErrorCode::NotHere);
    assert_eq!(fail(&mut w, cy, "bribe", bribe(bo, 1_001)), ErrorCode::Insufficient);
    // A note has at most 200 bytes, is not blank and has no control
    // character.
    for note in ["x".repeat(201), "   ".to_string(), "a\u{7}b".to_string()] {
        let args = json!({ "to": bo, "amount": 100, "note": note });
        assert_eq!(fail(&mut w, cy, "bribe", args), ErrorCode::InvalidArgs);
    }
    assert_eq!((w.cash(cy), w.cash(bo)), (Money(1_000), Money(0)));
    assert_eq!(notice(&w, bo, "crime.bribe_received"), None);
    w.assert_conserved();
}

#[test]
fn report_crime_takes_only_what_the_resident_saw_and_only_one_time() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &SHARP]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let dee = resident_at(&mut w, "Dee", 0, STREET.offset(0, 2));
    let far = resident_at(&mut w, "Far", 0, STREET.offset(0, 30));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));

    // Far saw nothing, so it cannot put the blame on anybody.
    assert_eq!(fail(&mut w, far, "report_crime", json!({ "case": case })), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, far, "report_crime", json!({})), ErrorCode::InvalidArgs);
    // The thief knows its own crime, but not as a witness.
    assert_eq!(fail(&mut w, cy, "report_crime", json!({ "case": case })), ErrorCode::NotFound);

    // A witness is 25 points, the victim is 40. Each counts one time.
    ok(&mut w, dee, "report_crime", json!({ "case": case }));
    assert_eq!(fail(&mut w, dee, "report_crime", json!({ "case": case })), ErrorCode::Conflict);
    w.step(1);
    assert_eq!(case_view(&mut w, far, case)["evidence_by_source"]["testimony"], json!(25));
    assert!(!w.has_condition(cy, "wanted"));
    ok(&mut w, bo, "report_crime", json!({ "case": case }));
    w.step(2);
    let view = case_view(&mut w, far, case);
    assert_eq!(view["evidence_by_source"]["testimony"], json!(65));
    assert_eq!(view["reported"], json!(true));
    assert!(w.has_condition(cy, "wanted"));
    assert_eq!(
        law(&w, dee).expect("the witness knows the case")["known_cases"],
        json!([{ "case": case, "kind": "theft", "role": "witness", "suspect": cy, "reported": true }])
    );
    w.assert_conserved();
}

#[test]
fn a_witness_that_did_not_recognize_the_thief_adds_no_evidence() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let dee = resident_at(&mut w, "Dee", 0, STREET.offset(0, 2));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    assert_eq!(notice(&w, dee, "crime.witnessed").expect("a notice").data.get("suspect"), None);
    assert_eq!(
        ok(&mut w, dee, "report_crime", json!({ "case": case })),
        json!({ "case": case, "reported": true, "named_suspect": false })
    );
    ok(&mut w, bo, "report_crime", json!({ "case": case }));
    w.step(2);
    let view = case_view(&mut w, bo, case);
    assert_eq!(view["evidence"], json!(0));
    assert_eq!(view["reported"], json!(true));
    assert_eq!(view["witnesses"], json!(2));
    assert!(!w.has_condition(cy, "wanted"));
    w.assert_conserved();
}
