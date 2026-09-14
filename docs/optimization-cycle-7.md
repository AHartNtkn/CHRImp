# Cycle 7: historical implementation attempts

Correction: these attempts ended before meaningful architectural evaluation.
The earlier BLOCKED labels did not establish external blockers or infeasibility.
An unresolved correctness failure is described below, but no candidate performance
comparison was completed. The records do not justify excluding these architectural
ideas or imposing smaller implementation bounds on a subsequent campaign.

## Round 1, candidate 1: transient control lowering

No engine implementation or executable lowering prototype was completed; nothing was eligible for integration.

The candidate targeted the diagnosed cost in `fresh-contract`: transient control rows are posted into the graph and later collected/released. The worker established a semantic boundary instead of weakening the API. Committed controls are observable through `Engine::facts`, late snapshots, and late stepping even with history disabled. A pending-syntax substitute would therefore change observable state, snapshot retention, or fresh occurrence multiplicity.

The focused fixture passed 2 ordinary and 2 diagnostics release tests, including late snapshot retention through cancellation, distinct fresh witnesses for equal control tuples, late stepping, and complete reclamation. Broad validation was not claimed because an existing notebook startup assertion stopped the broader run. No candidate comparison was made, and no runtime or timeout was used as a verdict.

Baseline evidence and worker artifacts are preserved under [cycle-7 candidate evidence](optimization-evidence/cycle7/r1_c1_transient_control_lowering/). The worker branch/commit was `codex/opt/transient-control-lowering` / `981302f`; its strongest untested follow-up is a virtual committed-occurrence representation with explicit promotion to the general graph on observation/escape. That is a materially different representation question, not an automatic continuation follow-up.

The next candidate is the independent `central_occurrence_support` simplification: test whether one versioned occurrence-support authority can remove duplicated support rewrites and their release work without increasing dead-posting scans or retained snapshot state. A separate `segmented_graph_ownership` candidate remains in this round for breadth.

## Round 1, candidate 2: central occurrence support

The worker completed a diagnostics/pinned-root slice but no central-support implementation or workload comparison; diagnostics-only code is not an optimization result.

The slice added counters for fact-support accesses, secondary support writes, and secondary-row reads, plus a pinned-cursor fixture covering conditional support, equal-tuple multiplicity, old roots, and eventual reclamation. Fifty-one focused release/diagnostics tests passed. The counters establish the event boundary needed for a later implementation, but do not establish avoided work, allocation, retention, or non-inferiority. The full diagnostics run hit the same notebook CLI startup assertion recorded in candidate 1; no broad-suite pass is claimed.

Evidence is preserved under [candidate 2 evidence](optimization-evidence/cycle7/r1_c2_central_occurrence_support/). The diagnostic branch/commit is `codex/opt/central-occurrence-support` / `523d3a4`; it remains unintegrated. The hypothesis is unresolved because pruning, coordinate substitution, pinned versions, and dead-posting reclamation still need an implementation-level comparison.

The final round-1 candidate was the materially different `segmented_graph_ownership` hypothesis: test whether region-level graph ownership can eliminate per-node release traversal and collection bookkeeping without survivor-copy, cross-region, or retained-reader costs replacing it.

## Round 1, candidate 3: segmented graph ownership

The worker reached a committed instrumentation and experiment declaration, but no executable segmented-ownership mechanism or candidate comparison was completed.

The baseline instrumentation covered ownership operations, allocation, retention, collection and reclamation, and the preserved baseline passed 72 focused release/diagnostics tests across store, graph, pruning, ownership, inspection, history, cancellation, matching and collection. The partial `Region`/`NodeRef` conversion in `src/store.rs` remained uncommitted and was never built or measured. Therefore there is no evidence yet for either avoided per-node release work or introduced survivor-copy, cross-region, or retained-reader cost; no timing or timeout was used as a verdict.

Evidence is preserved under [candidate 3 evidence](optimization-evidence/cycle7/r1_c3_segmented_graph_ownership/). The worker branch/commit was `codex/opt/segmented-graph-ownership` / `5f91f1e`; the uncommitted partial implementation was discarded with the isolated worktree. The ownership hypothesis remains unresolved; the unfinished implementation does not establish a limitation of the architecture.

Round 1 ended with no integration winner. The candidates targeted fresh-contract graph/store insertion, collection, and release costs, but none reached a completed candidate comparison. Their unfinished implementations do not establish which architectural direction should be chosen next.

## Round 2 selection

Round 2 keeps the established fresh-contract diagnosis but changes the mechanisms under test. The selected frontier is: (1) compiler-certified omission of port indexes for relations absent from every source rule head, (2) factored execution of conservative alpha-equivalent isolated components, and (3) a source-certified single-world executor with flat occurrence storage for the deterministic fresh-contract subset. These are ordered from the smallest representation simplification to the largest replacement foundation. A proposed shared failure certificate was not selected because its first benefit is failure-heavy constructor workloads, which are not part of the current diagnosed fresh-contract cost.

Each candidate must preserve committed observation, snapshots, cancellation, multiplicity, fresh identities and cleanup, and must compare graph/allocation/collection/release work plus its own preparation, execution, observation and cleanup costs. No incumbent-counter disappearance, runtime ratio or timeout is sufficient for integration.

## Round 2, candidate 1: unused port-index omission

A narrow source certificate and filtered lookup mechanism was implemented, but correctness is unresolved and no workload comparison was collected.

The candidate preserved occurrences and incidence records while omitting PORT entries only for relations absent from source rule heads. The new certificate and filtered-lookup tests passed, and a calibration counted 260 omitted writes, 6 fallback lookups and 641 scanned rows with complete graph reclamation. Expanded validation passed 94 library checks but failed the collection-pressure obligation at `src/engine/collection.rs:991` (`deferred && deferred_ticks > 0`). The failure may be a contingent interaction with collection pressure or a semantic defect; it must be reproduced on baseline and candidate before the idea can be assessed. No candidate workload, allocation, pruning, release or retention comparison was run.

Evidence is preserved under [round-2 candidate 1 evidence](optimization-evidence/cycle7/r2_c1_unused_port_index_omission/). The worker branch/commit was `codex/opt/unused-port-index` / `cc6ce6a` (instrumentation only); the candidate implementation remains unintegrated. Introduced preparation flags, filtered cursor branches, linear fallback scans and their retention costs remain unmeasured. A comparison still requires baseline/candidate reproduction of the collection-pressure failure; do not weaken that obligation.

## Round 2, candidate 2: factored isolated components

Boundary tests and baseline diagnostics were completed, but no executable factoring mechanism or isolation certificate was implemented; no candidate observations were collected.

Five boundary tests passed in both ordinary and diagnostics release modes, and nine baseline observations covered fresh-contract in grouped/interleaved order, fresh-unmerged, one-copy, rewrite128, held output and snapshots. Fresh-contract grouped cases recorded 2,048 logical applications, 3,648 body posts and 23,040 sampled graph nodes at 64 groups; at 256 groups they recorded 8,192 applications, 14,592 posts and 92,160 nodes. These establish the comparison baseline only. The intended template preparation, identity/event mapping, frontier, expansion, retention and release costs were never exercised.

Evidence is preserved under [round-2 candidate 2 evidence](optimization-evidence/cycle7/r2_c2_factored_isolated_components/). The worker branch/commit was `codex/opt/factored-isolated-components` / `008f480`; it remains unintegrated. The hypothesis is unresolved; its strongest next test is one canonical transition shared by two equivalent groups while stepping, snapshotting, advancing, cancelling and releasing their separate logical frontiers.

Round 2 candidate 3 is the larger source-certified single-world executor with flat occurrence storage. It must be treated as a replacement foundation, with explicit costs for preparation, flat records, identity/attachment migration, snapshots, observation expansion and cleanup—not merely as a zero-graph-node result.

## Round 2, candidate 3: single-world flat occurrences

A flat-executor prototype was started, but it was not compiled, validated, connected to the measurement runner or measured.

The prototype contains a source-level flat record/identity/attachment direction and an experiment declaration, but all intended costs remain unmeasured: recognition and opcode preparation, flat-record/live-bit writes, union-find and attachment migration, pending bodies, dead-slot retention, snapshot copy-on-write, observation canonicalization/expansion, cancellation, queues and destruction. No semantic checks, workload observations or runtime-based verdict were claimed. The strongest next test is a small two-copy/two-level boundary fixture covering equal-tuple multiplicity, distinct fresh witnesses, a snapshot between application and posting, stepping, cancellation and complete reclamation before connecting maintained diagnostics.

Evidence is preserved under [round-2 candidate 3 evidence](optimization-evidence/cycle7/r2_c3_single_world_flat_occurrences/), including the unintegrated prototype source. The worker branch had no candidate commit and remains unintegrated. Round 2 ended without an integration winner: candidate 1 had an unresolved correctness failure before comparison, candidate 2 had not implemented factoring, and candidate 3 had not compiled or validated its prototype. No architectural rejection follows from those incomplete results.
