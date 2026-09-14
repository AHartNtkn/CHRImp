# Rehearse round 003: canonical certified occurrence support

Implementor recommendation: **KEEP** this evaluated slice. Corrected landing
disposition: **KEEP**, integrated as `4c9ae4e` on `codex/shared-relational-engine`.
Parent before the experiment was `01d5d60320777adbf71a828a78ed914cbfbbc851`;
the evaluated implementation is commit `c8c0ed6dc1cda40011b7d849bd20e12fa303cf5b`.
All work was performed in `/tmp/chrimp-opt-round003-versioned-structural-occurrences`.

## Implemented mechanism and limits

Certified structural occurrences now use the persistent RELATION column as
their authoritative support version. Its key contains the relation and unique
occurrence ID; the existing immutable payload contains ordered raw variable
fields. A pinned graph root selects the old support version. Post, narrowing,
and removal stage this entry before yielding through remaining index updates.
`Graph::fact` reconstructs the borrowed generic fact view from that entry and
payload. Graph collection and archive protection trace the same authoritative
entry. There is no second FACT entry for these occurrences.

Preparation now distinguishes a certified occurrence with all ports indexed
from an uncertified occurrence. This extends storage eligibility to unary
structural relations, not just the three relations with omittable fields in the
behavior notebook. Existing whole-program rejection of interfering consumers
is unchanged. Uncertified programs/relations and standalone public graphs retain
the generic path. Source observations, relation cursors, raw port filtering,
conditional identities and tuple indexes continue to use the same APIs.

This is a concrete partial implementation of versioned occurrence storage, using
the engine's existing persistent column/root mechanism. It does not introduce a
separate immutable chain, eliminate RELATION/INCIDENCE/required PORT entries, or
remove all structural update work. Ordered fields were already immutable and
shared. Reusing existing root versioning avoids a second ownership, coordinate
transport, reconstruction-cache or collector subsystem. The original candidate's
larger storage replacement is not claimed implemented.

Each changed certified occurrence update removes one supported index leaf and
its duplicate write, plus the associated persistent path/update and subsequent
traversal work. The diagnostic `avoided_fact_writes` counts these changed updates;
unchanged/false posts do not claim savings. Remaining specialized write counts
exclude the immediately staged authoritative entry, which remains included in
`graph_writes`. Never add these overlapping counters as exclusive work.

## Equivalent completed endpoints

Five diagnostics-feature samples per side, using the maintained `perf.py` runner
and exact notebook validators. I is always `App(App(S, K), Hole(16))`; S is always
`S`. Both are validated first-answer prefixes, not exhausted searches. Medians:

| Measure | I baseline → candidate | S baseline → candidate |
|---|---:|---:|
| Applications | 126 → 126 | 134 → 134 |
| Source posts | 244 → 244 | 222 → 217 |
| Sampled peak graph nodes | 3,088 → 2,807 (-9.10%) | 3,069 → 2,797 (-8.86%) |
| Peak requested bytes | 2,009,458 → 1,975,822 (-1.67%) | 1,811,278 → 1,745,917 (-3.61%) |
| Total requested allocation bytes | 15,845,028 → 15,670,993 (-1.10%) | 15,156,971 → 15,195,101 (+0.25%) |
| Graph writes | 2,678 → 2,529 | 2,640 → 2,465 |
| Direct avoided FACT writes | 0 → 149 | 0 → 150 |
| Sampled peak conditions | 672 → 665 | 567 → 568 |
| Validator allocation bytes | 2,920 → 2,920 | 5,056 → 5,056 |
| Cleanup allocation bytes | 5,672 → 5,672 | 6,152 → 6,312 |

I candidate visits remain 770; the first raw samples' matching/commit dispatches
are 23,629/3,453 → 23,640/3,470. S visits are 705 → 691, with dispatches
23,122/3,369 → 22,621/3,379. Shorter updates can alter permitted scheduling and
collection timing. S's five fewer posts and the extra 25-write difference are
not attributed to the direct removed record. S retains 2,173 graph nodes at its
final source checkpoint versus 2,153, despite the lower peak. No individual
tick reduction is used as architectural evidence.

`comparisons.json` contains the unchanged maintained comparator's three-metric
comparisons for each target/retention point. All qualify decreases in measured
graph storage and requested residency; S also qualifies its small total
allocation increase. Statistical change is not a materiality verdict. Five
samples and this finite control set do not establish universal no-regression
or stable timing claims. B/C/W and rwLog were not rerun.

## Retention, projection and reclamation

Five samples per side at each point; cyclic ternary payloads, four owners or
continuing siblings, and 64 continued applications. Held application counts
match exactly, as do resumed counts and per-rule totals.

| Probe / rows | Held applications, both | Held graph nodes | Total allocated bytes | Peak requested bytes |
|---|---:|---:|---:|---:|
| Fixed archive / 16 | 64 | 310 → 277 | 1,437,148 → 1,417,461 | 185,868 → 181,618 |
| Fixed archive / 64 | 64 | 1,179 → 1,049 | 3,144,242 → 3,067,115 | 503,751 → 502,327 |
| Inspectors / 16 | 321 | 310 → 277 | 3,747,910 → 3,728,223 | 301,269 → 296,853 |
| Inspectors / 64 | 935 | 1,178 → 1,049 | 11,092,828 → 10,994,885 | 611,237 → 606,557 |
| Held output / 16 | 75 | 192 → 160 | 1,494,492 → 1,484,069 | 183,182 → 178,996 |
| Held output / 64 | 84 | 661 → 533 | 2,715,616 → 2,673,065 | 307,142 → 290,668 |

At fixed progress, eliminated storage grows with payload width: 32/128 nodes
for held output at 16/64 rows; approximately 33/129 for frozen projections.
The held checkpoints can include one additional transient node. This is a
linear reduction in representation size, not a change in asymptotic complexity.
Held-output reductions are 16.67%/19.36%; fixed archives/inspectors are about 11%.
Direct avoided writes are exactly the 16/64 certified payload posts, with zero
fallback lookups on these source workloads.

All 60 structural retention runs pass every ordered-field, cyclic-identity,
payload/multiplicity and release oracle. Fixed archives and inspectors each
deliver four checked projections with **zero additional source applications**.
Held output completes its one finite answer beside live divergent siblings;
resumed applications are 93/156 on both sides for 16/64 rows.

Cancellation retains intentionally pinned snapshots. Final release then leaves
zero graph nodes, occurrences, conditions, pending storage, descriptors,
history, snapshots, inspectors and queued release batches. The still-live
engine coordinate is released at destruction. All notebook runs additionally
pass `prepared_released`; final requested live bytes are 1,729 on both sides
(harness/process state). Lifecycle final requested-live values, including
remaining harness state, are recorded per sample in `measurements.json`.

## Displaced costs and phase accounting

Behavior preparation has 9 certified occurrence plans instead of 3 sparse-only
plans, with 23 remaining opcodes instead of 14. Retained plan payload grows
1,048 → 1,310 bytes. Setup requested allocations grow by 60 bytes on I and S;
temporary preparation allocations also disappear when uncertified port vectors
are dropped. No per-occurrence version metadata or reconstruction cache is added.
`fact` looks up the immutable row before choosing the support key, and collection
recognizes certified RELATION entries. Generic reads/writes still pay the small
plan dispatch. Existing retained roots share versions and existing incremental
release reclaims their nodes; ordinary execution adds no history ownership.

At 64 rows, fixed-archive engine allocation falls 1,727,608 → 1,671,160 bytes and
inspection allocation stays 362,448. Inspector engine allocation falls
9,769,236 → 9,691,972 and inspection stays 271,432. Cleanup allocation stays
14,520 for both. Held-output engine allocation falls 2,353,884 → 2,311,564 and
cleanup stays 4,752. Lifecycle setup, payload validation and report construction
are included in whole-process totals, though some are charged to `other` by
the existing harness; zero separate `setup`/`validator` fields do not mean free
preparation or validation. Phase checkpoints are cumulative and not summed.

The 64-row raw lifecycle clock medians below charge the held-reader boundaries.
Admitted/held/resumed are cumulative milliseconds from interaction start; cancel
and release have separate clocks. They are diagnostic prefix observations: the
maintained summary intentionally leaves ongoing-source latency incomplete even
when an interaction milestone finishes. These numbers are not exhaustion times.

| Probe | Admitted | Held | Resumed/validated | Cancel | Final release |
|---|---:|---:|---:|---:|---:|
| Fixed archive | 3.550 → 2.742 | 3.897 → 3.080 | 4.893 → 4.047 | 0.01549 → 0.01560 | 0.12754 → 0.09635 |
| Inspectors | 8.804 → 8.287 | 9.221 → 8.709 | 10.275 → 9.735 | 0.01577 → 0.01609 | 0.10387 → 0.09747 |
| Held output | 1.448 → 1.166 | 1.884 → 1.588 | 2.544 → 2.249 | 0.10599 → 0.09984 | 0.00237 → 0.00237 |

Notebook phase medians in milliseconds (diagnostic clocks, including contention
and instrumentation; not acceptance/ranking evidence):

| Phase | I baseline → candidate | S baseline → candidate |
|---|---:|---:|
| Parse | 0.9120 → 0.6571 | 0.8871 → 0.7169 |
| Prepare | 0.3007 → 0.2136 | 0.2845 → 0.2082 |
| Engine initialization | 0.01172 → 0.00772 | 0.01127 → 0.01016 |
| Source through End, including validation | 23.1374 → 20.9172 | 19.6146 → 20.2782 |
| Validator subset | 0.01454 → 0.02086 | 0.01461 → 0.01631 |
| Cancellation/collection | 0.4247 → 0.4081 | 0.3868 → 0.3146 |
| Engine and Prepared destruction | 0.03168 → 0.03258 | 0.03338 → 0.03317 |

Validator allocation and validated payloads are identical; the tiny clock
differences are not interpreted as additional semantic work. S's engine
allocation grows 37,852 bytes and cleanup grows 160 bytes. Its peak conditions
grow by one median node and its final graph by 20 nodes; those costs do not
outweigh its 272-node sampled peak reduction and lower requested residency.

All 160 generic/control samples pass: rewrite 16/32/64/128, partial-hit
16/32/64/128, duplicate heads 2/4/8/16, and prepare-shape 1/2/4/8. Applications,
graph writes and exact result oracles agree. Non-preparation allocation totals
increase by exactly 58 bytes per point (diagnostic reporting); preparation
totals fall by 18. Requested peaks are identical except duplicate-heads-2,
which increases by 38 bytes. These are not significant regressions.

## Reproduction and evidence

Builds and executions were separate. Baseline measure was built before any
production edits; test-only edits do not affect that example. Both binaries use
release plus diagnostics, the same allocator and unchanged measurement/oracle
code. No common-harness patch is needed.

```sh
cargo build --offline --release --features diagnostics --example measure --target-dir target/round003-baseline
cargo build --offline --release --features diagnostics --example measure
```

Binary SHA-256, baseline then final candidate:

```text
dda034092d8a0ddba3a6f650e25b1b59f22566c8211ca68bc3a3073fa1b250e0
acf437fe541516b87f975e2a6451a4aa8557d8ecf974070b326c9f6b2e1c1423
```

For each side, substitute its binary and new output directory:

```sh
python3 examples/perf.py --binary BINARY --out OUT --repeat 5 --warmup 0 --seconds 10 --total-seconds 60 -- notebook-behavior-i 1 50000000 3 --detail
# Also notebook-behavior-s with identical arguments.
python3 examples/perf.py --binary BINARY --out OUT --repeat 5 --warmup 0 --seconds 10 --total-seconds 60 -- life-archive-fixed-structural 4 50000000 3 --rows 64 --work 64 --cadence 4
# Also life-inspections-structural and life-held-output-structural; rows 16 and 64.
python3 examples/perf_suite.py deep --binary BINARY --out OUT --repeat 5 --seconds 180 --sample-seconds 10 --only rewrite --only partial-hit --only duplicates --only prepare-shape
```

Runner/native limits supervise measurements, not this implementation or campaign.
All requested observations complete without censoring. Main raw cohorts are
`evidence/round003/{baseline,final,controls-baseline,controls-candidate}`;
an earlier `candidate` cohort is retained locally but excluded from the reported
comparison after refining the unchanged-update diagnostic and extending staged
collection test coverage. No semantic implementation change separates cohorts.

`measurements.json` copies selected existing campaign statistics, sample paths,
SHA-256s and semantic/lifecycle endpoints; it is descriptive evidence, not a
new measurement system. Raw samples and binary outputs remain in this worktree.
To reproduce comparisons:

```sh
python3 examples/perf_compare.py BEFORE AFTER --metric allocations.total_allocated_bytes --metric allocations.process_peak_requested_bytes --metric workload.memory_counts.sampled_peak.graph_nodes --out OUT.json
# Retention uses phase.interaction_held.0.memory.graph_nodes for the third metric.
```

## Test-first implementation and final verification

Before production edits, the new
`graph::update_ownership_tests::certified_occurrence_versions_share_one_support_record_and_pin_payloads`
test failed meaningfully: `certified occurrence still duplicates its versioned
support in FACT`, left 2, right 0. Its preceding checks cover duplicate IDs,
old roots, conditional narrowing, removal, raw lookup and retained payloads.
After implementation it passes for both ternary cyclic and unary occurrences.
The direct failing invocation was:

```sh
timeout --kill-after=5s 60s target/release/deps/chr-c3caca7f2213860a --exact graph::update_ownership_tests::certified_occurrence_versions_share_one_support_record_and_pin_payloads --nocapture
```

The new integration test checks unary/cyclic fields across a merging alternative
and two distinct identical successful alternatives, then complete cancellation.
Existing per-tick staged collection tests now also cover certified occurrences
with every port and tuple indexed, false posts and staged removal, including
condition collection between each update tick. Existing raw fallback tests
cover wrong-port/wrong-relation filtering, old cursors and release.

Final builds:

```sh
cargo test --offline --release --all-targets --features diagnostics --no-run
cargo test --offline --release --all-targets --no-run --target-dir target/round003-native
```

Final executions and static checks:

```sh
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --features diagnostics --quiet
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --target-dir target/round003-native --quiet
CHR_MEASURE_BINARY=target/release/examples/measure timeout --kill-after=5s 60s python3 -m unittest discover -s tests -p measure_observation_test.py
cargo clippy --offline --all-features --lib --bins --examples --test constructor_lowering --test normalization_observation -- -D warnings
cargo fmt --check
git diff --check
```

Results: **484 diagnostic tests, 458 ordinary release tests, 3 Python observation
tests; all pass.** Full logs are `tests-diagnostics.txt` and `tests-native.txt`.
Each actual test invocation used exactly `timeout --kill-after=5s 60s`; builds
used `--no-run`. An initial full-suite CLI failure was investigated: the sandbox
rejected local socket binding with OS error 1. Rerunning with loopback permission
passes the full notebook/CLI suites. There were no test timeouts.

The suites cover nonbinding heads, identity/conditional support, duplicate
occurrences, ordered propagation history, fresh body variables, explicit-choice
multiplicity, committed execution, finite progress beside divergence, sharing,
source Post/Application/Failure observations, pending bodies, stepping, history,
snapshots, cancellation, compaction and final reclamation. This is tested
evidence rather than a universal proof. The independent landing review
confirmed the same semantic, lifecycle, and storage evidence before integration.

KEEP is recommended for the measured representation/storage reduction and
directly removed update work, with no significant regression in the measured
controls or lifecycle phases. The larger candidate's unimplemented ambitions
do not raise the bar for this useful slice; the small S allocation increase
does not erase its independently valuable storage improvement.
