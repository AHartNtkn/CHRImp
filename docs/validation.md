# Validation

The implemented language includes variable-only relation arguments, explicit equality, nonbinding multihead matching, all three CHR head modes, fresh body variables, explicit disjunction, committed rule scheduling and residual answers without deduplication. The local notebook supports text and graph editing, ordered-port diagrams, execution controls, alternative inspection and optional history.

## Evidence

| Requirement | Evidence |
|---|---|
| Parsing, variable scope and common editor model | `tests/syntax.rs`, `tests/program.rs`, `web/notebook.test.mjs` |
| Nonbinding matching, equality, consumption and propagation | `tests/matching.rs`, `tests/identity.rs`, `tests/commit.rs`, `tests/semantics.rs` |
| Explicit search, shared execution and exact answer multiplicity | `tests/semantics.rs`, `tests/balanced.rs`, `tests/output.rs` |
| Finite progress beside divergence and bounded delivery | `tests/progress.rs`, `tests/step.rs`, `tests/output.rs` |
| Continuing reclamation and retained-state ownership | `tests/lifecycle.rs`, `tests/compaction.rs`, `tests/cancel.rs`, `tests/arc_ownership.rs` |
| Notebook protocol, editing, recovery and recording | `tests/notebook.rs`, `tests/inspection.rs`, `tests/pending_body.rs`, `web/*.test.mjs` |
| Autonomous notebook execution and shared CLI driver | `src/runtime.rs`, `tests/cli.rs`, `tests/notebook.rs`; the HTTP test leaves execution running without requests, then reads its retained answers |
| Arithmetic, type synthesis, behavior synthesis and lambda notebooks | `examples/*.chrnb`, `tests/notebook_arithmetic.rs`, `tests/notebook_synthesis.rs`, `tests/notebook_lambda.rs` |
| Automatic structural normalization, conditional dispatch and source observation | `tests/constructor_lowering.rs`, `tests/normalization_observation.rs`, `tests/cli.rs`, `tests/notebook.rs` |
| Validated public execution plans and borrowed graph access | Compile-fail examples in `src/program.rs` and `src/engine.rs`, plus `tests/lifecycle.rs` |

Notebook tests check execution without browser requests, exact replay of retained output, explicit pause/resume/step/cancel, and control responsiveness during continuing execution. The synthesis checks use independent SK reduction and type inference; the lambda benchmark checks a permitted reduction history and its exact residual graph.

Fair service includes source work, completion, observation and reclamation. A frozen pending-work view excludes unfinished scopes; children are admitted before their parent obligation retires. The FIFO mutation lane revalidates against current state before publication. Immutable readers and explicit snapshots retain their dependencies. Tests exercise these obligations under divergence, conditional updates, collection and cancellation; they are not a formal proof over every program.

Coordinate unit tests exhaust every source/target pair through 19 sparse
publications against a Boolean truth table, including repeated keys and segment
boundaries. They check frozen targets during publication, functional-image
fallback, collection at transport/discard suspensions, interior retirement,
cache-only Boolean roots, and bounded composition retention. Existing integration
tests cover snapshots, inspections, history, runtime replay, held output,
cancellation and fair finite progress through the same engine.

Prepared programs derive constructor applicability once, and both ordinary and recording engines share that plan. Exact same-tag consistency and complete cross-tag failure rules support conditional identity attachments; whole-program consumer checks protect their validity. Programs outside that admitted class execute general CHR. Recognized disjunctions reuse known conditional tag support, retaining the selected original arm and its field semantics; uncovered support executes the original generative disjunction. Independent choices retain their multiplicity, and cycles require no occurs check.

Structural applications retain their source rule IDs and names. A logical step stops after head consumption with the actual RHS pending; field equalities then execute as source bodies, and terminal failure retains the mutation lane until its active support is updated. Recording captures actual source-corresponding Post, Merge, Application and Failure events. Dispatch inspection exposes only alternatives still pending. History controls recording without selecting a different executor. This schedule need not match historical prototype schedules, so their timing results are not production measurements.

Ordinary execution keeps pending syntax in live bodies until the first requested
view. `tests/pending_body.rs` checks descriptor-free execution, fresh variables
and both answers, first capture at 80 suspension points, and views opened during
cancellation after task discard begins. Existing descriptor-epoch, conditional
projection, step/history and alias-retention tests also cover the promoted path.
The maintained `life-pending-snapshot` and `life-pending-cancel` measurements
exercise growing pending frontiers and exact post-cancellation syntax/release
oracles; see `docs/performance.md`.

Graph updates finalize at most eight private index writes together while keeping
their existing yield and publication positions. `tests/store.rs` checks ordered
batch writes against an independent map, full-width keys, unchanged roots,
counts, frozen/stale/foreign rejection, retained snapshots and reclamation.
The graph unit tests cover collection at every suspension for generic and
certified 9/32-port updates, abandonment at 68 wide-update positions, and stale
root rejection before deferred progress. `tests/cancel.rs` includes a wide
conditional rewrite. The maintained `wide-rewrite` oracle checks every ordered
port, distinct query identities and duplicate occurrence multiplicity.

`tests/batch_coalescing.rs` adds a diagnostic forward ordered-map oracle for
reverse Store batch coalescing: dense and full-width keys, duplicate overwrite
order, final no-ops, insert/delete transitions, empty inputs, both sides of
eight-key promotion, immutable snapshots/cursor pins through collection, and
complete bounded release. Its resource checks reject repeated quadratic suffix
scans and heap/hash work for long batches over at most eight distinct keys.
The maintained allocator covers preparation, batch work, validation and drops;
Round 018's paired audit charges table growth and preserves unscoped harness
traffic separately.

Persistent indexes pack multiple entries from one aligned eight-key final-word
interval into a page. Three-word prefix certificates remain exact; sparse
boundaries retain crit-bit branches. `tests/store.rs` checks dense storage,
scalar/batch edits against independent ordered maps, boundary ranges/counts,
immutable snapshots, filtering with collection at each suspension, discard,
and bounded final release. Store unit tests check interrupted page archive
registration, archive reuse/invalidation, and weak-prefix identity witnesses.
Collection and filtering expose one scalar payload at a time within a page.

## Performance and limits

The existing `measure` executable covers unsuccessful and successful joins, conditional equality and consumption, correlated choices with two answers, independently sized answer streams, recursive reachability, preparation, continuing execution, retained archives, and actual notebook programs. `life-archive` holds a fixed number of snapshots while measuring 2,048 additional applications; `life-history-choice` grows history with a live choice. These distinguish archive size from useful execution work.

Compare geometric sizes within a regime. Exact tuple/identity/multiplicity checks remain enabled; validator time is reported separately. First-answer latency ends at a complete answer. Prefix success does not claim search exhaustion. Collection-flag ticks include time awaiting the maintenance lane, so they are not an internal phase profile. Sampled object counts are neither byte usage nor exact peaks.

Measured changes, using deterministic `advance(1)` work and the same validated workload on each side:

| Workload | Before | After | Change |
|---|---:|---:|---|
| Rejected three-head join, 128 rows per head | 900,799 | 78,118 | Selective partner planning and complete-tuple lookup |
| Repeated aliases, 128 aliases and probes | 368,639 | 71,092 | Bound index planning by the cost of scanning the relation |
| 256 empty answers | 890,774 | 197,435 | Shared support summaries during enumeration |
| 256 index leaves sharing a 64-choice condition | 83,199 | under 4,000 | Reuse each condition's substitution within the pass |
| 2,048 applications with 2,048 fixed snapshots | 469,468 | 83,645 | Cache immutable archive reachability |

These are isolated comparisons, not additive speedups. The current combined suite takes 78,116 ticks for the rejected join and 204,424 for 256 empty answers. Fixed-archive continuation takes 83,008 / 82,141 / 83,645 ticks with 128 / 512 / 2,048 snapshots. Growing history with a live choice takes 563,304 / 1,132,656 / 2,271,360 ticks for 8,192 / 16,384 / 32,768 applications. Every archive run checks retained answers and complete reclamation after release.

Three-run median process CPU time confirms material improvements for the matching probes: rejected join 98.5→11.7 ms, multiport rejection 29.7→9.0 ms, and repeated aliases 52.6→9.1 ms. These include process startup and answer validation and are specific to the measured machine.

Boolean decomposition order adapts independently of semantic choice identities. A grouped 12-pair correlation probe retains 35 condition nodes after collection, versus 12,284 with its original order. This is a bounded heuristic; some orderings still cost more. Archive protection also has a memory cost: condition nodes are 88 rather than 64 bytes, and persistent index records are 112 rather than 104 bytes on the measured 64-bit build. Ordinary execution does not retain snapshots or history unless requested.

Output, explicit history and genuine search frontiers can grow. Unrestricted synthesis may exceed its budget; an incomplete search is not evidence that no answer exists. A faster probe is meaningful only with correct answers, preserved fairness, and complete reclamation after ownership is released.

The type-driven identity probe reaches its first validated answer in 10,344,621 ticks. Behavior-synthesis timings are recorded below. Two nested lambda identity applications reach their first validated answer in 126,600,717 ticks (about 29.8 seconds in that run); cleanup fully reclaims unowned payloads. This remains expensive. A phase profile of its first 50 million ticks attributes 26.69 million to search/matching/commit and 13.79 million to completion scanning. Actual collector work, including subsequent cleanup, totals about 8.02 million ticks. The collection-status counter must not be used to attribute that run to GC.

Final targeted checks of unchanged-condition proof reuse, older-prefix projection and unchanged-input completion reuse did not demonstrate another major end-to-end improvement. Remaining Boolean-processing hotspots are documented as performance limits, not proven unavoidable costs. The optimization pass stops at this evidence boundary rather than treating every possible improvement as unfinished delivery.

## Behavior synthesis comparison (2026-09-12)

The behavior notebook permits cyclic relational structures. Its 19 saved queries use the same evaluator and synthesis rules. Three sequential native release runs per target measured the first complete answer, excluding parsing, preparation, validation and subsequent cleanup. The command was `target/release/examples/measure notebook-behavior-TARGET 1 50000000 15` for `i`, `k`, `b`, `c`, and `w`. Successful answers passed independent SK reduction; all runs completed cleanup.

| Behavior target | CHR median first answer | Existing rwLog median first answer | CHR / rwLog |
|---|---:|---:|---:|
| Identity (I) | 134.208 ms | 0.522035 ms | 257× |
| Constant (K) | 20.346 ms | 0.165143 ms | 123× |
| Composition (B) | No answer within budget | 936.279 ms | — |
| Swap (C) | No answer within budget | No answer within 15 s | — |
| Duplicator (W) | No answer within budget | 20.209 ms | — |

CHR limits are 50 million ticks or 15 seconds, whichever comes first; a budget result is not search exhaustion. The rwLog values reuse the existing three-run measurements at commit `6e45ef1d62672fda8fd63bc1cb896c99f9224e3f`; rwLog was not rerun. These compare the same behavioral targets, not identical internal operations or search order. The current CHR checks do not impose finite structure globally, so agreement on these returned witnesses does not establish identical accepted domains.

## Running checks

```sh
cargo test --offline --all-targets
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
node web/notebook.test.mjs
node --test web/*.test.mjs
cargo run --release --offline --example measure -- sparse 128
cargo run --release --offline --example measure -- rejected3 128
cargo run --release --offline --example measure -- answers 256 --rows 0
cargo run --release --offline --example measure -- life-archive 512
cargo run --release --offline --example measure -- notebook-type-i 1
cargo run --release --offline -- --notebook
```

Socket tests require loopback access. Choose validation according to the behavior and ownership affected by a change. A commit alone does not require another browser session, performance matrix or complete test run.
