# Preimplementation hypothesis and boundary

Baseline: 2d6faa69c95068475601c567dfa9b27c308b75e2, assigned branch
codex/opt/central-occurrence-support. Required nightly reports
1.95.0-nightly (18d13b533 2026-02-09). All work stays in this worktree.

The source-backed bounded prototype uses the existing FACT namespace as the
single versioned occurrence support authority. RELATION, PORT (including tuple)
and INCIDENCE leaves carry TRUE membership markers. A nonzero-to-nonzero
liveness update changes FACT only. Zero transitions eagerly remove memberships;
there are no persistent tombstones or deferred-compaction policy. Occurrences
resolve support at their immutable cursor root. Arrangements produce identities
without looking up support in their bucket-only root; each subscriber already
reads FACT from its own traced root. Pruning resolves each secondary leaf's FACT
support against its base root before intersecting active support, keeps/removes
the marker, and uses the actual support to seed identity reachability.

Constructor attachments, identity edges, rank and propagation history have
distinct semantics and remain supported entries. This is occurrence-index
centralization, not elimination of every Condition in the general graph. Atomic
publication, staged FACT ownership and pinned/archived root tracing remain.

Falsifiable prediction: conditional support replacement avoids secondary writes
and persistent path copies without changing occurrence identity, multiplicity,
aliasing, cycles, propagation eligibility, finite answers or optional observation.
Source checks must show markers only in secondary occurrence namespaces and a
single FACT replacement. New FACT lookup work and pruning costs are charged.
Unconditional post/removal still edits all memberships, so the primary fresh
workload may show no gain. That would leave the broader representation idea
unresolved, not refute it. No whole-engine simplification claim follows merely
from moving support reads.

Budget: bounded single prototype and one diagnostic cohort within the existing
cycle hour/120-second execution budget; no new timing campaign or timing claim.
Diagnostics use one baseline and candidate observation per workload initially,
plus a second unchanged/candidate observation for calibration where complete.
No statistical runtime inference, timing helper, runtime ranking or alpha claim.
All repetitions and failures retained. Run exclusive serially with no local
builds/tests running; timing ownership is not requested because runtime is not
an acceptance metric. Report exact semantic work and uncertainty from collector
address-order variation; do not claim statistical noninferiority from these runs.

Frozen workloads: fresh-contract 64/256 and fresh-unmerged 64/256, rows4 depth4
grouped; answers 8; common-wide 16; life-archive-rotate-conditional 2 rows3
work24 cadence4; partial-join 512 rows128. Native budget 50 million dispatches,
5 seconds per source/cleanup phase, external 12 seconds and 1 GiB per sample.
Existing independent oracles unchanged; fixtures/diagnostic extensions applied
symmetrically before producing baseline binary. Compare setup, execution,
validator/inspection/delivery, cleanup allocation calls/bytes, source and
after-cancel graph/occurrence/condition/history/reader counts, collection and
store-release work, support writes/lookups and arrangement retention. Raw phase
records retained, not summed when cumulative. Zero tolerance for unexplained
material adverse architectural work/retention; missing coverage yields
INCONCLUSIVE/BLOCKED. KEEP requires actual consolidation, full correctness and
no material adverse cost across these obligations; fresh integration still owed.

Required validation before measurements: focused graph/prune/matching/identity,
shared-arrangement and inspection/compaction tests; full release targets and doc
tests with/without diagnostics, formatter, diagnostics Clippy/all targets and
maintained Python diagnostics parsing/observation checks. Any environmental or
baseline failures must be distinguished and logged, never silently skipped.
