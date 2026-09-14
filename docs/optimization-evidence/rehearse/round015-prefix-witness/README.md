# Rehearse Round 015: shared immutable restriction prefixes

Recommendation: **KEEP** the evaluated implementation for its material removal
of repeated projection work and retained row buffers. The measured replacement
costs do not outweigh those gains. This is implementor advice; no integration,
campaign outcome, or landing-worktree modification was performed.

| Item | Revision |
|---|---|
| Accepted baseline | `6bf6e1ddeab3e4cc90d9f0ab665d712624471f48` |
| Shared semantic/allocation probes | `fcbe3cc`, `2304369` |
| Baseline plus those probes | `0ccbf0b` |
| Implementation | `67bf8cb87ef46eac38f775e106a86320e834d04f` |
| Evidence | `4ef6090b27a243d2ee0813434c5a9f970b400954` |

This report is the following commit. All candidate commits are on
`codex/opt/round015-prefix-witness` in
`/tmp/chrimp-opt-round015-prefix-witness`. The implementation commit's parents
include both probe commits; retain them when integrating. The landing remains
clean at `6bf6e1d`. The selected field was malformed, so selection was recovered
read-only from the helper: candidate 0. A mismatched explanation in verdict 0-1
is documented in notes.md for the orchestrator to reconcile.

## Actual implementation

The baseline already reuses an unchanged entire bucket via `WeakRoot`.
The new mechanism also reuses unchanged subtrees within changed buckets.
Buckets of 129–4096 rows split incrementally into ordered Store subtrees with
at most 128 rows. Each subtree has an exact weak allocation-identity witness,
including Store owner; weak ownership prevents in-place mutation and address
reuse. This is the generation certificate, with no additional global numeric
snapshot generation or Boolean chronology change.

Unchanged fragments share their existing projected partition buffers. Changed
fragments rebuild one row per service call. A separate bounded whole-prefix
plan shares traversal and pins earlier producers for lagging readers, including
after registry eviction or collection. There are at most 32 cached whole plans
and 32 directly cached row producers; each admitted bucket is bounded by 4096
rows. Active readers may retain evicted plans, as they retain baseline producers.
Plan links and traversal stacks are additional storage, charged in allocations.
Completed projections retain IDs/links, not Conditions or Store payload roots.
Unfinished cursors and pending subtree roots are traced through the reader's
snapshot. Last-reader cancellation clears unfinished plan ownership.

Small buckets use the original whole-bucket producer. Matching still rechecks
singleton identities before subscription, reads support from its snapshot, and
preserves distinct occurrences and ordered output. Subtree splitting does not
enumerate additional alternatives or change the scheduler's service policy.
The implementation adds one branch/subscription/reader transition per relevant
service call; these are measured replacement costs.

## Equal-obligation measurements

The graph/matcher growth probe prepares the same rule and initial graph on both
sides, completes nine queries around eight relevant duplicate insertions, then
reads all nine retained snapshots. Each version returns exactly its expected
ordered occurrence IDs and support: 36 matched pairs on the forward pass and
36 on readback. It includes preparation, publication, matching, result checking,
retention, collection, release, and final graph/arena/prepared destruction.

| Initial rows | Projected/retained rows, baseline → candidate | Reduction | Process requested peak, baseline → candidate |
|---:|---:|---:|---:|
| 128 | 1,188 → 164 | 86.20% | 910,057 → 697,347 (−23.37%) |
| 512 | 4,644 → 548 | 88.20% | 3,410,057 → 2,534,955 (−25.66%) |
| 2,048 | 18,468 → 2,084 | 88.72% | 13,402,825 → 9,882,443 (−26.27%) |

At 2,048 rows the candidate performs 143 splits, 152 fragment probes, 24 builds
and 128 fragment hits, retaining 152 plan links across nine snapshots.
Readback reuses those plans and performs no additional row projections.
All these metadata/buffer costs are in the requested-byte observations.
Graph allocations match exactly at 4,684 / 17,380 / 68,116, as do final cleanup
ticks at 2,759 / 9,887 / 38,351. Both sides finish with zero graph nodes,
occurrences and restriction rows.

Total allocated bytes across each full probe interval fall from
1,233,438 / 4,428,382 / 17,202,238 to
817,022 / 2,728,762 / 10,379,018. Allocation calls are mixed:
6,005 → 5,950; 20,593 → 20,565; 78,796 → 78,943 (+0.19%). This increase is
retained as a cost, not used as a gate against independent work/storage gains.
Final harness live bytes are 5,058 versus 5,060; executable-name length differs
by two bytes. The maintained allocator measures requested Rust allocations,
not RSS or allocator metadata. The three sizes run in increasing order in one
process; peaks are cumulative high-water marks, and each later size exceeds
the preceding peak. Diagnostic serialization occurs between checkpoints and
contributes to subsequent traffic. This is one exact mechanism observation per
size, not a repeated timing experiment.

The maintained perf runner collected **84 completed observations**, three per
side at 14 points, with no warmups, censoring or native oracle failure. Every
process completed cleanup. I/S each deliver their first independently
SK-validated answer (126/134 applications, 73/125 scalars). Both I/S use zero
shared restriction producers here: they are controls, not evidence of a gain.
Other controls cover 64/256/1024-row partial joins with eight probes, successful
and unsuccessful joins, wide64 rewriting, conditional archives and inspections,
recorded history, runtime sessions/replay, and fair progress beside divergence.

At size 256, both maintained partial-join cases project **768 → 510 rows
(−33.59%)**. Their peak retained rows change 735 → 510 (miss) and 657 → 510
(hit); matching dispatches fall by 193. At size 1024, both project 1024 rows
and the candidate adds **169 matching dispatches** and 72 partition lookups.
There is no gain claim for that unchanged-bucket regime. Size 64 keeps the
original work. All paired per-rule obligations apart from matching dispatches
agree, including applied/started/rejected commits, found matches, indexed
candidate visits and tasks. The audit retains dispatch differences explicitly.

## Costs, retention and limits

Across maintained workloads, total requested allocation changes range from
−0.65% to +0.60%; allocation calls from +0.00% to +0.42%. Most requested peaks
change by 0–0.24%. The largest increase is the inspection control:
155,026 → 158,078 bytes (+1.97%); source live rises only 86,301 → 86,365.
I/S peaks rise +0.19%/+0.22%, with no claim of speedup. The additional registry
header, subscriber representation and diagnostic fields have costs even where
no large restriction is used. Diagnostic JSON also adds reporting allocations.
These small measured control costs do not outweigh the direct 23–26% storage
and 86–89% repeated-work improvements. They are not assumed to be zero or
attributed solely to noise. Full phase distributions are in summary.json/raw.

Cleanup allocation bytes match at every maintained point; final process live
bytes also match at every point (including I/S at 1,729). No retained prefix
plans/rows remain after cancellation. The runtime record lacks a source
allocation checkpoint; that metric is unavailable, not zero. Collection can
clear both bounded caches, and payload release remains the existing Store
mechanism. Plan/fragment teardown adds bounded reference drops, rather than
removing the need to account for cleanup. No unbounded history is introduced.

The conclusions apply to the evaluated workloads and growth pattern. They do
not establish universal gains for sparse irregular prefixes, churn beyond the
cache capacities, arbitrary mask populations, or larger-than-4096 buckets
(which retain the general matcher). Runtime was diagnostic only; no ranking or
acceptance decision uses it. No rwLog benchmark was run.

## Verification and reproduction

Candidate diagnostics: **522 tests pass**, counting the final 116-test library,
integration coverage and 50 measure/oracle tests; the two socket binaries pass
with loopback access after their original sandbox failures. Ordinary mode:
**228 selected library/semantic/lifecycle tests pass**. Clippy on production
library/measure with diagnostics, formatting and diff checks pass. There were
no test timeouts. Baseline tests retain their original two environment failures;
the matched growth probe passes on both implementations.

New coverage checks threshold splits, ordered traversal, allocation-generation
invalidation, stale/foreign-root rejection, nine frozen cutoffs and duplicate
occurrences, ten interrupted service positions with changed-version readers,
and lagging-reader shared work after cache clear. Existing tests cover equality,
head identity, conditional support, fresh variables, multiplicity, progress,
snapshot/history ownership, cancellation, CLI/notebook execution and replay.
The maintained measure tests challenge independent semantic/resource oracles.
This is concrete coverage, not a formal proof over all programs.

Builds and tests were separate. `evaluate.py` records every exact command, cwd,
exit status and output in logs, and invokes each prebuilt test binary through
`timeout --kill-after=5s 60s`. The diagnostic matrix command was:

```sh
python3 docs/optimization-evidence/rehearse/round015-prefix-witness/evaluate.py perf
```

It uses `examples/perf.py --binary ... --repeat 3 --warmup 0 --out ... --
CASE SIZE 50000000 5 ... --detail`; the exact 14-point matrix is in the script.
The growth command selects
`growing_prefix_versions_preserve_order_cutoffs_and_release --nocapture --test-threads=1`
from each prebuilt diagnostic shared_restrictions binary under the same guard.

To inspect/recheck archived data from this directory:

```sh
tar -xzf raw.tar.gz
python3 summarize.py
timeout --kill-after=5s 60s python3 audit.py
```

The audit passes 42 paired samples. Original sandbox failures and reporting
defects are retained/documented in logs and notes.md, alongside binary hashes.
The acceptance rule is preserved verbatim in acceptance.md. KEEP is recommended
on the demonstrated independent work and storage gains; integration and the
official campaign decision remain with the landing task.
