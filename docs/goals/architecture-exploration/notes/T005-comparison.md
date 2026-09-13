# Direct structural normalization: comparative result

The source-derived representation pays beyond normalization priority alone. It is a promising compiler transformation, not yet a sufficient architecture for difficult synthesis. Production execution remains unchanged.

Prototype: `/tmp/chr-arch-structural`, commit `f2cf1ce`. It recognizes a complete consistency/clash subsystem from rule structure and whole-program interference, then maintains conditional descriptions directly. Ordinary description rows and the remaining evaluator rules still execute. Baseline, generic normalization priority, and direct normalization are selectable in one command.

## Comparable completed executions

Five repetitions per configuration/case, rotated order, serialized through maintained `examples/supervise.py`; offline release build without diagnostics, including setup, output and cleanup. Native limit three seconds, external limit five seconds, address-space limit 1 GiB. The campaign consumed 4.21 seconds; all 60 executions completed with expected answer counts and zero reported engine storage after cancellation. Finite description cases additionally assert exact output occurrence and query-variable partition.

Median total-process milliseconds:

| Case | Baseline | Priority | Direct |
|---|---:|---:|---:|
| Unrestricted identity synthesis, first answer | 129.43 | 73.79 | 49.47 |
| Conditional equality/evaluator interaction, two answers | 10.30 | 9.49 | 4.80 |
| 64 descriptions at one root, complete answer | 18.33 | 18.71 | 6.49 |
| 256 descriptions at one root, complete answer | 424.18 | 65.30 | 18.31 |

For the 256-description case, priority and direct perform the same 255 coalescences and 510 field equalities. Direct improves total time 3.57x over priority; source execution alone is 61.53 versus 14.66 ms. The mechanism avoids generic matching/commitment for the recognized operations while counting conditional identity maintenance, attachments and collection. The comparison does not establish that the retained conditional foundation is optimal.

Identity synthesis improves 2.62x versus baseline and 1.49x versus priority. Its search trajectories differ: baseline/priority/direct commit 383/181/187 applications before first answer. Thus its total gain combines scheduling and cheaper execution; it is not a same-derivation measurement. Finite controlled mutations establish the representation benefit independently.

One instrumented identity run per configuration, for memory rather than timing: peak requested allocation was 3,678,677 / 1,744,880 / 1,828,230 bytes. Direct is about 5% above priority at this different first-answer prefix and about half baseline. Cumulative engine allocated bytes were 84,373,748 / 55,836,244 / 26,906,884. After cleanup, engine memory counters were zero; process live requested allocation remained 344,041 / 271,492 / 271,488 bytes because code/harness allocations remained. These measurements do not imply zero process memory or prove a general memory advantage.

## Adverse synthesis check

Composition (B) and duplicator (W) were each run once with priority and direct under the same three-second source limit. No configuration produced an answer. Earlier baseline measurements already establish five censored observations per query; they were reused, not retimed here. These capped prefixes do not establish completed-runtime speedups.

B completed cleanup in both modes. Its direct prefix had 166,755 graph nodes versus priority's 76,833, while executing more applications (3,673 versus 2,995); neither endpoint alone establishes memory efficiency. W priority completed cleanup. W direct exhausted the probe's one-million-step cancellation allowance with live state. The source search was censored, and cancellation is under targeted investigation; this observation is not accepted as successful lifecycle validation. No repeated timing campaign is justified until that concrete failure is understood.

## Correctness and boundary

The worker reports 356 release tests passing and 12 focused tests also passing with diagnostics. The parent independently reran all 12 focused release tests and the release probe build successfully. Source-derived oracles cover actual evaluator interaction, conditional overlap, distinct roots with identical shapes, cyclic fields and supported identity partitions. Focused tests cover recognition under source renaming/reordering, whole-program interference, fair finite-sibling progress, sharing, collection and suspended normalization. Historical per-rule stepping within fused normalization is outside this experimental command.

Evidence: `/tmp/chr-structural-comparison/results.json`, driver `/tmp/chr-structural-compare.py`; memory/challenging-query observations in `/tmp/chr-structural-challenge`, driver `/tmp/chr-structural-challenge.py`. These are bounded experiment artifacts, not a new maintained perf subsystem.

Decision: advance source-derived direct operations as an architectural component. Do not spend effort tuning its inner loops. Resolve the observed cancellation result, then compare its scope with the other contenders and select only experiments that could change the recommended configuration.
