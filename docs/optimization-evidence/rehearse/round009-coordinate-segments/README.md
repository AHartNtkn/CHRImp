# Rehearse Round 009: coordinate segments and exact prefix transport

**Recommendation: KEEP the evaluated bounded implementation.**

Accepted baseline: `8e025bc82ea4ed229b53a62804ffa9bb6183e584`.
Implementation: `6970a96f62cdb22a142a89a14fe79a24c8145206` on
`codex/opt/round009-coordinate-segments`, in
`/tmp/chrimp-opt-round009-coordinate-segments`. The campaign landing branch
and accepted revision have not been changed. The supplied [acceptance
contract](acceptance.md) is preserved verbatim and was checked against
`.rehearse/round-009/acceptance.md` with `cmp`.

## Actual mechanism

The coordinate log now groups up to eight append-only, immutable sparse maps
in a segment. A single publication needs no separate run allocation. A second
publication promotes the segment to a bounded run; retirement clears individual
slots without renumbering any remaining epoch. Every original epoch lease and
source map remains independently identifiable. This packs the outer directory;
it does **not** eliminate the original maps or their assignments.

There are two bounded, exact reuse paths:

1. One recent immutable composition covers an exact interval of at most eight
   singleton constant deltas. The first assignment to a repeated key wins:
   a constant introduced by an earlier substitution cannot change under later
   substitutions. Functional images and nonsingleton maps use the existing
   per-epoch transform. Eligibility and construction each inspect at most eight
   entries. Replacement or retirement releases at most eight cached entries.
2. One recent completed transport prefix records exact source and destination
   epochs, input condition identity, and output condition identity. A reader can
   reuse it only at the same source/input and only if its frozen target includes
   the entire recorded prefix. It then continues from that exact destination.
   This also shares work between readers of a single epoch, including general
   functional maps, without composing those maps algebraically.

Source leases pin all maps required by a running transport. Cache records do
not extend epoch ownership: retiring the first map in their interval invalidates
them. Composition images are immortal Boolean constants. The completed prefix's
input and output are explicitly traced Boolean roots until invalidation.
In-flight transforms retain their own images and use existing incremental
discard. No application, alternative, occurrence, variable or answer is cached.

The initial segmented-only version produced no I/S transport reduction because
their epochs generally retire before another joins the segment. Exact completed
prefix reuse supplied the measured notebook gain. A per-segment composition
cache was also implemented and then bounded to one recent composition across
the whole log, preventing derived-map retention from growing with old-reader
age. These were implementation refinements within this candidate, not separate
Rehearse outcomes. General functional composition and replacing each source
map with an inline assignment representation remain outside this slice.

## Demonstrated work reduction

Five samples per side used the maintained `measure`/`perf.py` path. Behavior I
and S have identical applications, validated answer multiplicity, output scalar
counts, publications, assignments, and completed epoch crossings on both sides.

| First validated answer | Applications / scalars | Publications / assignments | Search epoch crossings | Transform starts, before → after | Boolean transform work, before → after |
|---|---:|---:|---:|---:|---:|
| Behavior I | 126 / 73 | 5 / 10 | 37 | 37 → 33 | 577 → 517 (**−10.40%**) |
| Behavior S | 134 / 125 | 3 / 3 | 15 | 15 → 13 | 325 → 293 (**−9.85%**) |

The original engine did not separately expose transform starts; for this
completed per-epoch baseline they equal its completed epoch crossings. The
candidate records starts directly. I has four exact prefix hits and S has two.
Neither builds a multi-epoch composition. The gains remove repeated Boolean
traversal, rather than treating a change in tick meaning as improvement.

The maintained comparator reports a decrease in Boolean transform work for
both cases. It also reports increases in engine allocated bytes and requested
peak bytes; see [I comparison](behavior-i-comparison.json) and
[S comparison](behavior-s-comparison.json). These are descriptive/statistical
change classifications, not the project's acceptance decision.

The diagnostic coordinate scaling test independently executes the accepted
per-epoch transforms and compares three complete readers with the segmented
path. All readers have source cutoff zero, the same final target, the same
immutable maps, and independently checked unchanged Boolean results. The three
inputs alternate polarity so composition reuse is tested separately from the
one-result cache. Raw results are in [scaling.log](scaling.log).

| Publications = assignments | Equal crossings | Transform starts, reference → candidate | Boolean work, reference → candidate | Candidate composition entries processed |
|---|---:|---:|---:|---:|
| 8 | 24 | 24 → 3 | 216 → 27 | 8 |
| 64 | 192 | 192 → 24 | 1,728 → 216 | 192 |
| 512 | 1,536 | 1,536 → 192 | 13,824 → 1,728 | 1,536 |

Transform starts and traversal fall **87.5%** at each size. Construction probes,
entry insertion, cache replacement, and release are additional work, recorded
separately rather than summed with Boolean operations as interchangeable units.
At eight publications the two later readers reuse the composition. At larger
sizes the single cache is replaced while crossing segments, so compositions
are rebuilt for each reader. This is a measured constant-factor improvement,
not a claim of better asymptotic complexity or an end-to-end notebook speedup.

## Storage, preparation and displaced costs

The original source maps and epoch leases remain. The public memory gauge
counts them as before and adds run/cache records. An early draft substituted
segment counts for retained-map counts; that would have hidden live ownership
records. It was corrected before the final measurements. No general storage
reduction is claimed.

The scaling probe retains 36 / 211 / 1,611 coordinate records at 8 / 64 / 512
publications, compared with the accepted log's analytic count 25 / 193 / 1,537.
Before transport the candidate counts are 26 / 201 / 1,601. These are object
counts, not equal-sized bytes. The extra retained composition is bounded at one
map with eight assignments, plus one completed prefix; run metadata grows one
record per eight epochs. All original maps drain, and all three probes finish
at the empty engine's single current epoch. The 16 / 128 / 1,024 release calls
do not imply free cache destruction: an invalidation may release up to eight
extra constant entries in that call.

Measured end-to-end allocation medians include preparation, execution,
validation, inspection, cancellation, release and final object destruction:

| Workload | Engine allocation calls | Engine allocated bytes | Process requested-byte peak | Cleanup allocated bytes |
|---|---:|---:|---:|---:|
| Behavior I | 41,678 → 41,713 | 14,046,588 → 14,065,756 (+0.136%) | 1,971,530 → 1,975,384 (+0.195%) | 13,736 → 13,736 |
| Behavior S | 40,033 → 40,087 | 13,444,660 → 13,466,628 (+0.163%) | 1,723,871 → 1,726,639 (+0.161%) | 20,232 → 20,232 |
| Pending cancellation 64 | 443 → 443 | 266,216 → 266,216 | 350,218 → 354,832 (+1.317%) | 27,008 → 27,008 |
| Pending snapshots 64 | 443 → 443 | 266,216 → 266,216 | 371,184 → 375,798 (+1.243%) | 13,640 → 13,640 |

I/S setup allocation is unchanged at 115,047 / 129,404 bytes. Their cleanup
ticks rise by one (3,468→3,469 and 2,259→2,260); prefix-root tracing adds a
bounded phase even with an empty cache. Sampled Boolean peaks also rise:
I 659→669, S 562→580. Cached roots and changes to collection traversal/order
are real retention costs. These counters are not exact byte peaks.

Across the 24 points with matching recorded endpoints, engine allocated-byte
median changes range from −0.462% to +0.163%, and process requested peaks from
−12.267% to +1.317%. The large decrease is conditional archive rotation; it has
zero prefix hits, and this experiment does not establish a transport mechanism
for that decrease. It is retained as observed evidence, not used as the central
gain claim. New diagnostics/report serialization and the larger engine object
are included in process-wide allocations. Per-category allocation/free traffic
describes executing code, not the final owner of each object. No runtime ratio
determines this recommendation.

## Semantic progress and lifecycle coverage

The final matrix has 25 paired points, five samples per side, followed by five
additional serial lambda samples: **254 completed samples and one censored
sample**. All raw outcomes, including censoring, are retained. The
[endpoint audit](endpoint-audit.json) compares every completed sample's source
applications, answers/scalars, relevant phase data, coordinate publications and
assignments. It finds matching signatures on 24 points; lambda is explicitly
different. Exact graph/identity/multiplicity validation remains inside the
maintained workload runner and is not replaced by this signature audit.

Coverage includes behavior I/S, graph-bits, duplicate heads, successful partial
joins, native rewrite, 256 empty answers, fair growth, six fresh-contract size
and order points, wide rewrites, history choices, pending cancellation/snapshots,
held output, fixed/rotating conditional archives, inspectors, and runtime replay.
The lifecycle points retain 64 ordered-port payload rows and continue 64 source
applications. In particular:

- Fixed and rotating archives admit at application 4, hold through 68, and
  validate four retained answers with zero additional projection source work.
  Rotation performs 16 replacements, overlaps five views, checks 36 choice
  pins, and releases at application 69 on both sides.
- Inspectors admit at 130, hold through 194, and validate four answers without
  advancing source applications. The snapshot handles are already released;
  inspectors retain their own dependencies.
- Held output admits at 8, continues to 72 while partly unread, and finishes
  its exact answer at 80 on both sides.
- History-choice reaches 512 applications. Runtime completes eight answers,
  repeats 136 frozen batches, performs identical recorded read/request/scheduler
  counts, and checks session, owner and spool release.

Each case includes cancellation and final release. Final allocation live-byte
checkpoints match the baseline on these controls; workload-specific engine
memory and retained-owner checks pass. Explicit snapshots/history continue to
pin their own original graph/choice state; no new coordinate lease is added to
them. Searches and completion readers keep exact epoch leases. Existing
compaction pin cutoffs, snapshot projection, archive rotation and runtime replay
therefore retain their original ownership protocols.

Lambda 2 completes the same independently validated answer and 83 scalar events,
but candidate completed samples execute 5,394 applications versus baseline
5,339, with 10,863 versus 10,808 search crossings. Publications/assignments remain
8/31. These are different source-work intervals, so their substantial observed
transport-work difference is **not equal-progress architectural evidence**.
One candidate sample hit the inherited 45-second source limit during concurrent
build/evaluation activity, delivering no answer; its cancellation completed.
All other four samples completed. A repeat with no concurrent build work and
the same limits completed five of five, with source times 25.84–26.07 seconds.
The timing context is diagnostic, not proof of the censor's cause or a timing
acceptance gate. The original censored sample is never converted into a success.

## Correctness and verification

The initial coordinate suite had 13 passing existing tests and two intentional
red tests for the proposed representation/traversal changes. Separate red tests
then established missing single-epoch result reuse and unbounded per-segment
composition retention. Their logs are preserved. The final tests include:

- 3,360 truth-table checks over every source/target pair through 19 publications,
  repeated assignment keys, warm/cold reuse, and segment boundaries;
- functional-image fallback, targets frozen before later publication, and
  interior-reader retirement without applying earlier assignments;
- collection at transport/discard suspensions, cache-only Boolean DAG roots,
  48 cancellation suspension positions, and complete original-map release;
- exact diagnostics calibration and the 8/64/512-publication work probe.

Both final all-target suites passed after the localhost-only tests were rerun
with loopback access: **44 binaries per build, 483 native tests and 513 diagnostic
tests**. Three observation-equivalence tests and three compile-fail doctests
passed. Production Clippy (`--lib --bins --example measure -D warnings`),
`cargo fmt --check`, and `git diff --check` passed. Builds were separate from
test execution; every test invocation used `timeout --kill-after=5s 60s`.
No test hit that limit. Sandbox CLI/notebook failures and successful loopback
reruns remain visible in the logs. Native and diagnostic final executables use
separate build directories to prevent feature-mode interference.

A manual review checked source/target validity, constant composition order,
bounded work between yields, cache trace roots, retirement, and cancellation.
It tightened the frozen-target regression to warm a newer multi-epoch composed
prefix before resuming an older reader. Temporarily removing the target-bound
guard made that test fail; restoring the guard passed both library suites.
The mutation and restored-suite logs are included. This test refinement is
included with the evidence commit; production behavior is the implementation
revision above.
No reviewer subagent tool was available; this report does not claim independent
agent review. These checks support the tested semantics, not a formal proof for
every possible program.

## Acceptance and reproducibility

KEEP follows the verbatim independent-dimensions rule: removing approximately
10% of coordinate Boolean work on two real validated notebook workloads and
87.5% of traversal/starts in the exact segmented probe is a material work gain.
The measured allocation, root-retention and cleanup increases are explicit and
small in the full workload controls; the composition cache's retained images
are bounded independently of log length. No significant regression in required
progress, sharing, answer multiplicity or reclamation was demonstrated. The
extra implementation complexity supplies these exact reuse paths and their
ownership checks; the narrower result and lack of a general storage gain are
not grounds to discard the demonstrated work improvement.

The recommendation does not rest on aggregate ticks, total allocation as a
summary gate, the unexplained archive peak decrease, or lambda's unequal source
progress. Other independent workload dimensions with no reuse remain controls.
Five samples and uncalibrated host scheduling do not establish universal latency
equivalence or rare-event behavior. No rwLog benchmark was run and no candidate
or campaign time budget was introduced.

`run.py` reuses completed exact-baseline cohorts when available and otherwise
uses the existing Round 007/008 maintained configuration: five samples, no
warmup, 500,000,000 source/cleanup ticks, 45-second workload phase limits,
55-second external supervision, and the existing 600-second cohort limit.
It retains censored outcomes and rejects failed or incomplete cohorts. Build
commands, toolchain, revisions and binary hashes are in [environment.json](environment.json).
`test_binaries.py` runs the prebuilt Cargo manifests with the required guard.

[raw-samples.json.gz](raw-samples.json.gz) contains every final cohort's typed
records, stdout/stderr, configuration, supervision results, phase measurements
and raw counters. [measurements.json](measurements.json) selects useful medians,
varying ranges, unavailable observations and representative endpoints; the raw
archive retains all omitted detail. `collect.py` reproduces both artifacts and
the endpoint audit. The serial lambda cohort uses the same `perf.py` arguments
with output directory `candidate-lambda2-serial`; it is supplemental evidence,
not a replacement for the original cohort. Log and comparator artifacts live
beside this report so the evidence does not depend on temporary directories.
