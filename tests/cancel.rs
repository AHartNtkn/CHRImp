use chr::engine::{Engine, InspectionError, ViewId};
use chr::observe::Output;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;

fn engine(program: &str, query: &str) -> Engine {
    Engine::new(Arc::new(
        prepare(
            &parse_program(program).unwrap(),
            &parse_query(query).unwrap(),
        )
        .unwrap(),
    ))
}
fn drain(e: &mut Engine) -> usize {
    let applications = e.applications();
    for ticks in 1..500_000 {
        e.advance(1);
        assert_eq!(
            e.applications(),
            applications,
            "source application published after cancellation"
        );
        if e.cancel_done() {
            return ticks;
        }
    }
    panic!("cancellation did not finish");
}
fn empty(e: &Engine) {
    let m = e.memory();
    assert_eq!(e.pending_tasks(), 0);
    assert!(e.exhausted());
    assert_eq!(m.graph_nodes, 0, "graph nodes");
    assert_eq!(m.occurrences, 0, "occurrences");
    assert_eq!(m.history_nodes, 0, "history nodes");
    assert_eq!(m.history_records, 0, "history records");
    assert_eq!(m.pending_nodes, 0, "pending nodes");
    assert_eq!(m.conditions, 0, "condition nodes");
    assert_eq!(m.choices, 0, "choice births");
}
fn maintain(e: &mut Engine) {
    for _ in 0..200_000 {
        if !e.collecting() {
            return;
        }
        e.maintain(1);
    }
    panic!("maintenance did not finish");
}
fn inspect(e: &mut Engine, snapshot: ViewId) -> Vec<Output> {
    let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
    let mut output = vec![];
    for _ in 0..200_000 {
        e.advance_inspection(id, 1).unwrap();
        if let Some(event) = e.take_inspection_output(id).unwrap() {
            output.push(event);
        }
        if e.inspection_status(id).unwrap().done {
            e.release_inspection(id).unwrap();
            maintain(e);
            return output;
        }
    }
    panic!("held snapshot did not project after source cancellation");
}

#[test]
fn cancellation_at_task_and_gc_boundaries_never_resumes_source_or_leaks_scratch() {
    let cases = [
        ("loop(X) <=> X=Y,loop(Y).", "loop(A)"),
        (
            "p(X),q(X) ==> r(X,Z). merge(X,Y) <=> X=Y.",
            "p(A),q(B),(merge(A,B);merge(B,A))",
        ),
        (
            "keep(X) \\ consume(X) <=> out(X,Y).",
            "keep(A),consume(A),consume(B),(A=B;true)",
        ),
        ("p(X),p(Y),p(Z) ==> joined(X,Y,Z).", "p(A),p(B),p(C),p(D)"),
    ];
    for (program, query) in cases {
        for prefix in (0..1500).step_by(23) {
            let mut e = engine(program, query);
            for _ in 0..prefix {
                e.advance(1);
                e.take_output();
            }
            let before = e.memory();
            let apps = e.applications();
            e.cancel();
            e.cancel();
            assert!(e.canceled());
            assert!(!e.cancel_done());
            assert_eq!(
                e.memory().graph_nodes,
                before.graph_nodes,
                "cancel request must not traverse or mutate source storage"
            );
            e.advance(0);
            assert_eq!(e.applications(), apps);
            drain(&mut e);
            empty(&e);
            e.advance(10);
            e.cancel();
            assert!(e.cancel_done());
            empty(&e);
        }
    }
    // Exercise requests arriving throughout root gathering, semantic pruning,
    // and physical collector phases, including held owner leases.
    for gc_prefix in (0..400).step_by(7) {
        let mut e = engine("loop(X) <=> X=Y,loop(Y).", "loop(A)");
        e.advance(1800);
        e.request_collection();
        e.maintain(gc_prefix);
        e.cancel();
        drain(&mut e);
        empty(&e);
    }
}

#[test]
fn queued_scalar_survives_source_cancellation_without_finishing_projection() {
    let query = std::iter::repeat_n("(true;true)", 20)
        .collect::<Vec<_>>()
        .join(",");
    let mut e = engine("", &query);
    // This graph has a million causal alternatives. A prefix reaches primary
    // projection, whose one queued event must survive cancellation unchanged.
    for _ in 0..100_000 {
        e.advance(1);
        if e.exhausted() {
            break;
        }
    }
    for _ in 0..10_000 {
        e.advance(1);
    }
    e.cancel();
    let steps = drain(&mut e);
    assert!(
        steps < 20_000,
        "discard enumerated alternatives instead of scratch"
    );
    assert!(matches!(e.take_output(), Some(Output::Begin { .. })));
    assert_eq!(e.take_output(), None);
    assert!(e.delivery_done());
    empty(&e);
}

#[test]
fn snapshots_survive_cancel_and_release_triggers_physical_reclamation() {
    let mut e = engine("loop(X) <=> X=Y,loop(Y).", "loop(A),(tag(A);tag(A))");
    e.advance(3000);
    maintain(&mut e);
    let snapshot = e.capture_snapshot().unwrap();
    let active_inspection = e.start_inspection(Some(snapshot), vec![]).unwrap();
    e.advance_inspection(active_inspection, 40).unwrap();
    e.cancel();
    drain(&mut e);
    assert!(e.inspection_status(active_inspection).unwrap().canceled);
    assert!(e.inspection_status(active_inspection).unwrap().done);
    assert_eq!(e.take_inspection_output(active_inspection).unwrap(), None);
    assert!(e.memory().graph_nodes > 0);
    assert_eq!(e.snapshots().count(), 1);
    let events = inspect(&mut e, snapshot);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Output::Fact { .. }))
    );
    assert!(events.iter().any(|event| matches!(event, Output::End)));
    e.release_inspection(active_inspection).unwrap();
    maintain(&mut e);
    e.release_snapshot(snapshot).unwrap();
    assert!(
        !e.cancel_done(),
        "released view needs reclamation before close"
    );
    maintain(&mut e);
    assert!(e.cancel_done());
    empty(&e);
}

#[test]
fn maintenance_is_collection_only_and_post_cancel_inspections_are_not_canceled() {
    let mut e = engine("loop(X) <=> loop(X).", "loop(A)");
    e.advance(1800);
    maintain(&mut e);
    let snapshot = e.capture_snapshot().unwrap();
    let applications = e.applications();
    e.request_collection();
    maintain(&mut e);
    assert_eq!(e.applications(), applications);
    e.cancel();
    let newer = loop {
        match e.start_inspection(Some(snapshot), vec![]) {
            Ok(id) => break id,
            Err(InspectionError::Busy) => e.maintain(1),
            Err(error) => panic!("{error}"),
        }
    };
    drain(&mut e);
    assert!(!e.inspection_status(newer).unwrap().canceled);
    e.cancel_inspection(newer).unwrap();
    for _ in 0..100_000 {
        e.advance_inspection(newer, 1).unwrap();
        if e.inspection_status(newer).unwrap().done {
            break;
        }
    }
    e.release_inspection(newer).unwrap();
    maintain(&mut e);
    e.release_snapshot(snapshot).unwrap();
    maintain(&mut e);
    empty(&e);
}

fn discard_owned<T: chr::trace::Trace>(
    job: &mut T,
    g: &mut chr::graph::Graph,
    a: &mut chr::condition::Arena,
    root: chr::store::Root,
    conditions: impl Fn(&T) -> Vec<chr::condition::Condition>,
    mut discard: impl FnMut(&mut T) -> bool,
) -> usize {
    use chr::trace::{Cursor, Step};
    for ticks in 1..20_000 {
        let mut expected = conditions(job);
        expected.sort();
        let mut cursor = Cursor::default();
        let mut traced = vec![];
        for step in 0..expected.len() * 20 + 2000 {
            match job.trace(&mut cursor) {
                Step::Root(c) => traced.push(c),
                Step::Pending => {}
                Step::Done => break,
            }
            assert!(step + 1 < expected.len() * 20 + 2000);
        }
        traced.sort();
        assert_eq!(
            traced, expected,
            "discard trace must include every remaining nested owner"
        );
        let mut gc = g.collect([root.clone()].into_iter());
        while !gc.done() {
            if let Some(c) = gc.tick(g) {
                traced.push(c);
            }
        }
        drop(gc);
        let mut gc = a.collect(traced.into_iter());
        while !gc.tick(a) {}
        drop(gc);
        let nodes = (g.index_node_count(), a.node_count());
        let before = conditions(job).len();
        let done = discard(job);
        assert_eq!(
            (g.index_node_count(), a.node_count()),
            nodes,
            "discard must not execute graph or condition operations"
        );
        if ticks > 1 {
            assert!(
                before.saturating_sub(conditions(job).len()) <= 12,
                "one tick dropped a large child map"
            );
        }
        if done {
            assert!(discard(job));
            return ticks;
        }
    }
    panic!("owned discard did not finish");
}

#[test]
fn wide_wake_and_matching_children_discard_incrementally_and_remain_traceable() {
    use chr::condition::{Arena, Condition};
    use chr::graph::{Graph, UpdateStatus};
    use chr::identity::Merge;
    use chr::matching::Matches;
    use chr::wake::{Wake, WakeStatus};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let code = Arc::new(
        prepare(
            &parse_program("p(X),q(X) ==> true.").unwrap(),
            &parse_query("p(A),q(A)").unwrap(),
        )
        .unwrap(),
    );
    let mut g = Graph::new(&code.signatures);
    let mut a = Arena::default();
    let p = code.signatures.iter().position(|s| s.name == "p").unwrap();
    let q = code.signatures.iter().position(|s| s.name == "q").unwrap();
    let mut update = g.post(g.empty(), p, vec![0], Condition::TRUE).unwrap();
    let anchor = update.occurrence();
    let mut root = loop {
        if let UpdateStatus::Complete(r) = update.tick(&mut g) {
            break r;
        }
    };
    drop(update);
    for variable in 1..=160 {
        let mut merge = Merge::new(&g, root, 0, variable, Condition::TRUE);
        root = loop {
            if let Some(r) = merge.tick(&mut g, &mut a) {
                break r;
            }
        };
        let mut update = g.post(root, q, vec![variable], Condition::TRUE).unwrap();
        root = loop {
            if let UpdateStatus::Complete(r) = update.tick(&mut g) {
                break r;
            }
        };
    }
    let mut wake = Wake::new(&g, root.clone(), 0, Condition::TRUE);
    let mut found = 0;
    for _ in 0..100_000 {
        if matches!(wake.tick(&g, &mut a), WakeStatus::Found { .. }) {
            found += 1;
        }
        if found == 128 {
            break;
        }
    }
    assert_eq!(found, 128);
    let ticks = discard_owned(
        &mut wake,
        &mut g,
        &mut a,
        root.clone(),
        |w| w.condition_roots().collect(),
        Wake::discard_tick,
    );
    assert!(
        ticks >= 128,
        "wide seen map and membership scratch must yield"
    );
    assert!(wake.condition_roots().all(|c| c.is_terminal()));
    assert!(catch_unwind(AssertUnwindSafe(|| wake.tick(&g, &mut a))).is_err());
    let mut matches = Matches::new(
        &g,
        root.clone(),
        code,
        0,
        Condition::TRUE,
        Some((0, anchor)),
    )
    .unwrap();
    let mut wide = false;
    for _ in 0..100_000 {
        matches.tick(&g, &mut a);
        if matches.condition_roots().count() >= 128 {
            wide = true;
            break;
        }
    }
    assert!(wide, "exercise an owned source with a large membership map");
    let visits = matches.candidate_visits();
    let ticks = discard_owned(
        &mut matches,
        &mut g,
        &mut a,
        root,
        |m| m.condition_roots().collect(),
        Matches::discard_tick,
    );
    assert!(ticks >= 100);
    assert_eq!(matches.candidate_visits(), visits);
    assert!(matches.condition_roots().all(|c| c.is_terminal()));
    assert!(catch_unwind(AssertUnwindSafe(|| matches.tick(&g, &mut a))).is_err());
}

#[test]
fn physical_collection_between_cancel_steps_keeps_staged_owners_valid() {
    for prefix in (400..1600).step_by(71) {
        let mut e = engine(
            "p(X),q(X) ==> r(X,Z). merge(X,Y) <=> X=Y.",
            "p(A),q(B),(merge(A,B);merge(B,A))",
        );
        e.advance(prefix);
        let apps = e.applications();
        e.cancel();
        for _ in 0..3000 {
            if e.cancel_done() {
                break;
            }
            if !e.collecting() {
                e.request_collection();
            }
            // Explicitly finish the current physical pass without source work,
            // then grant one cancellation step before requesting the next pass.
            maintain(&mut e);
            e.advance(1);
            assert_eq!(e.applications(), apps);
        }
        assert!(e.cancel_done());
        empty(&e);
    }
}
