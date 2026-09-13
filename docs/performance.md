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

The output directory must be new. It contains `perf.data`, raw and folded stacks, `flame.svg`, workload/build/profiler logs and `profile.json` with command, revision/dirty status, binary hash, toolchain, build flags, host, limits, sample count and completion status. Raw profiles are local artifacts, not committed test fixtures. The collector checks that folding preserves sample count and refuses empty profiles or malformed stack parsing. Unknown native frames may remain visible rather than being attributed to engine functions.

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
