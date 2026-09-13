# Whole-head baseline discriminator

Using the maintained examples/supervise.py with the existing release CLI, five round-robin observations per case completed in 4.12 seconds total. External per-run limit was 3 seconds, with 0.1 seconds interrupt grace, 1 GiB address-space limit; no native CLI timeout exists. No source edits or engine optimization were needed.

| Case | Size per input relation | Median total ms |
|---|---:|---:|
| Empty disconnected join | 8 | 5.01 |
| Empty disconnected join | 16 | 14.40 |
| Empty disconnected join | 32 | 80.26 |
| Empty disconnected join | 64 | 691.77 |
| One matching t/v pair, n² hit outputs | 8 | 5.90 |
| One matching t/v pair, n² hit outputs | 16 | 23.35 |
| Unchanged 32-occurrence rewrite control | 32 | 3.40 |

All 35 observations matched exact expected ordered relational multisets, distinct query identities and one complete answer. Empty cases retain precisely the 4n input occurrences; hit cases additionally produce every hit(Ai,Cj,D0) once. Control range was 3.00–3.66 ms. Times include launch, parsing, execution, JSON output and cleanup; tiny cases include substantial launch overhead. RSS includes the supervisor launch floor. No claimed engine-only time or formal asymptotic proof.

The larger empty cases show strong avoidable amplification: 2x input from 32 to 64 costs 8.62x total time despite linear residual size and no rule applications. This supports a whole-head feasibility prototype, not yet a claim that maintained views are necessary or superior to a better on-demand plan. Native compilation remains a separate hypothesis.

Raw evidence and exact generator: /tmp/chr-architecture-discriminator/results.json and /tmp/chr-architecture-discriminator.py. The generator is retained beside this note for reproduction; it uses the maintained supervisor rather than a new measurement framework.

Next compare a source-driven whole-head plan and a simple on-demand plan, including setup/output. First prototype may restrict to static propagation queries to test this mechanism; consuming/merging maintenance must be assessed before a general architecture recommendation.
