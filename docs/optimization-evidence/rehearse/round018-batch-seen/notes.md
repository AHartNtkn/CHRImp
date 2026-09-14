# Round 018 execution notes

Accepted baseline: `9f5fd4a3d334b62253304256a88776861cd08878`.
Worktree: `/tmp/chrimp-opt-round018-batch-seen`.
Branch: `codex/opt/round018-batch-seen`.

Scope: only Store batch scratch duplicate elimination, plus shared diagnostics,
semantic/lifecycle oracles and evidence. No rwLog runs or landing changes.

Plan:
1. Build/check accepted baseline; commit common diagnostics and ordered-map probes.
2. Measure baseline and reverse exact-key seen-set implementation on the same probes.
3. Charge scratch allocations and unique/small controls; preserve bounded graph updates.
4. Run maintained wide/update and lifecycle workloads, audit exact semantic and work
   vectors, validate ordinary/diagnostic builds, and commit a KEEP/DISCARD report.

Builds and test execution are separate. Every test process is guarded by
`timeout --kill-after=5s 60s`. Raw commands/output and reports will accompany
the final evidence. No runtime-based acceptance or new campaign candidates.

## Executed revisions and commands

- Accepted source: `9f5fd4a`.
- Common instrumentation/oracle: `8f83bc5`.
- Initial unconditional set: `b3c668c`.
- Final inline/promotion implementation, documentation and audit tools: `3dbe870`.
- Matched baseline probe harness: `fa22ee1`, in
  `/tmp/chrimp-round018-baseline-harness`. It adds the final small-key resource
  test to `8f83bc5`; only the candidate scaling test remains ignored there.
  This evidence-only commit is not part of the landing sequence.

The final binaries were built from the dirty tree subsequently committed as
`3dbe870`. Maintained records accurately retain the earlier commit plus dirty
status at run time; binary/source hashes are in raw `logs/identities.log`.
The matched baseline differs from the candidate only in coalescing and its
diagnostic hooks (plus the intentionally ignored resource test). Both have the
same test inventory, avoiding a 152-byte test-harness startup-peak difference.

All commands run in the candidate worktree unless stated otherwise:

```sh
cargo test --offline --release --features diagnostics --all-targets --no-run
cargo build --offline --release --features diagnostics --example measure
CARGO_TARGET_DIR=/tmp/round018-ordinary-target cargo test --offline --release --all-targets --no-run
python3 docs/optimization-evidence/rehearse/round018-batch-seen/verify.py /tmp/round018-final-diag-build.log /tmp/round018-raw/tests/diagnostics
python3 docs/optimization-evidence/rehearse/round018-batch-seen/verify.py /tmp/round018-final-ordinary-build.log /tmp/round018-raw/tests/ordinary
```

The verification helper executes every emitted prebuilt test binary with
`timeout --kill-after=5s 60s`, one binary at a time. Final totals are 534
diagnostic and 491 ordinary tests. Both configurations initially failed only
the two loopback tests in the restricted sandbox. Their exact final binaries
were rerun with local socket permission:

```sh
timeout --kill-after=5s 60s target/release/deps/cli-346cf51f2471214e --exact notebook_default_origin_is_stable_and_port_override_is_honored --test-threads=1
timeout --kill-after=5s 60s target/release/deps/notebook-900599a3335db509 --exact loopback_http_serves_the_notebook_and_enforces_request_boundaries --test-threads=1
timeout --kill-after=5s 60s /tmp/round018-ordinary-target/release/deps/cli-a1ec6b78568a9305 --exact notebook_default_origin_is_stable_and_port_override_is_honored --test-threads=1
timeout --kill-after=5s 60s /tmp/round018-ordinary-target/release/deps/notebook-16d3c912bbb4c991 --exact loopback_http_serves_the_notebook_and_enforces_request_boundaries --test-threads=1
```

All four retries passed. No test timed out. The baseline Store suite passed 16
tests, and the matched baseline probe suite passed three with its candidate
scaling contract ignored. Running that contract explicitly against the original
instrumented baseline failed as expected; the final candidate passes it.
The full final test sets include accepted page, prefix memo/witness, coordinate,
chronology, partition, semantic, snapshot, cancellation and progress oracles.

```sh
cargo clippy --offline --release --lib --bin chr --example measure --features diagnostics -- -D warnings
CARGO_TARGET_DIR=/tmp/round018-ordinary-target cargo clippy --offline --release --lib --bin chr --example measure -- -D warnings
cargo fmt --check
git diff --check
```

These checks passed. Lint scope is production library/CLI/measure, not a claim
that every historical integration-test lint is clean.

## Measurement reproduction

Build each source revision separately and copy its prebuilt binaries to
`/tmp/round018-raw/bin/{baseline,candidate}-probe` and
`/tmp/round018-raw/bin/{baseline,candidate}-measure`. Baseline measure uses
`8f83bc5`; baseline probe uses `fa22ee1`. Candidate uses `3dbe870`.
For either SIDE:

```sh
python3 docs/optimization-evidence/rehearse/round018-batch-seen/reproduce.py probes /tmp/round018-raw/bin/SIDE-probe /tmp/round018-raw/probes/SIDE --repeat 3
python3 docs/optimization-evidence/rehearse/round018-batch-seen/reproduce.py maintained /tmp/round018-raw/bin/SIDE-measure /tmp/round018-raw/maintained/SIDE --repeat 3
python3 examples/perf_suite.py routine --binary /tmp/round018-raw/bin/SIDE-measure --out /tmp/round018-raw/suite/SIDE --repeat 3 --seconds 120 --only rewrite --only partial-miss --only partial-hit --only duplicates --only fresh-contract --only fairness --only archive --only sessions-history
python3 docs/optimization-evidence/rehearse/round018-batch-seen/audit.py /tmp/round018-raw
```

Use new output directories for maintained runners. The raw records hold every
actual command, native limit, outcome, scalar/answer/application vector, phase
allocation and diagnostic checkpoint. `audit.json` checks exact synthetic
results, page/mutation/cleanup counts, real resource balances and all paired
maintained rule vectors, while retaining collector variations and allocation
statistics. These scripts orchestrate the existing probe/measure/perf runners;
they do not replace their semantic validators or introduce a benchmark engine.

There are 248 synthetic fixtures (744 samples per side), 23 detailed maintained
points (69 per side) and 16 routine-suite points (48 per side), all completed.
The initial unconditional implementation has 456 synthetic samples and three
wide-arity-64 samples. `pre-lazy` observations preserve the intermediate inline
implementation before delaying RandomState construction until promotion.

## Accounting repairs and interpretation

The initial parser expected a diagnostic at column zero; Rust's test harness
prints its test name on the same line. The parser now recognizes the explicit
`batch_probe=` delimiter and requires exactly one record. This was a reporting
failure, not a failed engine test.

Final hash requests include promotion and Hash::hash calls during table growth.
The initial unconditional diagnostics counted insert requests only; do not
compare their hash-request number as if it used the final definition. Actual
allocator traffic remains measured throughout, including all reallocations.

Every Store operation in a probe is phase-scoped; constructing/cloning its
cursor/root performs no allocation. One final candidate sample
(`256/7/dense/insert`, repeat 0) recorded 144 bytes of concurrent unscoped
test-harness allocation after its start. The raw process live delta is retained.
The audit separately requires scoped fixture allocation/free balance after
subtracting the intentionally retained report string; that balance is zero for
every sample, and every Store reports zero remaining nodes. This does not claim
that every process-wide final live delta is exactly zero.

The final baseline/candidate binary path differs by one character. Its one-byte
argument allocation explains the constant whole-process delta in deterministic
controls; interval allocation differences exclude startup. Requested peaks
before validation expose scratch costs; final peaks can be dominated by the
independent semantic oracle and serialized exact-result report. Neither metric
is silently substituted for the other.

Runtime, RSS and dispatch ticks are retained as context, not acceptance gates.
Hash equality counts vary with randomized hashes; three repeats are descriptive,
not a calibrated statistical speed/regression study. No rwLog benchmarks ran.
