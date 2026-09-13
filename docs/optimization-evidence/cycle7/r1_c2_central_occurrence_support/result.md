## OPTIMIZATION RESULT

**Slug / round:** r1_c2_central_occurrence_support / cycle 7, round 1, candidate 2
**Kind:** simplification
**Verdict:** BLOCKED
**Reason:** Closed at the user's instruction after committing diagnostics and pinned-root tests. No central-support implementation was completed. This is neither a KEEP_CANDIDATE nor evidence against the architectural hypothesis.
**Worktree / branch / candidate commit:** /tmp/chr-cycle7-opt-central-occurrence-support / codex/opt/central-occurrence-support / central-support commit: none. Diagnostics/fixture commit: 523d3a4a6d7a231d9a6e581cdb8029953c607ff4.
**Baseline manifest / source commit:** manifest.json in this evidence directory / 2d6faa69c95068475601c567dfa9b27c308b75e2. Instrumented baseline binary and its SHA-256 are retained; build began before committing its identical source changes, as recorded in baseline-build/run.json.
**Protocol / helper identity:** protocol/measurement-protocol.md SHA-256 51c48a61afd54a188f2a907232c6b0ba6ca7711c02bb6d832b0e4f843e28e798; protocol/analyze_timings.py SHA-256 b384ab842efb90cf915938bae7ee029f7ac886c6ad55c207402ea7c2f3c0e97c. Helper not executed. Repository architecture rules supersede timing acceptance; no runtime verdict.
**Evidence directory:** /tmp/chr-cycle7-opt-central-occurrence-support/docs/optimization-evidence/cycle7/r1_c2_central_occurrence_support
**Correctness:** Nightly 1.95.0, offline release diagnostics: 51 focused tests passed across central_support, graph, graph_prune, shared_restrictions, matching and identity (baseline-focused/). Diagnostics measure build passed (baseline-build/). Full diagnostics/all-targets run failed at notebook_default_origin_is_stable_and_port_override_is_honored: empty stdout instead of the loopback URL (baseline-all-diagnostics/). Same failure is documented in committed candidate-1 baseline evidence; sandbox interference is suspected, not established. Unrestricted retry request was interrupted, with no execution record. Formatter and diff checks passed (closeout-format/, closeout-diff/). Full ordinary/diagnostics completion, doc tests, Clippy and Python checks remain unperformed; no integration-ready correctness claim.

### Workloads

Declared in hypothesis.md: fresh-contract64/256 and fresh-unmerged64/256, rows4 depth4 grouped; answers8; common-wide16; life-archive-rotate-conditional2 rows3 work24 cadence4; partial-join512 rows128. Each has **N=0 workload measurements**. Planned units were event counts, requested bytes/allocation calls, retained object counts and after-cleanup reclamation. All mechanism, adverse-cost and noninferiority gates remain unassessed. Effect estimates, uncertainty bounds and allocated statistical alpha: N/A; no statistical analysis or runtime comparison. No fixture oracle changed. No measurement ownership was held.

The counter calibration is a semantic test, not a workload measurement: a real nullary post followed by removal records FACT writes 1→2 and secondary writes 1→2; explicit fact access and set_liveness produce at least two additional authority reads, and one relation result produces exactly one secondary-row event. This establishes those instrumented event boundaries only. No engine allocation, collection, release, reader/archive cost or scaling benefit was measured.

### Files Changed

Committed diagnostics: src/graph/diagnostics.rs, src/graph.rs, src/engine.rs and examples/measure.rs. Adds diagnostics-only counters and reporting for FACT lookups, secondary rows, changed FACT/secondary writes, nonterminal secondary writes and reserved prune authority lookups. Graph write counts exclude bottom-up pruning and coordinate substitution; existing collection/compaction diagnostics still account for their continuation work. The new prune lookup counter is zero because centralization was never implemented, not because pruning has no support work.

Committed symmetric fixture: tests/central_support.rs. Checks equal-tuple occurrence multiplicity, conditional replacements, removal, pinned relation/port/incidence cursors, graph/condition collection and eventual zero graph nodes/occurrences. It also calibrates the counters above. All five source files are in commit 523d3a4; there are no uncommitted source edits. hypothesis.md, run.py, protocol copies, logs, manifest, result and instrumented baseline binary remain untracked durable local evidence. No parent-checkout source, shared skill files or docs/optimization-state.md was edited.

### Insights

Source establishes an existing versioned FACT support entry plus duplicated support in RELATION, PORT/tuple and INCIDENCE leaves. Current nonzero support changes still rewrite those indexes: this commit does not consolidate them. Arrangements cache IDs and subscriber matching already reads support at the subscriber's root, but the producer uses a bucket-only cursor; membership-only production would need a distinct raw identity read. Constructor attachments carry separate conditional semantics and cannot simply become occurrence membership markers.

Pruning currently filters supported leaves and seeds identity reachability from incidence support. A central authority must preserve these conditional seeds and every suspended/root-tracing obligation. Generic store coordinate substitution can remove a FACT when its support maps to FALSE; blindly retaining TRUE secondary markers could leave stale memberships. That path needs explicit treatment alongside pinned and archived versions before the proposed no-tombstone invariant is proven.

Unproven: avoided secondary rewrites/path copies, authority lookup overhead, dead membership/compaction behavior, full support read-path migration, pinned/archive coherence after coordinate changes, phase allocation and release gains, and non-adverse cost across the declared workloads. The narrowly proposed eager-removal version would still copy insertion/deletion paths on unconditional fresh workloads. The broader architecture remains unresolved. No further implementation or measurement was performed after closeout was requested.
