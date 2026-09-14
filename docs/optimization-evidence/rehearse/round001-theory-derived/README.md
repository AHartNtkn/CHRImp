# Rehearse 001: certified structural field updates

Original implementor recommendation: **DISCARD the evaluated candidate**. That
recommendation is superseded by the corrected campaign disposition below.
Corrected campaign disposition: **KEEP the evaluated candidate**. Accepted
baseline before this experiment: `e96efeef47a0de272ec3f24fd9169b270c36581d`.
The evaluated revision is commit `f7dd15e6408e0f912aeeaef8c156faf80727ba68`,
integrated on `codex/shared-relational-engine` by landing commit `01d5d60`.
Historical cycle-7 BLOCKED labels were not used as campaign controls.

## Actual implementation and proof boundary

The baseline already recognizes structural consistency/clash rules and executes
them through conditional constructor attachments. This experiment does **not**
claim to introduce that existing lowering or a replacement theory machine.
Its executable slice extends preparation with a whole-program field-use
certificate and immutable sparse graph-update opcodes. For an already certified
structural relation, retain the identity-key index and every field whose variable
is repeated anywhere in a rule's heads. Omit only payload-only PORT indexes.
Rule-head matching binds such a variable on its only use, so it cannot request
that field as a prebound lookup. Include lowered source heads in this analysis.
Existing constructor recognition rejects unknown/interfering theories; unrelated
relations keep generic updates. Explicit alternatives, propagation, other source
rules and cyclic structures retain the existing general execution semantics.

Posting/consuming a certified occurrence now interprets only required relation,
field, incidence and optional tuple-index writes. FACT payloads, occurrence IDs,
all incidence indexes, conditional supports, source observations and scheduling
remain present. Identity/clash execution itself is unchanged. Public raw field
lookup falls back to a pinned incidence cursor filtered by relation and exact
ordered raw port; duplicate occurrences stay distinct. Missing-index count is a
safe upper bound, and projected-bucket caching declines absent fields.

This removes physical persistent-index writes and retained nodes, not just tick
accounting. It does not remove the larger FACT/RELATION/INCIDENCE posting and
consumption costs, general condition work, or matching outside the certified
slice. Changed job lengths can change permitted committed scheduling and
collection timing; reductions in applications/conditions are not individually
attributed to the omitted writes.

Displaced costs include the prepared opcode buffers, an additional optional
plan reference in each graph, generic update dispatch, a larger occurrence
cursor, and scanning on external raw lookup. Behavior preparation produces
three sparse relations (`app`, `cons`, `constant`), five omitted fields and
14 update opcodes, retaining 1,048 payload bytes (excluding Arc header and
allocator overhead). I/S setup allocation rises by 2,348 bytes. Candidate
diagnostic medians: existing recognition 51,518/58,764 ns; new certificate plus
code generation 8,665/9,073 ns. These are included in prepare, not free work.

## Maintained measurement commands and provenance

All source runs use `examples/perf.py` / `examples/perf_suite.py` and the existing
SK/behavior validators, not a separate benchmark engine. Every notebook command
is exactly `notebook-behavior-{i,s,b,c,w} 1 50000000 3`: stop at the first validated
answer or the three-second native source-work limit, then cancel and release.
The outer runner limits are measurement censoring limits, not candidate budgets.
No rwLog benchmarks were rerun.

From the experiment worktree, baseline binaries were built before production
edits, with these target directories:

```sh
cargo build --offline --release --example measure --target-dir target/rehearse-baseline
cargo build --offline --release --features diagnostics --example measure --target-dir target/rehearse-baseline-diag
cargo build --offline --release --example measure
cargo build --offline --release --features diagnostics --example measure --target-dir target/rehearse-candidate-diag
```

For each of baseline/candidate, run the following once with the diagnostic binary
and once with the ordinary release binary; substitute `BINARY` and `COHORT`:

```sh
python3 examples/perf_suite.py deep --binary BINARY --out evidence/COHORT \
  --repeat 5 --seconds 180 --sample-seconds 10 \
  --only notebook-behavior-i --only notebook-behavior-s \
  --only notebook-behavior-b --only notebook-behavior-c --only notebook-behavior-w
```

Cohorts are `baseline-diag`, `candidate-diag`, `baseline-native`, and
`candidate-native`. The first cohorts predate the supplemental final-destruction
clock described below; whole-process allocation totals already include final
destruction. Rebuilding the final patch includes that small common harness
addition. The main engine code is identical across these candidate cohorts.

Unsupported/adversarial controls, both diagnostic binaries:

```sh
python3 examples/perf_suite.py deep --binary BINARY --out evidence/controls-SIDE \
  --repeat 5 --seconds 120 --sample-seconds 10 \
  --only rewrite --only partial-hit --only duplicates --only prepare-shape
```

This covers rewrite and partial-hit widths 16/32/64/128, duplicate-head widths
2/4/8/16, and preparation heads 1/2/4/8. Input dimensions, source applications and
oracles are matched, not inferred from tick counts.

The new structural payload lifecycle probes and explicit final prepared-owner
check require identical harness extensions on the old engine. For these only,
`target/rehearse-baseline-source` is an archive of `e96efee` with
[baseline-harness.patch](baseline-harness.patch) applied with `patch -p1`
(the patch uses zero context; `git apply` needs `--unidiff-zero`). Every `src/` file was
hash-checked against that revision. Build there with:

```sh
cargo build --offline --release --features diagnostics --example measure \
  --target-dir ../rehearse-baseline-lifecycle-diag
```

For each side, probe in `life-held-output`, `life-archive-fixed`,
`life-inspections`, and width in 16,64:

```sh
python3 examples/perf.py --binary BINARY \
  --out evidence/retention-final-SIDE/PROBE-WIDTH \
  --repeat 5 --warmup 0 --seconds 10 --total-seconds 60 -- \
  PROBE-structural 4 50000000 3 --rows WIDTH --work 64 --cadence 4
```

Archives/inspectors use cyclic ternary payloads and a separate nullary rewrite
driver; the source remains live while held roots are collected and projected.
Held output retains a completed finite answer beside divergent siblings.
All ordered ports and identities are validated, not just counts or the key.

Detailed source-to-End/validator samples use the same `perf.py` runner, repeat 3,
`--warmup 0 --seconds 10 --total-seconds 60`, with
`notebook-behavior-i 1 50000000 3 --detail` and similarly S. See `detailed-*`.
The final common-harness cohort uses the same command with repeat 1 for all five
letters, under `final-release-SIDE/LETTER`; it adds an explicit timed Engine drop
and a weak Prepared-owner assertion after cancellation.

Each sibling JSON contains median/minimum/maximum metrics, native configuration,
sample paths, completion/censoring, exact witnesses, and release checks. Its
`raw_root` points to the retained local runner outputs under
`/tmp/chrimp-opt-theory-derived-execution/evidence`. Raw outputs are ignored, not
deleted. The committed summaries preserve work, allocation phases, storage,
process peak RSS, preparation, source/validator, retention and cleanup metrics.
They are descriptive summaries of existing runner records, not a new benchmark.

## Completed first-answer comparison

Five diagnostic samples per side. Every I answer is
`App(App(S, K), Hole(16))`; every S answer is `S`, checked by the existing exact
structural oracle and behavior evaluator through End. These are first-answer
prefixes, not exhaustive-search claims. Values below are medians, baseline to
candidate. Requested byte totals include preparation, execution, validation,
reporting and cleanup; peaks are requested live bytes, not RSS.

| Metric | I baseline → candidate | S baseline → candidate |
| --- | ---: | ---: |
| Whole-process requested allocation bytes | 16,168,244 → 15,842,541 (-2.01%) | 15,793,572 → 15,157,453 (-4.03%) |
| Peak requested residency bytes | 2,029,024 → 2,009,426 (-0.97%) | 2,009,588 → 1,811,246 (-9.87%) |
| Sampled peak graph nodes | 3,540 → 3,088 (-12.77%) | 3,352 → 3,069 (-8.44%) |
| Sampled peak conditions | 587 → 677 | 623 → 566 |
| Source applications | 128 → 126 | 135 → 134 |
| Source posts | 249 → 244 | 237 → 222 |
| Matcher candidates | 780 → 770 | 744 → 705 |
| Directly avoided field-index writes | 0 → 170 | 0 → 186 |
| Candidate fallback lookups / candidates | 0 / 0 | 0 / 0 |
| Validator requested bytes | 2,920 → 2,920 | 5,056 → 5,056 |
| Cleanup requested bytes | 5,712 → 5,672 | 5,632 → 6,152 |

`perf_compare.py` comparisons of total allocated bytes, requested peak bytes and
sampled peak graph nodes report decreases in both five-sample comparisons
([compare-i.json](compare-i.json), [compare-s.json](compare-s.json)). This is not
a calibrated no-regression claim or sufficient architectural materiality.

Ordinary native source-to-End medians were I 19.565 → 18.979 ms and
S 18.264 → 16.745 ms. These are diagnostic only; some build/measurement activity
overlapped and no runtime result determines acceptance.

The common final-release detailed sample explicitly charges every boundary:

| Phase, milliseconds | I baseline → candidate | S baseline → candidate |
| --- | ---: | ---: |
| Parse | 0.659248 → 0.637993 | 0.712408 → 0.665785 |
| Prepare (including recognition/codegen) | 0.188531 → 0.191553 | 0.188110 → 0.188201 |
| Engine initialization | 0.010275 → 0.009579 | 0.009528 → 0.006868 |
| Source through End, including validator | 22.132273 → 22.710035 | 20.234892 → 20.423961 |
| Validator subset | 0.013870 → 0.019415 | 0.015065 → 0.015284 |
| Cancellation and collection | 0.475965 → 0.433731 | 0.515725 → 0.379337 |
| Final Engine and Prepared release | 0.031544 → 0.031462 | 0.031544 → 0.032083 |

These single-sample phase clocks close an accounting gap; they are not rankings.
The independent three-sample detailed cohorts preserve additional phase data.

## Incomplete three-second source intervals

All B/C/W samples on both engines delivered zero complete answers and were
honestly censored. Cancellation completed. Their source applications differ,
so these bytes/storage cannot establish savings at equivalent semantic progress.
In particular, lower B residency is not evidence that the difficult search became
cheaper overall, and higher C/W allocation is not a completed-answer regression.

| Case | Applications baseline → candidate | Allocated bytes baseline → candidate | Requested peak bytes baseline → candidate | Sampled graph nodes baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| B | 4,224 → 3,825 | 1,111,597,440 → 871,958,189 | 62,349,737 → 47,867,785 | 152,618 → 108,618 |
| C | 4,179 → 4,243 | 1,084,066,980 → 1,126,047,201 | 51,527,825 → 50,005,985 | 122,790 → 124,215 |
| W | 3,747 → 4,289 | 979,453,060 → 1,111,692,497 | 52,358,147 → 53,251,891 | 118,224 → 133,547 |

Candidate avoided PORT writes: B 4,257, C 5,177, W 4,887, with zero fallback
lookups. These counters demonstrate the executed mechanism, not equivalent search
progress or a substantial overall gain.

## Retained observations, unsupported cases and reclamation

All 60 final structural retention samples completed their exact payload and
release oracles. Fixed archives and inspectors resume projection with **zero**
additional source applications, preserving shared execution. The following held
checkpoints have matched source application counts; all values are medians.

| Probe / rows | Held applications, both | Held graph nodes baseline → candidate | Total requested bytes baseline → candidate | Peak requested bytes baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| Fixed archive / 16 | 64 | 374 → 310 | 1,457,554 → 1,437,137 | 190,423 → 185,868 |
| Fixed archive / 64 | 64 | 1,435 → 1,179 | 3,273,640 → 3,144,231 | 506,220 → 503,751 |
| Inspectors / 16 | 321 | 374 → 310 | 3,768,316 → 3,747,899 | 308,818 → 301,269 |
| Inspectors / 64 | 935 | 1,434 → 1,178 | 11,231,026 → 11,092,817 | 637,546 → 611,237 |
| Held output / 64 | 84 | 917 → 661 | 2,808,112 → 2,715,605 | 336,406 → 307,142 |

Held-output width 16 completes correctly but has different admitted source
applications (73 → 75) and slightly greater candidate allocation/residency;
it is not used as a matched-work saving. Rotating structural archives are
exposed by the runner but were not part of this final measurement selection.

The 16 unsupported/control points × five samples × two sides all completed.
Applications and answer oracles agree. Requested allocation overhead is small
but not zero: rewrite-16 713,678 → 719,755 bytes; rewrite-128
4,740,579 → 4,746,656; partial-hit-16 1,158,755 → 1,168,792;
partial-hit-128 6,353,362 → 6,384,927; duplicates-2 390,553 → 397,398;
duplicates-16 7,099,722 → 7,112,679. Preparation-shape adds 89 allocated and
88 peak bytes at each point. Cursor layout and extra diagnostic reporting are
charged. This finite control set found no material regression; it is not a
universal performance guarantee for unsupported programs or arbitrary external
raw-port consumers, which can incur a larger incidence scan.

Cancellation checks find zero unowned occurrences, graph/condition/pending
storage and retained observation handles after owners are released. The
supplemental final-release cohort verifies that Engine destruction releases the
last Prepared owner on all ten runs, including censored B/C/W. Final requested
live bytes are 1,729 on both sides (remaining harness/process state, not an engine
object gauge); the current engine coordinate is released at destruction, after
the pre-drop cancellation gauges. Raw cursor tests also collect old pinned roots
and then verify complete graph-node and occurrence reclamation after release.

## TDD, final review and validation

Before production edits, added and ran the real semantic test
`engine::normalization::structural_field_tests::certified_fields_preserve_conditional_identity_and_raw_observation_without_port_index`.
It exercises two explicit alternatives, conditional identity coalescence, exact
query identities, raw port observation and cancellation. Its first run failed at
the intended unused-index assertion (`left: 2`, `right: 0`); after the minimal
lowering implementation it passed. The direct binary invocation was:

```sh
timeout --kill-after=5s 60s target/release/deps/chr-b5b07d024a3c67e2 \
  --exact engine::normalization::structural_field_tests::certified_fields_preserve_conditional_identity_and_raw_observation_without_port_index --nocapture
```

Additional tests check joined and repeated-identity head consumers, conditional
raw lookup filtering, duplicate occurrence identity, old cursor roots and full
release. The new held-reader probe first failed as an unknown case, then passed
after extending the maintained harness. The final-release oracle first failed
for the absent Prepared-release field and passed after the common harness change.

Investigated failures rather than treating them as candidate deadlines: a new
unit fixture initially inverted `release_tick` completion and timed out; this
test loop was corrected. The first wide archive fixture waited for all actively
rewriting payload rows to coexist and censored on both implementations. Static
structural payload plus a separate driver fixed the fixture while retaining
exact projection, source-progress and reclamation obligations. Those initial
`retention-*` raw attempts are not mixed into `retention-final-*` evidence.
Sandboxed socket tests could not bind (OS error 1); the full final suites ran
with local socket access and passed. No source semantic failure remains in the
tested set.

Builds were completed separately before final test execution:

```sh
cargo build --offline --release --example measure
cargo build --offline --release --features diagnostics --example measure --target-dir target/rehearse-candidate-diag
cargo test --offline --release --all-targets --no-run
cargo test --offline --release --all-targets --features diagnostics --no-run
timeout --kill-after=5s 60s cargo test --offline --release --all-targets
timeout --kill-after=5s 60s cargo test --offline --release --all-targets --features diagnostics
CHR_MEASURE_BINARY=target/rehearse-candidate-diag/release/examples/measure \
  timeout --kill-after=5s 60s python3 -m unittest discover -s tests -p measure_observation_test.py
cargo fmt --check
cargo clippy --offline --all-features --lib --bins --examples \
  --test constructor_lowering --test normalization_observation -- -D warnings
git diff --check
```

Results: 456 release tests and 482 diagnostic tests, each suite across 43 test
binaries; three Python observation tests; builds, formatting, relevant Clippy
and whitespace checks pass. All actual test invocations were guarded with the
required 60-second timeout. Broad all-target Clippy additionally encounters the
inherited `tests/condition.rs:33` Boolean truth-table `overly_complex_bool_expr`
lint; that unrelated test was left intact. Relevant Clippy is clean.
Committed verification logs accompany this report.

Final review checked sparse/generic opcode ordering, tuple hashes, unchanged
source-consistency execution, whole-program head-variable use, raw observation
fallback, pinned roots, cancellation and evidence phase boundaries. No reviewer
subagent facility was available; this is implementor review, not an independent
review claim. Full suites cover identity-only heads, ordered propagation tuples,
fresh body variables, occurrence multiplicity, explicit-choice multiplicity,
finite progress beside divergence, sharing, source normalization observation,
snapshots, history and reclamation. Coverage is evidence, not a formal proof of
all possible programs.

## Decision and materially different follow-up

KEEP under the corrected Rehearse/AGENTS acceptance rule. The executable
mechanism avoids measured index writes and held nodes, with all tested semantic
and release obligations intact. It reduces sampled peak graph nodes by 12.77%
on I and 8.44% on S, lowers S peak requested residency by 9.87%, and avoids
170/186 structural index writes, without a material semantic, control, or
reclamation regression. The completed I/S comparisons reach the same validated
first-answer endpoints; B/C/W remain honestly censored and are not used as
equivalent-progress savings. The candidate is a scoped representation/storage
optimization, not a claim of broad semantic-work reduction. The original
implementor recommendation overweighted whole-process allocation and is
superseded by this corrected disposition.

A distinct future proposal could represent certified structural occurrences in
versioned identity/field storage and reconstruct source Post/Application and
snapshot observations from those versions, thereby eliminating ordinary
FACT/RELATION/INCIDENCE posting and consumption rather than merely unused PORT
indexes. It must retain duplicate occurrence identities, conditional supports,
freshness and shared branch work, conservatively fall back on escaping consumers,
and charge observation reconstruction and pinned-version reclamation. It should
compete as a new Rehearse proposal; it is not claimed implemented here.
