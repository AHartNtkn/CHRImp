#![cfg(feature = "diagnostics")]

use chr::{
    engine::Engine,
    observe::Output,
    program::prepare,
    syntax::{parse_program, parse_query},
};
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

fn finish(e: &mut Engine) -> (u64, u64) {
    let (mut events, mut answers) = (0, 0);
    for _ in 0..100_000 {
        let before = e.diagnostics().advance_iterations;
        e.advance(1);
        assert_eq!(e.diagnostics().advance_iterations, before + 1);
        if let Some(event) = e.take_output() {
            events += 1;
            answers += u64::from(matches!(event, Output::End));
        }
        assert_eq!(
            e.diagnostics().advance_iterations,
            e.diagnostics().dispatch.total()
        );
        if e.delivery_done() {
            return (events, answers);
        }
    }
    panic!("finite diagnostic control did not finish");
}

#[test]
fn direct_rules_report_real_applications_and_output() {
    let mut e = engine("p(X) ==> q(X). q(X) ==> r(X).", "p(A),p(B)");
    let (events, answers) = finish(&mut e);
    let d = e.diagnostics();
    assert_eq!(e.applications(), 4);
    assert_eq!(d.rules.len(), 2);
    for r in &d.rules {
        assert_eq!(r.applied, 2);
        assert_eq!(r.tasks_created, 2);
        assert_eq!(r.found, 2);
        assert_eq!(r.direct_anchor_matches, 2);
        assert_eq!(r.indexed_candidate_visits, 0);
        assert!(r.matching_dispatches > 0 && r.commit_dispatches > 0);
    }
    assert_eq!(
        d.rules.iter().map(|r| r.applied).sum::<u64>(),
        e.applications()
    );
    assert_eq!(d.body_posts, 6);
    assert_eq!((d.output_events, d.complete_answers), (events, answers));
    assert_eq!(answers, 1);
    assert_eq!(d.certificates_published, 1);
    assert_eq!(d.tasks_created, d.tasks_completed);
    serde_json::to_value(d).unwrap();
}

#[test]
fn explicit_choices_failures_and_duplicate_answers_are_not_deduplicated() {
    for (query, choices, failures, answers) in [
        ("true;true", 1, 0, 2),
        ("fail;true", 1, 1, 1),
        ("fail", 0, 1, 0),
    ] {
        let mut e = engine("", query);
        let (events, observed) = finish(&mut e);
        let d = e.diagnostics();
        assert_eq!(d.choice_births, choices);
        assert_eq!(d.fail_applications, failures);
        assert_eq!(d.fail_support_changes, failures);
        assert_eq!(observed, answers);
        assert_eq!((d.output_events, d.complete_answers), (events, observed));
        assert_eq!(e.applications(), 0);
    }
}

#[test]
fn indexed_candidates_are_distinct_from_direct_anchors() {
    let mut e = engine("p(X,X) ==> hit(X).", "p(A,A),p(B,C)");
    finish(&mut e);
    let r = &e.diagnostics().rules[0];
    assert_eq!(e.applications(), 1);
    assert_eq!(r.applied, 1);
    assert_eq!(r.direct_anchor_matches, 0);
    assert_eq!(r.found, 1);
    assert_eq!(r.indexed_candidate_visits, 2);
}

#[test]
fn counters_survive_cancellation_and_belong_to_each_engine() {
    for (program, query) in [("p(X) ==> p(X).", "p(A)"), ("p(X,X) ==> p(X,X).", "p(A,A)")] {
        let mut e = engine(program, query);
        let untouched = engine("", "true");
        for _ in 0..1000 {
            e.advance(1);
            e.take_output();
        }
        let before = e.diagnostics().clone();
        assert!(before.rules[0].applied > 0);
        e.cancel();
        for _ in 0..100_000 {
            e.advance(1);
            assert_eq!(
                e.diagnostics().advance_iterations,
                e.diagnostics().dispatch.total()
            );
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        let d = e.diagnostics();
        assert_eq!(d.rules, before.rules);
        assert!(d.dispatch.cancellation > 0);
        assert!(d.tasks_canceled > 0);
        assert_eq!(d.tasks_created, d.tasks_completed + d.tasks_canceled);
        assert_eq!(untouched.diagnostics().advance_iterations, 0);
        assert_eq!(untouched.diagnostics().output_events, 0);
        let before_zero = d.clone();
        e.advance(0);
        assert_eq!(e.diagnostics(), &before_zero);
    }
}

#[test]
fn competing_commits_count_rejections_without_inventing_applications() {
    let mut e = engine("p(X) <=> q(X). p(X) <=> r(X).", "p(A)");
    finish(&mut e);
    let d = e.diagnostics();
    assert_eq!(e.applications(), 1);
    assert_eq!(d.rules.iter().map(|r| r.applied).sum::<u64>(), 1);
    assert_eq!(d.rules.iter().map(|r| r.rejected).sum::<u64>(), 1);
    for r in &d.rules {
        assert_eq!(r.tasks_created, 1);
        assert_eq!(r.found, 1);
        assert_eq!(r.commits_started, r.applied + r.rejected);
    }
}

#[test]
fn merges_and_scheduler_transitions_count_actual_work() {
    let mut e = engine("p(X,X) ==> hit(X).", "p(A,B),A=B,A=B");
    finish(&mut e);
    let d = e.diagnostics();
    assert_eq!(e.applications(), 1);
    assert_eq!(d.body_merges, 2);
    assert_eq!(d.merge_support_changes, 1);
    assert!(d.dispatch.wake > 0);
    assert!(d.task_parks > 0);
    assert_eq!(d.task_parks, d.task_wakes);
    assert_eq!(d.tasks_created, d.tasks_completed);
    let task_dispatches = d.dispatch.init
        + d.dispatch.body
        + d.dispatch.activation
        + d.dispatch.search
        + d.dispatch.wake
        + d.dispatch.discard;
    assert_eq!(
        task_dispatches,
        d.tasks_completed + d.task_parks + d.task_requeues
    );
}

#[test]
fn choices_scan_obligations_and_held_output_is_counted_once() {
    let mut e = engine("", "a();b()");
    for _ in 0..100_000 {
        e.advance(1);
        if e.diagnostics().output_events > 0 {
            break;
        }
    }
    assert_eq!(e.diagnostics().output_events, 1);
    e.advance(1000);
    assert_eq!(e.diagnostics().output_events, 1);
    assert_eq!(
        e.diagnostics().advance_iterations,
        e.diagnostics().dispatch.total()
    );
    let (events, answers) = finish(&mut e);
    assert_eq!(e.diagnostics().output_events, events);
    assert_eq!(answers, 2);
    assert_eq!(e.diagnostics().complete_answers, answers);
    assert!(e.diagnostics().obligation_rows_scanned > 0);
    assert!(e.diagnostics().certificates_started >= e.diagnostics().certificates_published);
}

#[test]
fn collection_request_and_step_gate_have_honest_iteration_accounting() {
    let mut e = engine("", "true");
    e.request_collection();
    assert!(e.collecting());
    assert_eq!(e.diagnostics().dispatch.collection, 0);
    finish(&mut e);
    assert!(e.diagnostics().dispatch.collection > 0);

    let mut e = engine("p() ==> q().", "p()");
    e.request_step(vec![]).unwrap();
    for _ in 0..100_000 {
        e.advance(1);
        assert_eq!(
            e.diagnostics().advance_iterations,
            e.diagnostics().dispatch.total()
        );
        if e.step_status().done {
            break;
        }
    }
    assert!(e.step_status().done);
    assert_eq!(e.applications(), 1);
    let before = e.diagnostics().clone();
    e.advance(1000);
    assert_eq!(
        e.diagnostics().advance_iterations,
        before.advance_iterations + 1
    );
    assert_eq!(
        e.diagnostics().dispatch.step_gate,
        before.dispatch.step_gate + 1
    );
    e.resume().unwrap();
    finish(&mut e);
}

#[test]
fn inspection_dispatch_does_not_inflate_source_output() {
    let mut e = engine("", "true");
    finish(&mut e);
    let source = (
        e.diagnostics().output_events,
        e.diagnostics().complete_answers,
    );
    let id = e.start_inspection(None, vec![]).unwrap();
    for _ in 0..100_000 {
        e.advance(1);
        e.take_inspection_output(id).unwrap();
        assert_eq!(
            e.diagnostics().advance_iterations,
            e.diagnostics().dispatch.total()
        );
        if e.inspection_status(id).unwrap().done {
            break;
        }
    }
    assert!(e.inspection_status(id).unwrap().done);
    assert!(e.diagnostics().dispatch.inspection > 0);
    assert_eq!(
        (
            e.diagnostics().output_events,
            e.diagnostics().complete_answers
        ),
        source
    );
    e.release_inspection(id).unwrap();
}

#[test]
fn physical_maintenance_counts_actual_collection_without_source_dispatch() {
    let mut e = engine("", "true");
    let untouched = engine("", "true");
    e.request_collection();
    assert_eq!(e.diagnostics().shared.collection.started, 0);
    for _ in 0..100_000 {
        e.maintain(1);
        if !e.collecting() {
            break;
        }
    }
    assert!(!e.collecting());
    assert_eq!(e.collections(), 1);
    let d = &e.diagnostics().shared;
    assert_eq!(d.collection.started, 1);
    assert_eq!(d.collection.completed, 1);
    assert!(d.collection.arena > 0 && d.collection.graph > 0);
    assert_eq!(d.collection.compact, 0);
    assert_eq!(d.compaction.transform.calls, 0);
    assert_eq!(d.coordinates.publications, 0);
    assert_eq!(e.diagnostics().advance_iterations, 0);
    assert_eq!(untouched.diagnostics().shared.collection.started, 0);
    let before = e.diagnostics().clone();
    e.maintain(0);
    assert_eq!(e.diagnostics(), &before);
}

#[test]
fn continuing_choice_reduction_reports_substitution_and_reclaims_coordinates() {
    let mut e = engine("loop() <=> loop(),(fail;true).", "loop()");
    for _ in 0..1_000_000 {
        e.advance(1);
        assert!(e.take_output().is_none());
        if e.applications() >= 32 {
            break;
        }
    }
    assert!(e.applications() >= 32);
    e.request_collection();
    for _ in 0..1_000_000 {
        e.advance(1);
        assert!(e.take_output().is_none());
        if !e.collecting() {
            break;
        }
    }
    assert!(!e.collecting());
    let d = &e.diagnostics().shared;
    assert!(d.compaction.choices_examined > 0);
    assert!(d.compaction.boolean.calls > 0);
    assert!(d.compaction.transform.calls > 0);
    assert!(d.compaction.graph_index_steps > 0);
    assert!(d.compaction.history_index_steps > 0);
    assert!(d.compaction.pending_index_steps > 0);
    assert!(d.coordinates.publications > 0 && d.coordinates.assignments_published > 0);
    let published = d.coordinates.assignments_published;
    let apps = e.applications();
    e.cancel();
    for _ in 0..1_000_000 {
        e.advance(1);
        if e.cancel_done() {
            break;
        }
    }
    assert!(e.cancel_done());
    assert_eq!(e.applications(), apps);
    assert_eq!(e.memory().coordinate_records, 1);
    assert!(e.diagnostics().shared.coordinates.epochs_retired > 0);
    assert!(e.diagnostics().shared.coordinates.assignments_published >= published);
    assert_eq!(e.diagnostics().shared.unclassified_conditional_work, None);
}
