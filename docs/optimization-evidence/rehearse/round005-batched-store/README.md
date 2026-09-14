# Rehearse 005: bounded graph-store batches

**Recommendation: KEEP the implemented partial slice.** Accepted baseline:
`0c120a402c3b864544c9dffa9988e3004ce6896e`. Implementation:
`071f22581a457538c62d51632129eef7c830043c`, branch `codex/opt/batched-store`,
worktree `/tmp/chrimp-opt-round005-batched-store`.
This is the implementor's recommendation under AGENTS.md, not the campaign's
landing decision. No campaign instructions, acceptance rules or history changed.

## Actual mechanism

`Store::batch` accepts ordered insertions/removals in caller-owned scratch. It
keeps the last write to each key, removes net no-ops against the base, sorts the
remaining keys, and descends their combined crit-bit paths. Each affected node
is made unique at most once per batch; entirely deleted subtrees are released
without copying their nodes. Unchanged subtrees retain their identity. Values,
cached leaf counts, prefix ranges and the existing weak-root COW semantics stay
on the same representation.

The engine integration is **graph index updates only, in chunks of at most eight
writes**. Update reconstructs a chunk's keys from its existing immutable plan
and argument vector. No overlay buffer, root, condition handle, allocation or
mode flag is added to a suspended Update. The authoritative FACT/RELATION entry
is still staged immediately; each old external root remains coherent. The
other indexes finalize at the eighth existing yield position or the existing
end position, and only Complete publishes the coherent result. Tuple hashing
still processes every ordered port once, including repeated identities.

This is not a general suspended transaction registry and does not batch an
entire arbitrarily wide update into one service turn. Pending/history stores,
condition operations, identity writes and single scalar Store methods are
unchanged. Partial chunks cannot be read externally. Graph retirement accounting
reads earlier writes in the chunk before the base, preserving logical ordering
and duplicate-key behavior. Finalization checks the existing GcLease exclusion;
every Update tick checks owner, freeze and root validity before progress.
Collection traces the existing staged root and cancellation uses existing owned
arguments and deferred child-release queues. No new reclamation protocol exists.

## Completed comparisons and removed work

The maintained `measure`/`perf.py` tools ran five diagnostics samples on each
side at 29 paired points: **290 completed samples**, with all semantic and
lifecycle validators enabled. Requested bytes include the harness and all
measured phases, not allocator metadata or RSS. Object peaks are sampled counts.

The new maintained `wide-rewrite` case independently varies tuple groups (SIZE)
and arity (`--rows`). Each tuple has two distinct identical occurrences, both
rewritten through p and q to done. There are exactly four applications per
group, one complete answer, and two residual tuples per group. The oracle checks
every ordered port, duplicate multiplicity and distinct query identities. The
same harness additions were applied to baseline production code.

Here “path mutations” sums the existing unique/copied counters. It counts actual
ownership checks, mutation bookkeeping and ancestor refreshes, not rule work,
all index reads, all instructions, or CPU time. Values below are medians; these
wide work/allocation values agree across all five samples.

| Groups / arity | Applications / answer | Path mutations, before → after | Graph allocations, before → after | Execution bytes removed |
|---|---:|---:|---:|---:|
| 4 / 0 | 16 / 1 | 168 → 168 | 224 → 224 | 0 |
| 4 / 4 | 16 / 1 | 1,671 → 829 (-50.39%) | 1,080 → 1,066 | 1,792 |
| 4 / 16 | 16 / 1 | 8,187 → 3,061 (-62.61%) | 3,576 → 3,520 | 7,168 |
| 4 / 64 | 16 / 1 | 40,731 → 13,609 (-66.59%) | 13,560 → 13,336 | 28,672 |
| 16 / 4 | 64 / 1 | 8,797 → 4,819 (-45.22%) | 5,576 → 5,538 | 4,864 |
| 16 / 16 | 64 / 1 | 38,899 → 16,101 (-58.61%) | 17,344 → 17,192 | 19,456 |
| 16 / 64 | 64 / 1 | 187,411 → 68,601 (-63.40%) | 64,960 → 64,352 | 77,824 |
| 64 / 16 | 256 / 1 | 183,409 → 80,995 (-55.84%) | 82,124 → 81,588 | 68,608 |

At 16/64, unique mutations fall 147,408 → 29,206; copies fall 40,003 → 39,395.
The 608 eliminated graph records account for exactly 77,824 execution bytes
(128 bytes per allocation). All 20,800 graph writes, 96 source posts, 64
applications, one answer, 32 facts, 2,048 ports and 3,138 scalar output events
remain. Source applications and each rule's matching/commit counters agree.

The total process allocation is 10,423,819 → 10,345,977 bytes. Eighteen bytes
of that difference belong to `other`, also present on the unchanged empty-answer
and unposted-pending controls; they are not an engine optimization. Execution
alone is 9,549,312 → 9,471,488. Setup stays 478,440 bytes, validator 158,968,
cleanup 4,672. Delivery is charged to source/validator in this runner; its
separate zero allocation category does not mean delivery has no cost.

Requested peak stays 1,417,777 bytes. Sampled graph peak **increases**
8,552 → 8,590 (+38, 0.44%); pending peak stays 65 and conditions zero. Deferring
writes changes intermediate root/release residency. Source ticks increase
161,717 → 161,755, including 32 more collection-status ticks and six more
release dispatches; cleanup stays 4,242 ticks. The proposal is accepted for
removed path mutation/allocation, not for fewer ticks or reduced peak storage.
Other displayed wide points have identical graph and requested-byte peaks.

The unchanged maintained comparator qualifies the 16/64 unique/copy and
execution-allocation decreases, **and the graph-peak increase**, while requested
peak is uncertain/identical. See `comparison-wide16-a64.json`. The 38 transient
nodes do not outweigh 118,810 eliminated path mutations and 608 eliminated
allocations at the same completed obligations. No asymptotic improvement is
claimed: a bounded chunk still revisits ancestors across chunks.

### Existing controls

| Workload | Completed applications / answers | Path mutations, before → after | Execution allocation, before → after |
|---|---:|---:|---:|
| rewrite 128 | 256 / 1 | 14,346 → 12,428 | 4,224,472 → 4,224,472 |
| partial-join-hit 32, probes 4 | 4 / 1 | 3,689 → 2,222 | 1,350,124 → 1,350,124 |
| duplicate-heads 8, groups 4 | 224 / 1 | 10,181 → 8,065 | 3,360,016 → 3,360,016 |
| fresh-contract 4, copies/depth 4 | 128 / 1 | 15,163 → 10,637 | 3,689,640 → 3,689,000 |
| empty answers 256 | 0 / 256 | 0 → 0 | 13,652,472 → 13,652,472 |
| continuing alias 128 | 1,024 / finite sibling delivered | 21,857 → 14,689 | 20,718,500 → 20,587,556 |
| history-choice 64 | 512 / retained history checked | 10,241 → 7,168 | 5,637,744 → 5,637,744 |

Unchanged allocation on several controls does not cancel their independent
path-work reduction. Partial-join requested peak falls 309,032 → 307,752;
rewrite, duplicates, fresh, empty answers, alias and history requested peaks
are unchanged. Preparation metrics and setup/validator allocations match.
All compared source per-rule counter medians and completed post/merge counts
match across the 29 points. Raw records preserve varying collector work.

Behavior I and S reach the same independently checked first answers with
126/134 applications and 244/217 posts. Their path counts are
20,808 → 13,253 and 19,956 → 12,852. These are **prefix plus cancellation**
comparisons, not exhausted searches. S has six fewer finalized private graph
writes (2,465 → 2,459) at the stop, because its final partial chunk is discarded
by cancellation. Its whole allocation/work difference therefore must not be
claimed as equal background index progress. The completed wide cases establish
the primary gain without that ambiguity. I graph writes remain 2,529.

I/S graph peaks stay 2,807/2,777. I conditions fall 672 → 666; S conditions
grow 568 → 572 and pending nodes 136 → 138. S requested peak grows by 136 bytes
in the final cohort (1,717,909 → 1,718,045); raw ranges overlap some earlier
observations. I peak stays 1,962,330. Completion scans stay 2,792/2,648.
These small variations do not establish a general memory gain. Cancellation
allocation remains 13,576/20,072 bytes, including pending-syntax promotion.

`fair-grow 4 --rows 8` delivers its finite sibling beside continuing disjunctive
growth after the same 1,029 applications and 1,798 posts. Exact answer, pending
and condition peaks, and cancellation allocations match. Its source and cleanup
remain finite; the 726,101 → 726,109 source tick difference is not a gain.

## Retention, cancellation and displaced costs

All six conditional interaction cases retain four owners/siblings and continue
64 applications, with widths 16/64. Fixed snapshots reach 68 held applications;
inspectors 106/194; held output 68/72. Snapshots and inspectors each project four
exact retained graphs and choice pins with zero source applications during
projection. Held output delivers its finite answer and reaches 68/80 applications
on resumption, identically on both sides. These are matched milestones, not
comparisons at an arbitrary tick cutoff.

| Retained owner / width | Execution bytes, before → after | Requested peak, before → after |
|---|---:|---:|
| fixed archive / 16 | 6,234,704 → 6,256,148 | 248,936 → 248,936 |
| fixed archive / 64 | 9,750,024 → 9,728,088 | 758,744 → 758,744 |
| inspector / 16 | 10,423,740 → 10,423,740 | 190,711 → 190,711 |
| inspector / 64 | 36,491,284 → 36,491,284 | 497,902 → 497,902 |
| held output / 16 | 4,065,680 → 4,063,504 | 141,798 → 138,478 |
| held output / 64 | 5,541,404 → 5,538,972 | 245,158 → 245,158 |

The archive-16 allocation increase is retained in the assessment: +21,444
execution bytes (0.34%). Its before range is 6,209,448–6,263,624 and after range
6,242,180–6,270,936. The maintained comparator leaves that change uncertain.
Archive-64 has the opposite median direction and overlapping ranges. Collection
address order can alter scratch allocations; no archive allocation improvement
or proven regression is claimed. Requested peaks and held graph/condition/
pending ownership agree. Their graph path counts still fall (1,296 → 1,202 and
3,248 → 2,866). Concrete extra storage is small and neither a significant
offset to the wide gains nor a demonstrated scaling regression.

Held-output-16 peak is variable: baseline 140,622–145,390 and final
125,606–141,566. A preliminary candidate cohort reached 145,390. The final
comparator qualifies a decrease; this report does not generalize it as a peak
reduction mechanism. Inspectors' preparation, projection and cleanup costs are
included even where older lifecycle code attributes setup to `other`.
At width 64, fixed-archive inspection stays 393,152 bytes, inspector projection
282,596, and both cleanup allocations 4,752. Held-output cleanup stays
6,784/6,864 at widths 16/64. History cleanup/inspection remain 112,180/1,192;
alias cleanup stays 6,664 bytes.

The pending-cancel/snapshot controls at 64 and 256 tasks execute no posts or
applications, then check every pending relation/port, first and subsequent
capture, promotion, cancellation and release. Their engine, inspection, cleanup
and peak requested allocations are unchanged. At 256 tasks: setup 3,851,580,
execution 1,566,696; snapshot inspection 171,552 and cleanup 25,064;
cancellation-only cleanup 97,472 bytes. This change does not move graph costs
into preparation, syntax capture or cancellation. All final unowned graph,
occurrence, condition, pending, descriptor, history, snapshot, inspection and
release-queue counts reach zero. The live empty Engine coordinate epoch remains
until destruction. Notebook/pending probes also check final Prepared release.

### Cost of the batch itself

There is a stack array of eight key/optional-condition pairs and bounded
recursive traversal (at most 256 key bits), but no retained or heap-allocated
overlay. `Update` and Store layouts are unchanged. Keys are reconstructed once
at finalization; an unfinished chunk has no extra disposal work. The base and
returned roots retain ordinary Arc ownership. The array and recursion are not
included in requested-heap counters; this is an explicit stack cost.

Coalescing and logical previous-value checks are quadratic in chunk size;
normalization also sorts at most eight keys. Scalar base lookups remain, and
the recursive algorithm performs prefix comparisons and partitions. It is not
correct to equate the removed path count with total removed instructions. For
the completed 16/64 case there are 160 updates, each with 129 secondary writes:
2,720 finalizations (sixteen full chunks and one single-key tail per update),
20,640 reconstructed keys, and at most 143,360 duplicate-key comparisons across
the two normalization/accounting scans. This bounded scratch/bookkeeping replaces
repeated ancestor mutation, not all index work. The measured 608-record allocation
reduction remains after charging finalization, retention and complete cleanup.

Up to eight writes now occur in one service turn, while earlier positions do
bookkeeping only. The cap is independent of relation arity, so there is no
unbounded entire-relation publication pause. Runtime remains diagnostic only.
For 16/64, prepare/source-plus-delivery/validator/cleanup medians in ms are
0.249/27.151/0.388/0.571 → 0.253/24.128/0.393/0.579. First-event/first-answer
are 25.880/26.978 → 22.872/23.946. Full clocks, max-advance latency, separate
Engine/Prepared destruction and process outcomes remain in the raw records.
Some cohorts overlap compilation or tests and small clocks vary; no runtime
improvement, latency bound, or runtime regression certification is claimed.

The added batch traversal is 107 production lines plus the graph integration.
The bounded normalization/stack costs and measured small transient/cumulative
increases do not outweigh the repeatable completed-work improvements. KEEP is
warranted for this slice; full Store overlays and other stores remain outside
the implemented claim and are not acceptance requirements.

## Verification and reproduction

Test-first evidence: the 32-port path-work test failed on baseline with
554 mutations, exactly the scalar control. The wide oracle test failed before
its case existed, then passed real engine runs at arities 0, 1, 8 and 17 and
rejected missing/extra occurrences, reversed ports and aliasing. The deferred
stale-root test exposed progress before invalid-root rejection; the final guard
repairs that regression. Tests also exercise full-width map equivalence, net
no-op roots, retained pins, complete subtree deletion, frozen/foreign owners,
GC after every update suspension and abandonment at 68 positions.

Final results: **495 diagnostics Rust tests, 469 ordinary Rust tests, and three
Python observation-equivalence tests pass**. Builds and tests were separate;
every test/build-via-test invocation used `timeout --kill-after=5s 60s` and none
timed out. The first sandboxed full run failed its CLI socket test; the full
suites pass with loopback access. Production, examples and changed-test Clippy
passes with warnings denied. Formatting and diff checks pass. Full test logs
are committed beside this report.

```sh
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --features diagnostics --no-run
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --target-dir target/round005-native --no-run
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --features diagnostics --quiet
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --target-dir target/round005-native --quiet
CHR_MEASURE_BINARY=target/release/examples/measure timeout --kill-after=5s 60s python3 -m unittest discover -s tests -p measure_observation_test.py
cargo clippy --offline --all-features --lib --bins --examples --test store --test cancel -- -D warnings
cargo fmt --check
git diff --check
```

Baseline/common-harness worktree: `/tmp/chrimp-round005-baseline-harness`, detached
at `0c120a4`. Its only changes are the exact candidate `examples/measure.rs` and
`examples/measure/families.rs` diffs. No baseline production file changed.

```sh
# Baseline/common-harness worktree:
cargo build --offline --release --features diagnostics --example measure --target-dir /tmp/chrimp-opt-round005-batched-store/target/round005-baseline
# Candidate worktree:
cargo build --offline --release --features diagnostics --example measure
python3 docs/optimization-evidence/rehearse/round005-batched-store/run.py
python3 docs/optimization-evidence/rehearse/round005-batched-store/collect.py
```

The run script only supplies arguments to maintained `examples/perf.py`: five
samples, no warmup, external ten-second sample limit, sixty-second cohort limit,
source/cleanup limits of 50 million ticks / three seconds, detailed observation.
These are per-run measurement censoring limits, not a candidate or campaign time
budget. Every cohort completed. Exact options and commands are in the records.
The original baseline rewrite-128 cohort predates the wide-case harness addition;
its existing workload path and production code are unchanged. Preliminary
candidate observations remain locally under `evidence/round005/before-stale-check`
and are excluded from the final 290 samples. No rwLog benchmarks were rerun.

Baseline measured binary SHA-256:
`7f8b69cf5b3b8d0fa9eb60e377bfb499fe419122373adfaa3e973b3da83cb1a5`.
Final candidate measured binary SHA-256:
`f14dbdb45c10abe1f1ed3d155b19d40c0f39700638ddc3f569bbec68ef29a6cd`.
Only the `graph_writes` documentation comment changed after the final build.

`measurements.json` contains all maintained medians, varying ranges, missing
fields and first-sample endpoints. `raw-samples.json.gz` preserves every typed
sample, outcome, statistic and command/provenance record. Its uncompressed
SHA-256 is `7cb11076ab130fc86eb8b730ecc93831d9c8c911123c4fc6ed04ecbd63756fbd`.
`collect.py` packages existing records; it is not a benchmark implementation.
The three committed comparison JSON files use `examples/perf_compare.py` and
declare their metrics. Their within-case tests do not establish global
regression absence or broad statistical coverage. Unavailable/incomplete fields
remain explicit, and conclusions are limited to the measured intervals.
