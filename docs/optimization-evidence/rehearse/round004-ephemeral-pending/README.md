# Rehearse 004: demand-materialized pending syntax

**Recommendation: KEEP the implemented partial slice.** Baseline: `4c9ae4e`.
Implementation: `1b1981e85acf645e225ba9b73c95041ef0d67ba3` in
`/tmp/chrimp-opt-round004-ephemeral-pending`, branch `codex/opt/ephemeral-pending`.
This is the implementor's recommendation under AGENTS.md, not a campaign landing
decision. No campaign control files or acceptance rules were changed.

## Actual mechanism and boundary

Ordinary execution now keeps remaining syntax solely in its already-live `Body`
values. It does not allocate persistent body descriptors, synchronize descriptor
parts at each transition, or replace a pending leaf merely to attach a new body
descriptor. This removes descriptor-map storage, allocation, ownership references
and collector traversal. No additional per-body representation is introduced.

The persistent `Pending.scope` certificates, their immutable cursors, and the
completion/step algorithms remain in place. **This does not implement the full
ephemeral pending-task registry or remove ordinary completion scans.** Keeping the
existing conservative frozen union avoids changing its progress proof. The safe
syntax slice has independently useful measured storage/work benefits.

The first requested snapshot or current inspection scans queued and parked tasks,
materializes their nonempty syntax, then permanently switches to the existing
persistent descriptor/epoch path before returning. History-enabled engines start
on that path. The public view boundary rejects capture during collection, and
recording already has persistent syntax when a task is temporarily off-queue.
No source work advances during promotion. Queue access is constant per entry;
parked access uses ordered-map successor lookup, and each live body requires a
descriptor-map insertion and a persistent-index update. First capture is thus no
longer constant work; subsequent captures retain the existing root publication.

Cancellation must preserve views opened **during** discard, including syntax of
already-discarded bodies. Its request remains O(1). Before discarding tasks it
materializes syntax one scheduled entry per cancellation turn. The scan holds
only scalar positions: bodies and certificate roots provide existing GC ownership.
There is no new condition-root or collector path. A capture arriving partway
through that barrier drains the remaining scan before freezing the view.

Compaction can remove a FALSE certificate before its task finishes discard;
promotion skips these scope-free tasks. A full-suite lambda failure exposed this
case during implementation, and the existing exact lambda oracle now passes.

Changed production files: `src/engine.rs`, `src/engine/obligations.rs`,
`src/engine/inspection.rs`, `src/engine/cancel.rs`, `src/engine/diagnostics.rs`.
Tests extend `tests/pending_body.rs` and the persistent descriptor-epoch unit
test. `examples/measure/pending.rs` and its lifecycle routing add two probes to
the maintained measurement executable; performance/validation docs describe them.

## Equivalent completed work and storage

Five diagnostics-feature samples per side at each of 22 paired points: **220
completed samples**, all with maintained semantic/lifecycle oracles enabled.
The following values are medians. Requested bytes are Rust allocator requests,
not RSS; descriptor/node peaks are sampled engine counts, not exact byte peaks.

| Workload | Applications / answers, both | Peak descriptors | Peak requested bytes | Total allocated bytes |
|---|---:|---:|---:|---:|
| rewrite 32 | 64 / 1 | 97 → 0 | 187,554 → 163,682 (-12.73%) | 1,265,646 → 1,232,576 |
| rewrite 128 | 256 / 1 | 385 → 0 | 714,847 → 616,031 (-13.82%) | 4,746,899 → 4,621,509 |
| answers 256, rows 0 | 0 / 256 | 511 → 0 | 1,936,378 → 1,789,018 (-7.61%) | 15,979,948 → 14,025,570 (-12.23%) |
| fresh-contract 4, copies 4, depth 4 | 128 / 1 | 341 → 0 | 450,975 → 342,679 (-24.01%) | 4,153,939 → 4,042,309 |
| duplicate-heads 8, groups 4 | 224 / 1 | 261 → 0 | 843,712 → 788,392 (-6.56%) | 3,799,767 → 3,707,817 |
| partial-join-hit 32, probes 4 | 4 / 1 | 76 → 0 | 330,600 → 309,032 (-6.52%) | 1,745,832 → 1,722,714 |
| behavior I, first answer | 126 / 1 | 146 → 0 | 1,975,822 → 1,961,690 (-0.72%) | 15,670,996 → 15,526,398 |
| behavior S, first answer | 134 / 1 | 226 → 0 | 1,745,917 → 1,717,909 (-1.60%) | 15,195,104 → 15,050,594 |

I remains `App(App(S, K), Hole(16))`; S remains `S`. These are independently
validated first-answer prefixes, not exhausted searches. Both sides retain the
same median source applications, posts, merges, task admissions, indexed visits,
matching/commit dispatches and graph writes on these probes. I visits/matching/
commit work are 770/23,640/3,470; S 691/22,621/3,379. All exact tuple, fresh
identity, duplicate witness, and answer-multiplicity oracles pass.

Completion rows scanned are unchanged: 362 for empty answers, 2,792 for I,
2,648 for S, zero for the displayed choice-free controls. The source pending
collection service count falls 817 → 10 for answers, 390 → 5 for rewrite-128,
346 → 5 for fresh-contract, 952 → 422 for I, and 1,020 → 409 for S. These are
entered collector continuations, including its descriptor work, not CPU time.
The eliminated descriptor storage and allocations establish the gain; a lower
total tick count is not its acceptance criterion.

The six finite controls' peak graph, condition and pending-index counts are
unchanged. I's peak pending nodes grow 186 → 196 and peak conditions 665 → 672;
its peak graph remains 2,807. S's peak pending nodes fall 139 → 136, median
conditions remain 568, and peak graph falls 2,797 → 2,777. Collection timing can
change transient ownership. I's small transient increases do not outweigh its
146 eliminated peak descriptors and lower whole-process requested residency.

The unchanged maintained comparator qualifies decreases in all three selected
metrics (total allocation, requested peak, descriptors) for all eight rows;
see `comparisons.json`. Five samples and this finite workload set do not establish
universal absence of regressions. No runtime metric determined acceptance.

## Retention, history, cancellation and costs displaced between phases

All conditional interaction cases use four owners/siblings, 64 continued
applications and cadence four. Sizes below are committed payload widths.

| Probe / rows | Held applications, both | Held descriptors | Total allocated bytes | Peak requested bytes |
|---|---:|---:|---:|---:|
| fixed archive / 16 | 68 | 7 → 7 | 6,810,018 → 6,790,580 | 248,904 → 248,904 |
| fixed archive / 64 | 68 | 7 → 7 | 10,882,852 → 10,820,902 | 758,712 → 758,712 |
| inspectors / 16 | 106 | 4 → 4 | 10,924,236 → 10,912,110 | 189,899 → 190,039 |
| inspectors / 64 | 194 | 3 → 3 | 37,506,522 → 37,458,868 | 497,870 → 497,870 |
| held output / 16 | 68 | 7 → 0 | 4,434,514 → 4,364,036 | 148,006 → 141,950 |
| held output / 64 | 72 | 7 → 0 | 5,992,150 → 5,897,032 | 266,694 → 245,126 |

Archives/inspectors retain the same descriptors after promotion. All four
projections per run validate the exact committed payload and retained choice
pins with **zero additional source applications**. Held-output runs deliver
their finite answer beside continuing siblings; resumed applications match
(68 at width 16, 80 at width 64). Cancellation and final release pass all owner
and reclamation oracles. The 140-byte inspector peak increase is consistent
with the larger diagnostic report, not a new retained arrangement.

At 64 rows, fixed-archive execution allocation falls 9,807,016 → 9,742,712 bytes,
inspection stays 393,152 and cleanup stays 4,752. Inspector execution falls
36,541,292 → 36,491,284; inspection stays 282,596 and cleanup 4,752. Held-output
execution falls 5,638,876 → 5,541,404; cleanup grows 4,912 → 6,864. Preparation
and validation in the older lifecycle cases are partly charged to `other`;
zero separately scoped fields do not mean those activities are free.

The history-choice control completes 512 applications on both sides. It retains
1,029 descriptors through cancellation and releases all of them afterward.
Execution/cleanup/inspection allocations are identical: 5,637,744 / 112,180 /
1,192 bytes. Total allocation grows only 402 bytes and peak 140 bytes from
diagnostic reporting. Ordinary alias churn completes 1,024 applications and its
finite sibling answer on both sides; total allocation falls 21,937,597 →
20,953,231 and requested peak 277,133 → 156,821 bytes.

Core preparation and validator allocation counts are unchanged. Cleanup is
unchanged at 4,672 allocated bytes for the finite controls. I cancellation now
visits 53 tasks and materializes 24 descriptors, increasing cleanup allocation
5,672 → 13,576 bytes; S visits 51 and materializes 40, increasing cleanup
6,312 → 20,072. Those costs are included in the table's total allocations.
The added counters contribute 402 reporting bytes in the ordinary two-checkpoint
cases. Normal builds contain no diagnostic counters. Engines carry a mode flag
and scalar promotion position, without new per-task allocation.

### Deliberately adverse first-capture/cancellation controls

The new maintained probes independently validate an exact prefix with N tasks,
zero posts/applications and N unposted relations each having N equal ports.
They vary N = 16, 64, 256. Snapshot mode captures twice, cancels, validates both
views including every relation/port, then releases everything and checks final
Prepared ownership. Cancellation mode starts from the same unposted prefix.

| N | Descriptors before capture | Execution bytes deferred | First capture ms, baseline → candidate | Second capture ms, baseline → candidate |
|---:|---:|---:|---:|---:|
| 16 | 16 → 0 | 5,952 | 0.005952 → 0.015875 | 0.000456 → 0.000429 |
| 64 | 64 → 0 | 19,616 | 0.006159 → 0.036587 | 0.000551 → 0.000410 |
| 256 | 256 → 0 | 82,560 | 0.004873 → 0.104947 | 0.000486 → 0.000361 |

After capture both sides own N descriptors. The execution allocation reduction
moves **in full** to first inspection or cancellation on these controls. At
N=256, snapshot execution allocation is 1,649,256 → 1,566,696, inspection is
88,992 → 171,552, and cleanup stays 25,064 bytes. Requested peak is identical
at 2,331,996; total allocation increases 1,452 reporting bytes. No end-to-end
allocation removal is claimed for this workload. Cancellation-only N=256
cleanup allocation grows 14,912 → 97,472, and total grows 1,032 reporting bytes.
Its cancellation clock grows 0.141327 → 0.265483 ms and visits 256 tasks before
discard. This is a real bounded-service cost, not a free cancellation path.

At N=256 snapshot preparation/admission/projection/cancel/release/destruction
clocks in ms are 6.958/2.698/9.938/0.146/0.099/0.043 →
7.131/2.429/10.000/0.141/0.099/0.040, plus the separately shown captures.
These raw phase medians are diagnostic context. The maintained summary leaves
some unfinished-source latency fields unavailable; the completed phase records
in the raw evidence retain their clocks. First-capture linear task visitation
is an explicit API tradeoff, and very large requested frontiers can cause a
larger synchronous pause. The tested maximum adds about 0.1 ms. This tradeoff
and the one-time cancellation pass do not outweigh the demonstrated ordinary
storage reductions; no universal first-capture latency bound is claimed.

All final release checks leave zero graph/occurrence/condition/history/pending/
descriptor/snapshot/inspector/release-batch storage. The empty live Engine's
coordinate epoch remains until destruction. Notebook and new pending probes
also verify final Prepared-owner release. Allocation snapshots include final
Rust destruction; remaining harness/process allocations are preserved in raw
records and are not misidentified as retained Engine ownership.

## Verification and reproduction

Test-first: before production changes, the new ordinary-body storage test failed
at tick 1 with one descriptor instead of zero. Its complete form also checks two
separate answers, duplicate relation count, fresh body variables, periodic
collection, no implicit history, cancellation and zero final storage.
Additional tests cover first capture through both APIs at 80 suspension points,
GC before promotion, snapshots/inspectors retained through cancellation, and
views opened across cancellation suspensions after discard begins. Existing
tests cover nonbinding identities, cycles, ordered propagation multiplicity,
explicit disjunction, committed scheduling, divergence fairness, stepping,
history, normalization observation, old readers and conditional compaction.

Builds and executions were separate:

```sh
cargo test --offline --release --all-targets --features diagnostics --no-run
cargo test --offline --release --all-targets --no-run --target-dir target/round004-native
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --features diagnostics --quiet
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --target-dir target/round004-native --quiet
CHR_MEASURE_BINARY=target/release/examples/measure timeout --kill-after=5s 60s python3 -m unittest discover -s tests -p measure_observation_test.py
cargo clippy --offline --all-features --lib --bins --examples --test pending_body -- -D warnings
cargo fmt --check
git diff --check
```

Results: **488 diagnostics Rust tests, 462 ordinary Rust tests, three Python
observation checks, all pass**. Full Rust logs are committed here. Every test
invocation had `timeout --kill-after=5s 60s`; none timed out. The sandbox first
denied socket binding (OS error 1); the full suites pass with loopback access.
An all-target Clippy attempt rejects the pre-existing independent truth-table
expression in `tests/condition.rs:33` as overly complex; verified unchanged at
baseline. The production/example/changed-test Clippy scope above passes.

The baseline was built before production edits. For the new pending probes only,
a detached worktree `/tmp/chrimp-round004-baseline-harness` at `4c9ae4e` received
the identical `examples/measure/pending.rs` and lifecycle routing changes.
No baseline production file changed. That common-harness binary was also used
for the later controls. Earlier I/S measurements use the initial baseline
binary and unchanged I/S harness; the new cases do not alter those paths.

```sh
# From the baseline/common-harness worktree:
cargo build --offline --release --features diagnostics --example measure --target-dir /tmp/chrimp-opt-round004-ephemeral-pending/target/round004-baseline
# From the candidate:
cargo build --offline --release --features diagnostics --example measure
```

Common-harness baseline SHA-256:
`33ca813b96242f73b7f5fbc1685a2c42727089c0bb01b11dbaf53834ec38e0ca`.
Candidate measured binary SHA-256:
`6eaf4f2533db4f2f462c2649fcd7f0e9ac59186ae67e4757de9b23fa6ae62a36`.
Only an explanatory source comment changed after that binary's final build.

Each measured point used the maintained runner:

```sh
python3 examples/perf.py --binary BINARY --out OUT --repeat 5 --warmup 0 --seconds 10 --total-seconds 60 -- CASE SIZE 50000000 3 OPTIONS
```

Cases/options: rewrite 32/128, fresh-contract 4, partial-join-hit 32 and
duplicate-heads 8 use `--rows 4 --detail`; answers 256 uses `--rows 0 --detail`;
behavior-I/S use size 1 and `--detail`; history-choice 64 and alias 128 use
`--detail` without `--rows`; pending-cancel/snapshot use sizes 16/64/256 and
`--detail`; conditional fixed-archive/inspectors/held-output use size 4 with
`--rows 16` or `64`, `--work 64 --cadence 4`. Exact commands, process limits,
source configurations and outcomes are included with every raw campaign.

One initial history-control invocation incorrectly supplied `--rows`; the runner
rejected it before execution. Its failed local cohort is retained, excluded
from the 220 samples, and replaced by the correctly configured `-checked`
cohort. Preliminary `candidate/` and `rewrite128/` cohorts are also excluded.
The final analysis uses all 22 pairs under `baseline/` and `final/` with matching
names. Some final notebook clocks overlapped compilation and are visibly noisy;
they are not runtime-improvement evidence or an acceptance/ranking input.

`measurements.json` packages selected existing medians, varying ranges, missing
counts and first-sample endpoints. `raw-samples.json.gz` preserves **every** raw
typed sample, process outcome and full campaign statistics/provenance. Its
uncompressed SHA-256 is
`b27df262894f475621cce7926bd231bdadb07f5586ad4239f47a0f1bc9d11cea`.
Regenerate the package with `python3 docs/optimization-evidence/rehearse/round004-ephemeral-pending/collect.py`.
The collector packages existing observations; it is not a new benchmark system.

The eight core comparisons use `examples/perf_compare.py BEFORE AFTER` with
metrics `allocations.total_allocated_bytes`,
`allocations.process_peak_requested_bytes`, and
`workload.memory_counts.sampled_peak.obligation_descriptors`.
No rwLog benchmarks were rerun. No runtime regression certification is claimed.

KEEP is warranted by the actual removed representation and measured storage/
allocation/collection work, with preserved semantic and lifecycle obligations.
The deferred first-view/cancellation costs and small transient I/diagnostic
increases are explicitly included above; none is a significant offsetting cost
in this measured scope. The unimplemented full registry is a limit of the claim,
not a reason to discard this demonstrated improvement.
