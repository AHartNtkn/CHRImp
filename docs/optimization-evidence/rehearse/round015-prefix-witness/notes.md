# Provenance and exploratory observations

Accepted production baseline: 6bf6e1ddeab3e4cc90d9f0ab665d712624471f48.
Worktree: /tmp/chrimp-opt-round015-prefix-witness.
Baseline harness worktree: /tmp/chrimp-round015-baseline-harness at 0ccbf0b.
Shared probe commits: fcbe3cc and 2304369 (baseline cherry-pick 0ccbf0b).
Implementation: 67bf8cb87ef46eac38f775e106a86320e834d04f.

The task's selected field was `[object Object]`. Read-only helper selection
(`python3 -B tools/rehearse/rehearse.py select .rehearse/round-015`) returned
candidate 0, 3 votes, mean confidence 0.6783333333333333. The implementor did
not rerun judges, record an outcome, or edit campaign control files. Verdict
0-1's explanation discussed release batching as B although its prompt's B
was adaptive partitions. The orchestrator should reconcile that discrepancy;
this report evaluates only the returned candidate 0.

The first local implementation did not retain all shared fragment producers
for a lagging subscriber. Review caught that eviction/collection could make
it repeat work, so the implemented shared prefix plan fixes the issue before
the final comparison. The first 13-test check passed in 0.06 s; at 2048 rows
it reported 2084 projections and zero final nodes. This was exploratory, not
the final ownership implementation or an independent candidate outcome.

All final raw perf observations are in raw.tar.gz. There were 84 completed,
zero censored, zero native oracle failures. The original sandbox socket
failures are retained in logs for both sides. Candidate retries with loopback
access passed 4 CLI and 21 notebook tests. The baseline sandbox failures were
not reclassified as passes. No test exceeded its 60-second guard.

An initial evidence parser missed `start` because Rust's test-name prefix was
on the same stdout line: summarize.py raised KeyError('start'). It now searches
within each line. Initial audit.py assumed lifecycle `work` was a dictionary
and raised AttributeError; these cases use an integer. Its next version
incorrectly required identical matching dispatches and failed the 1024-row
case. The final audit keeps those replacement transitions in audit.json and
checks identical remaining per-rule obligations. The assertion failure is
preserved in logs/audit.log; these were reporting defects, not native failures.

The test runner initially used target names as log names, overwriting one
`chr` test log with the other binary of that name. It now uses executable
names. The final 116-test library run is retained separately in final-lib.log.
The original two invocations both returned zero in the task transcript.

The baseline harness cherry-pick initially failed with a read-only index.lock
error. The same explicit cherry-pick succeeded after allowing that isolated
worktree's Git metadata write. No automatic approval rejection occurred.

Post-measurement additions were the Store unit test and documentation only;
the production implementation measured in the matrix is the committed one.
Final diagnostic measure SHA256:
cbd26e338ac7994aa5433839a465e6a36f16dd48d7164a6a92fe71bf7f934e36.
Baseline diagnostic measure SHA256:
a83cfbe110a651667cd992faf0c90b79b49584aeaf4a965bee74a4d85a8ab6e3.

Build commands (executed separately from tests):
`cargo build --offline --release --features diagnostics --tests --example measure`
in both worktrees; `cargo build --offline --release --tests` in the candidate;
`cargo test --offline --release --features diagnostics --example measure --no-run`.
All completed successfully. The complete guarded execution commands and
native outcomes are in evaluate.py, test logs, and each raw perf record.
