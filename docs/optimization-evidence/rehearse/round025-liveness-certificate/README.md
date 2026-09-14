# Round 025: weak graph liveness certificates

Recommendation: **KEEP** the evaluated implementation, based on accepted
`0a4ff81aadfb52d9c0c0b6e7563a12fe303916f0`. No landing-checkout files were changed.
The implementation and this evidence are preserved together in the experiment
commit; obtain its full revision with `git log -1 --format=%H -- src/graph/prune.rs`.
The experiment worktree is `/tmp/chrimp-opt-round025-liveness-certificate`.

The main result is less semantic pruning, with the same source applications,
answers, retained payloads, and physical collection obligations. Runtime and
total allocation were not acceptance gates. The implementation deliberately uses
full pruning for changed graphs with identity dependencies; it does not claim a
general incremental transitive-closure algorithm for those graphs.

## Mechanism and correctness argument

Graph retains one non-owning certificate: an exact Store root allocation witness,
active Condition identity, exact arena owner, Boolean representation epoch, and either exact ordered
external seeds or a proof that all three identity namespaces are empty. Conditions
are arena-owned, never-reused identifiers. The witness uses the existing WeakRoot
generation/ownership mechanism; no certificate owns graph payloads, descendants,
conditions, occurrence rows, or snapshots.

For an unchanged graph with identity records, reuse requires the same active
support, ordering epoch, and every ordered `(variable, support)` seed. Losing a
body seed can reclaim an identity path even when the graph root has not changed.
At most 64 seeds are copied; larger frontiers use the existing complete traversal.
Every supplied seed is still validated, including on a hit. Public owner, stale
root, and frozen-collector checks execute before certificate use.

For a graph without PARENT, CHILD, or RANK records, there are no upward identity
dependencies to mark. The namespace range probe certifies an empty transitive
frontier through bounded trie paths. Conditional pruning still intersects all
affected occurrence/attachment supports with active support, but skips the
variable worklist, marked map, parent scans, and second identity filter. The
existing unconditional no-op path remains available.

Scalar and batched graph mutations carry the certificate to their exact result
root only while the empty-identity proof remains valid. Removed support and
support already equal to the certificate's active region are immediately safe.
Other occurrence/attachment writes enter an exact key journal. Any identity
write, unrelated root, changed active support, changed ordering epoch, or journal
overflow discards the proof and falls back. The old weak witness is dropped
before mutation, so it does not itself force a copy-on-write root allocation.

The journal encodes all four full-width key words as unsigned varints. Its 256-byte
cap admits at most 64 key events, including duplicates. Geometric backing is
explicitly capped at 256 bytes. At pruning, decoding and sorting produce at most
64 keys; the filter returns unaffected immutable subtrees without visiting their
leaves. Page handling preserves its existing scalar yields. Each prune tick still
allocates at most one index node. No identity-dependent closure is approximated.

Completed pruning releases the input root, worklists, Boolean jobs, and journal
scratch. Graph collection invalidates witnesses whose roots lost authority,
even if an untraced caller still holds a strong handle. Engine cancellation
clears the certificate when releasing source roots. Physical graph/history/
pending/arena collectors continue to run on certificate hits. Requested snapshots
and inspections keep their existing ownership and cutoff rules.

## Matched engine evaluation

The maintained `measure` runner completed **252 samples at 42 points**, three per
side, with alternating side order. All returned zero and passed their independent
oracles. Every paired source had identical per-rule applications, posts, merges,
choice births, failure counts, and lifecycle oracle phase data. PREFIX_COMPLETE
remains a prefix, not exhaustion.

Primary rotating conditional archive: four owners, eight payload rows, 32 held
applications, cadence one. Both sides admit four applications, reach 36 during
the held interval, inspect four snapshots with **zero additional source
applications**, release 36 choice pins while execution continues, and exit at
37 applications. Collection passes remain 72, graph collector service calls
remain 23,439, and median arena collector calls remain 78,843.

| Same archive obligations | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Graph-prune service calls | 31,192 | 17,202 | −44.9% |
| Graph scalar-fallback visits | 10,202 | 5,376 | −47.3% |
| Engine allocation calls | 13,548 | 13,442 | −106 |
| Engine requested allocation bytes | 5,895,096 | 5,863,336 | −31,760 |

The primary case changes active support between passes: it has 36 certificate
probes, 35 empty-frontier passes, and **zero exact-root hits or dirty-filter
visits**. Its gain comes from proving the transitive frontier empty, not a claim
that archive rotation leaves every semantic root unchanged. Mutation proof
maintenance performs 140 checks. Its removed work is the map/worklist and second
filter; physical collection is not skipped or deferred.

Independent payload-width, duration, owner-count, and cadence sweeps:

| Archive point (rows / work / owners / cadence) | Prune before → after | Scalar fallback before → after |
| --- | ---: | ---: |
| 32 / 32 / 4 / 1 | 113,536 → 58,386 | 38,978 → 19,872 |
| 128 / 32 / 4 / 1 | 442,912 → 223,122 | 154,082 → 77,856 |
| 8 / 128 / 4 / 1 | 148,264 → 95,874 | 36,986 → 18,912 |
| 8 / 32 / 1 / 1 | 29,076 → 15,514 | 9,396 → 4,722 |
| 8 / 32 / 8 / 1 | 35,056 → 19,466 | 11,626 → 6,248 |
| 8 / 32 / 4 / 4 | 28,624 → 14,634 | 12,082 → 7,256 |

Fixed conditional archives reduce pruning 28,147 → 13,785; partly unread
conditional inspections reduce it 46,641 → 22,707. Held conditional output drops
2,947 → 1,070. These include retained-reader projection, release, and cancellation.

Controls include rewrite 8/64/256, wide rewrite, alias consumption, fresh
contraction, recursive reachability, correlated bits, duplicate heads, identical
empty answers, pending snapshots/cancellation, notebook I/K/arithmetic, and actual
runtime sessions/replays. The independent validators retain ordered arguments,
identity separation, fresh locals, occurrence and answer multiplicity, and
permitted committed execution. Fair-loop reaches its finite answer at 33/112
applications for 2/8 continuing siblings on both sides; fair-grow at 519/3,841.

## Sparse deltas, replacement costs, and adverse controls

The same public-Graph fixture and maintained allocator run on both builds at
128/1,024/4,096 rows and 0/1/8/32/65 mutations. Each fixture checks two distinct
occurrences per equal tuple, every ordered port, old snapshot support, physical
collection with retained readers, repeated pruning, and final zero rows/index
nodes/condition nodes. The zero-mutation case measures an actual certificate hit
across physical collection. These are mechanism measurements, not extra engine
answers or a new benchmark framework.

One changed occurrence has seven affected index keys:

| Rows | Prune scalar visits before → after | Prune allocation bytes before → after |
| --- | ---: | ---: |
| 128 | 2,558 → 102 | 23,176 → 9,720 |
| 1,024 | 20,478 → 146 | 114,216 → 12,536 |
| 4,096 | 81,918 → 174 | 415,240 → 14,328 |

The replacement filter takes 228/316/372 service calls and seven Boolean leaf
intersections, respectively. It adds at most one range-membership search per
visited subtree, over at most 64 sorted keys, plus bounded encoding/decoding and
sorting. Those searches and node reads are not free, and scalar-visit counts are
not instruction counts. Their work grows with the changed paths, versus the
baseline's full graph walks and variable map. This supported cost argument and
the allocation phases account for the replacement work. Exact repeated-root
pruning performs zero scalar visits and zero requested allocation bytes, versus
2,558/20,478/81,918 visits and 15,024/103,248/402,480 bytes.

Dense/overflow cases still finish full conditional occurrence pruning and keep
the empty-frontier benefit. Eight mutations already overflow the byte journal in
this tuple fixture: this is measured fallback, not an incremental-hit claim.
For example, 1,024 rows/eight changes uses 20,478 → 9,728 scalar visits and
125,872 → 23,968 prune allocation bytes.

Mutation cost was explicitly checked with **12 further runs canceled immediately
after mutation, before any subsequent prune**. One mutation adds 56 bytes of
journal allocation traffic: 8,152 → 8,208 at 128 rows and 12,760 → 12,816 at 4,096
rows. Eight changes add 504 bytes: 18,912 → 19,416 (+2.7%) and 25,184 → 25,688
(+2.0%). At 65 changes the same 504-byte increment is +0.42% / +0.34%.
No graph-node allocation is added by journal maintenance. Cancellation frees the
unused journal, with complete zero-payload reclamation. The initial plain-key
vector added 3,968 bytes for eight changes (+21% at 128 rows); that cost was
repaired before acceptance. Final cold traffic, encoding work, and retained
metadata are assessed here at the mutation component scale, not hidden inside
whole-query totals.

Every graph write also pays a nullable-certificate check. Graph and active prune
handles have a constant larger header; generic identity passes may copy/compare
up to 64 seeds. For example, alias-consume 32 adds 20 scalar visits to 75,848 and
2,792 engine allocation bytes to 7,208,356. Life-alias adds five visits to 13,475
and 800 bytes to 5,406,432. Ordinary rotating archive has unchanged pruning and
scalar visits, and 998,072 → 1,002,104 engine bytes (+0.40%, no extra allocation
calls). The measurements do not show a significant regression in those affected
paths. These costs do not grow with the number of archived alternatives.

Preparation, source execution, validation/delivery, inspection, cancellation,
physical collection, Engine destruction, and Prepared destruction retain separate
allocator phases in the raw records. No work is claimed removed merely because
it moved phases. Final live Engine counts retain its current coordinate epoch;
all execution payload counts are zero after release, and final destruction checks
release the last Prepared owner.

## Storage uncertainty resolved by additional observations

Three-sample peaks on the wider archives were inconclusive: both executables
showed two allocation-peak modes. This is consistent with the documented
address-ordered collection of shared variable groups; no storage gain is claimed.
An initial 12-per-side follow-up remained uneven, so a separate 64-per-side
comparison was completed at both affected widths (256 additional runs).
The table below records that first 64-per-side confirmation.

| Rows | Baseline mean peak | Candidate mean peak | High-mode counts before / after |
| --- | ---: | ---: | ---: |
| 32 | 924,779 B | 927,510 B | 34/64 / 35/64 |
| 128 | 2,096,551 B | 2,096,505 B | 32/64 / 32/64 |

At 32 rows, ranges are 824,106–1,015,890 and 824,106–1,015,842 bytes. At 128 rows,
they are 2,027,455–2,167,903 and 2,027,455–2,168,447. These observations resolve the
apparent 23%/7% median increases in the first three samples as shifts between
already-present modes, rather than a demonstrated growing retained payload.
They do not establish exact storage equivalence or a universal confidence bound.
After the final arena-owner correction, another 64-per-side confirmation at both
widths (256 runs) found mean peaks of 918,549 → 903,668 bytes at 32 rows and
2,100,874 → 2,100,799 bytes at 128 rows. High-mode counts were 32/64 → 27/64 and
34/64 → 34/64. Ranges were 824,106–1,015,834 on both sides at 32 rows, and
2,027,455–2,168,471 → 2,027,455–2,167,927 at 128 rows. This confirms the same
two-mode behavior on the final executable; no storage improvement is claimed.
The supported acceptance claim is substantial semantic-work reduction with small
measured replacement costs; it is not lower peak memory or faster runtime.

## Verification and repaired defects

- Diagnostic release all-target suite: **554 passed**, 46 test binaries.
- Ordinary release all-target suite: **508 passed**, 46 test binaries.
- Diagnostic release Clippy with `-D warnings`, formatting and whitespace checks.
- New tests cover repeated conditional identity roots and seed loss, weak witness
  retirement, empty transitive frontiers, sparse mutation and identity invalidation,
  full-width journal encoding/overflow, actual Boolean reordering, and physical
  collection at every dirty-prune suspension, and foreign-arena rejection on
  an exact-root certificate with universal active support and seeds.
- Existing semantic, lifecycle, pending-body, ownership, cancellation, progress,
  history, snapshot, normalization, and runtime suites pass unchanged.

The initial repeated-root test failed at 39 → 39 calls before certificate reuse.
The empty-frontier test caught unnecessary variable marking. The sparse-delta
test initially caught a full scan (775 calls). The full graph-prune suite caught
an input root retained after completion; completion now releases it. Delta writes
were changed to resumable filtering to preserve the one-allocation-per-tick
contract. A new release-test loop initially inverted `release_tick`'s completion
flag and timed out; the helper was corrected and rerun. The first unprivileged
CLI socket test could not bind loopback; the complete suites then passed with
loopback access. None of these failures was accepted as a remaining defect.
Final ownership review found that universal active support and universal seeds
could hide a foreign arena on an exact-root hit. The regression test failed
before adding an exact arena-owner check to the certificate. Both complete test
suites, the 252-sample engine matrix, all 42 mechanism/cancellation runs, and the
final 256-run storage confirmation were rerun after that correction.

Measurement fixes apply equally to both sides: the Round 024 harness fixes
capture a posted surviving continuation and use a semantic unposted prefix;
baseline diagnostics expose the same zero-initialized certificate fields so
serialization does not manufacture a roughly 3 KB peak difference. An initial
nonconditional eight-row capture was censored on both builds; the maintained
one-row nonconditional control completes, while conditional payload widths are
swept independently. The mechanism parser now handles libtest's prefix on the
first output line and requires all phase records. Resuming a comparison rejects
a changed executable or workload. No rwLog benchmark was run.

## Evidence and exact reproduction

[summary.json](summary.json) contains per-point medians, ranges, all three sample
metrics and semantic/lifecycle checks, plus all three storage follow-ups.
[mechanism.json](mechanism.json) contains every mechanism/cancellation phase,
allocation record, command, environment selector, and exit status.
[raw.json.gz](raw.json.gz) preserves every typed engine record, command and build
manifest, including all storage follow-ups. Local stdout logs remain under
`/tmp/chrimp-round025-evidence/{owner-verified,mechanism-owner,cancel-owner,storage-control,storage-confirm,storage-owner-confirm}`.
[validation.json](validation.json) records the exact test commands and totals;
[validation-logs.json.gz](validation-logs.json.gz) preserves the full final logs
and the focused failures used while developing the change.

The baseline is `/tmp/chrimp-round024-baseline-harness` at `0a4ff81`, with
[baseline-harness.patch](baseline-harness.patch) and the identical
`tests/liveness_certificate.rs` fixture from this commit. The patch changes only
measurement observation/schema and semantic capture predicates. It does not
include the candidate. Earlier Round 024 evidence was read from its preserved
worktree, because its discarded report is absent from the accepted checkout.

Build in each selected worktree, separately from execution:

```sh
cargo build --offline --release --features diagnostics --example measure
cargo test --offline --release --features diagnostics --all-targets --no-run
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --all-targets
cargo test --offline --release --all-targets --no-run
timeout --kill-after=5s 60s cargo test --offline --release --all-targets
cargo clippy --offline --release --features diagnostics --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

From the candidate worktree, after building both diagnostic measure executables
and the diagnostic `liveness_certificate` test on both sides:

```sh
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/compare.py /tmp/chrimp-round025-evidence/owner-verified
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/mechanism.py /tmp/chrimp-round025-evidence/mechanism-owner
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/mechanism.py /tmp/chrimp-round025-evidence/cancel-owner --cancel
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/compare.py /tmp/chrimp-round025-evidence/storage-control --only rotate-r32-w32-o4-c1 --only rotate-r128-w32-o4-c1 --repeat 12
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/compare.py /tmp/chrimp-round025-evidence/storage-confirm --only rotate-r32-w32-o4-c1 --only rotate-r128-w32-o4-c1 --repeat 64
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/compare.py /tmp/chrimp-round025-evidence/storage-owner-confirm --only rotate-r32-w32-o4-c1 --only rotate-r128-w32-o4-c1 --repeat 64
python3 docs/optimization-evidence/rehearse/round025-liveness-certificate/summarize.py /tmp/chrimp-round025-evidence
```

Every native measurement/test invocation in these scripts uses
`timeout --kill-after=5s 60s`. The engine runner additionally has its explicit
50,000,000-work / ten-second phase limit. New output directories are required
after executable changes. Final validation logs are
`/tmp/round025-owner-{diagnostic-tests,ordinary-tests,clippy}.log`; build logs
and repaired-test logs use the same `/tmp/round025-` prefix.
