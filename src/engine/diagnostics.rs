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
    pub certificates_published: u64,
    pub output_events: u64,
    pub complete_answers: u64,
}
