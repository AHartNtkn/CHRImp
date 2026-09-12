# Frozen 294c010 cross-engine refresh

Completed: 2,457 measured execution runs, 294 checked warmups, 45 separate ordinary work-reference runs, and 1,638 fresh preparation measurements. Every finite run completed and its exact semantic oracle passed. Search additionally ran traced per-cell references and the established semantic fixtures in both builds. No production files were edited. Later single-head discovery is outside this comparison.

## Result

Current is not uniformly competitive with the older runtime. At n=128, median paired full reused-preparation cost ratios versus older Active are:

| Regime | Current/older Active | Interpretation |
|---|---:|---|
| Alias chain | 13.29 | Current slower |
| High-degree merge | 18.06 | Current slower; noisy primary cell, secondary 11.23 |
| Sparse selective join | 8.87 | Current slower; older Global is faster still |
| Dense Cartesian join | 3.10 | Current slower; secondary independently 3.09 |
| Cyclic triangle matching | 0.323 | Current faster |
| Common work, two alternatives | 8.37 | Current slower despite half the physical source applications |
| Distinct work | 19.98 | Current slower |
| Early failure | 12.36 | Current slower |
| Common work, 128 alternatives | 0.520 | Current faster while preserving all 128 duplicate answers |

The last four rows use the default older build. Older COW control ratios respectively 8.30, 19.71, 12.03, 0.522 support the same conclusions. Each build has its own paired current control; differences between absolute default/COW times are not isolated effects of COW.

All 8/32/128 results, cold charges and paired bootstrap intervals appear in [TABLES.md](TABLES.md). Full phases, preparation components, ticks and work counts are in [summary.json](summary.json). There is no cross-regime average. At n=128 older Global is particularly expensive on dense (28.54 seconds) and cyclic (6.91 seconds), whereas older Active takes 252.5 and 128.1 milliseconds in that same secondary pass. Policy choice materially changes the comparison.

Scaling is regime-specific: cyclic approaches parity at 32 and wins at 128; dense remains slower than Active despite a narrowing ratio. Wide search benefits from shared source execution but still delivers the required quadratic scalar output. Two-way common work does not amortize the current engine's overhead enough to win. These measurements do not identify a particular internal bottleneck or justify an implementation change by themselves. The next comparison should qualify the selected latest commit separately, retaining this uniform 294c010 baseline.

## Versions and execution

- Current: `294c010ebab311cf652d282398a3a2e38f85d4a5`, extracted with git archive into `current/`.
- Older: `4ed9c045dc4eccfca58fb25e39a4f13f46a1a223`, copied from the established frozen search comparator into `older/`.
- Source generators, translators, semantic normalization and exact oracles come from `/tmp/chr-perf-1dc1dc7/comparison/refresh-ec38f70/harness` and `/tmp/chr-perf-1dc1dc7/comparison/search-5ab3ea5`. Search main source is unchanged. Ordinary changes separate tracing from timing, add trace labels/warmups and preparation diagnostics; its workloads and oracle are unchanged.
- Rust release, offline, rustc 1.97.1. See metadata.json for compiler identity and flag environment. No added dependencies or production optimization.
- CPU affinity exactly `[10]`, inherited by every measured child. Sequential execution with rotating runner order, two warmups per cell. No competing jobs were launched by this task during measurement. Affinity does not provide exclusive CPU ownership or eliminate system contention/frequency changes.
- Ordinary primary: current versus older Active, 21 pairs per cell. Secondary: current/Active/Global, seven paired repetitions per cell. Search: current/Active/Global, 21 paired repetitions in each default and older `arena-cow` build.
- Bootstrap intervals resample within-cell paired ratios 10,000 times with fixed seed 5321. They describe observed repeat variability, not independent machine reproducibility.

Absolute timings drift between passes: dense128 current is 1.640 seconds primary versus 0.773 seconds secondary, while its within-pass Active ratio remains 3.10 versus 3.09. Degree128 is less stable (18.06 versus 11.23). Default/COW current controls also differ in absolute time. Preserve both observations; do not attribute cross-pass changes to a code feature. Small percentage claims are not supported by these data.

## Timing boundaries

All reported execution comparisons have tracing OFF and history recording OFF. Ordinary earlier refresh timings enabled older tracing inside the timed run; those results are not a like-for-like performance baseline for this refresh. Search traced references occur outside warmups and timed samples. Older timed application counts are qualified reference counts, not trace-off instrumentation; current application counts are asserted in each run.

Reused-preparation full cost is `init_us + runtime_output_us + engine_drop_us + output_drop_us`. It includes public engine construction/query import, source execution through complete delivery, observation/output construction, and disposal. Search subtracts terminal branch disposal from runtime and reports it in engine_drop, so the sum counts it once. Ordinary older output is owned Answer data and current output is retained scalar events; exact normalization/tuple validation occurs outside runtime. Search constructs the common normalized Answer representation inside each timed run; its canonical exact verification is outside runtime. Validation before final output disposal can affect cache state.

Current source-exhaustion time and delivery tail are available in raw CSV. Its observation interleaves with source; older `observe_us` is a subset of runtime_output, not an additional charge or directly equivalent phase. Engines count different primitive ticks (including older ordinary advance batches of 2048 versus current advance(1)); ticks are work diagnostics, not equal work units.

Cold/preparation-charged columns are an **additive cost model**, not measured end-to-end cold executions, cache-cold runs or process startup. Each is a timed execution sample plus a fresh preparation sample at the same repetition index: common parse + engine preparation + older translation + parsed AST disposal + sole prepared-owner disposal. Old disposal includes its retained converted query template. We ran 21 fresh preparation samples after two warmups for every case/size/build; the seven-repetition secondary table uses the first seven corresponding samples. Generation is outside timing. Both engines are charged the same current parser; older receives an additional conversion charge. This is not a comparison of native parsers. Compile/code-generation cost is excluded, and ordinary runtime timing uses PreparedRuleset rather than a generated specialized matcher. No timing work ran concurrently with preparation compilation.

## Semantic and work qualification

Ordinary: exact ordered tuples are derived from input query bindings, with complete relation coverage and rejection of missing, duplicate or unexpected tuples. Alias requires every query binding to share identity; degree requires exactly the intended hub identity; other bindings remain distinct. Dense validates every Cartesian pair, not only n² facts. Cyclic input is n disjoint directed triangles and requires exactly the three rotations per triangle, not transitive closure. Traced references check physical application count, per-rule count, ordered occurrence-tuple uniqueness and distinct head occurrences. Fixtures cover nonbinding reads, alias wakeup, duplicate occurrences, fresh body locals and ordered self joins.

Search: the variable-only admission translation carries all query variables, including those confined to inactive alternatives, with independent local scopes. Exact answer multisets preserve identity, ordered relation arguments and occurrence/answer multiplicity. Fixtures additionally exercise nested inactivity, failure, aliasing, fresh choices, committed scheduling and finite siblings beside divergence. Per-cell reference traces validate source work separately from the single admission application.

For size n: common has 2 answers and 2n facts/ports/bindings; distinct likewise; failure has 1 answer and n facts/ports/bindings; common-wide has n answers and n² facts/ports/bindings. Current performs 2n source applications in all four. Older performs 4n for common, 2n for distinct/failure and 2n² for common-wide. The current shared execution advantage is therefore explicitly checked without collapsing duplicate delivery.

The refresh is a timing/semantic comparison of these finite qualified workloads. It is not a new full-suite, memory-retention, HTTP or cancellation qualification.

## Evidence and reproduction

- `run.py`, `run.log`, `metadata.json`, `affinity.json`: exact frozen timing orchestration and configuration.
- `ordinary/{qualification.log,workchecks.csv,workchecks.log,active.csv,global.csv}`: ordinary qualification and timing.
- `search/paired-{default,cow}.{csv,log}`: search raw timing and qualification logs.
- `ordinary/preparation.csv`, `search/preparation-{default,cow}.csv`: fresh preparation and disposal samples.
- `summarize.py`: asserts row/repetition counts, trace boundaries, stable qualified ordinary counts and all search answer/fact/port/binding/application formulas; generates tables and summary.
- `SHA256SUMS`: source, binaries, raw evidence and report hashes; `SOURCE_SHA256SUMS` inventories frozen engine source trees.

The saved `run.py` describes a complete rerun but writes its output paths. To preserve this evidence, copy the entire directory to a new /tmp directory before running it. Binaries remain runnable. Preparation commands, likewise writing only into a fresh copy:

```
taskset -c 10 ordinary/target/release/preparation > ordinary/preparation.csv
taskset -c 10 search/default-preparation > search/preparation-default.csv
taskset -c 10 search/cow-preparation > search/preparation-cow.csv
python summarize.py
```

No measurements incorporate 62706a5 or 9c3b023.
