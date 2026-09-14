# Rehearse Round 021 candidate 4: mutation-lane payload FIFO

**KEEP.** The evaluated implementation is
`03f721178213efc712a935dc2670f5a778ed82eb`, based on landing
`cb155d95feba1581a535932065d6823d8f03a4de`. It removes
`parked: BTreeMap<u64, Scheduled>` and stores the suspended payload directly
in the mutation-lane FIFO. It preserves Round 020's removal of the duplicate
requested-owner index. Integration is left to the landing owner.

Experiment: `/tmp/chrimp-opt-round021-lane-payload`, branch
`codex/opt/round021-lane-payload`. Separate baseline instrumentation checkout:
`/tmp/chrimp-round021-baseline-harness`, detached at `cb155d9`. The sole Astra
implementer made no edits to the landing checkout. The supplied context was
read from `.rehearse/context-round-021.md`; the requested nested
`.rehearse/round-021/context-round-021.md` did not exist. Candidate index 4 and
the round's acceptance snapshot were read completely.

## Implemented mechanism and ownership

`Waiting` is Task(Scheduled), Completion, or Collection. A failed task acquire
sets a dispatch-local parking flag; the scheduler immediately moves that same
payload into the FIFO before another dispatch or owner acquisition. A runnable
task cannot already be waiting. On release, the front entry reserves the lane
and its task payload moves directly into the runnable queue. The task ID is
stored only in Scheduled, not repeated in a tree key and scalar waiter. Immediate
acquisition performs no FIFO allocation. Completion/collection still use their
separate flags to suppress retries, with clearing on dequeue/acquisition.

The FIFO uses boxed arrays of four entries and a VecDeque of block pointers.
This is storage segmentation, not a second task index or a box per task. Each
payload has exactly one FIFO owner. At most two empty blocks are reused; excess
retired blocks are freed as their last entry leaves. The small pointer directory
contracts below half occupancy toward 1.5 times the live block count, with a
four-pointer minimum. No live payload moves during directory resizing.

Queued lookup is constant-time from head offset and logical index. At most six
payload slots are unused in the live head/tail blocks, plus eight in the two
spares. Pointer capacity is at most twice its live block count (minimum four).
An empty previously used FIFO retains at most **992 bytes**: 960 payload-slot
bytes plus 32 directory bytes. Cancellation releases even that backing.
Both Scheduled and Waiting are **120 bytes** on this toolchain. Ordinary Engine
size is **5,216 -> 5,224 bytes**, an eight-byte inline cost, not hidden heap use.

Collection, compaction, pending-syntax promotion and normalization inspection
now traverse FIFO payloads. Source execution is frozen while resumable scanning
uses array positions. Skipping scalar owners inspects at most two entries.
The existing task trace cursor still yields individual roots and retains all
graph/history/condition/epoch owners. No pending certificate is removed merely
because its task parks. Captures promote the same syntax and retain persistent
snapshot roots; ordinary execution acquires no history.

Cancellation promotes syntax before discarding anything, then incrementally
discards runnable and FIFO payloads. It keeps each payload owned until its
discard continuation finishes, processes at most one FIFO entry per turn, and
clears scalar flags as those entries leave. Frozen inspections remain valid.
Root and directory release occurs after payload discard and inspection handling,
followed by collection. FIFO order replaces task-ID order for canceled-payload
destruction; no source application runs during that phase. A still-queued
scalar owner adds one cancellation turn (one in fair-growth 1 and 8).

## Equal work and final measurements

The primary matrix is **34 points x 3 repeats x 2 sides = 204 completed runs**.
Every run uses maintained `perf.py`/`measure` semantic oracles. Another
**72 runs** repeat conditional archives (12 per side at 1/4/8 owners), and
**24 runs** use maintained `perf_suite.py` archive/fairness selection.
All 300 final implementation/control observations completed without censoring.

`audit.json.gz` and `archive-audit.json.gz` retain every typed diagnostic,
result, lifecycle phase and allocation checkpoint, with exact commands and
all numeric diagnostic differences. `runs.tar.gz` includes the underlying raw
campaigns, per-sample JSON, stdout/stderr and supervisor/build-context records
for these 300 observations. These are measurement evidence, not alternate
workloads or semantic oracles. `integration-tests.tar.gz` contains the final
logs for all 41 integration targets.

Across every diagnostic checkpoint, the audit requires identical per-rule work,
task creation/completion/cancellation, parks/wakes/requeues, posts/merges/failures,
choice births, syntax promotion, output multiplicity and complete answers.
Requests, enqueues, grants, peak FIFO occupancy, explicit payload transfers,
completed parked traces and discard work also match exactly. The final candidate
counts each wake as one direct handoff; the baseline counts zero direct handoffs.

Except for conditional archives, every non-waiter diagnostic is equal apart
from condition HashMap effective capacity and the explained extra cancellation
turns. Conditional archives also vary in physical arena collection/chronology
work, including unchanged baseline repeats. The audit requires every iteration
difference to equal arena-collection plus cancellation differences, and the
coordinate-cleanup probe difference to equal the same amount. It permits no
changed matcher, compactor, publication, trace-task or source-dispatch work.
These checks establish equal source scheduling/work in addition to equal answers;
no architectural benefit is inferred from tick reductions or runtime.

Requested-byte medians below charge actual allocations/reallocations. They are
not RSS or allocator metadata. All sizes use `--detail`, rows 8 for fair-growth,
and the exact commands in `run.sh`.

| Workload | Applications / source answers | Execution bytes before -> after | Process peak before -> after | Source live bytes, sample 0, before -> after |
|---|---:|---:|---:|---:|
| rewrite 64 | 128 / 1 | 2,026,456 -> 1,968,104 | 317,251 -> 310,787 | 110,401 -> 108,945 |
| rewrite 256 | 512 / 1 | 8,973,640 -> 8,703,296 | 1,230,568 -> 1,198,232 | 416,738 -> 412,210 |
| rewrite 1024 | 2,048 / 1 | 39,553,568 -> 38,444,424 | 4,847,553 -> 4,715,569 | 1,640,819 -> 1,624,003 |
| fair-growth 1 | 264 / 1 | 6,783,180 -> 6,633,916 | 1,581,808 -> 1,559,080 | 1,483,227 -> 1,460,499 |
| fair-growth 4 | 1,029 / 1 | 57,843,676 -> 56,877,004 | 10,714,826 -> 10,598,634 | 8,583,900 -> 8,461,980 |
| fair-growth 8 | 3,841 / 1 | 194,550,620 -> 191,594,820 | 31,386,158 -> 30,991,478 | 27,679,040 -> 27,281,896 |

Rewrite 1024 retains all **5,119 parks and wakes**, removes the same number of
parked-tree insertions and handoff removals, and completes **2,668 parked-task
traces in 38,281 service calls** on both sides. Execution allocation calls fall
**195,635 -> 195,061** (medians). New FIFO peak backing is 127,456 bytes, including
payloads and directory; comparing it with the old 16,384-byte scalar FIFO alone
would omit the old parked tree. All 10,238 explicit payload transfers remain;
the candidate eliminates B-tree payload relocation and tree navigation. Directory
growth/contraction has a conservative **24,304-byte move bound**, versus the old
scalar FIFO's 32,640-byte growth bound, before counting old B-tree movement.

Fair-growth 8 preserves **13,597 parks / 10,768 wakes**, 2,831 explicit births,
and the same finite answer while growing alternatives continue. The source has
2,830 pending tasks; 2,829 are parked, plus one scalar FIFO owner. Peak/current
candidate FIFO backing is 345,528 bytes, replacing both the scalar FIFO and
parked tree. All 5,208 task traces and 140,313 trace service calls match. The
final growth/contraction pointer-move bound is **68,032 bytes**, versus 130,944
scalar FIFO growth bytes plus uncounted B-tree movement before.

### Replacement costs and other phases

Preparation, validator and delivery allocations are exactly unchanged throughout
the primary matrix. Inspection allocation traffic is also unchanged except one
conditional-archive-4 sample discussed below. Final process live requested bytes
match exactly at every point. Released engine gauges are zero except its one
still-owned coordinate epoch; final Engine/Prepared drops are also covered.

Directory contraction shifts a small amount of allocation into cancellation:

| Workload | Cleanup allocated bytes before -> after | Cleanup allocation calls before -> after |
|---|---:|---:|
| fair-growth 1 | 66,808 -> 68,048 | 206 -> 214 |
| fair-growth 4 | 59,992 -> 67,280 | 179 -> 193 |
| fair-growth 8 | 949,064 -> 965,624 | 3,008 -> 3,024 |

This is charged, not treated as a free or disappearing cost. At fair-growth 8,
16 additional small pointer-directory allocations and their 16,560 bytes replace
thousands of parked-tree removal/traversal operations during discard. Combined
execution and cleanup still save **2,939,240 allocated bytes** by medians, and
about 1,491 allocation calls. Payload destruction remains one original task
discard step at a time. Contraction moves only pointers; the 68,032-byte bound
above already includes this cleanup. There is no new traversal of retained task
payloads and no increased semantic or root-tracing work. This is a favorable
component-level exchange of logarithmic tree maintenance for amortized constant
FIFO operations with much less storage, not a claim that every individual
allocation category decreases. All other primary cleanup allocations are equal.

`answers 64 --rows 0` delivers all 64 equal empty alternatives. Execution bytes
fall **2,484,516 -> 2,478,204**, peak **417,794 -> 416,850**, while allocation
calls rise **5,096 -> 5,101**. The five extra smaller allocations are a real
cost. They accompany removal of 61 tree insertions and 61 tree removals and
reduce requested traffic and retained bytes; directory movement is bounded by
496 bytes versus 896 scalar FIFO bytes plus old tree moves. This small call
increase does not outweigh the demonstrated storage and navigation reduction.
It is not dismissed using a whole-process percentage.

Ordinary/conditional inspections (1/4/8 readers), fixed/rotating archives
(1/4/8 retained snapshots), and held output preserve their exact retained-graph
oracles and source application counts. Deterministic cases save 624 execution
bytes, 560 source-live/peak bytes, with identical allocation-call counts. The
two reusable blocks prevent small-frontier churn. Pending cancel/snapshot
16/64/128 perform zero acquisitions and have identical phase allocation, work,
snapshot syntax and final release. Tiny rewrite 1 saves one 64-byte allocation;
eight empty prepared uses save eight allocations/512 bytes. Raw-probes rows
16/128 save 22,848/65,808 execution bytes and preserve all identities/answers.

Conditional rotating archives have existing allocator/collector variation. The
12-repeat extension gives execution-byte ranges:

| Owners | Baseline range | Candidate range | Peak range before -> after |
|---|---:|---:|---:|
| 1 | 10,165,452–10,385,416 | 10,198,164–10,372,200 | 472,218–474,394 -> 471,658–473,834 |
| 4 | 11,815,680–12,255,324 | 11,989,032–12,261,260 | 519,842–682,570 -> 519,282–681,466 |
| 8 | 14,629,064–15,487,580 | 14,942,636–15,409,328 | 735,138–741,402 -> 734,562–740,282 |

The single primary candidate archive-4 inspection allocation of 200,720 extra
bytes also occurs on the unchanged baseline. Both sides' extended inspection
range is exactly 3,609,168–3,809,888 bytes and 5,708–5,709 calls. All non-arena
work and source obligations remain identical. These broader costs overlap; no
speedup, total-byte benefit or regression is asserted for this noisy family.
The deterministic FIFO replacement and bounded storage argument still apply.
Unscoped `other` includes report/harness traffic; the one-byte shorter candidate
binary pathname is excluded from engine benefit claims.

### Development corrections

The first contiguous FIFO passed semantic checks but retained 122,880 bytes
after rewrite 1024 finished. Contracting that payload buffer fixed retention
but introduced excessive allocation, especially during growing-frontier cleanup.
The final four-entry blocks avoid both defects. A one-spare version then exposed
30,576–55,536 extra execution bytes on repeated conditional-inspection frontiers;
the final two-spare version removes that churn. These were development iterations
of the same selected FIFO mechanism, not separately accepted implementations.
Their local evidence remains under `pilot-*`, `matrix`, `matrix-blocks` and the
corresponding saved executables in `/tmp/chrimp-round021-evaluation`.

## Verification and reproduction

Toolchain: rustc `1.95.0-nightly (18d13b533 2026-02-09)`, Cargo
`1.95.0-nightly (fe2f314ae 2026-01-30)`, Linux `6.8.0-138-generic`, x86_64.
`baseline-diagnostics.patch` applies to `cb155d9` and only instruments the old
FIFO/map, trace and discard paths. It adds the same diagnostic schema without
changing execution; all 121 baseline library tests pass. No benchmark machinery,
language expectations, or workload definitions were changed.

Builds were separate from validation. Cargo's example-test executable needed a
separate no-run build; all final invocations used the prebuilt artifacts:

```sh
cargo build --offline --release --features diagnostics --all-targets
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --example measure --no-run
cargo build --offline --all-targets
timeout --kill-after=5s 60s cargo test --offline --lib --no-run
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --lib
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --example measure
for file in tests/*.rs; do
  name=${file##*/}; name=${name%.rs}
  timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --test "$name"
done
timeout --kill-after=5s 60s cargo test --offline --lib parking_tail_tests -- --nocapture
timeout --kill-after=5s 60s cargo test --offline --lib engine::cancel::tests
timeout --kill-after=5s 60s cargo test --offline --test condition
timeout --kill-after=5s 60s cargo test --offline --doc
cargo clippy --offline --all-targets --features diagnostics -- -D warnings
cargo fmt --check
git diff --check
```

Results: **123 library + 50 measure + 364 integration = 537 release diagnostic
tests passed**, across all 41 integration targets. Debug additionally passed
four parking-tail tests, two cancellation tests, 29 condition tests and three
compile-fail doctests. No test timed out. Static checks passed. The first CLI
server check lacked sandbox loopback access; its four tests and all 21 notebook
tests passed with loopback access, as did the subsequent complete integration
loop. The new diagnostic trace test covers 64 interruptions inside physical
collection followed by cancellation, frozen inspection readback and complete
release. The growth test checks 485 handoffs, stable queued payload addresses,
nonmonotonic IDs, block retirement/reuse and scalar flag invariants.

Saved binary SHA-256 (production code matches the implementation commit):

```text
edb17b45b041c7456925a457719666bad619c1ab0898c23c71ad3f955c20fed8  measure-before
76f8eaefe7bfde370a9e464209ddf9845ef8c0299117e1db90c9b9a95fe57806  measure-after
```

`run.sh OUT BEFORE AFTER` runs the maintained comparison, extended archives and
suite with those prebuilt binaries. Each perf.py invocation is guarded with
`timeout --kill-after=5s 60s`, and uses three repeats, no warmup, a ten-second
external sample bound and a 50-second invocation bound. Native workloads use
50,000,000 ticks / three seconds with detailed observation. Extended archives
use twelve repeats. Each suite invocation uses `routine --only archive --only
fairness --repeat 3 --seconds 50 --sample-seconds 5`, also guarded at 60 seconds.
These are individual measurement bounds, not an experiment time budget.

The actual final matrix is `/tmp/chrimp-round021-evaluation/final`, extended
archive runs are in `archive-audit`, and final suite sides are `suite-before`
and `suite-final`. To reproduce in a new output location:

```sh
bash docs/optimization-evidence/rehearse/round021-lane-payload/run.sh \
  /tmp/chrimp-round021-reproduce \
  /tmp/chrimp-round021-evaluation/measure-before \
  /tmp/chrimp-round021-evaluation/measure-after
python3 docs/optimization-evidence/rehearse/round021-lane-payload/audit.py \
  /tmp/chrimp-round021-reproduce /tmp/chrimp-round021-reproduce-audit.json.gz
python3 docs/optimization-evidence/rehearse/round021-lane-payload/audit.py \
  /tmp/chrimp-round021-reproduce-archive-audit /tmp/chrimp-round021-archive-audit.json.gz
```

Acceptance rests on removing tree ownership/navigation and payload relocation,
measured lower allocation/live/peak storage, equal semantic work and complete
reclamation, after charging the pointer-directory and allocator-call tradeoffs.
The small fixed Engine-size increase and identified replacement operations do
not outweigh these gains. No rwLog benchmark was rerun, and no landing commit
or campaign-history integration was performed.
