# Round 017 execution and reproduction

Candidate: `/tmp/chrimp-opt-round017-adaptive-restrictions`, branch
`codex/opt/round017-adaptive-restrictions`, starting from accepted `33947a6`.
Instrumentation/baseline adapter and initial oracles: `ae05a1d`.
Baseline harness: `/tmp/chrimp-round017-baseline-harness`, `d236ca4`
(the adapter plus isolated-case and cancellation-oracle extensions).
Evaluated implementation: `91c833c`, depending on `ae05a1d`.
The landing checkout was inspected read-only and remains clean at `33947a6`.

## Builds (separate from test execution)

From the candidate root:

```
cargo test --offline --release --features diagnostics --test shared_restrictions --no-run --message-format=json
cargo test --offline --release --features diagnostics --all-targets --no-run --message-format=json
cargo build --offline --release --features diagnostics --example measure
cargo test --offline --release --all-targets --no-run --message-format=json
cargo build --offline --release --features diagnostics --bin chr
```

From the baseline harness:

```
cargo test --offline --release --features diagnostics --test shared_restrictions --test store --no-run --message-format=json
cargo build --offline --release --features diagnostics --example measure
```

All builds completed. Compiler JSON artifacts are archived under `builds/`.
The initial focused build ran against the baseline adapter in the candidate
directory. Capacity-two, capacity-four, and capacity-one focused builds followed
in that order, each changing only `INLINE_PARTITIONS` and each followed by the
three probes. Capacity two was restored for the full builds and measurements.
The final focused rebuild enables the previously ignored resource test in normal
diagnostics test runs; it changes no production behavior.

## Test and measurement commands

`evaluate.py` records exact executable arguments, cwd, exit and stdout/stderr in
`logs/`. Every executable test invocation uses `timeout --kill-after=5s 60s`.
The scripts use compiler JSON paths in `/tmp/round017-SIDE-build.json`; archived
copies can be restored there or adjusted after building in another directory.

```
python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/evaluate.py tests
python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/evaluate.py isolated
python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/evaluate.py perf
python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/evaluate.py probes threshold2
python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/evaluate.py probes threshold4
python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/evaluate.py probes threshold1
```

`perf` uses the maintained `examples/perf.py`, three observations without warmup,
22 points, 50 million source ticks and a five-second native phase bound, with
`--detail`. Native semantic/resource oracles remain enabled. The routine matrix
was also run on both prebuilt binaries:

```
python3 examples/perf_suite.py routine --binary /tmp/chrimp-opt-round017-adaptive-restrictions/target/release/examples/measure --repeat 3 --out docs/optimization-evidence/rehearse/round017-adaptive-restrictions/suite/candidate
python3 examples/perf_suite.py routine --binary /tmp/chrimp-round017-baseline-harness/target/release/examples/measure --repeat 3 --out docs/optimization-evidence/rehearse/round017-adaptive-restrictions/suite/baseline
```

Both routine commands returned 2 solely because their three W synthesis runs
were censored. Each side completed the other 78 samples. These are coverage and
exact-cost observations, not calibrated statistical timing/regression claims.
No rwLog benchmark was run.

Audits:

```
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/analyze.py
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/thresholds.py
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/suite_audit.py
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round017-adaptive-restrictions/verify_logs.py
```

All audits exit 0: 66 detailed pairs, 78 completed routine pairs, 12 isolated
table cases and both growth/churn probes. Test audit: candidate 530 passed,
ordinary 491 passed, baseline 35 passed plus one intentionally ignored
candidate resource contract. Raw failures and corrected reruns are retained.

## Failures, corrections and limits

- The resource test first failed as intended on the baseline: 131 hash requests
  for one key/128 rows, versus zero required. It passes on the candidate and is
  now an ordinary diagnostics test.
- The first paused-reader oracle expected reuse before that reader had actually
  subscribed. Baseline correctly repeated 16 rows after collection cleared the
  cache. The corrected fixture establishes subscription first; both sides pass.
- Initial CLI/notebook HTTP failures were sandbox loopback denials. Reruns with
  loopback permission passed. The first diagnostic CLI retry encountered the
  ordinary binary written by the ordinary build; restoring the diagnostics CLI
  and rerunning its complete test binary passed. No source fix was needed.
- `cargo clippy --offline --all-targets --features diagnostics -- -D warnings`
  fails on unchanged `tests/condition.rs:33` under this installed nightly. The
  baseline reproduces the same `overly_complex_bool_expr` lint. Production-only
  strict Clippy passes. Both complete ordinary/diagnostics target sets pass with
  `-D warnings -A clippy::overly_complex_bool_expr`. The oracle was not changed.
- Initial routine invocations misspelled `--repeat` as `--repeats`; parser
  failures are retained separately from the corrected actual campaigns.
- One baseline-harness patch attempt ran from the wrong cwd, producing an empty
  patch. It changed no code. The correct candidate diff was then applied, and
  committing the harness used permitted access to shared Git metadata.
- The two `chr` test artifacts initially shared a log label. Both were rerun
  with unique executable names; the test audit uses those complete logs.
- Threshold sweeps run cases in one process: later peaks are cumulative. Final
  selected/baseline isolated cases use fresh processes. Process peaks include a
  nine-byte binary-path difference; live/call/byte interval deltas subtract the
  initial checkpoint. Growth prints between checkpoints add reporting traffic;
  full data is retained, and final graph/occurrence/table storage is asserted zero.
- Hash bucket probes, per-word comparison cost, instruction totals, contention
  and statistically calibrated performance regressions were not measured.
  Censored W frontiers reached different work intervals and are not compared as
  equivalent work. No timing outcome determined candidate selection or KEEP.

For an archived checkout, unpack `raw.tar.gz` here to restore `logs/`, `raw/`,
`suite/` and `builds/`, then run the audits. `verify_logs.py` also needs the
archived build JSON restored to the named `/tmp` paths. The archive includes
all canonical evaluated observations, the failed probes above, and build/binary
metadata. Repeated diagnostic sanity invocations share canonical log labels.
