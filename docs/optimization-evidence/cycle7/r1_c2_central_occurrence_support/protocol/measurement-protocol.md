# Measurement Protocol

This is the shared acceptance protocol for workers, standalone measurement, and final integration. Pin this file and the analysis helper by version/hash for each assignment. Never change an experiment's acceptance rules after inspecting its outcomes.

## Declare the experiment

Before confirmatory timing, record:

- Immutable baseline/candidate source and build identities, runtime dependencies/assets, commands, toolchain, flags, and hardware.
- Performance/simplification kind, sensitive primary and representative secondary workloads, shared fixture revision, expected outputs, and units.
- Per-workload metric, minimum worthwhile improvement, maximum tolerable regression, sample count/design, warmup/reset policy, and analysis method.
- Error allocation for final confirmation and any preplanned additional sampling or sequential analysis. Do not collect until significance appears.

Use project/user bounds. Fractions such as 0.02 mean 2%. Do not invent a permissive regression tolerance. If no tolerance is specified, use zero and report inconclusive evidence honestly; ask a narrow question only when accepting a nonzero slowdown is necessary. Minimum improvement may be zero when no practical minimum is specified. A subsystem improvement is not a whole-application speedup.

N=10 may be a pilot, not a universal final sample count or detectable-effect guarantee. Plan confirmation from variance, target effect, and allocated alpha, then collect fresh samples. Do not pool exploratory samples selected because they looked favorable into confirmation.

## Resource ownership and sampling

Worktrees isolate files, not CPU/GPU load, memory bandwidth, services, or build caches. The orchestrator grants exclusive timing ownership only after competing builds/tests/benchmarks have finished or paused, and prevents new competing work until release. Record contamination and invalidate affected comparisons; do not silently discard slow samples.

Default to independent measurement blocks, each containing one baseline and one candidate run. Randomize which version runs first using a recorded seed. Use identical work and symmetric reset/warmup policies. Always running baseline first does not control ordering effects. Allow state to settle between blocks. Inner iterations or multiple timings from one shared setup are not automatically independent samples.

Retain every raw output, timing, pair ID, unit, run order/timestamp, exit status, and build identity. Diagnose failed runs and rerun the comparison; do not remove failures or outliers to obtain acceptance. Any legitimate exclusion policy must be declared in advance, applied symmetrically, and fully reported.

## Default paired analysis

Run the bundled standard-library helper in the permitted Python 3 environment:

```bash
python3 "$SKILL_DIR/scripts/analyze_timings.py" "$SAMPLES_JSON" > "$ANALYSIS_JSON"
```

Resolve paths first and check exit status before reading or publishing output. Do not switch a mandated toolchain or hand-calculate an acceptance verdict when the required runtime is unavailable.

One JSON input contains one workload. Arrays align by measurement block:

```json
{
  "design": "paired",
  "unit": "ms",
  "baseline": [100,102,99,101,100,103,98,102,100,101],
  "candidate": [80,82,79,81,80,83,78,82,80,81],
  "alpha": 0.05,
  "min_improvement": 0.01,
  "max_regression": 0.02
}
```

These bounds are an interface example, not default tolerances. Alpha is the allocated per-claim error level, not automatically .05 for every campaign test.

The estimand is the population median of paired ratios `candidate_i / baseline_i`. The helper performs exact one-sided sign tests at `1-min_improvement` and `1+max_regression`. For N independent blocks and K ratios strictly on the claimed side, p is `sum(comb(N,j), j=K..N) / 2**N`. Ties count against the claim, making this conservative. The helper also returns one-sided order-statistic bounds at confidence `1-alpha`; null means no finite bound is available. The two one-sided bounds are not a two-sided `1-alpha` interval.

This requires independent, representative blocks with a stable ratio distribution. It avoids a symmetry assumption but may need more samples than a justified parametric test. The helper cannot detect mislabeled samples, interference, or incorrect work; experimental design and provenance remain mandatory.

The median of paired ratios is not the ratio of separate medians. This gate does not prove mean-runtime, tail-latency, memory, or throughput guarantees. If those are the project's objective, predeclare a validated analysis for that metric with the same improvement/non-inferiority obligations, instead of substituting a median claim.

## Acceptance

- **Performance KEEP_CANDIDATE:** required correctness checks pass; primary `improvement_supported` is true; every secondary `noninferiority_supported` is true. Record the primary's bounds too.
- **Simplification KEEP_CANDIDATE:** the stated simplification is present, correctness checks pass, and `noninferiority_supported` is true for every workload. A speedup is not required.
- **DISCARD:** a valid investigation establishes an unacceptable tradeoff, or implementation is abandoned after documented findings. Repair recoverable correctness failures; preserve what was actually tested.
- **INCONCLUSIVE:** valid data establish neither acceptance nor a disqualifying effect. Nonsignificance never means neutral. Follow only the declared sampling plan or retain this status without merging.
- **INVALID_MEASUREMENT / BLOCKED:** contamination, mismatched identities/workloads, failed runs, or missing prerequisites prevent inference. Repair recoverable causes; these statuses do not disprove the hypothesis.

Worker gates are provisional. After combining with the latest accepted baseline and performing cleanup, run correctness checks and fresh final confirmation on that exact revision. Individual candidate speedups do not establish a combined speedup.

## Multiple comparisons and repeated selection

Exploration ranks hypotheses; fresh final confirmation decides integration. Record every confirmation attempt, including failures and retries. Use a predeclared familywise error budget across candidate/workload claims. For a finite campaign, allocate over planned claims (for example Bonferroni). For an open-ended campaign, a conservative valid allocation is `alpha_j = alpha_total / (j*(j+1))` for confirmation attempt j, split across that attempt's acceptance claims. These allocations sum to at most alpha_total. Another validated sequential policy may be declared in advance.

Do not reset the budget after a discard, success, or inconclusive result, or ignore failed attempts. Repeating confirmations until one passes recreates selection bias. Plan enough samples for the allocated alpha: even complete separation of N sign observations cannot give p below `2**(-N)`. Underpowered measurements do not disprove an optimization.

## Mann–Whitney U for independent samples

A project's established independent-sample design may use a validated Mann–Whitney implementation for distributional improvement. Do not apply its independent-sample null distribution to paired observations or interpret its statistic as a median-ratio confidence bound. Additional justified non-inferiority analysis is required; a failed regression test is insufficient.

The historical convention here is `U_fast = count(candidate_i < baseline_j) + 0.5*ties`. Some libraries return the complementary U when candidate is their first argument; check direction.

For N1=N2=10, independent continuous observations without ties:

| U_fast threshold | Exact upper-tail p | Valid claim |
|---|---:|---|
| 73 | 0.044604776 | p < .05 |
| 81 | 0.009271688 | p < .01 |
| 90 | 0.000752344 | p < .001 |
| 100 | 0.000005413 | complete separation |

U=78 gives p=0.017731495; U=83 gives p=0.005748122. Never round tied statistics into this table. Use an appropriate tie-aware implementation. Unadjusted thresholds do not replace campaign error allocation or practical effect bounds.

## References and verification

- [NIST sign test](https://www.itl.nist.gov/div898/software/dataplot/refman1/auxillar/signtest.htm)
- [SciPy Mann–Whitney documentation](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.mannwhitneyu.html)
- [Equivalence testing: why nonsignificance does not establish absence](https://doi.org/10.1177/1948550617697177)

After modifying the helper, run `python3 -m unittest discover -s "$SKILL_DIR/tests" -v`. These tests validate calculations, not sample independence or semantic correctness.
