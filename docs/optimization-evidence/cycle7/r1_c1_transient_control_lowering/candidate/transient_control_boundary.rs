//! Symmetric applicability fixture for cycle 7; no candidate implementation.
//! These public observations are available even when history was never enabled.
mod support;

use chr::engine::{Engine, ViewId};
use chr::observe::Output;
use support::{Reader, engine};

const PROGRAM: &str = "step0(K,I) <=> cell0(K,V),mark0(I,V),step1(V,I). \
                       step1(K,I) <=> end(I,K).";

fn relation(e: &Engine, name: &str) -> usize {
    e.program()
        .signatures()
        .iter()
        .position(|s| s.name == name)
        .unwrap()
}

fn project(e: &mut Engine, snapshot: ViewId) -> Vec<Output> {
    let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
    let before = e.applications();
    let mut output = vec![];
    for _ in 0..200_000 {
        e.advance_inspection(id, 1).unwrap();
        if let Some(event) = e.take_inspection_output(id).unwrap() {
            output.push(event);
        }
        assert_eq!(e.applications(), before);
        let status = e.inspection_status(id).unwrap();
        assert_eq!(status.error, None);
        if status.done {
            e.release_inspection(id).unwrap();
            return output;
        }
    }
    panic!("finite snapshot projection did not finish");
}

fn cancel(e: &mut Engine) {
    let before = e.applications();
    e.cancel();
    for _ in 0..200_000 {
        e.advance(1);
        assert_eq!(e.applications(), before);
        if e.cancel_done() {
            return;
        }
    }
    panic!("cancellation did not finish");
}

fn empty(e: &mut Engine) {
    e.request_collection();
    for _ in 0..200_000 {
        e.maintain(1);
        if !e.collecting() && !e.release_pending() {
            break;
        }
    }
    let m = e.memory();
    assert_eq!(e.pending_tasks(), 0);
    assert_eq!(m.graph_nodes, 0);
    assert_eq!(m.occurrences, 0);
    assert_eq!(m.conditions, 0);
    assert_eq!(m.pending_nodes, 0);
    assert_eq!(m.obligation_descriptors, 0);
    assert_eq!(m.history_nodes, 0);
    assert_eq!(m.history_records, 0);
    assert_eq!(m.choices, 0);
    assert_eq!(m.snapshots, 0);
    assert_eq!(m.inspections, 0);
    assert_eq!(m.restriction_nodes, 0);
    assert_eq!(m.release_batches, 0);
    println!("reclaimed={}", serde_json::to_string(&m).unwrap());
}

#[test]
fn ordinary_control_is_committed_and_late_snapshot_survives_cancellation() {
    let mut e = engine(PROGRAM, "step0(A,I)");
    let control = relation(&e, "step0");
    let mut found = false;
    for _ in 0..200_000 {
        e.advance(1);
        if !e.collecting() && e.facts(control).unwrap().count() == 1 {
            found = true;
            break;
        }
    }
    assert!(found, "committed control is observable before its consumer");
    assert_eq!(e.applications(), 0);
    assert_eq!(
        e.snapshots().count(),
        0,
        "ordinary history remains disabled"
    );
    let snapshot = e.capture_snapshot().unwrap();
    let expected = project(&mut e, snapshot);
    assert_eq!(
        expected
            .iter()
            .filter(|o| matches!(o, Output::Fact { relation, .. } if *relation == control))
            .count(),
        1
    );
    assert!(
        !expected
            .iter()
            .any(|o| matches!(o, Output::ExpressionRelation { relation } if *relation == control))
    );
    println!("held={}", serde_json::to_string(&e.memory()).unwrap());
    cancel(&mut e);
    assert!(e.memory().occurrences > 0, "snapshot retains the control");
    let actual = project(&mut e, snapshot);
    // Each projection has its own Begin identity; the captured stream is fixed.
    assert_eq!(&actual[1..], &expected[1..]);
    e.release_snapshot(snapshot).unwrap();
    empty(&mut e);
}

#[test]
fn equal_control_tuples_keep_distinct_fresh_applications_and_late_steps() {
    let mut e = engine(PROGRAM, "step0(A,I),step0(A,I)");
    let control = relation(&e, "step0");
    // Request stepping after ordinary execution has already posted a control.
    for _ in 0..200_000 {
        e.advance(1);
        if !e.collecting() && e.facts(control).unwrap().count() > 0 {
            break;
        }
    }
    assert_eq!(e.applications(), 0);
    let mut per_rule = [0; 2];
    for application in 1..=4 {
        e.request_step(vec![]).unwrap();
        for _ in 0..200_000 {
            e.advance(1);
            if e.step_status().done {
                break;
            }
        }
        let status = e.step_status();
        assert!(status.done);
        assert!(status.event.is_some());
        per_rule[status.rule.unwrap()] += 1;
        assert_eq!(e.applications(), application);
    }
    assert_eq!(per_rule, [2, 2]);
    e.resume().unwrap();
    let answer = support::finish(&mut e);
    let mark = relation(&e, "mark0");
    let witnesses = answer
        .rows
        .iter()
        .filter(|row| row.relation == mark)
        .collect::<Vec<_>>();
    assert_eq!(witnesses.len(), 2);
    assert_ne!(witnesses[0].occurrence, witnesses[1].occurrence);
    assert_eq!(witnesses[0].ports[0], witnesses[1].ports[0]);
    assert_ne!(witnesses[0].ports[1], witnesses[1].ports[1]);
    assert_eq!(
        support::facts(&e, &answer),
        ["cell0", "cell0", "end", "end", "mark0", "mark0"]
    );
    // Drain remaining delivery with the same reader contract as maintained tests.
    let mut reader = Reader::default();
    for _ in 0..200_000 {
        e.advance(1);
        assert!(reader.next(&mut e).is_none());
        if e.delivery_done() {
            break;
        }
    }
    assert!(e.delivery_done());
    cancel(&mut e);
    empty(&mut e);
}
