# Rehearse Round 020 candidate 1: remove the duplicate waiter index

Recommendation: **KEEP**. The mutation-lane FIFO and existing parked-task map
already uniquely represent task waiters. Removing `Engine.requested` eliminates
tree lookup/update/allocation work and retained tree nodes. Two booleans suffice
for the independently serviced completion and collection owners. No replacement
index, queue scan, root, allocation, or source dispatch is introduced.

Implementation: `6e63b75`, based on common diagnostics `acbf110`, whose parent is
landing `5cd67c5` (`f70fb01` plus the two current instruction commits). The final
evidence commit also adds a cancellation test with both flags pending.
`01b43ff` is a separate test-only Clippy annotation retaining an existing expanded
Boolean truth-table oracle. No landing files or branch state were changed.

Experiment: `/tmp/chrimp-opt-round020-waiter-index`, branch
`codex/opt/round020-waiter-index`. Baseline control checkout:
`/tmp/chrimp-round020-baseline-harness` at `acbf110`.
All implementation and evaluation were performed by the sole Astra implementer.

## Mechanism and ownership argument

A runnable task calls `acquire` at most once in one dispatch. A failed acquire
appends its owner and returns unfinished; the scheduler parks that same task
before another owner can be appended. It cannot run again until `release_lane`
pops its FIFO request, reserves its lane, removes it from `parked`, and wakes it.
An already owning task takes the existing immediate-success branch. Task IDs
are unique; the pending-obligation index, scopes, and reader epochs are unchanged.

Completion and collection can retry while queued, so `completion_waiting` and
`collection_waiting` suppress duplicate requests. The flags are cleared both on
immediate acquisition and FIFO handoff, never on a failed retry. Collection's
status/admission predicates use the collection flag in place of set membership.
Its explicit request flag remains distinct. Physical collection can still run
while its semantic writer request waits in the FIFO.

Cancellation retains its original source-discard, syntax-promotion, inspection,
root-release and physical-collection order. After discarding the owners it now
releases the scalar FIFO, flags and lane in one continuation instead of also
draining a duplicate tree entry per continuation. Inspection admission during
discard and retained-snapshot ownership are unchanged. Scalar FIFO destruction
already used `VecDeque::new()`; no previously bounded payload destruction was
moved into that step. Ordinary execution creates no new history.

Debug checks assert runnable/parked separation, tail uniqueness, no completed
task still waiting, and no parked replacement. Tests independently scan the
complete FIFO/map correspondence; those scans do not enter production execution.
The common diagnostics add only fixed counters, compiled out without the feature.

The original proposal's allocation wording was too strong: a Rust B-tree packs
several owners per node, rather than allocating one node per task. The measured
savings below are actual allocator calls/bytes, including node splits, merges,
reused empty roots and final frees. Ordinary Engine size is **5,240 → 5,216 bytes**
on this toolchain: the two flags fit existing padding and remove the 24-byte set
header. This stack/object reduction is additional to the measured heap savings.

## Equal progress and measured costs

Primary evidence comprises **144 completed diagnostic runs**: 24 matched
workload points × three repeats × two implementations. Every run uses the
maintained `measure` oracles and `perf.py` supervision. `audit.py` checks the
complete source work vectors, every diagnostic checkpoint, waiter balance and
post-release memory, while preserving allocation samples for every phase.
The compressed `audit.json.gz` contains all three runs' result/phase records and
allocation checkpoints, baseline counter vectors, and all numeric diagnostic
differences. It is generated data, not a replacement benchmark or semantic oracle.

Every source work vector is exactly equal across all six runs of each point:
per-rule matching, visits, commits and applications; posts/merges/failure;
completion scans and Boolean work; collection/compaction/coordinate work;
dispatches, task creation/completion/parks/wakes/requeues; FIFO requests, enqueues,
grants and peak occupancy; and output events/complete answers. There is no
displaced source work. Only cancellation dispatches, total entered iterations,
and corresponding coordinate-cleanup probes decrease after source cancellation,
by exactly the number of still-queued owners. The only other varying non-time
diagnostic is the condition HashMap's effective capacity in `fair-grow 4`;
the allocator still charges its actual backing. No changed queue traversal exists.

The following are medians of requested bytes; these are neither RSS nor allocator
metadata. Execution bytes exclude setup, validation, reporting and cleanup, all
of which are preserved separately in the audit. Process peaks include those
phases. Applications and source answer counts are exact in every repeat.

| Workload | Applications / answers | Peak FIFO | Execution allocated bytes, before → after | Process peak bytes, before → after | Cancellation dispatches |
|---|---:|---:|---:|---:|---:|
| rewrite 64 (parking tail) | 128 / 1 | 63 | 2,035,384 → 2,026,456 | 319,267 → 317,251 | 10 → 10 |
| rewrite 256 | 512 / 1 | 255 | 9,015,704 → 8,973,640 | 1,239,208 → 1,230,568 | 10 → 10 |
| rewrite 1024 | 2,048 / 1 | 1,024 | 39,729,024 → 39,553,568 | 4,881,921 → 4,847,553 | 10 → 10 |
| fair-grow 1, rows 8 | 264 / 1 | 172 | 6,805,932 → 6,783,180 | 1,584,502 → 1,578,742 | 531 → 359 |
| fair-grow 4, rows 8 | 1,029 / 1 | 915 | 57,992,892 → 57,844,780 | 10,744,394 → 10,714,826 | 2,761 → 1,846 |
| fair-grow 8, rows 8 | 3,841 / 1 | 2,830 | 195,005,452 → 194,546,460 | 31,482,542 → 31,386,158 | 8,509 → 5,679 |

At rewrite 1024, all 5,119 parks and wakes remain, while execution allocation
calls decrease **196,487 → 195,635** (852 calls). The source finishes with an
empty FIFO; its final live heap still saves 192 bytes from the old empty tree
root. At fair-grow 8, parks/wakes remain **13,597 / 10,768**, with 2,830 tasks
pending, the same 2,831 explicit choice births, and the same finite answer
delivered beside divergence. Source-checkpoint live requested bytes in sample 0
decrease **27,775,904 → 27,679,040** (96,864 bytes). At least 13,606 successful
set insertion operations, 10,776 grant removals, and the remaining 2,830
cancellation removals disappear; unsuccessful non-task retry insertions and
collection membership checks also disappear. These operations are replaced by
constant enum/boolean work. No source-work savings are inferred from fewer ticks.

The median execution allocation reduction at fair-grow 8 is 458,992 bytes and
2,236 calls. Observed allocated-byte ranges are 195,003,052–195,006,188 before
and 194,546,284–194,547,292 after; variation is far smaller than the reduction.
At fair-grow 4 the corresponding ranges are 57,992,524–57,992,988 and
57,843,212–57,845,244. This uncertainty is retained in the raw samples.

Controls and other affected paths:

- **Pending cancel/snapshot, sizes 16/64/128:** stop at precisely SIZE unposted
  tasks, SIZE² pending ports, zero posts/applications and zero acquisitions.
  All engine/setup/inspection/cleanup allocations and work are unchanged.
  Snapshot mode validates both views after cancellation and then releases them.
  These establish lifecycle preservation; they do not establish a waiter gain.
- **Inspections, 1/4/8 readers, rows 4, work 64:** the committed-view oracles
  validate exactly 1/4/8 views. Source applications are 88/238/612 on both sides.
  Each case saves one 192-byte allocation and three cancellation dispatches.
- **Conditional inspections with the same sizes:** source applications are
  68/83/119, including the same choice/failure work and pins. All view answers
  match; projection-only draining adds zero applications. Each saves one
  192-byte allocation and one cancellation dispatch. Inspection work and its
  allocated bytes do not increase in either inspection family.
- **Tiny/no-choice controls:** rewrite 1 and 16 save 192/672 execution bytes and
  1/3 allocations with identical work. `raw-probes 1 --rows 16/128` validates
  16/128 applications and one answer, saving 4,032/9,552 execution bytes and
  17/52 allocations; process peaks save 672/4,320 bytes. Eight empty prepared
  engine uses save eight allocations/1,536 bytes; process peak is unchanged.
- **Multiplicity:** `answers 64 --rows 0` delivers all 64 distinct alternative
  answers, with the same 61 parks and wakes, saving seven allocations/1,440
  execution bytes. It does not collapse equal answers.

Across the primary matrix, setup, validator, delivery and inspection allocation
traffic are unchanged. Cleanup allocated bytes/calls are unchanged. Frees occur
in execution, cleanup or the already-unscoped Engine destructor according to
ownership; all removed allocations have corresponding removed frees. Final
process live requested bytes are unchanged, and released engine memory gauges
are zero except its one owned coordinate epoch; final Engine/Prepared destruction
is also covered by the maintained probes and focused tests. The 1-byte decrease
in each process's `other` allocated/freed bytes is the shorter `measure-after`
argument path, **not an engine benefit**. This is excluded from every claimed
execution-byte saving.

Runtime was investigated only as a diagnostic. A three-sample instrumented
raw-probes control moved in the adverse direction. Two subsequent ordinary-build
12-sample comparisons had source/delivery medians 4.683 → 5.190 ms, then
5.168 → 4.912 ms, with overlapping 3.789–6.882 ms overall ranges. There is no
stable direction in those timing observations. Architecture acceptance instead
rests on removal of the redundant tree operations/storage, exact equal-work
vectors, and allocation/cleanup evidence at both tiny and growing component
scales. No statistical speedup or timing-equivalence claim is made.

## Validation and reproduction

Toolchain: `rustc 1.95.0-nightly (18d13b533 2026-02-09)`, Cargo
`1.95.0-nightly (fe2f314ae 2026-01-30)`, Linux 6.8.0-138-generic x86_64.
Baseline library: 118 tests passed after the common instrumentation build.
Candidate: 121 library tests, 50 maintained measurement tests, and 364 tests
across all 41 integration targets passed (**535 total**). Debug builds additionally
pass the three parking-tail tests, both cancellation unit tests, all 29 condition
tests and three compile-fail doctests. No test timed out.

Build separately before each test configuration:

```sh
cargo build --offline --release --features diagnostics --all-targets
cargo build --offline --all-targets
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --lib
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --example measure
for file in tests/*.rs; do
  name=${file##*/}; name=${name%.rs}
  timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --test "$name"
done
timeout --kill-after=5s 60s cargo test --offline --lib parking_tail_tests
timeout --kill-after=5s 60s cargo test --offline --lib engine::cancel::tests
timeout --kill-after=5s 60s cargo test --offline --test condition
timeout --kill-after=5s 60s cargo test --offline --doc
cargo clippy --offline --all-targets --features diagnostics -- -D warnings
cargo fmt --check
git diff --check
```

All commands passed. The CLI notebook-origin test initially could not start its
server under the filesystem/network sandbox; all four CLI tests passed when
rerun with loopback access, as did all 21 notebook tests. The first development
build caught a private test-helper call and an unused import; both were repaired.
Clippy initially rejected an intentionally expanded pre-existing truth-table
oracle; the local test-only annotation explains and preserves that oracle.

The baseline executable was built at `acbf110` using the release diagnostics
all-targets command above, then saved before implementation. The candidate
executable contains `6e63b75` production code. Subsequent changes add tests,
documentation and evidence only. Saved binary SHA-256:

```text
986227e8a77a0fe0f348ed5f959334a869e9cfda42b3366cda33c3c04d6cedf0  measure-before
dc10846209a7e76531ae6e9a540e6349cd8fdb931e45a8bcb9a1f234d696139c  measure-after
```

Primary measurement commands are enumerated in `run.sh` and each audit entry.
The actual artifact root is `/tmp/chrimp-round020-evaluation`:

```sh
bash docs/optimization-evidence/rehearse/round020-waiter-index/run.sh \
  /tmp/chrimp-round020-evaluation/matrix \
  /tmp/chrimp-round020-evaluation/measure-before \
  /tmp/chrimp-round020-evaluation/measure-after
python3 docs/optimization-evidence/rehearse/round020-waiter-index/audit.py \
  /tmp/chrimp-round020-evaluation > /tmp/chrimp-round020-evaluation/audit.json
```

Each point calls `timeout --kill-after=5s 60s python3 examples/perf.py --binary
BINARY --out OUT --repeat 3 --warmup 0 --seconds 10 --total-seconds 50 -- CASE
SIZE [OPTIONS] 50000000 3 --detail`. All 144 primary samples completed and the
audit exited zero. A preliminary rewrite-128 pair (six samples) and an initial
raw-probes SIZE-only sweep (12 samples, no workload growth) also completed but
are excluded from the primary matrix. That control was corrected to vary ROWS;
its replacement samples live under `control/`, which the audit selects. The
checked-in run script directly uses the corrected dimensions.

Ordinary timing control builds used `cargo build --offline --release --example
measure` in each checkout. For each of two rounds and each binary, the command
was `timeout --kill-after=5s 60s python3 examples/perf.py --binary BINARY --out
OUT --repeat 12 --warmup 1 --seconds 10 --total-seconds 50 -- raw-probes 1
50000000 3 --rows 128`. All 48 measured runs and four warmups completed; raw
campaigns remain under `timing/`. These do not gate KEEP.

The resulting change removes a growing redundant representation with constant
replacement state and preserves all measured work and ownership obligations.
Its modest whole-process percentages do not negate elimination of this entire
index and its scaling cost. There is no measured significant replacement cost
or affected-component regression. Integration is left to the landing owner.
