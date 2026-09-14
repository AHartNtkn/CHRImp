# Rehearse Round 018: reverse Store batch coalescing

Recommendation: **KEEP** the evaluated reverse exact-key seen set with eight
inline keys and lazy HashSet promotion. It removes repeated suffix comparisons,
including a substantial scaling cost for large public Store batches. The
measured graph paths retain their allocation behavior. This is implementor
advice for independent review, not a landing decision.

There is a real tradeoff: promoted batches allocate temporary hash storage.
The largest tested table backing is 16,912 bytes, and the largest observed
pre-validation process-peak increase is **19.86%**. This is a work/scaling
improvement, not a storage improvement. I judge the bounded temporary cost and
small implementation complexity worth the demonstrated reduction in repeated
key work, with no measured significant regression in the maintained execution
paths. A workload dominated by nine-distinct-key batches is an adverse case.

| Role | Revision |
|---|---|
| Accepted baseline | `9f5fd4a3d334b62253304256a88776861cd08878` |
| Common diagnostics and ordered/lifecycle oracle | `8f83bc5` |
| Preserved initial unconditional-set implementation | `b3c668c` |
| Final implementation, tests, maintained docs and audit tools | `3dbe870` |
| Raw evidence, acceptance and execution notes | `e592081` |
| Evidence-only matched baseline probe harness | `fa22ee1` |

This report is the following commit. The KEEP sequence is `8f83bc5`,
`b3c668c`, `3dbe870`, `e592081`, then this report commit. Do not cherry-pick
`fa22ee1`; its equivalent test is already in the final implementation.
Worktree: `/tmp/chrimp-opt-round018-batch-seen`.
Branch: `codex/opt/round018-batch-seen`.
The landing checkout remains clean at `9f5fd4a`.

## Implementation and semantics

The change is confined to Store batch scratch coalescing. It walks from the
last write to the first, remembers every last-seen key including no-ops, and
compacts changed writes into the already-read suffix. The retained slice moves
to the caller's prefix and enters the existing sort and batch application.
All four key words participate in hashing/equality.

Eight keys stay in a 256-byte stack array. A ninth distinct key creates the
randomized hasher and table, inserts the eight existing keys, and continues
there. There is no table or hasher initialization on the inline path. Table
growth is bounded by distinct input keys and its storage is released before
sorting or applying writes. Compaction adds at most one extra linear transfer
of retained entries; sorting and page work are unchanged.

No-op membership checks still run once per distinct key against the original
root. A final no-op still suppresses earlier changes. Deletions, ownership and
stale/frozen-root checks, page batching, publication and cancellation boundaries
are unchanged. Scratch stores keys, never roots or scalar owners. No persistent
history, cache, alternate executor or new scheduling rule is introduced.

## Equal-result work and storage comparison

The final synthetic evaluation contains **744 paired completed samples**:
248 fixtures, three fresh processes each. It covers 0/1/2/8/9/16/64/256 writes,
unique keys, repeated keys, 7/8/9-key promotion controls, dense and full-width
sparse keys, insertion, mixed deletion/overwrite, final no-ops and delete-all.
Both versions are checked against forward BTreeMap writes, exact ordered
cursor output and compacted write prefixes, immutable snapshots, cursor pins
through collection and complete deferred reclamation.

The table shows medians for dense mixed batches. Hash counts include actual
Hash::hash calls during promotion and growth; they are not merely inserts.
Equality/hash counts describe different operations, not interchangeable CPU
instructions. Hashing consumes all four words; equality may short-circuit.

| Writes / distinct keys | Baseline equalities | Candidate equalities | Candidate hashes | Extra allocation calls / bytes | Peak hash backing |
|---|---:|---:|---:|---:|---:|
| 16 / 4 | 54 | 36 | 0 | 0 / 0 | 0 |
| 64 / 16 | 888 | 84 | 78 | 2 / 1,616 | 1,072 |
| 256 / 64 | 14,304 | 232 | 354 | 4 / 7,984 | 4,240 |
| 16 / 16, unique | 120 | 37 | 30 | 2 / 1,616 | 1,072 |
| 64 / 64, unique | 2,016 | 39 | 162 | 4 / 7,984 | 4,240 |
| 256 / 256, unique | 32,640 | 62 | 690 | 6 / 33,360 | 16,912 |

Every extra scratch byte in these intervals is freed within the batch call.
For example, 256 unique mixed writes allocate 95→101 times and
12,416→45,776 bytes during batch preparation/application; the extra 33,360
bytes are freed before return. Page allocations, copies, shifts, inline slots,
mutation counts, retained writes, ordered roots and cleanup ticks agree in
every pair. These gains remove equality scans, not source rule applications.

The inline controls matter: 256 writes over one key retain 255 comparisons,
zero hashes and zero scratch allocation. Over eight keys they reduce
2,012→1,144 comparisons with no hash/heap cost. Conversely, nine unique writes
retain 36 comparisons and add nine hashes plus one 544-byte allocation. Final
no-op and delete-all batches pay promotion costs too when their key set is
large; exact root reuse does not make their scratch work free.

Peak requested storage is reported both before and after validation. For 256
unique mixed writes, the process peak through update rises
85,571→98,052 bytes (+14.59%); for the corresponding insertion-only case it is
62,836→75,317 (+19.86%). For 256 writes over 64 keys it rises
34,850→37,859 (+8.63%). Final process peaks differ only by the one-byte binary
path allocation in the complete probe interval, because independent oracle and
report construction dominate those peaks. That does **not** erase the earlier
scratch cost. Hash backing excludes the fixed stack array and table header;
the allocator includes real growth traffic and overlapping table allocations.

After dropping roots/readers, every Store has zero nodes and bounded release
steps. Scoped fixture allocations balance exactly after excluding its retained
report string. One candidate process records 144 bytes of additional concurrent
test-harness traffic in `other`; it remains visible in the audit and is not
reported as engine retention. See [notes.md](notes.md) for accounting details.

## Maintained execution and accepted mechanisms

There are **69 completed detailed pairs at 23 points**, plus **48 completed
routine-suite pairs at 16 points**. These use the existing measure/perf runners
and their independent semantic validators, including preparation, execution,
answer delivery, retention, cancellation and cleanup.

Wide-rewrite varies arity 0/1/8/16/64/256 and separately varies group count
16/64/256. At those group counts, both sides produce one answer, 32/128/512
residual occurrences, and exact per-rule application vectors
`[32,32]`, `[128,128]`, `[512,512]`. Every ordered port and duplicate
occurrence is validated.

Conditional alias-consume at 16/64/256 returns 17/65/257 answers, with
`[16,16]`, `[64,64]`, `[256,256]` applications and 289/4,225/66,049
residual facts across those answers. Repeated-alias with eight probes returns
one answer, 17 facts and application vector `[8]` at all three sizes.
Notebook I/S return one independently validated answer after 2,496/301
applications; their complete per-rule vectors are retained and paired exactly
in `audit.json`. Fair-grow delivers its finite answer after 1,029 applications
with vector `[1020,1,1,1,1,1,1,1,1,1]`, then completes cancellation/release.
Archive rotation, unread inspectors, pending cancellation, history, partial
joins, fresh contraction, duplicates and runtime-session turnover also complete.

All paired per-rule vectors, graph/page/mutation counters, field updates,
normalization, prefix-root memo, shared-restriction/prefix-witness diagnostics,
and before/after-cancel memory counts agree. Full counters and exact answer/work
records are in the archive. Some Boolean collection, cleanup-probe and sampled
peak counters vary between processes; the audit preserves them. These are
equivalent semantic obligations, not universally identical internal ticks.

The measured engine batches have at most eight writes, so **none promotes**:
there is no demonstrated end-to-end hash-path gain on these maintained cases.
They are controls for preserving accepted execution. Wide and repeated-alias
allocation calls and process peaks are identical; total bytes differ by one
byte from the binary path. Across the detailed matrix, median total allocation
changes range −0.00821% to +0.02773%, and median process-peak changes range
−0.26177% to +0.00360%. All routine-suite median peaks are identical; allocation
traffic changes are below 0.001%. Collector/random-table variation in controls
is not credited to the mechanism. Three repeats are not a calibrated runtime
regression assessment, and runtime did not determine the recommendation.

The initial unconditional set supplied a concrete adverse control: the
64-port, four-group wide case added 1,960 allocations and 628,005 total requested
bytes (+23.48%) despite unchanged semantic work. Its 456 synthetic comparisons
passed. The inline path removes that allocation regression and avoids hashes
for long small-key batches; the unconditional code and observations are retained.

## Validation, artifacts and recommendation boundary

**534 diagnostic tests and 491 ordinary tests pass**, including the final
resource and semantic/lifecycle tests. Builds were separate from test execution;
every test binary ran under `timeout --kill-after=5s 60s`. No test timed out.
Each configuration's two sandbox-blocked loopback HTTP tests passed with local
socket permission. Production Clippy with warnings denied and formatting pass.
The expected baseline scaling failure and all sandbox failures/retries are
preserved. No rwLog benchmarks were run.

Exact implementation paths:

- `/tmp/chrimp-opt-round018-batch-seen/src/store.rs`
- `/tmp/chrimp-opt-round018-batch-seen/src/graph.rs`
- `/tmp/chrimp-opt-round018-batch-seen/src/engine.rs`
- `/tmp/chrimp-opt-round018-batch-seen/examples/measure.rs`
- `/tmp/chrimp-opt-round018-batch-seen/tests/batch_coalescing.rs`
- `/tmp/chrimp-opt-round018-batch-seen/docs/performance.md`
- `/tmp/chrimp-opt-round018-batch-seen/docs/validation.md`

Evidence directory:
`/tmp/chrimp-opt-round018-batch-seen/docs/optimization-evidence/rehearse/round018-batch-seen/`.
[raw.tar.gz](raw.tar.gz) contains raw observations, all phase counters, audits,
test/build logs and binary/source hashes. [notes.md](notes.md) provides exact
reproduction commands and accounting caveats; [acceptance.md](acceptance.md)
preserves the campaign rule verbatim. `reproduce.py`, `verify.py` and
`audit.py` orchestrate the maintained tools and make the paired checks repeatable.

KEEP is limited to the demonstrated elimination of repeated key work and its
expected scaling, with unchanged measured small-batch execution. It does not
claim fewer semantic applications, lower storage, universal speedups, or a
worst-case linear bound for adversarial hash collisions. The temporary storage
increase and the nine-key control remain explicit costs for the landing review.
