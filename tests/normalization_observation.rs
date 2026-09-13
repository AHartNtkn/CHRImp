use chr::{
    engine::{Engine, SnapshotKind},
    observe::Output,
    program::prepare,
    syntax::{parse_program, parse_query},
};
use std::sync::Arc;
const SOURCE: &str = "same @ app(T,A,B) \\ app(T,C,D) <=> A=C,B=D. k_same @ k(T) \\ k(T) <=> true. clash @ app(T,A,B),k(T) <=> fail. choose @ k(T) \\ go(T) <=> (k(T),left(T);app(T,A,B),right(T)).";
fn engine(query: &str, history: bool) -> Engine {
    Engine::with_history(
        Arc::new(
            prepare(
                &parse_program(SOURCE).unwrap(),
                &parse_query(query).unwrap(),
            )
            .unwrap(),
        ),
        history,
    )
}
fn step(e: &mut Engine) {
    e.request_step(vec![]).unwrap();
    for _ in 0..200_000 {
        e.advance(1);
        if e.step_status().done {
            return;
        }
    }
    panic!("source step did not finish");
}
#[test]
fn construction_automatically_specializes_and_steps_original_rules() {
    let mut e = engine("k(T),go(T)", true);
    step(&mut e);
    assert_eq!(e.step_status().rule, Some(3));
    assert_eq!(e.program().rules()[3].name.as_deref(), Some("choose"));
    step(&mut e);
    assert_eq!(e.step_status().rule, Some(1));
    assert_eq!(e.applications(), 2);
    assert!(
        e.normalization_stats().known_arm_admissions > 0,
        "normal constructor must specialize"
    );
    assert!(
        e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::Application { rule: 1, .. }))
    );
}
#[test]
fn consistency_step_exposes_pending_source_equalities_before_executing_them() {
    let mut e = engine("app(T,A,B),app(T,C,D)", true);
    step(&mut e);
    assert_eq!(e.step_status().rule, Some(0));
    assert_eq!(e.applications(), 1);
    let snapshot = e
        .snapshots()
        .find(|s| matches!(s.kind, SnapshotKind::Application { rule: 0, .. }))
        .unwrap()
        .id;
    let events = project(&mut e, snapshot);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Output::PendingBegin { .. }))
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Output::Expression {
            operator: chr::observe::ExpressionKind::Equal
        }
    )));
    let variables = |events: &[Output]| {
        events
            .iter()
            .filter_map(|e| match e {
                Output::Variable { variable, .. } => Some(*variable),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let before = variables(&events);
    assert_eq!(
        before
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        5
    );
    assert!(e.normalization_stats().coalescences > 0);
    e.resume().unwrap();
    let mut answer = vec![];
    for _ in 0..200_000 {
        e.advance(1);
        if let Some(o) = e.take_output() {
            answer.push(o);
        }
        if e.delivery_done() {
            break;
        }
    }
    assert!(e.delivery_done());
    let after = variables(&answer);
    assert_eq!(after.len(), 5);
    assert_eq!(after[1], after[3]);
    assert_eq!(after[2], after[4]);
    assert_eq!(
        after
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    release(&mut e);
}

fn project(e: &mut Engine, snapshot: chr::engine::ViewId) -> Vec<Output> {
    let id = loop {
        match e.start_inspection(Some(snapshot), vec![]) {
            Ok(id) => break id,
            Err(chr::engine::InspectionError::Busy) => e.maintain(1000),
            other => panic!("{other:?}"),
        }
    };
    let mut events = vec![];
    for _ in 0..200_000 {
        e.advance_inspection(id, 1).unwrap();
        if let Some(event) = e.take_inspection_output(id).unwrap() {
            events.push(event);
        }
        if e.inspection_status(id).unwrap().done {
            e.release_inspection(id).unwrap();
            return events;
        }
    }
    panic!("inspection did not finish");
}
fn release(e: &mut Engine) {
    e.cancel();
    for _ in 0..200_000 {
        e.advance(1);
        if e.cancel_done() {
            break;
        }
    }
    assert!(e.cancel_done());
    let views: Vec<_> = e.snapshots().map(|s| s.id).collect();
    for id in views {
        e.release_snapshot(id).unwrap();
    }
    e.maintain(200_000);
    let m = e.memory();
    assert_eq!(
        (
            m.graph_nodes,
            m.conditions,
            m.pending_nodes,
            m.occurrences,
            m.snapshots,
            m.inspections
        ),
        (0, 0, 0, 0, 0, 0)
    );
}
#[test]
fn lowered_and_terminal_consumer_failures_pause_before_rhs_and_record_failure() {
    let source = format!("{SOURCE} ban @ k(T),forbid(T) <=> fail.");
    for (query, rule) in [("app(T,A,B),k(T)", 2), ("k(T),forbid(T)", 4)] {
        for history in [false, true] {
            let code = Arc::new(
                prepare(
                    &parse_program(&source).unwrap(),
                    &parse_query(query).unwrap(),
                )
                .unwrap(),
            );
            let mut e = Engine::with_history(code, history);
            step(&mut e);
            assert_eq!(e.step_status().rule, Some(rule));
            assert_eq!(e.applications(), 1);
            let snap = e.capture_snapshot().unwrap();
            let before = project(&mut e, snap);
            assert!(!before.iter().any(|o| matches!(o, Output::Fact { .. })));
            assert!(before.iter().any(|o| matches!(
                o,
                Output::Expression {
                    operator: chr::observe::ExpressionKind::Fail
                }
            )));
            assert_eq!(
                before
                    .iter()
                    .filter(|o| matches!(o, Output::PendingBegin { .. }))
                    .count(),
                1
            );
            e.resume().unwrap();
            for _ in 0..200_000 {
                e.advance(1);
                assert!(e.take_output().is_none());
                if e.delivery_done() {
                    break;
                }
            }
            assert!(e.delivery_done());
            if history {
                assert!(
                    e.snapshots()
                        .any(|s| matches!(s.kind,SnapshotKind::Application{rule:r,..} if r==rule))
                );
                let failure = e
                    .snapshots()
                    .find(|s| matches!(s.kind, SnapshotKind::Failure))
                    .unwrap()
                    .id;
                let failed = project(&mut e, failure);
                assert!(
                    !failed
                        .iter()
                        .any(|o| matches!(o, Output::PendingBegin { .. }))
                );
            }
            e.request_collection();
            e.maintain(200_000);
            let after = project(&mut e, snap);
            assert_eq!(&before[1..], &after[1..]);
            release(&mut e);
        }
    }
}
#[test]
fn dispatch_admission_does_not_duplicate_pending_source_alternatives() {
    let mut e = engine("k(T),go(T)", true);
    step(&mut e); // go was consumed; its original OR is pending.
    e.resume().unwrap();
    for _ in 0..200_000 {
        e.advance(1);
        if e.normalization_stats().known_arm_admissions > 0 {
            break;
        }
    }
    assert_eq!(e.normalization_stats().known_arm_admissions, 1);
    let snapshot = e.capture_snapshot().unwrap();
    let events = project(&mut e, snapshot);
    let left = e
        .program()
        .signatures()
        .iter()
        .position(|s| s.name == "left")
        .unwrap();
    let right = e
        .program()
        .signatures()
        .iter()
        .position(|s| s.name == "right")
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|o| matches!(o,Output::ExpressionRelation{relation} if *relation==left))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|o| matches!(o,Output::ExpressionRelation{relation} if *relation==right))
    );
    assert!(!events.iter().any(|o| matches!(
        o,
        Output::Expression {
            operator: chr::observe::ExpressionKind::Or
        }
    )));
    release(&mut e);
}
#[test]
fn prepared_plan_is_reused_by_normal_and_recording_engines() {
    let code = Arc::new(
        prepare(
            &parse_program(SOURCE).unwrap(),
            &parse_query("app(T,A,B),app(T,C,D)").unwrap(),
        )
        .unwrap(),
    );
    for history in [false, true] {
        let mut e = if history {
            Engine::with_history(code.clone(), true)
        } else {
            Engine::new(code.clone())
        };
        assert!(std::ptr::eq(e.program(), code.as_ref()));
        for _ in 0..200_000 {
            e.advance(1);
            e.take_output();
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done());
        assert_eq!(e.normalization_stats().coalescences, 1);
        if history {
            assert_eq!(
                e.snapshots()
                    .filter(|s| matches!(s.kind, SnapshotKind::Post { .. }))
                    .count(),
                2
            );
            assert_eq!(
                e.snapshots()
                    .filter(|s| matches!(s.kind, SnapshotKind::Merge))
                    .count(),
                2
            );
        }
        release(&mut e);
    }
}
