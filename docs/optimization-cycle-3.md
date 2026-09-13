# Cycle 3: prevent failing derivations; separate support construction from queries

Neither spike is integrated. Source-derived producer/consumer compilation gives a modest identity-synthesis improvement; the shared-expression backend substantially regresses real execution. Production remains at structural normalization plus conditional dispatch.

Five independent analyses prioritized execution/compiler transformations, representation, ownership, exact reasoning/fairness, and adverse workloads. The selected experiments test different causes: creating derivations already destined to fail, and requiring expanded condition representations. Static factorization and reader-separated ownership remain alternatives, not additional obligations. The parent also considered chain-reduced decision diagrams using [Bryant's primary paper](https://www.cs.cmu.edu/~bryant/pubdir/tacas18.pdf); its representation bounds do not establish a CHR runtime gain, so it was not selected in this cycle.

## Producer/consumer compilation

Experiment `f5c8648`, `/tmp/chr-loop3-producer-consumer`, branch `codex/spike-producer-consumer`. Prepared source rules certify terminal consumers of proposed constructor arms. Exact conditional identity determines rejection support. A marker may retire only through failure or while keeping an incompatible constructor at the same key; the certificate therefore applies to the actual behavior notebook. Mutable markers without that witness do not qualify. Surviving original bodies retain their occurrences and multiplicity; a source decision is needed only where both halves remain viable.

Ordinary and diagnostics all-target suites and documentation tests passed. Added tests exercise source renaming, actual notebook admission, marker consumption, conditional aliases, repeated ports, duplicate alternatives, stepping, snapshots, collection/cancellation and shared execution. Parent inspected the guard/support flow and obtained a separate review of the exclusion invariant.

Five alternating pairs of ordinary release runs measured complete process time, including preparation and release:

| Case | Baseline median | Compiler median |
| --- | ---: | ---: |
| Identity synthesis | 45.69 ms | 32.47 ms |
| Constant synthesis | 9.29 ms | 8.05 ms |
| Wide answers, 96 | 11.39 ms | 12.59 ms |
| Rewrite, 512 | 36.00 ms | 34.25 ms |
| Ring graph, 8 | 8.32 ms | 8.59 ms |

A pilot conditional-reader lifecycle measured 623 → 607 ms. Both baseline and spike produced no duplicator answer within three seconds. These establish a roughly 1.4x gain on one short synthesis target, not a substantial improvement on the harder search or retained-state problems. Hold the tested compiler outside production; no further guard tuning or combination is justified by this cycle's evidence.

## Shared support expressions

Experiment `f108957`, `/tmp/chr-loop3-demand-support`, branch `codex/spike-demand-support`. Immutable interned AND expressions and complemented edges retain composition. One iterative cofactor algorithm classifies results exactly before existing engine consumers use them. Simultaneous substitution, existential projection, tracing and reclamation operate on that representation. Nonterminal handles denote structural identity; explicit equivalence queries provide semantic equality.

The kernel passed exhaustive three-choice truth tables, substitution/projection and GC/trace/cleanup checks. The worker reported 347 passing broad tests and two additional kernel tests. Validation is incomplete: 13 tests exceeded completion/work allowances, one wake test still compares structural handles, and three tests were excluded (two restricted-loopback cases and one interrupted unrestricted-synthesis test). There is no claim that those failures prove semantic invalidity; the runtime evidence independently disqualifies integration.

One serialized release pilot was sufficient:

| Case | Baseline complete process | Expression backend |
| --- | ---: | --- |
| Identity synthesis | 40.64 ms | No answer in 3 s; cleanup also unfinished after 3 s |
| Conditional-reader lifecycle, 128 | 563.89 ms | Unfinished at 5 s |
| Wide answers, 96 | 13.10 ms | Unfinished at 3 s |
| Constant synthesis | 7.57 ms | 12.28 ms, complete |
| Rewrite, 512 | 33.27 ms | 27.88 ms, complete |
| Ring graph, 8 | 5.57 ms | 8.58 ms, complete |

Duplicator still produced no answer within three seconds. The identity run sampled 166,222 live expression nodes and about 93 MiB peak RSS. Linear retained-prefix construction did not translate into improved full-engine execution.

A maintained CPU profile of identity plus cleanup recorded 2,976 samples: exact classification appears in 65.3%, cofactor traversal in 66.0%, and physical condition collection in 0.7%. These are inclusive overlapping figures, not additive partitions. Much classification occurs inside semantic collection maintenance; calling that physical garbage collection would misidentify the cost. Evidence: `/tmp/chr-loop3-support-profile/flame.svg` and `stacks.folded`.

Reject this specific expression/cofactor backend. Cheap construction transfers substantial work to exact queries and their generated expressions. This does not establish that every shared-expression representation or query algorithm has the same limitation. Do not spend another cycle repairing test caps or tuning this rejected prototype merely to complete its implementation.

## Evidence and next decision

Base `d869b3d`; its engine and workloads are unchanged from the saved baseline binary. All timings use maintained supervision, result classification and workload validation. Raw comparisons: `/tmp/chr-loop3/compiler-{pilot,paired}/` and `/tmp/chr-loop3/support-pilot/`. The 78 observations used 29.36 seconds; the single CPU profile used 6.12 seconds. Total measured execution was about 35.48 seconds of the 120-second allowance. Builds/tests were stopped for timing; rwLog was not rerun.

No combined spike is warranted. The next experiment needs a changed premise: substantial reuse of exact reasoning, a representation with better query guarantees, or a sound execution boundary that avoids those questions. Reuse these counterexamples and profiles. None of those directions is automatically a required implementation.
