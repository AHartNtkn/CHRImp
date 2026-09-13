# Cycle 2: decide before constructing conditions

Neither experiment warrants integration. Both pass targeted semantic/lifecycle tests, but neither demonstrates a substantial end-to-end improvement. These results concern two bounded changes to the current condition paths; they do not establish the performance of a different condition representation.

Five independent analyses considered representation, demand, ownership, compilation/fairness and broader workload risks. They favored separating shared condition construction from exact semantic queries. The two spikes tested this principle in completion and coordinate compaction, using cycle 1 profiles to select the paths.

| Experiment | Mechanism | Decision |
| --- | --- | --- |
| Projected overlap, `950d747` | Decide whether independently projected choice arms overlap; construct the positive image only when a reduction is possible. | Hold outside production: mixed, modest differences. |
| Relative completion, `ec1822f` | Subtract frozen pending supports from captured active scope instead of constructing their aggregate union. | Hold outside production: no substantial gain; wide-answer timings adverse. |

## Measurements

Ordinary release binaries, common source baseline `168c297` (engine unchanged from measured `81951ad`), serialized runs through maintained supervision, result classification and workload validation. Five alternating pairs per short case after one pilot pair. Medians below are complete process milliseconds, including release. Separate baseline columns reflect each campaign. Small timing differences are not evidence of an architectural improvement.

| Case | Overlap baseline → spike | Completion baseline → spike |
| --- | ---: | ---: |
| Identity synthesis | 39.50 → 39.08 | 37.26 → 36.07 |
| Dependent choice lifecycle, 32 | 14.71 → 14.34 | 14.84 → 15.43 |
| Delayed correlated choices, 128 | 209.61 → 194.74 | 180.13 → 181.48 |
| Wide answers, 96 | 13.40 → 11.08 | 9.61 → 11.77 |
| Rewrite control, 512 | 27.82 → 30.54 | 27.49 → 27.22 |
| Ring graph, 8 | 5.77 → 5.89 | 6.27 → 5.31 |

Two additional alternating pairs per hard case: conditional-reader lifecycle at 128 turns measured 470.99 → 469.03 ms for overlap and 511.92 → 468.77 ms for completion, with substantial overlap between individual observations. Duplicator synthesis reached zero answers in all eight runs at the three-second source limit. Peak RSS estimates stayed approximately 86–87 MiB. Cleanup completed in every run; variable cleanup duration does not imply a synthesis speedup. Internal application counts differ but do not establish better time to an answer.

All 160 observations were completed or source-censored as declared, using about 35.55 seconds of the 120-second measurement budget. Builds and tests were quiescent during timing. No rwLog reruns. Raw observations: `/tmp/chr-loop2/{overlap,completion}-{pilot,paired,hard}/`.

## Validation and next decision

Both ordinary and diagnostics-enabled targeted suites passed. Overlap covers independent quantification, all retained/quantified cutoff positions in six representation orders, interrupted reordering, collection, incremental discard and the real compaction candidate path, plus existing compaction and condition tests. Completion covers frozen certificates, parent advancement, coordinate transport, collection, cancellation, recapture after an empty certificate, and existing semantics, compaction, inspection and progress tests. The predicate's exactness is checked against independent truth-table evaluation.

No combined spike is justified by these results. Production remains at structural normalization plus conditional dispatch. The next cycle should evaluate a broader representation/execution hypothesis capable of avoiding condition construction itself, including its exact semantic-query, release and fairness costs. Shared Boolean circuits are one candidate, not a selected implementation or mandatory deliverable. The retained-prefix and conditional-selector counterexamples from cycle 1 remain relevant adverse cases. Do not spend another cycle tuning these two paths without a changed premise.
