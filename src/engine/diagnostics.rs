//! Opt-in aggregate work counts, owned by one Engine for its entire lifetime.
//!
//! All units are event/dispatch counts, never durations or CPU percentages.
//! `advance_iterations` counts entered budget iterations (including a step-gate
//! stop), not the requested budget. Exactly one `dispatch` category is charged
//! per iteration. Categories include shared call overhead: store-release probes,
//! coordinate cleanup, collection probes and scheduler predicates. `store_release`
//! means release_store_tick returned true; `step_gate` means Yield or Stop. `collection`
//! means `collect_heap()` returned true; it is NOT a collecting-status sample,
//! nor proof of physical GC CPU work (the service also does semantic maintenance).
//! Calls to `maintain` are outside advance accounting. Cancellation includes its
//! internal discard, cleanup and inspection service; `discard` is ordinary task
//! dispatch with false scope. `ready` services a completion certificate and
//! `observer` services output projection. `idle` includes output backpressure.
//!
//! Rule entries use prepared-rule index as identity. Matching dispatches count
//! Discovery::tick calls; indexed visits are before/after deltas of
//! Matches::candidate_visits, including unsuccessful candidates. Direct anchor
//! Found results have a separate count and never contribute indexed visits.
//! Commit dispatches count Commit::tick calls; started/applied/rejected count
//! transactions. Scheduling counts are admissions and transitions, not distinct
//! programs or semantic alternatives. Completions include false-scope discards;
//! cancellation removals are separate. Requeues exclude admissions and lane wakes.
//!
//! Choice births count explicit binary decision births, including the binary
//! decomposition of larger disjunctions. Failure applications count completed
//! Fail bodies; support changes count those that actually change active support.
//! Posts/merges count completed body operations, with changed merges separate.
//! Certificates started count snapshots (including unsuccessful certificates),
//! rows scanned count yielded obligation rows, and published counts certificates
//! passed to Observe. Output events count primary source events, excluding
//! independent inspections, even if not consumed;
//! complete answers count produced End events, preserving answer multiplicity.
//! These aggregates survive task completion, collection and cancellation. No
//! clocks, execution history, global registry or atomics are used.

use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RuleDiagnostics {
    pub tasks_created: u64,
    pub matching_dispatches: u64,
    pub indexed_candidate_visits: u64,
    pub direct_anchor_matches: u64,
    pub found: u64,
    pub commits_started: u64,
    pub commit_dispatches: u64,
    pub applied: u64,
    pub rejected: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DispatchDiagnostics {
    pub store_release: u64,
    pub collection: u64,
    pub step_gate: u64,
    pub cancellation: u64,
    pub inspection: u64,
    pub ready: u64,
    pub observer: u64,
    pub init: u64,
    pub body: u64,
    pub activation: u64,
    pub search: u64,
    pub wake: u64,
    pub discard: u64,
    pub idle: u64,
}

impl DispatchDiagnostics {
    pub fn total(&self) -> u64 {
        self.store_release
            + self.collection
            + self.step_gate
            + self.cancellation
            + self.inspection
            + self.ready
            + self.observer
            + self.init
            + self.body
            + self.activation
            + self.search
            + self.wake
            + self.discard
            + self.idle
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    pub shared: SharedDiagnostics,
    pub advance_iterations: u64,
    pub dispatch: DispatchDiagnostics,
    pub rules: Vec<RuleDiagnostics>,
    pub tasks_created: u64,
    pub tasks_completed: u64,
    pub tasks_canceled: u64,
    pub task_parks: u64,
    pub task_requeues: u64,
    pub task_wakes: u64,
    pub choice_births: u64,
    pub fail_applications: u64,
    pub fail_support_changes: u64,
    pub body_posts: u64,
    pub body_merges: u64,
    pub merge_support_changes: u64,
    pub certificates_started: u64,
    pub obligation_rows_scanned: u64,
    /// One-way syntax barriers completed (first view or cancellation).
    pub syntax_promotions: u64,
    /// Queued/parked entries visited by the barrier, including non-body tasks.
    pub syntax_promotion_tasks: u64,
    /// Nonempty body descriptors allocated by the barrier; a subset of its tasks.
    pub syntax_descriptors_materialized: u64,
    pub certificates_published: u64,
    pub output_events: u64,
    pub complete_answers: u64,
}

/// Continuation calls and structural Job/Transform work. Calls include identity,
/// completion and cleanup steps. Transform work includes nested Boolean work.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ConditionalWork {
    pub calls: u64,
    pub work: u64,
}

/// Cumulative engine-owned services, including maintain and cancellation. These
/// are shared responsibilities, never attributed to a rule or an allocation
/// owner. Phase/index steps include nested work; they are not exclusive CPU time.
/// Conditional work inside matching, commits, observation, graph/history index
/// substitution and arena adaptation is unmeasured and explicitly null, not zero.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SharedDiagnostics {
    /// Completion-owned Boolean evaluation actions and continuation calls.
    pub completion_boolean: ConditionalWork,
    pub collection: CollectionDiagnostics,
    pub compaction: CompactionDiagnostics,
    pub coordinates: CoordinateDiagnostics,
    pub unclassified_conditional_work: Option<u64>,
}

/// Entered collection continuations by entry phase, excluding admission/probes.
/// Phases include transitions and bookkeeping. Arena includes representation
/// adaptation; root phases do not claim scalar-root units or physical GC CPU.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CollectionDiagnostics {
    pub started: u64,
    pub completed: u64,
    pub compact: u64,
    pub prune: u64,
    pub tasks: u64,
    pub parked: u64,
    pub births: u64,
    pub coordinates: u64,
    pub ready: u64,
    pub observe: u64,
    pub snapshots: u64,
    pub inspections: u64,
    pub rule_step: u64,
    pub trim_births: u64,
    pub seed_variables: u64,
    pub prune_graph: u64,
    pub graph: u64,
    pub history: u64,
    pub pending: u64,
    pub arena: u64,
    pub seed_boolean: ConditionalWork,
}

/// Compaction-owned condition jobs and calls to persistent substitutions.
/// Index steps include nested transforms and cleanup; pending_transform is a
/// measured subset, not additive work in the same units as pending_index_steps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CompactionDiagnostics {
    pub choices_examined: u64,
    pub boolean: ConditionalWork,
    pub transform: ConditionalWork,
    pub graph_index_steps: u64,
    pub history_index_steps: u64,
    pub pending_index_steps: u64,
    pub pending_transform: ConditionalWork,
}

/// Publications count nonempty maps; assignments count entries, not distinct
/// choices or bytes. Cleanup probes include no-ops; retirements/drains count
/// actual removals. Transport categories identify reader responsibility.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CoordinateDiagnostics {
    pub publications: u64,
    pub assignments_published: u64,
    pub cleanup_probes: u64,
    pub epochs_retired: u64,
    pub maps_retired: u64,
    pub assignments_drained: u64,
    pub search: TransportDiagnostics,
    pub completion: TransportDiagnostics,
}

/// Epoch crossings count completed transforms, not transport starts. Transform
/// calls/work are nested within transport calls; interrupted work stays recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TransportDiagnostics {
    pub calls: u64,
    pub epochs_crossed: u64,
    pub transform: ConditionalWork,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::condition::{Arena, Condition, Operation, Progress};
    use crate::engine::Engine;
    use crate::program::prepare;
    use crate::syntax::{parse_program, parse_query};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn engine() -> Engine {
        Engine::new(Arc::new(
            prepare(&parse_program("").unwrap(), &parse_query("true").unwrap()).unwrap(),
        ))
    }
    fn conjunction(arena: &mut Arena) -> (u64, u64, Condition) {
        let (xi, x) = arena.fresh_choice();
        let (yi, y) = arena.fresh_choice();
        let mut job = arena.start(Operation::And(x, y));
        for _ in 0..100 {
            if let Progress::Complete(c) = job.tick(arena) {
                return (xi, yi, c);
            }
        }
        panic!("finite conjunction");
    }

    #[test]
    fn transport_accounts_real_transform_work_and_last_reader_cleanup() {
        // Reference transforms execute independently in a separate arena, with
        // no engine diagnostics. Check actual results, calls and structural work.
        let mut reference = Arena::default();
        let (xi, yi, mut value) = conjunction(&mut reference);
        let mut calls = 0;
        let mut work = 0;
        for id in [xi, yi] {
            // Match the coordinate log's ownership: the transform is not the
            // last assignment-map owner and does not drain that map itself.
            let bindings = Arc::new(BTreeMap::from([(id, Condition::TRUE)]));
            let mut job = reference.substitute(value, bindings.clone());
            let mut complete = false;
            for _ in 0..100 {
                calls += 1;
                if let Progress::Complete(c) = job.tick(&mut reference) {
                    value = c;
                    work += job.work();
                    complete = true;
                    break;
                }
            }
            assert!(complete);
        }
        assert_eq!(value, Condition::TRUE);
        assert!(work > 0);

        let mut e = engine();
        let (xi, yi, input) = conjunction(&mut e.arena);
        let old = e.coordinates.current();
        e.coordinates
            .publish(Arc::new(BTreeMap::from([(xi, Condition::TRUE)])));
        e.coordinates
            .publish(Arc::new(BTreeMap::from([(yi, Condition::TRUE)])));
        let mut transport = e.coordinates.transport(input, &old);
        let mut complete = false;
        for _ in 0..100 {
            if let Progress::Complete(c) = e.transport_tick(&mut transport, true) {
                assert_eq!(c, Condition::TRUE);
                complete = true;
                break;
            }
        }
        assert!(complete);
        let d = &e.diagnostics().shared.coordinates;
        assert_eq!(d.search.epochs_crossed, 2);
        assert_eq!(d.search.transform.work, work);
        assert_eq!(d.search.transform.calls, calls);
        assert_eq!(d.search.calls, calls + 3); // two starts and final completion
        assert_eq!(d.completion, TransportDiagnostics::default());
        drop(transport);
        let before = e.memory().coordinate_records;
        for _ in 0..8 {
            e.cleanup_coordinates();
        }
        assert_eq!(e.memory().coordinate_records, before); // old still pins both maps
        assert_eq!(e.diagnostics().shared.coordinates.epochs_retired, 0);
        drop(old);
        for _ in 0..16 {
            e.cleanup_coordinates();
        }
        assert_eq!(e.memory().coordinate_records, 1);
        let d = &e.diagnostics().shared.coordinates;
        assert_eq!(d.epochs_retired, 2);
        assert_eq!(d.maps_retired, 2);
        assert_eq!(d.assignments_drained, 2);

        let mut current = e
            .coordinates
            .transport(Condition::TRUE, &e.coordinates.current());
        assert_eq!(
            e.transport_tick(&mut current, false),
            Progress::Complete(Condition::TRUE)
        );
        let d = &e.diagnostics().shared.coordinates.completion;
        assert_eq!(d.calls, 1);
        assert_eq!(d.epochs_crossed, 0);
        assert_eq!(d.transform, ConditionalWork::default());
    }

    #[test]
    fn conditional_poll_counts_identity_and_nontrivial_work_before_release() {
        let mut arena = Arena::default();
        let (_, x) = arena.fresh_choice();
        let (_, y) = arena.fresh_choice();
        let mut count = ConditionalWork::default();
        let mut identity = Some(arena.start(Operation::And(x, Condition::TRUE)));
        assert_eq!(measured_poll!(&mut identity, &mut arena, count), Some(x));
        assert!(identity.is_none());
        assert_eq!(count.calls, 1);
        assert_eq!(count.work, 0);
        let mut mixed = Some(arena.start(Operation::And(x, y)));
        let mut result = None;
        for _ in 0..100 {
            result = measured_poll!(&mut mixed, &mut arena, count);
            if result.is_some() {
                break;
            }
        }
        let result = result.expect("finite conjunction");
        for bits in 0..4 {
            assert_eq!(
                arena.evaluate(result, |id| bits & (1 << id) != 0),
                bits == 3
            );
        }
        // One split: evaluate pair, evaluate low, after-low, evaluate high,
        // after-high. Cleanup calls contribute no structural work.
        assert_eq!(count.work, 5);
        assert!(count.calls > count.work);
        assert!(mixed.is_none());
    }
}
