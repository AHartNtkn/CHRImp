# Round 022: packed Prepared arenas

Baseline: `3cc86725e56855d43597aabe3c78f55dcc937e0b`.
Implementation worktree: `/tmp/chrimp-opt-round022-prepared-arena`.
The landing checkout was not modified. This is the evaluated implementation;
integration and the campaign's final KEEP/DISCARD decision belong to the caller.

## Implemented representation

Atom argument lists and And/Or child IDs occupy one flat operand arena. Rule
heads occupy one flat atom arena; instructions and rule metadata have exact-sized
boxed backing. Native-width offset/length spans replace each fragment's Vec.
Post atoms remain inline in instructions. Both activation indexes use spans into
one flat `(rule, ordered-head)` record buffer. Source instruction IDs, repeated
head positions, duplicate explicit arms, and rule variable scopes are preserved.
Engines continue sharing the same immutable Prepared through their existing Arc.
No per-fragment Arc, pointer fixup, hash-consing, or runtime conversion is added.

A bounded-depth sizing walk counts nodes, heads and operands before filling
exact-capacity buffers. Parent child slots are reserved and initialized before
recursing; each child ID then fills its reserved slot. Trigger construction
counts, prefixes and fills final storage directly after constructor recognition;
it does not allocate temporary per-relation lists. The temporary merge-head
classification costs one byte per head, and is freed before prepare returns.

The replacement costs are included in preparation allocation/time measurements:
the sizing walk, child-slot initialization and fill, trigger count/fill passes,
merge-head classification, and existing recognition/slot assignment. Sizing
visits each source instruction/head once and reads lengths, not every argument.
Construction remains linear in nodes, operands and activation records, excluding
the unchanged interning and constructor analyses. Exact sizing removes arena
growth and copying. Every runtime span read uses safe slice bounds checks and
offset arithmetic; it performs no allocation, decoding scan, or reference-count
operation. Scheduling and cursor advancement boundaries are unchanged. There
is no claim of fewer source applications or improved asymptotic execution work.

On this 64-bit build Atom shrinks 32→24 bytes, Instruction 40→32 and RulePlan
96→88. Prepared's header grows by 32 bytes (320→352 with diagnostics); every
retention comparison charges this, padding, and actual buffer capacities.
Dropping flat instruction/operand/head buffers removes individual fragment
deallocations; rule/signature names retain their existing ownership.

The inspection API now uses `code.heads(rule)`, `code.args(atom)` and
`code.operands(span)`. `code.triggers()` yields borrowed target slices.
Span/packed-instruction equality compares positions, not reconstructed syntax.
All internal execution, constructor recognition and pending-expression inspection
consumers use the new accessors. No diagnostic syntax conversion is hidden in
ordinary execution.

## Paired evidence

`evidence.tar.gz` contains raw perf.py campaigns, commands, host/build metadata,
all phase/work records, summaries, validation logs, and exploratory counter
profiles. `reproduce.py` selects maintained workloads; `summarize.py` reads their
typed records. There are 34 completed matched workloads × 5 samples × 2 revisions
in each of diagnostic and ordinary builds: 680 completed observations. Every
selected pair has identical recorded semantic endpoints, including per-rule
application vectors and answer multiplicity. Native workload oracles validate
ordered ports, identities, residuals, prefixes, retained views and release.

| Workload | Prepare allocations before → after | Retained prepare heap before → after |
|---|---:|---:|
| Tiny, 0 inactive rules | 44 → 43 | 802 → 642 B |
| Tiny, 256 inactive rules | 5,697 → 4,921 | 128,505 → 99,785 B |
| Shape, 16 rules | 3,001 → 2,737 | 36,493 → 31,453 B |
| Shape, 64 rules | 11,839 → 10,805 | 144,157 → 124,141 B |
| Shape, 256 rules | 47,173 → 43,065 | 574,969 → 495,049 B |
| Wide body, 64 rules × 256 posts | 100,035 → 99,765 | 1,208,093 → 680,685 B |
| Deep body, 64 rules × 120 groups | 24,513 → 16,629 | 352,029 → 332,525 B |
| Wide heads, 16 rules × 64 heads × 8 ports | 22,101 → 20,977 | 347,661 → 338,525 B |
| rejected3, 128 | 3,545 → 3,146 | 40,236 → 35,820 B |
| repeated-alias, 128 | 1,750 → 1,609 | 34,475 → 26,115 B |
| 128 empty answers | 273 → 267 | 9,216 → 5,152 B |
| behavior-I notebook | 2,511 → 2,197 | 38,940 → 32,612 B |
| type-I notebook | 1,527 → 1,318 | 23,120 → 18,504 B |

These counters are exact and invariant across the five samples. Retained heap is
prepare allocations minus prepare frees, excluding the outer Prepared Arc;
add 336 B baseline / 368 B candidate per prepared object to include it. The
separate representation gauge includes the full Prepared header. For shape-64
it is 112,296→92,312 B; preparation allocation traffic is
1,052,926→967,759 B. Independent preparation repeats all these costs per use.

Execution allocation medians are identical in 30/34 cases. The other four have
overlapping unchanged-work ranges: wide answers 1,213,820–1,217,148 versus
1,215,484–1,217,148 B; conditional archive 13,877,940–14,132,220 versus
13,830,792–14,017,232 B; behavior-I 13,872,056–13,878,064 versus
13,873,960–13,876,296 B; type-I 840,636,688–840,682,016 versus
840,612,824–840,694,896 B. Notebook iteration differences are accounted for by
collection dispatch differences, with identical source work. All cleanup
allocation medians match. Delivery, validation, inspection and destruction
phase counts/ranges remain in the summary rather than being folded into a
whole-process allocation gate. There is no new allocation in span accessors.

All final Prepared drops allocate zero bytes and free exactly the retained
preparation heap plus its outer Arc allocation. All corresponding final live
requested-byte values are identical. Engine ownership and retained-reader
release oracles pass, including cancellation before posting, pending syntax
promotion and projection after cancel, fixed/rotating archives, and 2/8 partly
unread structural inspectors. The fair-loop case delivers its finite answer
beside four continuing siblings.

Ordinary clocks are diagnostic, not the acceptance metric. Shape-64 preparation
medians are 0.4350→0.4296 ms and Prepared drop 0.0242→0.0133 ms; the wide-body
preparation medians are 2.8303→2.8767 ms despite its 527,408-byte retained saving.
Small source-time increases also occur (fair-loop and fanout-256); these five-run
wall-time observations are not a calibrated architectural regression verdict.
Source work/allocation equality and the explicit constant-cost accessor change
bound the claim to storage/allocation improvement. Exploratory hardware profiles
are retained, but cross-PMU migration/scaling prevents a clean instruction-count
comparison; their counts are not combined or used to claim a CPU gain. The user
requested finishing this pass without additional long-running work.

The initial moving-loop inspection fixture (`life-inspections 2 --rows 16
--work 64`) did not reach admission within 3 s on either revision. Its incomplete
observations are retained, not compared as completed work. The selected sweep
uses the maintained structural variant with fixed cyclic payloads, preserving
independent inspector count, continuing work and exact projection obligations;
all selected inspection observations complete. This resolves the required
inspection comparison without mistaking an unfinished workload for failure.

## Validation and reproduction

- `cargo build --offline --release --features diagnostics --example measure`
- `cargo test --offline --features diagnostics --all-targets --no-run`
- Each of 45 prebuilt Rust test executables invoked separately with
  `timeout --kill-after=5s 60s`: 540 tests pass. The final 50-test measure binary
  was rerun after the last harness changes and passed. Two loopback-dependent
  CLI/notebook tests first failed under socket denial; both full binaries passed
  with loopback access (4 CLI and 21 notebook tests). The generic executable scan
  also invoked the CLI without arguments once; its usage exit 2 is not a test.
- `timeout --kill-after=5s 60s cargo test --offline --doc`: 3 pass.
- `timeout --kill-after=5s 60s node --test web/*.test.mjs`: 10 pass.
- Guarded Python unittest discovery: `measure_observation_test.py` (3 pass),
  `perf_test.py` (see log, pass).
- `cargo clippy --offline --features diagnostics --all-targets -- -D warnings`,
  `cargo fmt --check`, `git diff --check`: pass.

The new packed-arena test independently reconstructs every source body/head,
checks distinct postorder instruction positions and gap-free single ownership
of every operand slot, cloned-plan lifetime, repeated-head trigger order and
merge/indexed boundaries at widths 0/1/9/65 and nesting through 120 containers.
Existing tests cover source semantics, sharing, fair progress, normalization,
snapshots, history, autonomous notebook execution and complete reclamation.

For reproduction, apply `baseline-instrumentation.patch` to an isolated checkout
of the baseline. It adds identical allocation phase scopes, final owner checks
and the equivalent old-representation byte gauge. Build each measure binary as
above; build ordinary binaries separately using
`cargo build --offline --release --target-dir target/ordinary --example measure`.
Then run `reproduce.py BASELINE CANDIDATE NEW_OUT` and the same command with
`--ordinary`; `summarize.py OUT` records full metric ranges and semantic endpoints.
Each runner invocation contains its own required 60-second timeout. No rwLog
benchmarks were run.
