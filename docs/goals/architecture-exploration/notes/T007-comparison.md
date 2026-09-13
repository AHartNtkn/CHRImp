# Conditional dispatch comparison

Source-derived dispatch adds a useful improvement to direct normalization. It preserves the original selected body and creates ordinary alternatives on unknown support. The combination remains a scoped compiler result: difficult synthesis did not finish within the declared limits.

Prototype: `/tmp/chr-arch-conditional-dispatch`, commit `e5dcb37`, derived from `f2cf1ce`. The worker reports 363 release tests passing, including 19 focused tests with and without diagnostics and suspended dispatch ownership checks. The parent independently reran the 19 focused release tests successfully. No production default changed.

## Runtime and exact outputs

Five observations per case/configuration, alternating order, one ordinary release binary, serialized through the maintained supervisor. The 70-observation campaign took 63.36 seconds, below its 120-second budget. Every observation had a three-second source limit and five-second external limit. All 50 completed source executions passed the expected output checks and completed reclamation. The remaining 20 source searches were censored.

Median total-process milliseconds, including setup, output and bounded cleanup:

| Case | Direct normalization | Plus dispatch |
|---|---:|---:|
| Identity synthesis, first answer | 53.02 | 37.23 |
| Known-description chain, 32 steps | 13.68 | 9.04 |
| Known-description chain, 128 steps | 64.78 | 23.87 |
| Mixed known/unknown descriptions, three answers | 3.58 | 3.88 |
| Four independent fresh choices, sixteen answers | 4.36 | 4.34 |
| Composition synthesis | 5/5 censored | 5/5 censored |
| Duplicator synthesis | 5/5 censored | 5/5 censored |

The finite chain adds these rules to the full behavior program:

```chr
step(H,T) \ run(H,R) <=> (k(R),run(T,R);s(R),run(T,R)).
end(H) \ run(H,R) <=> done(R).
```

Runtime input supplies `step(H0,H1),...,step(H127,H128),end(H128)`, `run(H0,R)` and `(k(R);s(R))`. Exact output checks preserve all query identities, input relations, final control result and the two tag alternatives. At 128 steps, dispatch preserves 256 coalescences but avoids 256 clashes and reduces choice births from 256 to one. Source runtime is 61.47 versus 20.62 ms; total runtime improves 2.71x. No fixed application count prevents compiling away these source operations.

The mixed input starts with `(k(R);mark(R))`: the known region and two generated alternatives yield exactly three answers. The independent control creates a fresh `(k(D);s(D))` at each of four steps; all sixteen answers survive, with distinct fresh roots and binomial multiplicities, including equivalent visible shapes. Differences of a few tenths of a millisecond in these short process measurements do not establish a general overhead regression or gain.

Identity synthesis improves 1.42x in this paired comparison, but its trajectory differs: 187 versus 133 applications before first answer. Dispatch records only two known-arm admissions and 22 generative dispatches. Thus its speedup cannot be attributed entirely to avoiding choice-management work; the controlled chain supplies cleaner mechanism evidence.

## Memory and difficult searches

One separate instrumented observation per configuration/case measures requested allocation, not benchmark latency:

| Case | Peak bytes, direct | Peak bytes, dispatch | Cumulative engine bytes, direct / dispatch |
|---|---:|---:|---:|
| Identity first answer | 1,818,447 | 1,326,769 | 27,032,324 / 19,076,752 |
| Known chain, 128 steps | 736,174 | 850,120 | 40,667,716 / 21,960,412 |

The chain therefore has a measured tradeoff: 2.71x faster total execution with about 15.5% greater peak requested allocation, despite less allocation traffic. Identity uses less peak allocation at its different first-answer prefix. All four instrumented runs reclaimed reported engine storage; remaining process allocations belong to retained code/harness state. Small-case RSS is not used to rank memory because supervisor/pre-exec high-water accounting can dominate it.

B and W produced zero answers in every three-second observation. These are not completed-runtime comparisons. One dispatch B observation, two direct W observations and four dispatch W observations also exceeded the probe's fixed cleanup-step allowance. The reports preserve these outcomes explicitly; they are not silently counted as completed executions. A cleanup-only replay of dispatch W rep0 stopped at the same 16,778,496 source steps and 3,746 applications. All normalization/dispatch counters and graph/occurrence/pending/choice counts matched; condition-node counts varied slightly, so heap reproduction was not exact. Starting in coordinate compaction, it reclaimed all reported engine storage after 2,007,417 cleanup steps without further source applications, choice births or normalization/dispatch progress. No new lifecycle defect was observed. Evidence: `/tmp/chr-dispatch-w-cancel-release.log`. The campaign remains censored at its original cleanup allowance and does not establish bounded hard-search cleanup latency.

At the first dispatch W cap there were 80 known-arm admissions and 569 generative dispatches. This transformation has real opportunities, but does not broadly eliminate genuinely unknown synthesis choices or prove a solution to their global lifetime costs.

Evidence: `/tmp/chr-dispatch-comparison/results.json`, `/tmp/chr-dispatch-compare.py`; allocation evidence `/tmp/chr-dispatch-memory`; source oracles `/tmp/chr-dispatch-reference`. The command is `target/release/examples/constructor_probe direct|dispatch --behavior-i --first`, or the same modes with `PROGRAM --query BODY`; use `--seconds 3` for the declared source cap.
