use chr::engine::{Engine, InspectionError, SnapshotKind, ViewId};
use chr::observe::Output;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;

fn engine(program: &str, query: &str, history: bool) -> Engine {
    Engine::with_history(
        Arc::new(
            prepare(
                &parse_program(program).unwrap(),
                &parse_query(query).unwrap(),
            )
            .unwrap(),
        ),
        history,
    )
}
fn project(e: &mut Engine, snapshot: ViewId) -> Vec<Output> {
    project_selected(e, snapshot, vec![])
}
fn project_selected(e: &mut Engine, snapshot: ViewId, choices: Vec<(u64, bool)>) -> Vec<Output> {
    let id = loop {
        match e.start_inspection(Some(snapshot), choices.clone()) {
            Ok(id) => break id,
            Err(InspectionError::Busy) => e.maintain(1000),
            other => panic!("{other:?}"),
        }
    };
    let mut events = vec![];
    for _ in 0..100000 {
        e.advance_inspection(id, 1).unwrap();
        if let Some(event) = e.take_inspection_output(id).unwrap() {
            events.push(event);
        }
        if e.inspection_status(id).unwrap().done {
            e.release_inspection(id).unwrap();
            return events;
        }
    }
    panic!("projection did not finish")
}
#[test]
fn committed_rewrite_snapshot_keeps_the_unposted_rhs_after_resume_gc_and_cancel() {
    let mut e = engine("p(X) <=> q(X). q(X) <=> r(X).", "p(A)", true);
    e.request_step(vec![]).unwrap();
    for _ in 0..10000 {
        e.advance(1);
        if e.step_status().done {
            break;
        }
    }
    assert_eq!(e.applications(), 1);
    let event = e.step_status().event.unwrap();
    let snap = e
        .snapshots()
        .find(|s| matches!(s.kind, SnapshotKind::Application {event: id, ..} if id == event))
        .unwrap()
        .id;
    let expected = project(&mut e, snap);
    assert!(!expected.iter().any(|e| matches!(e, Output::Fact { .. })));
    assert!(
        expected
            .iter()
            .any(|e| matches!(e, Output::PendingBegin {event: id} if *id == event))
    );
    let q = e
        .program()
        .signatures
        .iter()
        .position(|s| s.name == "q")
        .unwrap();
    assert!(
        expected
            .iter()
            .any(|e| matches!(e, Output::ExpressionRelation {relation} if *relation == q))
    );
    assert!(
        expected
            .iter()
            .any(|e| matches!(e, Output::ExpressionVariable { variable: 0 }))
    );
    e.resume().unwrap();
    for _ in 0..10000 {
        e.advance(1);
        e.take_output();
        if e.delivery_done() {
            break;
        }
    }
    e.cancel();
    for _ in 0..10000 {
        e.advance(1);
        if e.cancel_done() {
            break;
        }
    }
    let actual = project(&mut e, snap);
    // Inspection IDs differ; the immutable body, bindings and graph do not.
    assert_eq!(&actual[1..], &expected[1..]);
    let snapshots = e.snapshots().map(|s| s.id).collect::<Vec<_>>();
    for id in snapshots {
        e.release_snapshot(id).unwrap();
    }
    e.maintain(100000);
    assert_eq!(e.memory().pending_nodes, 0);
    assert_eq!(e.memory().obligation_descriptors, 0);
}

// Interpret only the emitted syntax, independently of scheduler continuations.
#[derive(Debug)]
enum Expr {
    Relation(usize),
    Op(chr::observe::ExpressionKind, Vec<Expr>),
}
fn expand(expr: Expr) -> Vec<Vec<usize>> {
    use chr::observe::ExpressionKind::*;
    match expr {
        Expr::Relation(r) => vec![vec![r]],
        Expr::Op(Or, children) => children.into_iter().flat_map(expand).collect(),
        Expr::Op(And, children) => children.into_iter().fold(vec![vec![]], |left, child| {
            let right = expand(child);
            left.into_iter()
                .flat_map(|a| {
                    right.iter().map(move |b| {
                        let mut both = a.clone();
                        both.extend(b);
                        both
                    })
                })
                .collect()
        }),
        Expr::Op(True | Equal, _) => vec![vec![]],
        Expr::Op(Fail, _) => vec![],
    }
}
fn remaining(events: Vec<Output>) -> Vec<Vec<usize>> {
    use chr::observe::ExpressionKind::And;
    let mut alternatives = vec![];
    let mut obligations = vec![];
    let mut stack = vec![];
    let mut in_pending = false;
    for event in events {
        match event {
            Output::Begin { .. } => {
                assert!(obligations.is_empty());
            }
            Output::Fact { relation, .. } => obligations.push(Expr::Relation(relation)),
            Output::PendingBegin { .. } => {
                assert!(!in_pending);
                in_pending = true;
            }
            Output::Expression { operator } => {
                assert!(in_pending);
                stack.push(Expr::Op(operator, vec![]));
            }
            Output::ExpressionRelation { relation } => {
                assert!(in_pending);
                stack.push(Expr::Relation(relation));
            }
            Output::ExpressionEnd => {
                let expression = stack.pop().unwrap();
                if let Some(Expr::Op(_, children)) = stack.last_mut() {
                    children.push(expression);
                } else {
                    assert!(stack.is_empty());
                    obligations.push(expression);
                }
            }
            Output::PendingEnd => {
                assert!(in_pending && stack.is_empty());
                in_pending = false;
            }
            Output::End => {
                assert!(!in_pending);
                alternatives.extend(expand(Expr::Op(And, std::mem::take(&mut obligations))));
            }
            _ => {}
        }
    }
    for alternative in &mut alternatives {
        alternative.sort();
    }
    alternatives.sort();
    alternatives
}
#[test]
fn every_partial_nested_and_or_snapshot_contains_exact_remaining_expressions() {
    let mut e = engine("", "(a(A),b(A));(c(A),(d(A);e(A)))", true);
    let relations = |names: &[&str]| {
        let mut ids = names
            .iter()
            .map(|name| {
                e.program()
                    .signatures
                    .iter()
                    .position(|s| s.name == *name)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids
    };
    let mut expected = vec![
        relations(&["a", "b"]),
        relations(&["c", "d"]),
        relations(&["c", "e"]),
    ];
    expected.sort();
    let mut inspected = 0;
    for _ in 0..2000 {
        e.advance(1);
        if e.collecting() {
            e.maintain(100000);
        }
        // Once a normal form is published its region is no longer active.
        if e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::NormalForm))
        {
            break;
        }
        let snap = match e.capture_snapshot() {
            Ok(id) => id,
            Err(InspectionError::Initializing) => continue,
            error => panic!("{error:?}"),
        };
        let actual = remaining(project(&mut e, snap));
        assert_eq!(actual, expected, "partial state {inspected}");
        let choices = e.choices().map(|(&id, _)| id).collect::<Vec<_>>();
        for (index, choice) in choices.into_iter().enumerate() {
            for positive in [true, false] {
                let selected = remaining(project_selected(&mut e, snap, vec![(choice, positive)]));
                let names: &[&[&str]] = match (index, positive) {
                    (0, true) => &[&["a", "b"]],
                    (0, false) => &[&["c", "d"], &["c", "e"]],
                    (1, true) => &[&["c", "d"]],
                    (1, false) => &[&["c", "e"]],
                    _ => unreachable!(),
                };
                let mut wanted = names
                    .iter()
                    .map(|names| {
                        let mut ids = names
                            .iter()
                            .map(|name| {
                                e.program()
                                    .signatures
                                    .iter()
                                    .position(|s| s.name == *name)
                                    .unwrap()
                            })
                            .collect::<Vec<_>>();
                        ids.sort();
                        ids
                    })
                    .collect::<Vec<_>>();
                wanted.sort();
                assert_eq!(
                    selected, wanted,
                    "partial selected state {inspected}, choice {index}/{positive}"
                );
            }
        }
        inspected += 1;
        e.release_snapshot(snap).unwrap();
        e.maintain(100000);
    }
    assert!(inspected > 50);
    assert_eq!(e.choices().count(), 2);
}
#[test]
fn initial_and_completed_operation_records_have_coherent_pending_bodies() {
    let mut e = engine("", "a(A),A=B,b(B)", true);
    for _ in 0..10000 {
        e.advance(1);
        e.take_output();
        if e.delivery_done() {
            break;
        }
    }
    let snapshots = e.snapshots().collect::<Vec<_>>();
    let initial = snapshots
        .iter()
        .find(|s| matches!(s.kind, SnapshotKind::Initial))
        .unwrap()
        .id;
    let events = project(&mut e, initial);
    assert!(!events.iter().any(|e| matches!(e, Output::Fact { .. })));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Output::PendingBegin { .. }))
            .count(),
        1
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Output::Expression {
            operator: chr::observe::ExpressionKind::Equal
        }
    )));
    for snapshot in snapshots {
        let events = project(&mut e, snapshot.id);
        match snapshot.kind {
            SnapshotKind::Post { occurrence } => {
                let relation = events
                    .iter()
                    .find_map(|e| match e {
                        Output::Fact {
                            occurrence: id,
                            relation,
                        } if *id == occurrence => Some(*relation),
                        _ => None,
                    })
                    .unwrap();
                assert!(!events.iter().any(|e| matches!(e, Output::ExpressionRelation { relation: pending } if *pending == relation)));
            }
            SnapshotKind::Merge => assert!(!events.iter().any(|e| matches!(
                e,
                Output::Expression {
                    operator: chr::observe::ExpressionKind::Equal
                }
            ))),
            _ => {}
        }
    }
}
#[test]
fn large_body_projection_is_scalar_backpressured_and_cancellation_releases_its_roots() {
    let ports = vec!["Y"; 8192].join(",");
    let mut e = engine(&format!("p(X) <=> X=Y,q({ports})."), "p(A)", true);
    e.request_step(vec![]).unwrap();
    for _ in 0..10000 {
        e.advance(1);
        if e.step_status().done {
            break;
        }
    }
    assert_eq!(e.applications(), 1);
    let snap = e.capture_snapshot().unwrap();
    let view = e.start_inspection(Some(snap), vec![]).unwrap();
    let mut scalars = 0;
    let mut saw_variable = false;
    for _ in 0..1000 {
        e.advance_inspection(view, 1).unwrap();
        if let Some(event) = e.take_inspection_output(view).unwrap() {
            scalars += 1;
            saw_variable |= matches!(event, Output::ExpressionVariable { .. });
            assert!(e.take_inspection_output(view).unwrap().is_none());
        }
        // Every suspended Boolean/identity/projection phase faces a collection.
        e.request_collection();
        e.maintain(100000);
    }
    assert!(saw_variable && scalars < 1000);
    assert!(!e.inspection_status(view).unwrap().done);
    e.cancel_inspection(view).unwrap();
    for _ in 0..1000 {
        e.advance_inspection(view, 1).unwrap();
        assert!(e.take_inspection_output(view).unwrap().is_none());
        if e.inspection_status(view).unwrap().done {
            break;
        }
    }
    assert!(e.inspection_status(view).unwrap().done);
    e.release_inspection(view).unwrap();
    e.cancel();
    for _ in 0..100000 {
        e.advance(1);
        if e.cancel_done() {
            break;
        }
    }
    let ids = e.snapshots().map(|s| s.id).collect::<Vec<_>>();
    for id in ids {
        e.release_snapshot(id).unwrap();
    }
    e.maintain(100000);
    assert_eq!(e.memory().pending_nodes, 0);
    assert_eq!(e.memory().obligation_descriptors, 0);
}

#[test]
fn a_shared_application_projects_the_same_remaining_body_in_each_selected_sibling() {
    let mut e = engine("p(X) <=> q(X).", "p(A),(true;true)", true);
    for _ in 0..10000 {
        e.advance(1);
        if e.choices().next().is_some() {
            break;
        }
    }
    let choice = *e.choices().next().unwrap().0;
    e.request_step(vec![(choice, true)]).unwrap();
    for _ in 0..10000 {
        e.advance(1);
        if e.step_status().done {
            break;
        }
    }
    assert_eq!(e.step_status().shared, Some(true));
    let event = e.step_status().event.unwrap();
    let snapshot = e.capture_snapshot().unwrap();
    for positive in [true, false] {
        let outputs = project_selected(&mut e, snapshot, vec![(choice, positive)]);
        let mut source = None;
        let mut bodies = 0;
        for output in outputs {
            match output {
                Output::PendingBegin { event } => source = Some(event),
                Output::ExpressionRelation { relation } if source == Some(event) => {
                    assert_eq!(e.program().signatures[relation].name, "q");
                    bodies += 1;
                }
                Output::PendingEnd => source = None,
                _ => {}
            }
        }
        assert_eq!(bodies, 1);
    }
}

#[test]
fn balanced_ranges_project_original_flat_arms_at_each_suspension() {
    let mut e = engine("", "(a(A);b(A);c(A);d(A);e(A);f(A);g(A))", true);
    let mut inspected = 0;
    let mut saw_ranges = false;
    let mut selected_paths = 0;
    for _ in 0..2000 {
        e.advance(1);
        e.take_output();
        if e.collecting() {
            e.maintain(100000);
        }
        if e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::NormalForm))
        {
            break;
        }
        let snap = match e.capture_snapshot() {
            Ok(id) => id,
            Err(InspectionError::Initializing | InspectionError::Busy) => continue,
            error => panic!("{error:?}"),
        };
        let all = remaining(project(&mut e, snap));
        let mut expected = (0..7).map(|i| vec![i]).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(all, expected);
        let births = e
            .choices()
            .map(|(&id, b)| (id, b.instruction, b.start, b.split, b.end))
            .collect::<Vec<_>>();
        if births.len() == 6 {
            for arm in 0..7 {
                let path = births
                    .iter()
                    .filter(|(_, _, start, _, end)| *start <= arm && arm < *end)
                    .map(|(id, _, _, split, _)| (*id, arm < *split))
                    .collect();
                assert_eq!(
                    remaining(project_selected(&mut e, snap, path)),
                    vec![vec![arm]]
                );
                selected_paths += 1;
            }
        }
        for (id, instruction, start, split, end) in births {
            let chr::program::Instruction::Or(items) = &e.program().instructions[instruction]
            else {
                panic!()
            };
            assert_eq!(items.len(), 7, "original flat instruction retained");
            assert!(start < split && split < end);
            saw_ranges |= split - start > 1;
            for (positive, lo, hi) in [(true, start, split), (false, split, end)] {
                let actual = remaining(project_selected(&mut e, snap, vec![(id, positive)]));
                let mut wanted = (lo..hi).map(|i| vec![i]).collect::<Vec<_>>();
                wanted.sort();
                assert_eq!(actual, wanted, "choice {id}, {lo}..{hi}, state {inspected}");
            }
        }
        // Retained syntax must survive a collection before replay.
        e.request_collection();
        e.maintain(100000);
        assert_eq!(remaining(project(&mut e, snap)), expected);
        e.release_snapshot(snap).unwrap();
        e.maintain(100000);
        inspected += 1;
    }
    assert!(
        inspected > 10 && saw_ranges && selected_paths > 0,
        "inspected={inspected} ranges={saw_ranges} paths={selected_paths}"
    );
}

#[test]
fn original_nested_groups_remain_structurally_visible_in_initial_projection() {
    use chr::observe::ExpressionKind::{And, Fail, Or};
    let mut e = engine("", "(a(A);(b(A);c(A);d(A));e(A);fail)", true);
    let snapshot = loop {
        e.advance(1);
        if let Some(s) = e
            .snapshots()
            .find(|s| matches!(s.kind, SnapshotKind::Initial))
        {
            break s.id;
        }
    };
    let mut stack: Vec<(chr::observe::ExpressionKind, usize)> = vec![];
    let mut groups = vec![];
    for event in project(&mut e, snapshot) {
        match event {
            Output::Expression { operator } => stack.push((operator, 0)),
            Output::ExpressionRelation { .. } => stack.push((And, 0)),
            Output::ExpressionEnd => {
                let (kind, arity) = stack.pop().unwrap();
                if kind == Or {
                    groups.push(arity);
                }
                if let Some((_, n)) = stack.last_mut() {
                    *n += 1;
                }
            }
            Output::ExpressionVariable { .. } => {}
            _ => {}
        }
    }
    assert!(stack.is_empty());
    assert_eq!(groups, vec![3, 4]);
    // The source AST still has the explicit nested Or, rather than extra groups
    // for the binary encoding. Its failed arm also remains part of the template.
    assert!(
        project(&mut e, snapshot)
            .iter()
            .any(|o| matches!(o, Output::Expression { operator: Fail }))
    );
}
