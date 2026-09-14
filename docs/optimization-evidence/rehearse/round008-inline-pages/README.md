# Rehearse 008: boxed fixed-slot page payloads and page-run cursors

**Recommendation: KEEP the evaluated bounded slice.** Baseline: `19427c2`.
Implementation: `9dae543` on `codex/opt/round008-inline-pages`, worktree
`/tmp/chrimp-opt-round008-inline-pages`. The landing branch and campaign history
are unchanged until this result is independently integrated.

## Actual scope

The accepted Round 007 Store pages used `Vec<(u64, V)>` payloads. This slice
replaces that payload with a fixed eight-slot `PageEntries` structure held in a
boxed page payload, tracks occupied length, and performs bounded insertion and
removal shifts in place. It also transports a page range or filter continuation
through a compact `PageRun` frame instead of a temporary traversal vector. Sparse
crit-bit boundaries, immutable roots, no-op identity, stale/foreign checks,
duplicate keys, snapshots, filters, and deferred release remain unchanged.

This is not an unboxed change to the `Node` enum, a larger page, a new tree, a
general bulk cursor, or a reduction in logical joins/rule work. Page entries are
still copied when a persistent page root must be copied; only the bounded payload
allocation and cursor transport are specialized. An earlier direct-inline
variant was not part of this evaluated commit because its peak-memory behavior
was materially worse.

## Measurement

The maintained `run.py`/`perf.py` path compared the accepted Round 007 candidate
against this candidate at 25 matching points, five samples per side, for 250
completed samples. Baseline samples are the already archived Round 007 candidate
samples; candidate samples were freshly executed. Every point completed with the
same validated workload contract. No rwLog benchmark was run.

The principal demonstrated change is engine allocation-call count. Medians:

| Workload | Engine allocation calls, baseline → candidate | Engine allocated bytes | Whole-process requested peak |
|---|---:|---:|---:|
| graph-bits 7 | 7,784 → 6,094 (-21.71%) | 1,994,064 → 1,951,224 (-2.15%) | 520,826 → 528,306 (+1.44%) |
| lambda 2 first answer | 14,197,320 → 12,578,283 (-11.40%) | 4,910,781,192 → 4,863,170,928 (-0.97%) | 162,937,653 → 164,372,669 (+0.88%) |
| behavior I | 45,356 → 41,678 (-8.11%) | 14,005,532 → 14,046,588 (+0.29%) | 1,962,858 → 1,971,530 (+0.44%) |
| behavior S | 44,203 → 40,033 (-9.43%) | 13,468,308 → 13,444,660 (-0.18%) | 1,718,623 → 1,723,871 (+0.31%) |
| archive fixed | 29,795 → 25,329 (-14.99%) | 9,826,312 → 9,798,296 (-0.29%) | 759,306 → 760,234 (+0.12%) |
| archive rotate | 67,211 → 62,625 (-6.82%) | 22,134,620 → 22,124,004 (-0.05%) | 1,242,363 → 1,243,323 (+0.08%) |
| wide rewrite 16 | 73,746 → 70,666 (-4.17%) | 10,091,152 → 10,541,024 (+4.45%) | 1,369,955 → 1,379,331 (+0.68%) |

Across the controls, allocation-call reductions are generally 4–22% and are
largest where page cursors or many persistent page updates recur. Allocation
bytes are mixed: they fall on graph-bits, lambda, fresh-contract, and retained
reader cases, but rise on wide rewrite and some small controls. Whole-process
requested peaks rise about 0.1–1.6%; this is a real displaced cost and is not
treated as zero. Graph-node peaks and cleanup ticks are unchanged on the page
focused controls because the underlying Round 007 representation is retained.
The key wide16 allocation-call comparison reports a decrease, while allocated
bytes and requested peak report increases; the lambda comparison reports
decreases in both call count and bytes, with a requested-peak increase. The
comparison JSON files preserve these classifications and their raw samples.

The change removes concrete page-payload and traversal-buffer allocation events;
it does not claim that total mutation visits, semantic work, graph-node storage,
or cleanup work fell. The page-run cursor returns each scalar entry separately,
so entry work remains charged. Preparation, source execution, delivery and
validation, retained readers, archive rotation, cancellation, runtime replay,
and cleanup are present in the maintained records. No meaningful change in
applications, answer multiplicity, source progress, or residual identities was
observed in the controls.

## Correctness and lifetime evidence

The implementation adds five Store regression tests covering allocation-free
page updates, exact duplicate-valued keys, frozen roots and foreign/stale cursor
rejection, page-run range boundaries, filter continuation, interrupted cursors,
and final owner release. The independent timed all-target suites passed: all 44
native and diagnostics test binaries, including the five added Store tests; all
three maintained observation-equivalence checks; and production Clippy with
`-D warnings` plus `cargo fmt --check`.

The diagnostics and native test invocations used
`timeout --kill-after=5s 60s`; localhost-only CLI checks were rerun with
loopback access after the sandbox denied them. Final graph, condition, history,
pending, inspection, and release-owner checks passed. The existing all-target
Clippy truth-table warning in `tests/condition.rs:33` remains the known baseline
exception and is not introduced by this slice.

## Acceptance assessment and limits

Under the project’s independent-dimensions rule, the allocation-call reduction
is a material work/resource improvement at equivalent semantic progress. It is
not vetoed by the unchanged graph/cleanup dimensions, and total allocation is
not used as a summary gate. The measured peak increases and the wide-rewrite
allocated-byte increase are documented and bounded; they do not constitute a
significant semantic-work, progress, or scaling regression in the tested suite.
The claim is limited to reducing boxed page-payload and page-cursor allocation
churn in workloads that exercise the existing dense-page representation.

Artifacts: `measurements.json` contains all paired medians and first-sample
endpoints; `raw-samples.json.gz` retains all 250 samples; the four default and
two key comparison JSON files preserve comparator output; `tests-native.log`
records the agent’s native run. The candidate measurement root was
`/tmp/chrimp-round008-boxed` and is intentionally not required for repository
reproduction. `run.py` documents its reuse of accepted baseline samples and
`collect.py` performs the deterministic packaging.
