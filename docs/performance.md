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
