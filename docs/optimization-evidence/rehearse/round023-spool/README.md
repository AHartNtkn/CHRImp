# Round 023: acknowledged-prefix Spool compaction

KEEP recommendation, based on fc435f75ed7b7489dcc13c64868abbd1ec2a4f0a.
Candidate only; landing checkout and campaign history were not modified.
The user requested completion of this implementation pass without another long
measurement sweep. This report covers a short eight-case comparison (three
samples per side), plus focused follow-up on two cases, not a calibrated broad
regression campaign.

## Mechanism and replacement costs

After acknowledging the previous cached batch, compact only when consumed bytes
are at least 64 KiB and eight times the unread suffix. Flush, copy forward through
a 16 KiB stack buffer, flush, truncate, and reset offsets. Keep both descriptors
and the existing one-batch replay cache. The source scheduler and language engine
are unchanged. A failed partial overwrite rejects subsequent reads/appends.

Copied bytes are bounded by reclaimed bytes / 8 cumulatively, including repeated
copies. Before a read, the physical length is bounded by the larger of nine times
the unread suffix and unread bytes plus 64 KiB (plus a one-byte empty-file floor).
After that read, add the newly cached batch to the unread window in this bound.
The buffer is bounded; the synchronous number of chunks depends on suffix size.
Compaction never runs in replay or scheduler service. Fixed ordinary ownership
adds a poison flag; counters and clocks compile out without diagnostics.

The first implementation truncated empty suffixes to zero. A retained-session
follow-up reproduced 8.3 ms single-tick stalls and 10.4 ms cleanup versus about
2 ms baseline. A syscall trace established repeated zero truncation in that path.
This was consistent with the documented [ext4 replacement heuristic](https://www.kernel.org/doc/html/latest/admin-guide/ext4.html).
Keeping one physical byte avoids that trigger without retaining logical output.
The focused final comparison restored cleanup to 1.934 ms versus 1.965 ms.
This fix is part of the committed candidate, not an accepted regression.

## Measured evidence

Medians; storage is bytes, times milliseconds. Exact event hashes, delivered byte
counts, finite applications and background applications matched across all paired
samples. Every sample passed the independent residual/multiplicity oracle and
replay invariance checks, and had zero files/descriptors/bytes after close/drop.

| Case | Before-close logical before → after | Filesystem allocated before → after | Copied bytes |
|---|---:|---:|---:|
| 8 answers, 1 row, batch 1 control | 1,872 → 1,872 | 8,192 → 8,192 | 0 |
| 128 answers, 16 rows, batch 1 | 302,140 → 33,744 | 307,200 → 40,960 | 33,536 |
| Same, prefilled | 302,140 → 33,744 | 307,200 → 40,960 | 33,536 |
| Prefilled, batch 4096, replay every batch | 302,140 → 9,282 | 307,200 → 16,384 | 9,074 |
| 512 answers, 16 rows, prefilled | 1,208,392 → 15,095 | 1,212,416 → 20,480 | 149,088 |
| 8 answers, 512 rows, 4 retained, batch 4096 | 912,812 → 370,397 | 925,696 → 385,024 | 0 |
| 32 answers, 8 rows, batch 1, replay/background control | 41,070 → 41,070 | 53,248 → 53,248 | 0 |
| 128 answers, 16 rows, batch 4096, replay/background | 302,140 → 9,644 | 307,200 → 16,384 | 12,945 |

The first five cases and small background control are unaffected by the final
empty-suffix fix. The last focused check remeasured retained and background-batch
cases on the final code. Retained source/read/cleanup medians were
61.536/17.326/1.965 → 61.308/17.286/1.934. Background-batch medians were
45.311/8.765/0.585 → 44.503/8.985/0.589: 0.220 ms additional API read time
(2.5%), including 0.072 ms compaction, with no source or material cleanup increase.
Its internal Spool read clock increased 2.535 → 2.705 ms; this includes copying,
extra seeks and truncation, not just the copy loop. The added absolute work is
small against the affected delivery component, with 96.8% logical storage removed.

The large-tail case copied 149,088 bytes over two compactions taking 0.250 ms,
versus 1,193,297 bytes reclaimed. API read/source/cleanup were
175.463/568.940/1.846 → 174.042/564.165/1.603. The final retained case paid
0.133 ms for five zero-copy compactions. No extra source applications, requests,
or cleanup turns were introduced. Runtime destruction stayed about 1–2 microseconds.
No RSS reduction is claimed: final retained native RSS was 12,560 → 12,568 KiB,
background-batch 6,588 → 6,696 KiB, and the large-tail case 8,268 → 8,264 KiB.
Process supervisor RSS and all write/flush/read clocks remain in the raw records.
Storage savings, amortized copy bounds and small measured replacement costs support
KEEP; three-sample timings do not establish statistical equivalence or rare-tail bounds.

## Validation and reproduction

Builds were separate from execution. Final diagnostic all-target tests: 544 passed
across 45 test binaries, including exact compaction/append, acknowledgement,
background replay, error poisoning, and empty-suffix regression tests.
The ordinary all-target suite passed 499 tests before the final one-byte fix;
final ordinary compilation is checked separately. Clippy, formatting and whitespace
checks pass. Socket tests required loopback permission; the sandbox-only attempt
failed at CLI socket creation, then the full permitted run passed. No rwLog run.

```
cargo build --offline --release --features diagnostics --all-targets
timeout --kill-after=5s 60s cargo test --offline --release --features diagnostics --all-targets
cargo build --offline --release --all-targets
timeout --kill-after=5s 60s cargo test --offline --release --all-targets
cargo clippy --offline --release --features diagnostics --all-targets -- -D warnings
cargo check --offline --all-targets
cargo fmt --check
git diff --check
```

`baseline-harness.patch` applies to fc435f7 in a separate checkout. It adds the
same diagnostics, oracle and prefill control but does not call compaction; the
unused candidate method is included to keep comparison scaffolding identical.
Build both measure binaries with `cargo build --offline --release --features diagnostics --example measure`.
From the candidate worktree, run:

```
python3 docs/optimization-evidence/rehearse/round023-spool/check.py /tmp/new-round023 /tmp/chrimp-round023-baseline-harness
```

The script uses the maintained `examples/perf.py` runner and native
`target/release/examples/measure runtime-sessions`, with each invocation guarded
by `timeout --kill-after=5s 60s`. Optional trailing case names select focused checks.
It saves commands, native records, process RSS, allocation counts, all timings,
and paired semantic checks. No new benchmark executor is introduced.

Artifacts on this host:

- `/tmp/chrimp-round023-evidence/`: eight cases, 48 native observations.
- `/tmp/chrimp-round023-confirm/`: pre-fix follow-up reproducing the cleanup stall.
- `/tmp/chrimp-round023-final-check/`: final retained/background-batch comparison.
- `/tmp/chrimp-round023-close.trace`, `/tmp/chrimp-round023-trace-output.log`: targeted trace.
- `/tmp/chrimp-round023-final-tests.log`, `/tmp/chrimp-round023-ordinary-tests.log`.
- `summary.json`: preserved comparison summaries, including the adverse intermediate result.
