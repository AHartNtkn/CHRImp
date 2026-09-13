# Performance tools

Run measurements from `/home/ahart/Documents/CHRImp`. The full suite goal remains active; calibrated regression detection, expanded lifecycle coverage and independent challenges are still being implemented.

## CPU flame graphs

```sh
python3 examples/profile.py --out /tmp/chr-profile -- notebook-behavior-i 1 50000000 15
```

Open `/tmp/chr-profile/flame.svg`. Click a frame to zoom; use Search to highlight functions. Width represents sampled user-space CPU, not wall time, execution order, a tick count, or kernel/I/O waiting. The profile includes preparation, validation, output and cleanup; call stacks distinguish those from engine work. Short runs have substantial sampling uncertainty.

The command builds the optimized `profiling` Cargo profile with debug information and frame pointers, records CPU stacks with Linux `perf`, demangles Rust symbols with GNU `c++filt`, and uses `inferno-collapse-perf` and `inferno-flamegraph` to render the graph. These executables must be installed. It does not install dependencies or rerun rwLog.

Use `--cli` to profile the actual language CLI with a program and query:

```sh
python3 examples/profile.py --cli --seconds 5 --out /tmp/chr-cli-profile -- examples/proofs.chr --query 'edge(A,B,AB),edge(B,A,BA)'
```

This cyclic proof query keeps producing derivations; its profile intentionally ends at the external deadline.

The output directory must be new. It contains `perf.data`, raw and folded stacks, `flame.svg`, workload/build/profiler logs and `profile.json` with command, revision/dirty status, toolchain, build flags, host, limits, sample count and completion status. Raw profiles are local artifacts, not committed test fixtures. The collector checks that folding preserves sample count and refuses empty profiles or malformed stack parsing. Unknown native frames may remain visible rather than being attributed to engine functions.

`--seconds` bounds the entire profiled process including cleanup (default 40 seconds); an additional five-second interrupt grace allows perf to write its data before a hard kill. `--memory-mib` sets a per-process address-space ceiling (default 4096 MiB). `--frequency` selects sample frequency (default 499 Hz). A capped run is reported as censored, never a completed answer. Build/postprocessing have separate bounded timeouts. This is an instrumented CPU profile, not a baseline timing run.

## Generated relational workloads

```sh
cargo build --release --example measure
target/release/examples/measure graph-walks 8 5000000 10 --shape ring --rows 2 --seed 17
target/release/examples/measure proof-dag 7 5000000 10 --shape diamond --seed 17
target/release/examples/measure graph-bits 7 5000000 10 --shape random --seed 17
target/release/examples/measure duplicate-heads 4 5000000 10 --rows 3 --seed 17
target/release/examples/measure partial-join 16 5000000 10 --rows 8 --seed 17
```

Graph shapes are chain, ring, star, diamond, dense and seeded random. DAG workloads reject cyclic shapes. `--rows` varies edge/relation multiplicity for graph cases, group count for duplicate heads, and probe count for partial joins. `partial-join-hit` provides the corresponding successful-join control. Seed zero uses canonical query order; other seeds reproducibly vary order and random structure. Boolean seed zero uses disagreement constraints; other seeds mix agreement/disagreement.

The oracles check ordered tuple multisets and distinct query identities. Walks require distinct edge occurrences even on a self-loop; DAG proofs count every derivation, not just reachable pairs; duplicate heads check distinct fresh witnesses; partial joins check payloads and exact hit/miss behavior. Boolean graphs enumerate all assignments independently (at most 16 variables), including disconnected and unsatisfiable cases. These bounds belong to the benchmark oracle, not the language.

Vary dimensions independently before attributing scaling: repeated edge occurrences and multiple proofs can legitimately increase output. Equivalent answers alone do not require equivalent cost. Generator timings remain outside parse/prepare/execution measurements. Existing measurement commands report source and cleanup limits separately; the flamegraph wrapper also imposes an external deadline.

## Relevant checks

```sh
cargo test --release --example measure
cargo clippy --example measure -- -D warnings
python3 -m unittest discover -s tests -p profile_test.py
```

## Baseline and detailed observation

```sh
cargo build --offline --release --example measure
target/release/examples/measure notebook-behavior-i 1 50000000 15
target/release/examples/measure notebook-behavior-i 1 50000000 15 --detail
```

Default observation records phase totals, first-event/first-answer latency and periodic budget/memory checks. `--detail` additionally measures per-step maximum latency where supported, validator duration and collection-status samples. Both modes execute the same workload and all semantic validators. Unmeasured detail-only fields are `null`, not zero. First-event fields retain the existing `Some`/`None` textual notation until structured results are integrated. `source_delivery_ms` includes validation; the separately reported subtraction exists only in detailed mode. First-answer latency is elapsed time before validation of its final event, including work to handle preceding events.

Lifecycle deadline checks occur every 2048 work units and at phase completion; they cannot preempt one engine or API call. Lifecycle memory peaks are sampled every 2048 ticks and at application milestones in baseline mode, every source tick in detailed mode. External supervision is still required for stuck calls, output and destruction. Detailed observation and the allocation/work `diagnostics` build feature are independent; baseline timing uses neither. Mode identity and feature status are printed in each command result.

Check mode equivalence with actual core, generated, notebook, archive and runtime workloads:

```sh
cargo build --offline --release --features diagnostics --example measure
python3 -m unittest discover -s tests -p measure_observation_test.py
```

These checks compare validated endpoints and per-rule work, and require any total iteration difference to be explained entirely by collection dispatch counts. Repeated unchanged synthesis runs already vary in collector work; collection traverses shared variable groups in address order. Treat raw internal work counts as measurements requiring calibration, not universally deterministic language properties. Independent scaling/regression detection remains required.

## Opt-in work and allocation diagnostics

```sh
cargo build --offline --release --features diagnostics --example measure
target/release/examples/measure partial-join 16 5000000 10 --rows 8
target/release/examples/measure notebook-behavior-i 1 50000000 15
cargo test --features diagnostics --test diagnostics
cargo test --features diagnostics --example measure
```

`diagnostics=` lines contain JSON checkpoints before cancellation and after cancellation for core/notebook engines and lifecycle cancellation probes. Work counters are cumulative for that engine; subtract checkpoints for interval counts. Rule entries follow prepared-rule order. Indexed candidate visits, direct-anchor matches, found candidates, started/applied/rejected commits and dispatch counts are separate. Dispatch categories sum to entered `advance` iterations, including early stops; they are not CPU proportions. The collection category counts service dispatches, including semantic maintenance, not physical collector CPU. Explicit binary choice births are not a count of independent programs. See `src/engine/diagnostics.rs` for event boundaries.

Allocation JSON records process-wide successful allocation/reallocation counts, requested bytes allocated/freed, current live requested bytes and the lifetime high-water mark. A successful realloc counts an old-size free and a new-size allocation. These quantities exclude allocator metadata, fragmentation and internal transient realloc storage; they are not RSS. Categories describe the code executing each allocation or free, not the object retaining it. An object allocated during execution can be freed during cleanup, so per-category allocated-minus-freed is not retained ownership. `other` remains explicit for unscoped work, including report creation and portions of lifecycle/API handling. Inspection work is separate where scoped; source and inspection may otherwise share an engine service turn.

The final `allocations=` JSON is sampled after the workload function returns, before serializing that report and before process shutdown. Earlier reports contribute allocation traffic to subsequent checkpoints. Snapshots are consistent at quiescent single-thread checkpoints; atomics make counting safe across threads but do not make simultaneous counter reads an atomic snapshot. Engine object-count checkpoints remain available alongside byte accounting.

The `diagnostics` feature compiles engine counters and the measure allocator in. Default builds contain neither; these instrumented timings require observer-overhead calibration and must not be substituted for baseline timing. Allocation stack attribution, full ownership timelines, and baseline/detailed comparisons remain outstanding suite work.

## External limits and process resources

Build the desired binary first, then run it directly:

```sh
python3 examples/supervise.py --measure --seconds 10 --out /tmp/chr-run -- target/release/examples/measure rewrite 128 5000000 5
python3 examples/supervise.py --seconds 10 --out /tmp/chr-cli-run -- target/release/chr examples/proofs.chr --query 'edge(A,B,AB),edge(B,A,BA)'
python3 -m unittest discover -s tests -p supervise_test.py
```

The new output directory contains stdout/stderr and `run.json`: exact command, working directory, host, limits, status, elapsed time, user/system CPU, peak resident KiB, minor/major faults, voluntary/involuntary context switches and filesystem block counts. Build flags/revision belong to the campaign metadata still under construction.

Linux `pidfd` readiness enforces the wall deadline without busy polling. After the deadline, the process group receives an interrupt and then a hard kill after a one-second grace; CPU profiling uses five seconds to flush sampling data. A run remains censored even if it handles the interrupt and exits zero. Live children in the original process group are killed before reaping the leader; termination is checked for up to one second and reported separately. A normal exit with leftover group members is failed; a deadline remains censored with its cleanup result attached. The supervisor manages native CHR/profiler processes that stay in their launch group; it is not containment for commands that detach into another group/session. Launch/pre-exec and kernel-uninterruptible termination are not made preemptible by a user-space deadline. Address-space limits are per process, file-size limits per file, and CPU limits per process; these are not aggregate process-tree or disk quotas. SIGXCPU/SIGXFSZ are identified; other failures retain their status and stderr without guessing whether a cap caused them. `--measure` recognizes that harness's incomplete exit code; arbitrary CLI exit 2 is an error.

Resource accounting comes from Linux `wait4` for the launched process and descendants it waited for. It includes pre-exec/launcher work and, for profiling, perf itself. Peak RSS is a high-water mark, not summed process-tree memory or live allocation bytes. Fork/pre-exec memory can impose a floor on small workloads; `launch_parent_rss_kib` exposes the launch context. Filesystem blocks are kernel accounting, not bytes or syscall counts. These metrics complement engine/allocation measurements; subtracting CPU from wall time does not identify a particular blocking cause.

## Repeated typed measurements

```sh
python3 examples/perf.py --out /tmp/chr-repeat --repeat 5 --seconds 10 --total-seconds 120 -- rewrite 128 5000000 5
python3 examples/perf.py --diagnostics --out /tmp/chr-work --repeat 3 -- partial-join 32 5000000 5 --rows 16
cargo build --offline --release --example measure
python3 -m unittest discover -s tests -p perf_test.py
```

The repeat command builds before timing unless `--binary` selects an existing binary. It records the command, toolchain, host, build environment, revision and dirty-tree status. Prebuilt binary build flags remain unavailable. One warmup is the default; every warmup, unsuccessful sample and completed sample retains stdout/stderr plus an atomically published typed `<index>.json` record. `campaign.json` records outcome counts and completed-sample median, range, median absolute deviation, sample count and missing-value count. Unavailable values remain null. Censored runs never become completed timing samples; their raw observations remain available.

`measurement=` lines are versioned JSON emitted directly from measured values. Configuration, result, diagnostics, allocations and lifecycle phase records are separate; named memory gauges replace positional interpretation. The runner does not scrape display text and rejects missing/duplicate/inconsistent final results. Per-phase and per-rule diagnostic summaries retain scope; cumulative checkpoints must not be summed as exclusive work. The aggregate sample budget reserves interrupt/cleanup grace and can end before the requested repeat count, reported as aggregate censoring. A campaign alarm also bounds parsing and summarization; processing interrupted by that deadline retains a censored record and raw output. Prebuilt-binary toolchain/source details are explicitly unverified local context. Build has a separate five-minute cap. This repeat command is infrastructure for routine/deep comparisons; calibrated regression decisions and campaign selection remain unfinished and are explicitly reported as not performed.

## Retention and concurrency interactions

```sh
target/release/examples/measure life-held-output 2 5000000 5 --rows 3 --work 24
target/release/examples/measure life-archive-fixed 2 5000000 5 --rows 3 --work 24 --cadence 4
target/release/examples/measure life-archive-rotate 2 5000000 5 --rows 3 --work 24 --cadence 4
target/release/examples/measure life-inspections 2 5000000 5 --rows 3 --work 24
```

SIZE varies continuing siblings, retained snapshots, or concurrent inspections. `--rows` controls committed residual width; `--work` continued applications; `--cadence` the application interval between archive replacements. Snapshot admission may require additional source applications; actual work is recorded, so milestones are lower bounds. Held output pauses consumption after Begin while the source continues, then validates the full answer. Inspections remain partly unread during continued work. Fixed and rotating archives inspect their retained committed multisets, then release ownership. Pending syntax is outside the committed-view oracle. Every case checks source progress and eventual cleanup; none enables default history retention.

## Comparing repeated measurements

```sh
python3 examples/perf_compare.py /tmp/before /tmp/after --out /tmp/comparison.json
python3 examples/perf_compare.py /tmp/before /tmp/after --out /tmp/work-change.json --metric workload.work.ticks
python3 -m unittest discover -s tests -p perf_compare_test.py
```

The comparator consumes raw campaign samples with matching workload/observation configurations. It reports mean changes, descriptive medians, ranges and counts, using a two-sided permutation test of absolute mean differences with Holm correction across the requested metrics. Up to 10,000 assignments are enumerated exactly; larger tests use 9,999 seeded permutations with the conservative plus-one estimator. Fewer than five observations per side means insufficient evidence; missing values remain unavailable. The default processing budget is 30 seconds (`--seconds`). Failures, censoring, invalid records and comparison timeouts have distinct outcomes; completed subsets cannot establish the outcome of an incomplete campaign. Raw sample outcomes determine counts.

An increase/decrease is statistical change evidence, not a material regression verdict. Exchangeability/independence assumptions can fail under machine drift or ordered execution; uncertainty does not establish equivalence. Materiality, scaling policy and broad control calibration remain required suite work. Exit 0 means the comparison was calculated, not that performance passed an acceptance gate; exit 1 means failed/invalid evidence and exit 2 means censoring or a processing deadline.

The native result includes `native_peak_rss_estimate_kib`, read once at final-result emission from Linux `/proc/self/status` VmHWM. This covers the native harness address space through that point, including parsing, validation, allocator overhead and reporting so far; it excludes the launcher address space before exec. It remains null when unavailable. [Linux documents VmHWM as approximate](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html); the estimate can move slightly downward as accounting updates. A subprocess calibration touches 32 MiB and verifies both held and released observations expose at least 30 MiB above baseline. This tests a material signal, not exact byte accuracy. Wait4 RSS remains separate launcher-inclusive evidence. Requested-allocation peaks provide a different, precisely defined measurement under diagnostics.

Two unchanged seven-sample `rewrite 512` campaigns gave native peak medians 6,936 and 6,932 KiB, with ranges 6,888–7,084 and 6,816–7,084 KiB. No default metric showed a statistically qualified change; source-delivery medians were 24.84 and 25.68 ms. This is one local benign control, not full false-positive calibration or a universal noise bound.

## Preparation shape, reuse and direct destruction

```sh
python3 examples/perf.py --out /tmp/prep-reuse --repeat 5 --seconds 5 --total-seconds 40 -- prepare-reuse 64 5000000 5 --heads 4 --arity 4 --repeats 4 --width 16 --depth 8 --uses 8
python3 examples/perf.py --out /tmp/prep-independent --repeat 5 --seconds 5 --total-seconds 40 -- prepare-independent 64 5000000 5 --heads 4 --arity 4 --repeats 4 --width 16 --depth 8 --uses 8
python3 examples/perf_compare.py /tmp/prep-before /tmp/prep-after --out /tmp/prep-change.json --metric workload.parse_program_ms --metric workload.prepare_ms.0 --metric workload.uses.0.engine_init_ms --metric workload.elapsed_ms
```

`SIZE` is inactive rule count; zero is a control. Independently vary heads per rule, arity, extra repeated uses of one head variable, body width, nested conjunction depth and sequential engine uses. Defaults are 1/1/0/1/0/1. The generated inactive relations have no query occurrences. Every use still executes and validates `start(A)` rewriting to exactly `p(A)`; `--empty` instead validates one empty answer. Body nesting, including the width container, respects the parser's 128-container limit. Preparation-specific options apply only to these two cases.

Both cases generate/parse once. Reuse prepares once and uses the same prepared object for fresh engines; independent prepares from the same AST before every fresh engine. Typed records separate source generation, program/query parsing, preparation, engine initialization, execution/delivery/validation, direct engine destruction, final prepared-object destruction and syntax/source destruction. Sequential uses stay ordered within a process and are not independent timing repetitions. Every completed run checks all uses and prepared ownership release; the aggregate tick and wall budgets cover all uses and destruction. External supervision bounds calls that cannot be interrupted internally.

Select the relevant `workload.*` phase metric for comparisons; the core source-delivery default is unavailable for these cases. Three measured runs of each 64-rule shape above completed with all eight results validated. This supplies runnable preparation/reuse evidence; calibrated shape/size growth detection remains part of routine/deep campaign work.

## Bounded routine and deeper sweeps

```sh
python3 examples/perf_suite.py routine --out /tmp/chr-routine
python3 examples/perf_suite.py deep --out /tmp/chr-deep
python3 examples/perf_suite.py deep --list
python3 examples/perf_suite.py deep --only archive-cadence --only fresh-copies-interleaved --out /tmp/chr-interactions
python3 -m unittest discover -s tests -p perf_suite_test.py
```

The routine selection sweeps rewriting, partial-hit/miss matching, occurrence multiplicity, fresh identity contraction, reconvergent proofs, competing growth, archive turnover and preparation shape, plus end-to-end query points. Deep selection adds graph topologies/seeds, independently varied concurrent ownership/turnover/reuse, fresh-copy input orders and further notebook queries. `--only` selects families; `--list` displays every point and its risk question before execution. Seeds also shuffle point order to avoid always running larger inputs later. This does not make measurements independent or eliminate machine drift.

Defaults are twelve measured repeats plus one warmup per point, a five-second external sample limit, and a 120-second routine or 600-second deep execution/analysis budget. Each workload also has a declared three-second native phase limit. Flags can set these external budgets and repeats before execution. Builds occur once with a separate five-minute limit; `--binary` uses an existing executable, and `--diagnostics` builds the instrumented variant. Each point delegates to the same `perf.py` measurement/supervision path. Nested reporting remains subject to the suite deadline, and known outcomes are published before report I/O. The final small suite record is written after the execution/analysis alarm is disarmed.

`suite.json` contains the selected points, commands, risk questions, outcomes, raw campaign paths and adjacent-size scaling. Ratios retain both median operands; log-log exponents are descriptive, not complexity proofs. Censored or missing points do not connect a scaling slope. Necessary derivation/output growth remains visible alongside cost. A point interrupted during processing is distinct from a point never admitted. Completed failures remain failures even when later reporting is interrupted. Exit 0 means all requested observations completed; it is **not a regression acceptance gate**. Exit 1 records failed evidence, exit 2 bounded incomplete execution. Calibrated material-change and growth decisions remain unfinished work.

The initial 21-point routine selection finished in 36.8 seconds: 20 points completed five measured samples each; behavior-W synthesis had five censored samples at its declared limit. A subsequent 19-point deep selection covering fresh production/copy order, ring correlations, archive cadence and prepared reuse completed three samples per point in 1.71 seconds. These are coverage and bounded-execution checks, not evidence that all prospective risks are covered.

To extend coverage after a system change, identify the new independently growing input or ownership dimension, add it to an existing generator where possible, and supply a semantic/progress/resource oracle independent of the implementation being measured. Add a discriminating control and size/lifetime sweep to `plan()`. Then verify that the maintained comparison/detection path catches a material adverse variation and preserves a benign control; adding a case alone does not close the risk.

## Fresh production and equality propagation

```sh
target/release/examples/measure fresh-contract 4 5000000 5 --rows 4 --depth 4 --order interleaved
target/release/examples/measure fresh-unmerged 4 5000000 5 --rows 4 --depth 4 --order interleaved
```

SIZE is groups, rows is copies per group, and depth is production-chain length (default four, zero allowed). Each production consumes a step and creates a fresh level variable. Contracting equal-key cells equates their fresh values, consumes a duplicate occurrence, and enables matching at the next level. The unmerged case retains independent copy chains. Grouped, interleaved and reversed queries vary admission order; one copy and zero depth are equivalent controls.

The independent oracle checks all input identities, fresh-level separation, intended equalities, endpoints and exact residual multisets without prescribing numeric variable IDs or surviving occurrences. Application counts follow analytic production/contraction counts; internal ticks are measured, not fixed. Markers preserve one witness per producer, so output and validation necessarily grow with groups × copies × depth. Compare the contraction work with that required evidence volume rather than treating all cost growth as overhead.

## Calibrated cost and scaling regression signals

```sh
python3 examples/perf_regress.py /tmp/control-a /tmp/control-b /tmp/before /tmp/after --out /tmp/regressions.json
python3 examples/perf_regress.py /tmp/control-a /tmp/control-b /tmp/before /tmp/after --out /tmp/startup.json --metric process.wall_seconds
python3 -m unittest discover -s tests -p perf_regress_test.py
```

Use two independently executed unchanged suite campaigns as controls, with the same selected workloads, parameters and observation mode as the comparison. Controls describe observed variation on that host; they are not a confidence bound on future noise. The command reads raw validated samples, retains suite and campaign failure states separately, and tests both per-point costs and adjacent-size cost ratios. Ratio samples associate independent size runs by repeat index; they are not within-process paired measurements. Default metrics cover source/first-answer/cleanup or the corresponding preparation/session phases, native residency and whole-process wall time. Explicit `--metric` selections use the same detection path.

A regression signal requires a mean increase larger than the unchanged-control span and a two-sided permutation test qualified under Holm correction across **all** requested cost and scaling hypotheses. Means express average cost; medians and raw ranges remain descriptive evidence. Mean direction and ratio use the same statistic as the test. Larger families receive finer Monte Carlo resolution. Exact equal-size two-sided tests account for complementary assignments. Mathematically insufficient resolution, fewer than five observations, missing calibration, censoring and unavailable observations stay explicit and cannot establish a clean comparison. Twelve repeats are the suite default because five-sample comparisons cannot resolve many material changes after family correction; repeat count alone does not guarantee sensitivity under noisy data.

The comparison has a declared 300-second processing budget, adjustable with `--seconds`. Exit 1 indicates a regression signal or failed/invalid evidence; exit 2 indicates incomplete/insufficient evidence or timeout; exit 0 means no qualified regression was detected at the reported resolution. It does not establish equivalence, optimal complexity, broad coverage, or acceptable baseline speed. A wide control span can conceal relevant effects; inspecting resolution and performing independent sensitivity challenges remains necessary.

Native calibration used six points across rewriting, partial-hit matching and preparation with nine measured repeats. The held-out unchanged campaign had no qualified regression. An 80 ms startup delay applied outside the native executable produced six process-wall regression signals (observed mean increases approximately 80–82 ms), while native source timing remained separately reported. A synthetic 1,000-hypothesis control verifies that a threefold cost increase can cross the family threshold with twelve samples and sufficient permutation resolution. These checks establish specific instrument capabilities; the full independent prospective challenge set remains outstanding.

## Runtime session lifetime and replay

```sh
target/release/examples/measure runtime-sessions 8 100000 5 --closed 8 --retained 4 --rows 8 --batch 1 --replay-every 2 --work 32
python3 examples/perf_suite.py deep --only sessions-history --only sessions-retained --only sessions-batch --only sessions-replay --out /tmp/session-sweeps
```

This runs the native Runtime API and scheduler directly, without a browser or HTTP server. SIZE controls finite answer count; rows controls residual width. Closed sessions are completed, drained, closed and retired before the measured pair of a main query and a tiny query. Retained sessions remain owned after completion. Batch size counts scalar output events (1–4096). Replay cadence is measured in nonempty batches; zero disables replay. Work is a lower bound on continuing background applications; zero omits that source.

Each finite result has an independent residual/application oracle. Replays repeat an unacknowledged frozen batch after background progress, require identical contents/metadata, and verify that output reads themselves advance neither source. Reports separate setup, source, tiny-answer latency and work position, cleanup and Runtime destruction. Source includes fresh admissions, API handling and validation; detailed read/replay/validator clocks are opt-in. Requests plus scheduler turns form this case's work budget; they are not engine ticks. Setup/source share a budget; cleanup receives a separate equal budget, bounded externally by the sample deadline.

Linux spool evidence counts descriptors and unique logical file lengths belonging to this Runtime. History cleanup, retained-file count, closure/owner retirement and post-drop descriptor release are checked; these observations do not claim full heap reclamation. Other platforms report null spool evidence. Initial deadline exhaustion remains incomplete, and validated answer progress is published before another budgeted operation can exit. The four-dimensional selected sweep completed three samples at all 13 points in 1.82 seconds. Its progress/resource checks and censoring boundaries have dedicated tests.
