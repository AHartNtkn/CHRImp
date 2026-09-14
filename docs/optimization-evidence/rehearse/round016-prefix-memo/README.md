# Rehearse Round 016: bounded weak prefix-root memo

Recommendation: **KEEP** the evaluated implementation for its material reduction
in repeated Store prefix path inspections, with small measured storage and
allocation costs. This is implementor advice. No integration or campaign outcome
was recorded, and the landing worktree remains clean at `d141a9f`.

| Item | Revision |
|---|---|
| Accepted baseline | `d141a9f` |
| Shared initial instrumentation | `cca15e7` |
| Baseline plus matched diagnostics/churn oracle | `46030a2` |
| Evaluated implementation, probes and maintained documentation | `e7b0335` |
| Raw evidence, analysis and execution notes | `3d778db` |

This report is the following commit. Candidate worktree:
`/tmp/chrimp-opt-round016-prefix-memo`, branch
`codex/opt/round016-prefix-memo`. The implementation depends on `cca15e7`.
The baseline harness is `/tmp/chrimp-round016-baseline-harness`.

## Implemented mechanism and safety

Each Store has a lazy hash table with at most 32 entries. The key consists of
the input root allocation address, normalized prefix, and prefix word count.
Store ownership is implicit in the per-Store table and explicitly checked in
the weak input witness. Entries hold weak input and result witnesses; they
own no payloads or descendant roots. An empty result is also cacheable.

The weak input prevents allocation-address reuse and in-place mutation of the
witnessed root. Mutating a uniquely strongly owned but weakly witnessed root
therefore copies it. This is a real replacement cost, not a free certificate.
An exact root generation identifies its immutable subtree relationships. Every
request validates the supplied root before consulting the table, and every
upgraded result is checked against current Store collection validity. Foreign
and stale input roots remain rejected. This establishes snapshot membership
without re-traversing the path on a hit.

A miss executes the original traversal. At capacity, the next miss clears all
32 entries; collection clears entries and releases table backing. Individual
eviction/drop operations release weak allocation headers, with no recursive
payload traversal. Store destruction also releases the cache. Active readers
continue to own their snapshots through the existing mechanisms. No history,
answer, scheduling, producer-projection or cancellation semantics changed.

This is narrower than reusing a lookup across *different* input root identities:
an unrelated mutation that changes the Store root still misses, even if the
restriction subtree remains unchanged. Round 015 continues to reuse those
projected fragments after discovery. Round 016 avoids discovery on repeated
subscriptions/readbacks to the same cached generation; it does not duplicate
Round 015's projection mechanism.

## Equal-progress growth and churn

The maintained graph/matcher probe prepares an initial restriction bucket of
128/512/2048 rows. It completes nine matches around eight duplicate occurrence
insertions, retains the nine snapshots, then matches all nine again. Each pass
checks the exact ordered occurrence IDs, including multiplicity and cutoffs;
forward results additionally check true support. There are 36 matched pairs per
pass, 72 across both passes. The interval includes preparation, publications,
matching, result checking, retention, collection, release, and final destruction.

| Initial bucket rows | Requests | Inspected path nodes, baseline → candidate | Hits / misses | Requested process peak, baseline → candidate |
|---:|---:|---:|---:|---:|
| 128 | 18 | 144 → 72 (−50%) | 9 / 9 | 697,361 → 698,824 (+0.210%) |
| 512 | 18 | 162 → 81 (−50%) | 9 / 9 | 2,534,969 → 2,536,304 (+0.053%) |
| 2048 | 18 | 198 → 99 (−50%) | 9 / 9 | 9,882,457 → 9,883,792 (+0.014%) |

The candidate adds 18 hash-table lookups and nine insertions per interval, plus
rehashing during table growth, weak reference operations, locking and validity
checks. The baseline performs no memo lookups. Hashing includes root identity,
four normalized prefix words and word count. Internal hash-bucket probes and
instruction totals are not measured; the path counter establishes removed
node inspections, not a claim of 50% less total lookup work or CPU time.

Both sides project/retain 164 / 548 / 2084 rows. Graph allocations and graph
nodes match at 4,684 / 17,380 / 68,116. Cleanup ticks match at 2,759 / 9,887 /
38,351. Each candidate interval requests 2,340 additional allocated bytes and
three additional allocations. Readback live bytes increase by 2,359, including
table backing and dead allocation headers retained by weak keys. Weak ownership
does not keep a destroyed payload alive, but its allocation storage persists
until the last weak reference is released; allocator measurements charge it.

The maintained extension grows 33/65/129 versions at 128 bucket rows, exceeding
capacity. It validates 528 / 2,080 / 8,256 matched pairs **per pass**, reads every
retained cutoff, and finishes collection/release. Sequential readback churns
the table: all 66 / 130 / 258 lookups miss. Path inspections remain exactly
528 / 1,040 / 2,064 on both sides. Evicted entries are 64 / 128 / 256 before
final collection; at readback the cache owns two entries with capacity 56.
Capacity is table capacity, distinct from the enforced 32-entry bound.

Churn's extra total requested allocation is 10,148 bytes and five calls per
interval. Extra peak bytes are 5,223 (0.519% / 0.379% / 0.241%); extra readback
live bytes are 5,351. Producer diagnostics agree on both sides. Cleanup ticks
are identical at 4,163 / 6,227 / 10,643. Every probe finishes with zero graph
nodes, occurrences and restriction rows. Candidate final harness live is one
byte lower solely because its worktree/binary path is one character shorter.

The three sizes in each probe run in one process. Peaks are cumulative high-water
marks; each later size exceeds the preceding one. These are single exact
mechanism observations, not repeated timing experiments. Diagnostic printing
between checkpoints contributes later allocation traffic.

## Maintained workload controls and displaced costs

The existing perf runner collected 84 completed samples: three per side at 14
points. All native oracles and process cleanup checks passed; none were censored.
Controls include partial-join and partial-join-hit at 64/256/1024 rows with eight
probes, notebook I/S first independently SK-validated answers, wide64 rewriting,
conditional archive rotation and inspections, recorded choice history, runtime
sessions/replay and finite progress beside divergent growth.

All 42 pairs agree on answer/application/scalar obligations, exact per-rule
vectors, normalization, field updates and producer diagnostics. Source graph
allocation and copy-on-write counts also match at every available checkpoint.
No maintained control has a memo hit: partial joins make eight misses each,
with 112 / 128 / 144 unchanged path inspections; the other engine controls make
zero requests. Runtime does not expose a source checkpoint. No end-to-end
execution improvement is claimed for these controls.

Across maintained points, median requested peaks increase by 0.001–0.563%; the
largest is runtime. Total requested allocation changes by −0.088% to +0.074%,
and allocation calls by −0.001% to +0.019%. All final process live bytes agree.
After-cancel checkpoints show zero retained restriction rows/partitions/plans
and zero memo entries/backing capacity. The additional per-Store header and
stats are charged even where no prefix lookup occurs.

Full preparation, execution, delivery, validation, inspection, cleanup and
other allocation categories remain in `summary.json` and raw records. Small
collection/allocation differences occur within unchanged binaries too; they are
retained and not credited as memo gains. For example, archive arena collection
work varies 24,250–24,622 in baseline and 24,452–24,647 in candidate. Runtime
is diagnostic only and did not determine the recommendation.

## Validation, reproduction and limits

525 diagnostic tests pass, including the new generation/churn/release unit test,
474 library/integration tests in total and 51 example/oracle tests. Another 159
selected ordinary-mode tests pass. The baseline's 32 selected Store/restriction
tests pass. Two initial socket failures were resolved by rerunning the complete
prebuilt binaries with loopback permission; their original failures are retained.
No test timed out. Clippy, formatting and whitespace checks pass.

Focused coverage includes ignored suffix normalization, repeated hits, absent
prefixes, eviction, old/current payload isolation, weak-only root mutation,
dead weak upgrades, foreign/stale root rejection and full cache release. Existing
tests cover Store snapshots and freeze rules, conditional identity, cancellation
at suspension points, lagging readers after cache clear, ordered matching,
propagation multiplicity, history, runtime replay and finite progress. These
are concrete checks, not a proof over all programs.

Exact build and validation commands are in `notes.md`; individual commands,
cwd, exits and outputs are archived under `logs`. Builds used `--no-run` and
were separate from tests. Each prebuilt test invocation was guarded by
`timeout --kill-after=5s 60s`. Measurement and audit entry points:

```
python3 docs/optimization-evidence/rehearse/round016-prefix-memo/evaluate.py tests
python3 docs/optimization-evidence/rehearse/round016-prefix-memo/evaluate.py perf
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round016-prefix-memo/analyze.py
```

For archived evidence, run `tar -xzf raw.tar.gz` in this directory, then the
guarded `analyze.py` command. The audit checks 42 paired samples. Binary hashes
and full compiler artifact records are preserved. No rwLog benchmark was run.

KEEP is recommended on the demonstrated independent path-inspection reduction;
the bounded storage costs and unchanged measured mutation/projection/cleanup
work do not outweigh it. The memo is not a universal matching improvement:
single subscriptions, changed roots, collection barriers and sequential churn
can all eliminate reuse. Hash-table lookup costs, possible weak-induced copies
on other workloads, concurrent contention and larger working sets remain limits.
No total instruction-count, wall-clock speedup or universal scaling claim follows
from these observations. The acceptance rule is preserved in `acceptance.md`.
