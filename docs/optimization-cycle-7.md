# Cycle 7: first optimize-skill campaign

## Round 1, candidate 1: transient control lowering

**Status:** BLOCKED. No engine implementation or executable lowering prototype was completed; nothing was eligible for integration.

The candidate targeted the diagnosed cost in `fresh-contract`: transient control rows are posted into the graph and later collected/released. The worker established a semantic boundary instead of weakening the API. Committed controls are observable through `Engine::facts`, late snapshots, and late stepping even with history disabled. A pending-syntax substitute would therefore change observable state, snapshot retention, or fresh occurrence multiplicity.

The focused fixture passed 2 ordinary and 2 diagnostics release tests, including late snapshot retention through cancellation, distinct fresh witnesses for equal control tuples, late stepping, and complete reclamation. Broad validation was not claimed because an existing notebook startup assertion stopped the broader run. No candidate comparison was made, and no runtime or timeout was used as a verdict.

Baseline evidence and worker artifacts are preserved under [cycle-7 candidate evidence](optimization-evidence/cycle7/r1_c1_transient_control_lowering/). The worker branch/commit was `codex/opt/transient-control-lowering` / `981302f`; its strongest untested follow-up is a virtual committed-occurrence representation with explicit promotion to the general graph on observation/escape. That is a materially different representation question, not an automatic continuation follow-up.

The next candidate is the independent `central_occurrence_support` simplification: test whether one versioned occurrence-support authority can remove duplicated support rewrites and their release work without increasing dead-posting scans or retained snapshot state. A separate `segmented_graph_ownership` candidate remains in this round for breadth.

## Round 1, candidate 2: central occurrence support

**Status:** BLOCKED. The worker completed a diagnostics/pinned-root slice but no central-support implementation or workload comparison; diagnostics-only code is not an optimization result.

The slice added counters for fact-support accesses, secondary support writes, and secondary-row reads, plus a pinned-cursor fixture covering conditional support, equal-tuple multiplicity, old roots, and eventual reclamation. Fifty-one focused release/diagnostics tests passed. The counters establish the event boundary needed for a later implementation, but do not establish avoided work, allocation, retention, or non-inferiority. The full diagnostics run hit the same notebook CLI startup assertion recorded in candidate 1; no broad-suite pass is claimed.

Evidence is preserved under [candidate 2 evidence](optimization-evidence/cycle7/r1_c2_central_occurrence_support/). The diagnostic branch/commit is `codex/opt/central-occurrence-support` / `523d3a4`; it remains unintegrated. The hypothesis is unresolved because pruning, coordinate substitution, pinned versions, and dead-posting reclamation still need an implementation-level comparison.

The final round-1 candidate was the materially different `segmented_graph_ownership` hypothesis: test whether region-level graph ownership can eliminate per-node release traversal and collection bookkeeping without survivor-copy, cross-region, or retained-reader costs replacing it.

## Round 1, candidate 3: segmented graph ownership

**Status:** BLOCKED. The worker reached a committed instrumentation and experiment declaration, but no executable segmented-ownership mechanism or candidate comparison was completed.

The baseline instrumentation covered ownership operations, allocation, retention, collection and reclamation, and the preserved baseline passed 72 focused release/diagnostics tests across store, graph, pruning, ownership, inspection, history, cancellation, matching and collection. The partial `Region`/`NodeRef` conversion in `src/store.rs` remained uncommitted and was never built or measured. Therefore there is no evidence yet for either avoided per-node release work or introduced survivor-copy, cross-region, or retained-reader cost; no timing or timeout was used as a verdict.

Evidence is preserved under [candidate 3 evidence](optimization-evidence/cycle7/r1_c3_segmented_graph_ownership/). The worker branch/commit was `codex/opt/segmented-graph-ownership` / `5f91f1e`; the uncommitted partial implementation was discarded with the isolated worktree. The ownership hypothesis remains unresolved, but this round does not justify another ownership variant without a materially different mechanism or a better-bounded implementation plan.

Round 1 is complete with three blocked candidates and no integration winner. Candidates addressed the diagnosed fresh-contract graph/store insertion, collection and release costs, but the first two stopped at semantic/diagnostic boundaries and the third at implementation integration. The next round must prioritize a fresh architectural direction—such as compiler-derived query specialization, factored expression execution, or shared producer/control work—with explicit evidence of avoided work and all preparation, execution, observation and cleanup costs before implementation begins.

## Round 2 selection

Round 2 keeps the established fresh-contract diagnosis but changes the mechanisms under test. The selected frontier is: (1) compiler-certified omission of port indexes for relations absent from every source rule head, (2) factored execution of conservative alpha-equivalent isolated components, and (3) a source-certified single-world executor with flat occurrence storage for the deterministic fresh-contract subset. These are ordered from the smallest representation simplification to the largest replacement foundation. A proposed shared failure certificate was not selected because its first benefit is failure-heavy constructor workloads, which are not part of the current diagnosed fresh-contract cost.

Each candidate must preserve committed observation, snapshots, cancellation, multiplicity, fresh identities and cleanup, and must compare graph/allocation/collection/release work plus its own preparation, execution, observation and cleanup costs. No incumbent-counter disappearance, runtime ratio or timeout is sufficient for integration.
