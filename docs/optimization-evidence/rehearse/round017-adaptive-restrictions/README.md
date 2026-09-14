# Rehearse Round 017: adaptive shared-restriction partitions

Recommendation: **KEEP** the evaluated two-entry inline sorted table. It removes
the partition allocation for one/two-key producers and materially reduces their
representation cost. Promoted/empty producers have a disclosed 128-byte larger
header; the measured whole-process costs do not outweigh the demonstrated gain.
This is implementor advice, not a recorded campaign decision or integration.

| Item | Revision |
|---|---|
| Accepted baseline | `33947a6` |
| Shared instrumentation and initial oracles | `ae05a1d` |
| Baseline harness with final common oracles | `d236ca4` |
| Evaluated implementation and maintained documentation | `91c833c` |
| Raw evidence, audits and execution notes | `4e55e88` |

This report is the following commit. Worktree:
`/tmp/chrimp-opt-round017-adaptive-restrictions`, branch
`codex/opt/round017-adaptive-restrictions`. The implementation depends on
`ae05a1d`. The landing worktree remains clean at `33947a6`.

## Mechanism and preservation

`Producer.partitions` now wraps a sorted inline array holding two full-width
projected keys and their first/last row indices. A third distinct key promotes
once to the existing HashMap representation, reinserting at most two entries.
Repeated keys update the existing last link. Keys can move within the table;
occurrence rows and their next links retain their indices. No per-key heap
allocation or recursive ownership chain is introduced.

The row vector, projection mask, singleton checks, snapshot cutoffs, subscription
positions/support, cache ownership, abandonment and collection paths are
preserved. Neither prefix-plan nor prefix-root-memo logic changes. Promotion
executes inside the existing producer lock and one bounded producer service;
subscribers cannot observe an intermediate table. Ordinary execution gains no
history or extra retained roots.

The common baseline adapter retains the accepted get/get_mut/insert protocol
and HashMap layout. Both sides share new diagnostic fields and eliminate the
old diagnostic-only redundant lookup after insertion, using the insertion's
new-key flag instead. This is not credited as an adaptive-table gain. The new
diagnostics add 64 bytes per Graph's diagnostic statistics, absent in ordinary
builds. Per-producer layout costs below apply in ordinary builds too.

## Threshold and equal-result mechanism evidence

Each table fixture performs eight complete matches, checks exact ordered IDs
recorded at publication and true support, verifies one shared projection scan,
then collects/releases all graph, occurrence and table storage. The sweep varies
1–128 rows, distinct keys, and 128 repeated rows over one/two/four keys. Descending
keys exercise sorted insertion; duplicate occurrences retain equal arguments.

| Inline capacity | One/two-key table bytes | Extra header after promotion | Allocation behavior |
|---:|---:|---:|---|
| Baseline hash | 388 | — | Initial bucket allocation |
| 1 | 96 / 436 | 48 | Avoids allocation only for one key |
| **2** | **176 / 176** | **128** | Avoids allocation through two keys |
| 4 | 336 / 336 | 288 | Avoids allocation through four keys |

Capacity two reduces one/two-key table bytes **54.64%**, saving 212 live/requested
bytes and one allocation per producer over the full tested interval. Capacity
four saves only 52 bytes on one/two-key tables, though it wins at four keys;
its larger promoted header makes it a weaker general compromise here.

With 128 rows over one key, hash requests fall 138→0, replaced by 135 full-key
comparisons across the eight matches. Over two keys they fall 140→0, replaced by
271 comparisons. For 128 distinct keys they fall 392→386, with seven initial
comparisons, one promotion, and two reinsertions. This establishes removal of
hash requests, not a total CPU/instruction or internal bucket-probe reduction.
Each comparison can inspect multiple words. Promotion and entry shifts are
counted, and subsequent HashMap growth remains charged by the allocator.

Costs are explicit: promoted and empty tables add 128 bytes each; at three keys
the table footprint rises 388→516 (+33.0%). In isolated completed processes,
the largest peak increase is 26,137→26,274 bytes (+0.524%) at three keys, including
nine bytes from the longer candidate binary path. At 128 distinct keys the
increase is 581,421→581,558 (+0.024%). One/two-key isolated peaks fall 203 bytes;
interval live/allocation savings, normalized against start, are exactly 212.
Final isolated harness live deltas are zero. These are bounded representation
gains and costs, not a claim of a different large-table asymptotic complexity.

## Prefix growth, retained snapshots and cleanup

The accepted growth probe completes nine version matches around eight duplicate
insertions and reads all nine retained snapshots again. Exact multiplicity,
order, support and cutoffs pass on both sides. Row projections remain
164/548/2084 at 128/512/2048 initial rows. Prefix-plan counters, prefix-root memo
hits/misses and inspected paths, graph allocations and cleanup ticks all match.

| Initial rows | Retained table bytes, baseline→candidate | Process peak, baseline→candidate | Interval allocations saved |
|---:|---:|---:|---:|
| 128 | 23,904→22,336 | 698,889→697,330 | 8 |
| 512 | 86,304→85,120 | 2,536,369→2,535,194 | 8 |
| 2048 | 335,904→336,256 | 9,883,857→9,884,218 | 8 |

The large-row cost is +352 normalized live/requested bytes and +0.00365% raw
process peak. Small suffix fragments benefit from the inline table while dense
fragments retain its extra header. Hash requests fall 471→397, 1680→1588 and
6516→6352, replaced by 51/72/156 comparisons and 2/8/32 promotion reinsertions.

The 33/65/129-version cache-churn probe validates 528/2080/8256 ordered pairs
per pass and every retained snapshot. It saves 32/128/256 allocations and
6,528/26,624/53,248 requested bytes over complete intervals. Readback live storage
falls 6,528/6,656/6,656 bytes; peaks fall about 0.644%/0.471%/0.300%. Both sides
have identical cleanup and finish with zero graph nodes, occurrences and table
storage. Probe logging accounts for small retained harness deltas; it is not
engine retention. Peaks within each three-size probe are cumulative.

## Maintained performance matrix

The detailed matrix has **66 completed pairs** at 22 points: partial-hit/miss
joins at 1/2/4/8/64/256/1024 rows, notebook I/S first independently validated
answers, wide rewriting, conditional archive rotation/inspection, recorded
history, runtime replay and finite progress beside divergent growth.

All pairs agree on answer/application/scalar obligations, exact per-rule vectors,
original restriction diagnostics, normalization, field updates, graph allocation,
Store mutation, and both accepted prefix mechanisms. After-cancel checkpoints
release all restriction/memo backing. Median process peaks are unchanged at
20 points; archive rises 0.294%, S falls 0.043%. These two controls create no
restriction producers, so their small allocation/collection variation is not
credited to the mechanism. Total requested allocation changes range from
−0.321% to +0.022%, and allocation calls from −0.030% to +0.016%. All phase
allocations, retention checkpoints and raw ranges are retained.

The maintained routine suite adds **78 completed pairs** at 26 points, covering
fresh identity/contraction, occurrence multiplicity, proofs, preparation reuse,
arithmetic, lambda, type synthesis and session turnover. All native oracles and
paired completed-result checks pass. No completed point's median peak increases;
total allocation changes range −0.0291% to +0.0076%. Three W synthesis samples
per side are censored; their unequal unfinished work intervals are preserved
without an equal-progress cost comparison. The suite correctly returns censored,
not a universal pass. Runtime was diagnostic only, and three repeats are not a
calibrated statistical regression assessment.

## Validation and limitations

**530 diagnostics tests and 491 ordinary tests pass**, with no ignored candidate
tests; baseline checks pass 35 tests with the intentionally ignored candidate
resource contract. Focused coverage includes full-width multiport keys, sorted
promotion, unchanged first links, repeated last-link updates, paused readers,
cache eviction/collection, cancellation before/after promotion, old snapshot
cutoffs, conditional aliases/support, and complete reclamation. No test timed out.

The initial expected resource failure and corrected fixture/environment failures
are retained. Two HTTP tests needed loopback permission. A diagnostics CLI retry
needed the matching CLI binary restored after the ordinary build. Strict
production Clippy and formatting pass; all-target Clippy exposes an unchanged
Boolean-oracle lint also reproduced on baseline, and passes with only that lint
allowed. No unrelated code or landing files were modified.

Exact commands, build records, failures, audit results and reproduction guidance
are in [notes.md](notes.md); full observations are in `raw.tar.gz`. Acceptance
is preserved verbatim in `acceptance.md`. The recommendation rests on independently
valuable allocation/representation savings and bounded measured costs. Empty or
three-key-heavy workloads can lose storage; large-table hash probes, instruction
totals, concurrent contention and universal speedups remain unmeasured.
