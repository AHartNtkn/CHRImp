mod support;
use chr::engine::{Engine, InspectionError, SnapshotKind, ViewId};
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;
use support::{Answer, Reader, engine};
fn recording(program: &str, query: &str) -> Engine {
    Engine::with_history(
        Arc::new(
            prepare(
                &parse_program(program).unwrap(),
                &parse_query(query).unwrap(),
            )
            .unwrap(),
        ),
        true,
    )
}
fn collect(e: &mut Engine) {
    e.request_collection();
    for _ in 0..200000 {
        e.advance(1);
        if !e.collecting() {
            return;
        }
    }
    panic!("collection must finish");
}
fn capture(e: &mut Engine) -> ViewId {
    for _ in 0..200000 {
        match e.capture_snapshot() {
            Ok(id) => return id,
            Err(InspectionError::Busy | InspectionError::Initializing) => e.advance(1),
            Err(error) => panic!("{error}"),
        }
    }
    panic!("snapshot must become available")
}
fn inspection(e: &mut Engine, id: ViewId, gc: bool) -> Vec<Answer> {
    let mut reader = Reader::default();
    let mut answers = vec![];
    for _ in 0..200000 {
        e.advance_inspection(id, 1).unwrap();
        if let Some(event) = e.take_inspection_output(id).unwrap()
            && let Some(answer) = reader.push(event)
        {
            answers.push(answer);
        }
        let status = e.inspection_status(id).unwrap();
        assert_eq!(status.error, None);
        if status.done {
            return answers;
        }
        if gc {
            collect(e);
        }
    }
    panic!("finite projection must finish")
}
fn run(e: &mut Engine) -> Vec<Answer> {
    let mut reader = Reader::default();
    let mut answers = vec![];
    for _ in 0..200000 {
        e.advance(1);
        if let Some(answer) = reader.next(e) {
            answers.push(answer);
        }
        if e.delivery_done() {
            return answers;
        }
    }
    panic!("finite query must finish")
}
#[test]
fn history_is_opt_in_and_recorded_roots_survive_until_explicit_release() {
    let program = "edge(X,Y) ==> reverse(Y,X).";
    let mut ordinary = engine(program, "edge(A,B)");
    let first = support::finish(&mut ordinary);
    assert_eq!(support::facts(&ordinary, &first), ["edge", "reverse"]);
    run(&mut ordinary);
    assert_eq!(ordinary.snapshots().count(), 0);
    let mut e = recording(program, "edge(A,B)");
    let expected = run(&mut e);
    let snapshots = e.snapshots().collect::<Vec<_>>();
    assert!(
        snapshots
            .iter()
            .any(|s| matches!(s.kind, SnapshotKind::Initial))
    );
    assert!(
        snapshots
            .iter()
            .any(|s| matches!(s.kind, SnapshotKind::Application { .. }))
    );
    let normal = snapshots
        .iter()
        .find(|s| matches!(s.kind, SnapshotKind::NormalForm))
        .unwrap()
        .id;
    let id = e.start_inspection(Some(normal), vec![]).unwrap();
    for snapshot in snapshots {
        e.release_snapshot(snapshot.id).unwrap();
    }
    collect(&mut e);
    assert!(
        e.memory().occurrences > 0,
        "inspection independently owns its snapshot"
    );
    let actual = inspection(&mut e, id, true);
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].variables, expected[0].variables);
    assert_eq!(actual[0].rows, expected[0].rows);
    e.release_inspection(id).unwrap();
    collect(&mut e);
    assert_eq!(e.memory().occurrences, 0);
    assert_eq!(e.memory().graph_nodes, 0);
}
#[test]
fn selections_respect_birth_regions_and_preserve_duplicate_alternatives() {
    let mut e = engine("hold(X) <=> hold(X).", "hold(X),(a(X);(b(X);b(X)))");
    e.advance(10000);
    let snapshot = capture(&mut e);
    let choices = e.choices().map(|(&id, _)| id).collect::<Vec<_>>();
    assert_eq!(choices.len(), 2);
    for (selection, expected) in [
        (vec![], vec!["a", "b", "b"]),
        (vec![(choices[1], true)], vec!["b"]),
        (vec![(choices[1], false)], vec!["b"]),
        (vec![(choices[0], true), (choices[1], true)], vec![]),
    ] {
        let before = e.applications();
        let id = e.start_inspection(Some(snapshot), selection).unwrap();
        let answers = inspection(&mut e, id, false);
        let mut names = answers
            .iter()
            .flat_map(|a| a.rows.iter())
            .map(|row| e.program().signatures()[row.relation].name.as_str())
            .filter(|name| *name != "hold")
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, expected);
        assert_eq!(
            e.applications(),
            before,
            "inspecting a paused run must not execute rules"
        );
        e.release_inspection(id).unwrap();
    }
    e.release_snapshot(snapshot).unwrap();
    assert_eq!(e.snapshots().count(), 0);
}
#[test]
fn inspection_backpressure_and_cancellation_do_not_enumerate_remaining_answers() {
    let choices = (0..30).map(|_| "(true;true)").collect::<Vec<_>>().join(",");
    let mut e = engine("hold(X) <=> hold(X).", &format!("hold(X),{choices}"));
    e.advance(50000);
    let snapshot = capture(&mut e);
    let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
    e.advance_inspection(id, 10000).unwrap();
    assert!(!e.inspection_status(id).unwrap().done);
    assert!(e.take_inspection_output(id).unwrap().is_some());
    e.advance_inspection(id, 10000).unwrap();
    assert_eq!(e.release_inspection(id), Err(InspectionError::InProgress));
    e.cancel_inspection(id).unwrap();
    for _ in 0..1000 {
        e.advance_inspection(id, 1).unwrap();
        assert!(e.take_inspection_output(id).unwrap().is_none());
        if e.inspection_status(id).unwrap().done {
            break;
        }
    }
    let status = e.inspection_status(id).unwrap();
    assert!(status.done && status.canceled);
    e.release_inspection(id).unwrap();
    e.release_snapshot(snapshot).unwrap();
}
#[test]
fn stale_cross_run_and_future_choice_ids_fail_without_projection() {
    let mut e = recording("hold(X) <=> hold(X).", "hold(X),(true;true)");
    e.advance(1000);
    while e.collecting() {
        e.advance(1);
    }
    let initial = e
        .snapshots()
        .find(|s| matches!(s.kind, SnapshotKind::Initial))
        .unwrap()
        .id;
    let choice = *e.choices().next().unwrap().0;
    let id = e
        .start_inspection(Some(initial), vec![(choice, true)])
        .unwrap();
    for _ in 0..1000 {
        e.advance_inspection(id, 1).unwrap();
        assert!(e.take_inspection_output(id).unwrap().is_none());
        if e.inspection_status(id).unwrap().done {
            break;
        }
    }
    assert_eq!(
        e.inspection_status(id).unwrap().error,
        Some(InspectionError::UnknownChoice)
    );
    let mut other = engine("", "true");
    assert_eq!(
        other.start_inspection(Some(initial), vec![]),
        Err(InspectionError::UnknownSnapshot)
    );
    e.release_inspection(id).unwrap();
    assert!(e.inspection_status(id).is_err());
    e.release_snapshot(initial).unwrap();
    assert_eq!(
        e.start_inspection(Some(initial), vec![]),
        Err(InspectionError::UnknownSnapshot)
    );
}

#[test]
fn blocked_inspections_do_not_starve_source_or_primary_answer_delivery() {
    let mut e = engine("loop(X) <=> loop(X).", "(done(A);loop(B))");
    e.advance(1000);
    let snapshot = capture(&mut e);
    let first = e.start_inspection(Some(snapshot), vec![]).unwrap();
    let second = e.start_inspection(Some(snapshot), vec![]).unwrap();
    let before = e.applications();
    e.advance(10000);
    assert!(e.applications() > before + 5);
    let answer = support::finish(&mut e);
    assert_eq!(support::facts(&e, &answer), ["done"]);
    assert!(e.take_inspection_output(first).unwrap().is_some());
    assert!(e.take_inspection_output(second).unwrap().is_some());
    while e.collecting() {
        e.advance(1);
    }
    e.cancel_inspection(first).unwrap();
    e.cancel_inspection(second).unwrap();
    e.advance(10000);
    assert!(e.inspection_status(first).unwrap().done);
    assert!(e.inspection_status(second).unwrap().done);
    while e.collecting() {
        e.advance(1);
    }
    e.release_inspection(first).unwrap();
    e.release_inspection(second).unwrap();
    e.release_snapshot(snapshot).unwrap();
}

#[test]
fn collection_defers_view_mutations_but_allows_owned_scalar_delivery() {
    let mut e = engine("hold(X) <=> hold(X).", "hold(X)");
    e.advance(1000);
    let snapshot = capture(&mut e);
    let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
    e.advance_inspection(id, 10000).unwrap();
    e.request_collection();
    e.advance(1);
    assert_eq!(e.capture_snapshot(), Err(InspectionError::Busy));
    assert_eq!(
        e.start_inspection(Some(snapshot), vec![]),
        Err(InspectionError::Busy)
    );
    assert_eq!(e.cancel_inspection(id), Err(InspectionError::Busy));
    assert_eq!(e.release_snapshot(snapshot), Err(InspectionError::Busy));
    assert!(e.take_inspection_output(id).unwrap().is_some());
    while e.collecting() {
        e.advance(1);
    }
    e.cancel_inspection(id).unwrap();
    e.advance_inspection(id, 1000).unwrap();
    assert!(e.inspection_status(id).unwrap().done);
    assert_eq!(e.inspection_snapshot_info(id).unwrap().id, snapshot);
    e.release_inspection(id).unwrap();
    e.release_snapshot(snapshot).unwrap();
}

#[test]
fn recorded_failure_preserves_the_rejected_region_until_its_view_is_released() {
    let mut e = recording("bad(X) ==> fail.", "bad(A);good(A)");
    let answers = run(&mut e);
    assert_eq!(answers.len(), 1);
    assert_eq!(support::facts(&e, &answers[0]), ["good"]);
    collect(&mut e);
    let snapshots = e.snapshots().collect::<Vec<_>>();
    let failure = snapshots
        .iter()
        .find(|s| matches!(s.kind, SnapshotKind::Failure))
        .unwrap()
        .id;
    for snapshot in snapshots {
        if snapshot.id != failure {
            e.release_snapshot(snapshot.id).unwrap();
        }
    }
    e.cancel();
    for _ in 0..200000 {
        e.advance(1);
        if e.cancel_done() {
            break;
        }
    }
    assert!(e.cancel_done());
    let view = e.start_inspection(Some(failure), vec![]).unwrap();
    e.release_snapshot(failure).unwrap();
    let failed = inspection(&mut e, view, true);
    assert_eq!(failed.len(), 1);
    assert_eq!(support::facts(&e, &failed[0]), ["bad"]);
    assert_eq!(failed[0].variables, answers[0].variables);
    e.release_inspection(view).unwrap();
    collect(&mut e);
    assert_eq!(e.memory().graph_nodes, 0);
    assert_eq!(e.memory().conditions, 0);
    assert_eq!(e.memory().obligation_descriptors, 0);
}

#[test]
fn explicit_failed_alternatives_remain_distinct_in_recorded_history() {
    let mut e = recording("", "fail;fail");
    assert!(run(&mut e).is_empty());
    collect(&mut e);
    let failures = e
        .snapshots()
        .filter(|s| matches!(s.kind, SnapshotKind::Failure))
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 2);
    for snapshot in failures {
        let view = e.start_inspection(Some(snapshot), vec![]).unwrap();
        let alternatives = inspection(&mut e, view, true);
        assert_eq!(alternatives.len(), 1);
        assert!(alternatives[0].rows.is_empty());
        e.release_inspection(view).unwrap();
    }
    assert!(e.delivery_done());
    assert!(e.take_output().is_none());
}

#[test]
fn held_view_preserves_its_choice_prefix_while_later_choices_are_compacted() {
    use chr::observe::Output;
    fn stream(e: &mut Engine, snapshot: ViewId) -> Vec<Output> {
        let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
        let mut result = Vec::new();
        for _ in 0..100_000 {
            e.advance_inspection(id, 1).unwrap();
            if let Some(mut event) = e.take_inspection_output(id).unwrap() {
                if let Output::Begin { completion, .. } = &mut event {
                    *completion = 0;
                }
                result.push(event);
            }
            if e.inspection_status(id).unwrap().done {
                e.release_inspection(id).unwrap();
                return result;
            }
        }
        panic!("held view projection must finish");
    }
    let mut e = engine(
        "loop(X) <=> (fail;X=Y,loop(Y)).",
        "(left();right()),loop(A)",
    );
    while e.applications() < 8 {
        e.advance(1);
    }
    let snapshot = capture(&mut e);
    let last = e.snapshot_info(snapshot).unwrap().last_choice.unwrap();
    let prefix: Vec<_> = e
        .choices_after(None, Some(last))
        .map(|(&id, birth)| (id, birth.support, birth.decision))
        .collect();
    let expected = stream(&mut e, snapshot);
    assert!(!expected.is_empty());
    for target in [32, 64, 128] {
        for _ in 0..1_000_000 {
            if e.applications() >= target {
                break;
            }
            e.advance(1);
        }
        assert!(e.applications() >= target);
        collect(&mut e);
        assert_eq!(
            e.choices_after(None, Some(last))
                .map(|(&id, birth)| (id, birth.support, birth.decision))
                .collect::<Vec<_>>(),
            prefix
        );
        assert!(e.memory().choices < prefix.len() + 16, "{:?}", e.memory());
        assert_eq!(stream(&mut e, snapshot), expected);
    }
    while e.collecting() {
        e.maintain(1);
    }
    // A metadata lease can retain a selected choice while inspection targets
    // the current graph, independently of the earlier snapshot's graph.
    let current = e
        .start_inspection(None, vec![(prefix[0].0, false)])
        .unwrap();
    assert_eq!(inspection(&mut e, current, false).len(), 1);
    e.release_inspection(current).unwrap();
    // Release the only historical owner and finish the resulting maintenance.
    while e.collecting() {
        e.maintain(1);
    }
    e.release_snapshot(snapshot).unwrap();
    collect(&mut e);
    assert!(e.memory().choices < 16);
}
