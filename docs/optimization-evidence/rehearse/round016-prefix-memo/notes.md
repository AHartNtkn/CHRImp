# Round 016 execution notes

Only the selected bounded Store prefix-root memo was implemented. The landing
worktree was inspected read-only and remains clean at d141a9f. No assigned
Round 016 worktree existed, so the implementor created
`/tmp/chrimp-opt-round016-prefix-memo` from that revision. The matched baseline
is `/tmp/chrimp-round016-baseline-harness`, revision 46030a2.

The baseline consists of accepted code plus request/path counters, identical
measure JSON fields (zero memo diagnostics), and the maintained growth/churn
oracle. Its first shared instrumentation commit is cca15e7, also the candidate's
parent. The candidate's implementation/probes/documentation commit is e7b0335.

Build commands, executed separately from tests:

```
cargo test --offline --release --features diagnostics --no-run --message-format=json
cargo build --offline --release --features diagnostics --example measure
cargo test --offline --release --all-targets --features diagnostics --no-run --message-format=json
cargo test --offline --release --no-run --lib --test store --test shared_restrictions --test lifecycle --test cancel --test progress --test semantics --message-format=json
```

The first two were run in both worktrees; the latter two in the candidate.
Full build JSON/stderr and measure build output are archived under logs.
`evaluate.py tests` records each prebuilt invocation, its cwd, exit and output;
every test uses `timeout --kill-after=5s 60s`. The additional all-target example
and ordinary-mode binaries were selected from compiler artifacts and run through
the same `evaluate.run` function, with the same guard. No test timed out.

Two original candidate socket tests failed because loopback creation was
blocked (`Operation not permitted`); both complete binaries passed after
requesting sandbox escalation for loopback access. Original and successful
outputs are preserved separately. The exact reruns were:

```
timeout --kill-after=5s 60s target/release/deps/cli-346cf51f2471214e
timeout --kill-after=5s 60s target/release/deps/notebook-900599a3335db509
```

An initial attempt to commit baseline harness changes received a read-only Git
metadata error. The same scoped commit succeeded with sandbox escalation.
No landing files were edited, and no automatic approval rejection occurred.

Two early audit assertions were too strong: complete `work` objects differ in
collection dispatch counts, and result `work` contains ticks as well as semantic
obligations. Initial errors were `('archive', 'source', 'work')` and
`('fair', 'work')`. The final audit compares exact per-rule vectors, answer /
application / scalar obligations, normalization, field writes, all restriction
diagnostics and cleanup. It retains full collection work and allocation records
on both sides. For example, baseline archive arena collection work is
24622/24250/24577 across three runs; candidate is 24452/24542/24647. These vary
within unchanged binaries, and are not attributed as a memo gain. A one-off
inspection script also raised StopIteration when requesting a runtime source
checkpoint: runtime does not emit that checkpoint; it is unavailable, not zero.

No semantic or native performance oracle failed. All 84 samples completed;
no censored performance observations or test timeouts occurred. No rwLog runs.
Allocation differences include instrumentation and diagnostic serialization;
reported peaks are requested Rust allocations, not RSS. Only the mechanism
probes are single observations; maintained controls have three samples per side.

Validation commands:

```
python3 docs/optimization-evidence/rehearse/round016-prefix-memo/evaluate.py tests
python3 docs/optimization-evidence/rehearse/round016-prefix-memo/evaluate.py perf
timeout --kill-after=5s 60s python3 docs/optimization-evidence/rehearse/round016-prefix-memo/analyze.py
cargo clippy --offline --release --features diagnostics --lib --example measure -- -D warnings
cargo fmt --check
git diff --check
```

Final results: 525 diagnostic tests pass (474 library/integration plus 51 example
tests), 159 selected ordinary tests pass, and the matched baseline's 32 selected
tests pass. Growth/churn standalone allocation observations repeat their two
tests with a single test thread. The audit passes 42 paired samples. Clippy,
format and whitespace checks pass. Diagnostic stats add no conditional behavior:
ordinary execution runs the same memo.
