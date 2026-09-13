# Cycle 1: condition lifetime and representation

Decision: keep both experiments on their branches. Archive ownership provides a correct but modest end-to-end gain without changing retained growth. Global reverse decision order improves one case and causes severe regressions elsewhere. Neither is selected for production integration in this cycle.

## Evidence and experiments

Production baseline: `81951ad`, including automatic structural normalization and conditional dispatch. Integrated duplicator profiling still reaches no answer at its three-second source limit. Of 1,707 CPU samples, collection accounts for 62.4%, including compaction at 32.9%. A rotating-reader profile has 1,399 samples, with 58.3% in condition collection. These are inclusive, overlapping percentages; profiles include cleanup.

Five independent analyses prioritized incremental condition ownership. Parent analysis also identified near-quadratic retained prefix growth: four readers with fixed relational payload retain 2,413 / 9,039 / 34,189 condition nodes at 64 / 128 / 256 turnover steps. This justified comparing an ownership change with a representation change, rather than assuming the sampled collector itself was the sole cause.

| Experiment | Complete reader lifecycle | Ordinary control / adverse result | Decision |
| --- | --- | --- | --- |
| Incremental archive ownership, `861dc54` | Five alternating paired runs: turnover128 median 492 → 422 ms; turnover256 3,088 → 2,434 ms (1.27×). | Identity synthesis 37.3 → 37.8 ms; rewrite512 28.7 → 27.4 ms; graph-bits ring8 6.53 → 6.76 ms. Retained condition counts and source applications match. | Hold: below the intended major end-to-end/scaling improvement; no microtuning campaign. |
| Reverse fresh decision rank, `737090f` | One decisive pilot: turnover256 2,998 → 621 ms (4.82×). Retained prefix witness at128 decisions: 8,256 → 255 nodes. | Empty96-alternative delivery: 10.9 ms → censored at3 seconds, then365 ms cleanup. Identity synthesis 38.2 → 162.3 ms. Retained suffix witness stays quadratic. | Reject global reversal for integration. Actual time/resource regressions establish the decision independently of internal work-budget tests. |

Ownership changes protection only when root/edge ownership changes; working roots are still traced. The worker passed the diagnostics-enabled all-targets suite, documentation checks and release build. Parent independently passed all four new ownership tests and ten inspection tests. Focused source review found no blocker in overlapping roots, interrupted collection, suspended-reader retention or reordering. The ordinary paired runs validated their endpoints and complete reclamation.

Reverse rank retains chronological birth IDs, truth functions, causal birth supports and observed alternative multiplicity in targeted tests. Existing delivery and unrestricted-synthesis work gates exposed adverse cases; the wall-time pilot confirmed material regressions. It is not an eligible production change. Combining it with archive ownership would not address the no-archive wide-delivery regression, so no combined spike was warranted.

## What changed the architectural question

The 96-way disjunction contains conditionally born local decisions. Aggregating their supports can form a multiplexer, whose decision-diagram size is sensitive to selector/data ordering. A reverse-order wide-delivery profile (1,532 samples) places 31.7% under observation-index construction, 50.1% under condition collection, and 11.0% under sifting; these overlap. Only one sample is under completion. This supports investigating expensive support aggregation and its resulting graph growth, rather than attributing the cliff solely to sifting. It does not identify the exact first expanding Boolean operation.

The next useful question is how shared condition construction can handle both cumulative prefixes and conditional-selector correlations without paying for expensive eager aggregates. This is an architectural problem for independent brainstorming, not a prescription to tune the collector, reverse every order, or adopt a specific representation. Hard synthesis remains censored; no improvement to its completion time is established here.

## Reproducible local evidence

- Worktrees: `/tmp/chr-loop1-archive` and `/tmp/chr-loop1-order`; both branches are committed and clean.
- Maintained workloads: `life-archive-rotate-conditional 4 50000000 5 --rows 4 --work 128` or `256`; `answers 96 50000000 3 --rows 0`; `notebook-behavior-i 1 50000000 3`; `rewrite 512 50000000 3`; `graph-bits 8 50000000 3 --shape ring --seed 17`.
- Ordinary binary control and typed results: `/tmp/chr-loop1/baseline-measure`, `/tmp/chr-loop1/ownership-paired/summary.json`, `/tmp/chr-loop1/order-pilot/`. These use the maintained supervisor, result parser and validators, with 8-second external deadlines and 1 GiB address-space limits for paired/pilot runs. Baseline/source deadlines and censoring remain in the records.
- Flamegraphs: `/tmp/chr-loop1-w/flame.svg`, `/tmp/chr-loop1-readers-long/flame.svg`, `/tmp/chr-loop1-order-wide/flame.svg`.
- About57 seconds of serialized measurement, including profiles, used the 120-second cycle allowance. No rwLog reruns. An optional perf source-line report could not resolve symbols; function-level folded profiles supplied the attribution used here.
