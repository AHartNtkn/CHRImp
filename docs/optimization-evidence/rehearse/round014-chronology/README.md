# Rehearse Round 014: reclaimable Boolean chronology

Recommendation: **KEEP the evaluated implementation. Do not integrate from this
task.** It materially reduces dense peak/live storage, removes the historical
high-slot chronology-retention term, and preserves the measured semantic and
lifecycle obligations without significant measured regression. The isolated
branch contains separate implementation, evidence and report commits.

| Item | Revision |
|---|---|
| Accepted baseline | `225fe302fa45c7667684107ba96cba357cf103dd` |
| Implementation | `7e7b1f48ae0f3422ad21a1355ee214332629758c` |
| Evidence/data | `c3ddf45c22feaff3a54e0720c02c06d7ba3b6010` |
| Reused baseline diagnostic harness | `ca8a55d8bed1835c2174c87f8bc40c78128f8053` |

This report is the subsequent separate commit. Worktree:
`/tmp/chrimp-opt-round014-chronology`; branch:
`codex/opt/round014-chronology`. The landing checkout was not edited or integrated.
No campaign control/history update or rwLog benchmark was run.

## Implemented mechanism and limits

`src/condition.rs` replaces the baseline ordered node payload map with a hash
directory of live eight-node epochs. The complete monotonic 64-bit ID selects
the epoch and payload position. Empty epochs unlink and release immediately.
Each live epoch stores predecessor/successor links; no retired slot needs a
chronological descriptor. Before removing the current node, a frozen collector
saves its next live ID in one additional `Option<u64>`: 16 bytes, no heap lease
allocation, and no ownership of historical payloads. The existing exclusive GC
lease prevents allocation during traversal. Reset and canonical-table rebuild
passes only read payloads; sifting finishes outside active sweep traversal.

Unlike Round 013, IDs are not packed into 32-bit generation/slot halves. The
baseline checked 64-bit construction limit and chronological sift boundaries
are preserved. An empty epoch may later be recreated for a higher serial in
that epoch, but a retired ID is never assigned again. Weak canonicalization,
archive ownership indexes and suspended Job roots remain authoritative.

An epoch is 744 requested bytes on this build, including eight optional
88-byte payloads and 40 bytes of count/links. Lookup uses expected constant-time
hash resolution and direct indexing. Successor traversal inspects at most two
live epochs and sixteen positions, independent of retired population. This
replaces ordered payload/successor searches; the other archive/order trees remain.
There is no claim of fewer Boolean semantic operations or worst-case constant
hash lookup. Hash collisions, inserts/removals and rehash work are replacement
costs, not free operations.

Directory contraction runs at collection completion when effective capacity
exceeds twice live entries. It can scan old backing and rehash live entries in
one size-dependent call. Interrupted collection may retain directory capacity
until a subsequent completed collection; it does not need tombstones for its
cursor. No occupied payload moves. Up to seven vacant payload positions per
live epoch remain charged. This does not eliminate all sparse-block slack or
establish performance for every possible scattered-retention workload.

Other changed implementation files:

- `examples/measure.rs`: epoch/storage/work checkpoints.
- `examples/measure/notebooks.rs`: the same observation-only source-dispatch
  stopping boundary used by the saved baseline harness.
- `docs/performance.md`: diagnostic fields, work boundaries and limitations.

The evidence directory contains orchestration/reporting helpers and raw data;
the maintained `examples/perf.py` runner and workload oracles perform measurements.

## Equivalent progress and lifecycle coverage

The final comparison has 20 diagnostic points and five ordinary controls,
three samples per side, no warmups. All 150 final observations passed their
native oracles: 120 completed and 30 deliberately unfinished synthesis intervals.
Thirty additional ordinary observations that overlapped tests/builds are retained
but excluded from the reported comparison. The archive therefore preserves
**180 observations: 150 completed, 30 censored, zero native oracle failures**.
Every supervised process completed group cleanup.

- I/S deliver their first independent SK-validated answer: 126/134 applications,
  73/125 output scalars respectively.
- B/C/W/T/M stop at 500,000 non-collection dispatches, with 639/657/590/588/548
  applications and zero answers. These are named unfinished search intervals,
  not exhaustion or successful answers. Per-rule vectors and every other
  dispatch category agree at shared checkpoints; tick counts alone are not gains.
- Rotating conditional archives use 1/4/16 owners, four rows, 32 continuing
  applications and cadence four. Admission, held readers, projection, unpinning,
  cancellation and final release remain measured. Projection-only draining
  performs zero additional source applications.
- Conditional inspections, held output, recorded history, runtime sessions and
  replay, 256 distinct empty answers, finite progress beside growing siblings,
  full rewrite, and bits-chain 16/64/128 use their existing independent oracles.

Preparation, execution, answer delivery/validation, retention, cancellation and
final destruction stay in the runner. Source live bytes for archive cases are
after unpinning while execution remains live; they are not bytes solely owned
by held archives. Process peaks include earlier retained phases. Detailed
validator time is already included in delivery and is not added twice.

## Measured gains and regressions

Three-sample medians of process requested bytes, not RSS. Full distributions,
phase allocation traffic and raw observations are retained in `summary.json`,
`audit.json` and `raw.tar.gz`.

| Obligation | Peak bytes, baseline → candidate | Change | Source live bytes, baseline → candidate | Change |
|---|---:|---:|---:|---:|
| I first answer | 1,978,258 → 1,968,146 | −0.51% | 1,818,569 → 1,807,433 | −0.61% |
| S first answer | 1,726,637 → 1,711,174 | −0.90% | 1,559,461 → 1,550,461 | −0.58% |
| B unfinished | 6,286,231 → 5,922,207 | −5.79% | 4,962,679 → 4,762,975 | −4.02% |
| C unfinished | 6,228,503 → 5,934,583 | −4.72% | 4,651,247 → 4,563,727 | −1.88% |
| W unfinished | 6,158,778 → 6,025,290 | −2.17% | 5,999,089 → 5,864,577 | −2.24% |
| T unfinished | 6,088,396 → 5,818,380 | −4.43% | 5,928,707 → 5,657,667 | −4.57% |
| M unfinished | 6,244,103 → 6,003,759 | −3.85% | 6,084,414 → 5,843,046 | −3.97% |
| 256 empty answers | 1,791,946 → 1,631,978 | −8.93% | 1,554,418 → 1,404,642 | −9.64% |
| bits-chain 64 | 2,066,352 → 1,919,072 | −7.13% | 508,880 → 509,380 | +0.10% |
| bits-chain 128 | 6,865,611 → 5,678,491 | −17.29% | 965,727 → 966,971 | +0.13% |
| Archive, 1 owner | 244,082 → 245,946 | +0.76% | 172,506 → 170,594 | −1.11% |
| Archive, 4 owners | 401,138 → 403,146 | +0.50% | 230,298 → 230,754 | +0.20% |
| Archive, 16 owners | 624,387 → 612,747 | −1.86% | 260,699 → 259,427 | −0.49% |

The archive source checkpoints retain 6/9/11 epochs for 33/38/62 nodes:
4,464/6,696/8,184 payload bytes and estimated 152/288/288 directory bytes.
Round 013 retained 6,400/12,800/25,600 directory bytes at these same obligations
and regressed archive live storage 4.00%/5.77%/9.84% against the accepted baseline.
Those historical results are explanatory context, not the matched baseline here.

Sparse structural tests isolate the former historical-directory dependency:

| Initial nodes | Payload bytes, dense → one high survivor | Directory after retirement | Live chronology records | Reclaimed epochs |
|---:|---:|---:|---:|---:|
| 64 | 5,952 → 744 | 84 | 1 | 7 |
| 512 | 47,616 → 744 | 84 | 1 | 63 |
| 4,096 | 380,928 → 744 | 84 | 1 | 511 |

Round 013 needed 102,400 directory bytes for the 4,096-node case. The accepted
tree already avoids historical-slot growth; this test establishes that the new
representation removes the earlier slab's regression. It is a candidate
structural gauge, not a fabricated matched whole-process baseline allocation
measurement. Directory bytes estimate standard HashMap bucket/control backing;
process allocation counters are the byte authority. Each case evaluates the
survivor, rejects retired/complemented handles, admits a fresh chronological ID,
and finishes with zero payload/directory capacity after ownership release.

Other measured dimensions were reviewed. Fair-grow peak/live bytes improve
6.72%/8.46%. Inspections peak/live rise 0.99%/0.58%; held output peak rises 0.78%
and live falls 0.77%. History peak rises 0.04%; runtime is effectively unchanged;
rewrite peak/live are unchanged. These small costs do not outweigh the measured
dense gains and removal of historical chronology growth. Total allocated bytes
fall 1.40–1.62% on B/C/W/T/M and 4.10% on answers, but total allocation is not
used as a gate for the independent storage results.

## Work, displaced costs and uncertainty

Source directory get/get_mut calls for B/C/W/T/M are approximately
734k/847k/723k/749k/641k, with 20,936/25,592/21,248/22,344/18,408 chronology
position inspections. Baseline ordered access counts include range operations;
candidate node-access counts exclude the separately measured successor lookup.
Their raw totals are not interchangeable units of semantic work. Constructions
vary slightly with collection/representation maintenance despite identical
language progress; no Boolean construction reduction is claimed.

Candidate source rehash-entry counts for B/C/W/T/M are 548/1,645/1,056/1,255/915
(medians). These count relocated directory entries, not all bucket scans or CPU
instructions. The allocator charges directory growth/contraction and transient
backing, epoch allocation and reclamation, and final destruction. Directory and
slab inline headers, per-collector witness bytes and unused payloads remain
charged by process accounting.

Cleanup allocation counts match baseline at every diagnostic point. Allocated
cleanup bytes increase by 16 bytes at most points and 32 for history/runtime,
consistent with the extra scalar successor in collector storage. For example,
answers retain five cleanup allocations, 4,832 → 4,848 bytes; B retains 786,
159,368 → 159,384 bytes. This removes Round 013's free-index-tree cleanup churn.
Final process live bytes match baseline at every diagnostic point, including
1,729 bytes after synthesis engine/prepared-owner destruction. Unowned cancelled
engines have zero epoch/directory backing; recorded history intentionally retains
one condition until its separately validated release.

The quiet ordinary controls show median delivery changes −21.45% I, −4.09% S,
−13.14% answers and +4.03% rewrite. Three short samples have substantial noise;
these are diagnostic observations, not architecture ranking or proof of timing
equivalence. The initial overlapping ordinary samples remain separately labeled.
Diagnostic runs initially overlapped some tests, so their timing interpretation
is correspondingly limited. Process-local storage/work counters establish the
reported gains. No claim is made for unmeasured workloads or all fragmentation.

## Semantic audit, verification and reproduction

Diagnostics: **518 tests across 44 binaries pass** after two sandbox-blocked
socket binaries pass with loopback access. Ordinary: **488 tests across 44
binaries pass**. Final strengthened library coverage passes 115 diagnostic tests
and is included in the ordinary suite. Builds and executions were separate;
every test invocation used `timeout --kill-after=5s 60s`. No test timed out.
Production Clippy with diagnostics, formatting and diff checks pass.

New tests cover full serial width/exhaustion, complete current-epoch retirement
with a frozen successor, sparse high archives, stale/complemented handles,
recreated epochs, and 160 interruptions across ordinary/archive sweeps with
suspended Jobs, fresh allocation, cancellation and final reclamation. Existing
truth-table/sift tests, archive reorder/reference checks and integration suites
cover identity matching without merging heads, duplicate occurrences and ordered
propagation combinations, fresh body variables, explicit alternatives, committed
scheduling, exact answer multiplicity, finite progress, shared computation,
snapshots, observations, optional history, runtime replay and autonomous notebook
execution. No language, scheduler, graph, matching or runtime implementation was
changed. This is concrete test evidence, not a formal universal proof.

`verify.py` checks paired rule/dispatch vectors, exact unfinished dispatch
boundaries, projection-only progress, retained history, zero unowned backing,
native result statuses and process cleanup. It passes with all 180 raw records.
Original socket failures, build output, helper notes and censored observations
are preserved. `binaries.sha256` and `provenance.json` identify the exact builds.

From this worktree, extract `raw.tar.gz` into this report's directory, then run:

```sh
python3 docs/optimization-evidence/rehearse/round014-chronology/summarize.py
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round014-chronology/verify.py
```

`matched.py` specifies the complete final matrix and invokes maintained perf.py;
the exploratory main branch in inherited `evaluate.py` was not executed.
The data commit required retrying Git with permission to write the isolated
worktree index after a read-only sandbox error. It succeeded without touching
the landing checkout.

Under the unchanged acceptance rule, this evaluated implementation qualifies
for KEEP on material storage/representation gains. Integration remains explicitly
reserved for the landing task; this worktree only preserves the reviewable result.
