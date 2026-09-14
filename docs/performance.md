# Performance tools

## Mutation-lane payload FIFO (Round 021)

Suspended `Scheduled` values live directly in typed FIFO entries alongside
Completion and Collection. The FIFO uses four-entry payload blocks and a small
pointer directory; two empty blocks can be reused. Payload addresses do not
change while queued. Fully retired blocks and excess directory capacity are
released as the frontier drains, and cancellation releases all remaining backing.
There is no parked-task tree or duplicate requested-owner index.

`work.waiters` retains the Round 020 logical request/enqueue/grant counters,
including an immediate acquisition as an enqueue followed by a grant even though
it now needs no FIFO allocation. `entries` is actual occupancy; `peak_entries`
also includes those logical immediate acquisitions. Task parks/wakes are unchanged.
Additional diagnostics report current/peak `capacity_bytes` (all block slots,
two possible spares, and pointer-directory backing), `capacity_changes`, and
`growth_move_bytes_bound` / `shrink_move_bytes_bound`. The move bounds charge
twice the old directory length in pointer bytes at resizing, covering relocation
and wrapped-segment movement; actual realloc can extend in place. They do not
count CPU instructions or movement within the unchanged runnable queue.

`payload_moves` counts task transfers into and out of the FIFO, excluding final
discard; `direct_handoffs` counts transfers directly to the runnable queue.
`trace_steps` counts collection service calls on FIFO tasks, `trace_tasks`
completed task traces, `discard_steps` cancellation task-discard calls, and
`discarded_tasks` released payloads. All instrumentation compiles out normally.
The matched baseline patch counts the same events; its capacity/move fields
describe only its scalar FIFO, **excluding its separate parked tree**. Whole
phase allocations and live/peak checkpoints charge both representations.

The [Round 021 evidence](optimization-evidence/rehearse/round021-lane-payload/README.md)
includes the full paired matrix, conditional-archive controls, replacement costs,
and complete release checks. Pending-syntax controls perform no acquisitions.

## Mutation-lane waiter diagnostics (Round 020)

Engine diagnostic checkpoints include `work.waiters`: `requests`, `enqueued`
and `granted` arrays in Task / Completion / Collection order, plus `entries`
and `peak_entries` for the FIFO. Requests include calls by an owner already
holding the lane. Grants include immediate acquisitions and FIFO handoffs;
task wakes still count only parked-to-runnable transitions. Cancellation clears
the entry gauge when it releases the scalar FIFO. Instrumentation adds no queue
scan, allocation, or ordinary-build fields.

In Round 020, tasks parked immediately after a failed acquire, with the FIFO and
parked map as their waiting representation. Completion and collection retry through
separate flags, cleared on dequeue. The old duplicate BTreeSet is removed.
The [Round 020 report](optimization-evidence/rehearse/round020-waiter-index/README.md)
includes paired phase allocations and equal-work checks. The existing `rewrite`
size sweep exercises growing parked tails; `life-pending-*` stops before any
acquisition and supplies unaffected pending-syntax lifecycle controls.

## Store batch coalescing (Round 018)

Store batch preparation scans writes in reverse, keeps up to eight exact keys
inline, then promotes on a ninth distinct key to a temporary HashSet. Final
no-op writes enter the seen set too. Retained writes compact into the already
read suffix, move to the caller's prefix, and use the existing sort/application.
The table and its randomized hasher are created only on promotion and the table
is dropped before sorting/application. The stack holds eight 32-byte keys;
there is no persistent scratch table or extra root ownership.

Diagnostic checkpoints add `store_batches`: graph, propagation-history and
pending Stores in the same order as `store_mutations`. Each entry reports
`calls`, `input_writes`, `comparisons`, `hash_requests`,
`membership_checks`, `retained_writes`, `scratch_tables`,
`max_input_writes`, `scratch_peak_capacity` and `scratch_peak_bytes`.
Comparisons count full-key equality calls (inline or HashSet), not individual
words. Hash requests include actual calls during promotion and growth rehashing,
not just insert requests; bucket probes and hash instructions are not counted.
Scratch table bytes estimate the current standard-library bucket/control layout.
They exclude the fixed stack array and table header. These are per-call peaks,
never retained-storage gauges; the requested-allocation runner charges all real
table growth allocations/frees. The counters and their lock are diagnostics-only.

`tests/batch_coalescing.rs` reuses the maintained measure allocator and checks
ordered final rows, compacted writes, exact no-op root reuse, retained snapshots,
cursor pins through collection, and bounded release against a forward BTreeMap
oracle. `CHRIMP_BATCH_CASE=N/DISTINCT/dense|sparse/insert|mixed|noop|delete`
selects a single fixture in a fresh prebuilt test process. Invoke
`--exact batch_order_and_lifecycle_matrix --nocapture --test-threads=1` under
`timeout --kill-after=5s 60s`. The Round 018 reproduction script sweeps
0/1/2/8/9/16/64/256 writes and both sides of the eight-key promotion boundary.
The audit reports every phase and keeps test-harness `other` traffic separate
from the scoped fixture's release balance. Final process peaks can be dominated
by validation/reporting; inspect the update checkpoint and scratch allocation
traffic as well. See the Round 018 report for promoted-table costs and controls.

## Adaptive shared-restriction partitions (Round 017)

Each row producer keeps up to two distinct projected keys in a sorted inline
table. A third key promotes once to HashMap, copying at most two entries. Row
links remain in the existing occurrence vector; repeated-key occurrences retain
their order and multiplicity. Prefix plans and the Store prefix-root memo are
unchanged. The threshold was compared at capacities one, two and four: two saves
the tiny table allocation while limiting the header cost of promoted tables.

Restriction diagnostics add `inline_partition_limit`, `inline_comparisons`,
`partition_hash_requests`, `partition_promotions`, `promoted_entries`,
`shifted_inline_entries`, and current/peak `partition_table_bytes`.
Comparisons count full key comparisons, not individual words. Hash requests
count get/get_mut/insert calls, including explicit promotion inserts, not
internal bucket probes, rehash work or CPU instructions. Table bytes include
the inline/header space of all live producers (including evicted subscriber-owned
producers) and estimated HashMap backing. They exclude rows, locks and other
producer fields. Hash backing uses the current standard-library bucket/control
layout estimate; process requested-allocation checkpoints remain authoritative.
Collection/drop removes released producers from the current gauge.

The diagnostic `shared_restrictions` test `partition_table_threshold_matrix`
checks eight exact ordered matches per fixture, independently recorded occurrence
IDs and true support, one shared scan, retention, and complete release. Its
`CHRIMP_PARTITION_CASE=ROWS/KEYS` environment selector runs one listed case in a
fresh process so repeated-key peaks do not inherit earlier dense cases. Run the
prebuilt test alone with `--exact partition_table_threshold_matrix --nocapture
--test-threads=1` under `timeout --kill-after=5s 60s`. The accepted growing-prefix
and churn probes remain complementary version/snapshot controls. Tests also
cover promotion while readers are paused, last-reader cancellation before/after
promotion, cache collection, and full-width keys differing in every port.

## Store prefix-root memo (Round 016)

Store keeps at most 32 weak prefix lookup entries, keyed by its root allocation
identity and normalized prefix/word count. Store ownership is implicit in the
table and rechecked in each weak witness. Hits validate both the supplied root
and upgraded result; misses follow the original path. Capacity pressure clears
the bounded table, and collection releases its backing and weak allocations.
No entry owns scalar payloads or descendants. Weak input witnesses can force
copy-on-write on a subsequent mutation; these costs belong in comparisons.

Detailed measure checkpoints expose `prefix_lookup` (requests, inspected path
nodes) and `prefix_memo` (hits, misses, evicted entries, current entries, table
capacity). Requests include empty roots, which bypass memo lookup. A miss adds
a hash-table lookup and insertion; these counters do not count internal hash
bucket probes or represent total CPU work. Store's existing mutation and
allocation counters plus the maintained requested-allocation checkpoints expose
the associated storage costs.

The `shared_restrictions` growth test compares nine exact version matches and
nine retained-snapshot readbacks at 128/512/2048 initial rows. The additional
`growing_prefix_cache_churn_preserves_answers_and_release` test uses 33/65/129
versions at 128 rows, exceeding memo capacity, and validates every occurrence
in both passes before final collection/release. Run each prebuilt diagnostic
test alone under `timeout --kill-after=5s 60s`, with `--nocapture --test-threads=1`
for comparable allocation checkpoints. The Round 016 report records cold-miss
controls as well as the retained-version hits; this memo does not reuse a result
across different Store root identities merely because its subtree is unchanged.

## Shared restriction prefix witnesses (Round 015)

Buckets with more than 128 and at most 4096 rows are traversed as ordered
immutable Store subtrees of at most 128 rows. An exact weak allocation identity
plus Store owner is the generation certificate: holding the weak identity
prevents both in-place mutation and allocation-address reuse. Completed raw
projections contain occurrence IDs and partition links, never Boolean handles.
Each snapshot retains its own cutoff and matching reads support from that root.

The existing 32-entry producer registry now also serves these fragments. A
separate 32-entry registry shares whole-prefix plans, retaining earlier fragment
producer leases for lagging readers even across cache eviction/collection.
Last-reader cancellation abandons unfinished plans and releases pending roots.
Small buckets retain the whole-bucket path. One service call traverses one
branch, subscribes to one fragment, or performs one original producer step.
Splits, fragment probes, and reader transitions are replacement work, not free
row inspections; fewer service ticks alone are not a gain.

`shared_restrictions` adds `prefix_splits`, `prefix_probes`, `reused_prefixes`,
`rebuilt_prefixes`, `prefix_plans_created`, and `prefix_plans_reused` counters.
`created`/`reused` still count row producers, including fragments; a plan hit
is counted separately. `retained_rows`/`retained_partitions` count actual live
buffers once even when many plans own them. `cached_prefix_plans` and
`cached_prefix_links` count only registry-owned plans/links, excluding evicted
plans held by readers; allocator checkpoints include all ownership and slack.
Small graphs also pay the extra registry header and larger subscriber variant.

The diagnostic `growing_prefix_versions_preserve_order_cutoffs_and_release`
test uses the maintained measure allocator, prints phase checkpoints, and
independently checks nine version results plus all nine old-snapshot readbacks.
Run it alone with `--nocapture --test-threads=1` for allocation comparisons.
Reports are outside the sampled checkpoint, so their serialization affects
later checkpoints in `other`. It is a graph/matcher mechanism probe; maintained
`partial-join`, `partial-join-hit`, notebook I/S and lifecycle measurements
provide separate end-to-end controls. Round 015 evidence documents both.

## Boolean epoch experiment diagnostics

With `diagnostics`, `measure` emits `condition_slab` at the existing engine
checkpoints. Round 014 uses reclaimable eight-node epochs with full-width
monotonic IDs. The array fields are, in order:

1. Live nodes; vacant payload positions in live epochs; live directory entries;
   effective HashMap capacity; allocated epoch payload bytes (including links
   and live counts).
2. Node get/get_mut/remove calls; entries rehashed by directory contraction;
   node constructions; estimated directory backing bytes; live epochs;
   cumulative epoch reclamations; an alias of node access calls for the old
   diagnostic position (this is **not** a hash probe/CPU-step bound).
3. Live chronology records (one per live epoch); active frozen collector leases
   (zero or one); bytes of the collector's additional successor witness;
   chronology link bytes (included in payload bytes); directory read/write
   lookups; payload positions inspected by chronological traversal.

Directory lookups count `get`/`get_mut`, including allocation and unlinking;
HashMap insert/remove/rehash probes are not instrumented. Constructions and
reclamations expose their frequencies. Chronology examines at most two live
epochs and sixteen positions per successor, independent of retired epochs.
There are no separate generations or recycled node IDs. Directory contraction
can scan its old backing and rehash retained entries in one call, like the
existing size-dependent table allocations; its whole allocation/cleanup costs
remain in the runner. No occupied payload moves.

Directory bytes estimate the current standard HashMap bucket/control layout
from effective capacity; this is not an allocator-independent exact size.
Allocator checkpoints are authoritative for process requested storage. The
map and slab inline headers are part of the containing Arena, not the backing
estimate. Cursor leases reuse the existing GC freeze; the successor witness is
one `Option<u64>` (16 bytes on the measured build) per active collector, with no
heap allocation or retention of dead epochs. Epoch slack is at most seven nodes
per live epoch and is charged even when only one payload survives.

For matched unfinished notebook frontiers, diagnostics builds accept
`CHRIMP_MEASURE_SOURCE_DISPATCHES=N`. This only stops observation after N
non-collection dispatches and reports `source_dispatches` censoring. It does
not claim completion or equate those dispatches with semantic work: compare
per-rule vectors, output obligations, other dispatch categories and cleanup.
The ordinary build rejects this optional diagnostic control.

Run measurements from the selected repository or experiment worktree root. Current capabilities, evidence and remaining diagnostic questions are in `docs/goals/performance-suite/state.yaml`.

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
target/release/examples/measure wide-rewrite 16 5000000 10 --rows 64 --detail
```

Graph shapes are chain, ring, star, diamond, dense and seeded random. DAG workloads reject cyclic shapes. `--rows` varies edge/relation multiplicity for graph cases, group count for duplicate heads, and probe count for partial joins. `partial-join-hit` provides the corresponding successful-join control. Seed zero uses canonical query order; other seeds reproducibly vary order and random structure. Boolean seed zero uses disagreement constraints; other seeds mix agreement/disagreement.

The oracles check ordered tuple multisets and distinct query identities. Walks require distinct edge occurrences even on a self-loop; DAG proofs count every derivation, not just reachable pairs; duplicate heads check distinct fresh witnesses; partial joins check payloads and exact hit/miss behavior. Boolean graphs enumerate all assignments independently (at most 16 variables), including disconnected and unsatisfiable cases. These bounds belong to the benchmark oracle, not the language.

Vary dimensions independently before attributing scaling: repeated edge occurrences and multiple proofs can legitimately increase output. Equivalent answers alone do not require equivalent cost. Generator timings remain outside parse/prepare/execution measurements. Existing measurement commands report source and cleanup limits separately; the flamegraph wrapper also imposes an external deadline.

`wide-rewrite` varies tuple groups with SIZE and relation arity with `--rows`.
Each group posts two identical occurrences and rewrites both through `p`, `q`,
and `done`: exactly 4 × SIZE applications, one answer and 2 × SIZE residual
tuples. Its independent oracle checks ordered ports, multiplicity and distinct
query identities; arity zero is a duplicate zero-port control. It uses the same
preparation/execution/delivery/validation/allocation/cleanup runner as the other
core cases.

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

Default observation records phase totals, first-event/first-answer latency and periodic budget/memory checks. `--detail` additionally measures per-step maximum latency where supported, validator duration and collection-status samples. Both modes execute the same workload and all semantic validators. Unmeasured detail-only fields are `null`, not zero. Typed records expose first-event measurements directly; the display text is for reading. `source_delivery_ms` includes validation; the separately reported subtraction exists only in detailed mode. First-answer latency is elapsed time before validation of its final event, including work to handle preceding events.

Lifecycle deadline checks occur every 2048 work units and at phase completion; they cannot preempt one engine or API call. Lifecycle memory peaks are sampled every 2048 ticks and at application milestones in baseline mode, every source tick in detailed mode. External supervision is still required for stuck calls, output and destruction. Detailed observation and the allocation/work `diagnostics` build feature are independent; baseline timing uses neither. Mode identity and feature status are printed in each command result.

Check mode equivalence with actual core, generated, notebook, archive and runtime workloads:

```sh
cargo build --offline --release --features diagnostics --example measure
python3 -m unittest discover -s tests -p measure_observation_test.py
```

These checks compare validated endpoints and per-rule work, and require any total iteration difference to be explained entirely by collection dispatch counts. Repeated unchanged synthesis runs already vary in collector work; collection traverses shared variable groups in address order. Treat raw internal work counts as measurements requiring calibration, not universally deterministic language properties. Use controlled comparisons to assess changes in these measurements.

## Opt-in work and allocation diagnostics

```sh
cargo build --offline --release --features diagnostics --example measure
target/release/examples/measure partial-join 16 5000000 10 --rows 8
target/release/examples/measure notebook-behavior-i 1 50000000 15
cargo test --features diagnostics --test diagnostics
cargo test --features diagnostics --example measure
```

`diagnostics=` lines contain JSON checkpoints before cancellation and after cancellation for core/notebook engines and lifecycle cancellation probes. Work counters are cumulative for that engine; subtract checkpoints for interval counts. Rule entries follow prepared-rule order. Indexed candidate visits, direct-anchor matches, found candidates, started/applied/rejected commits and dispatch counts are separate. Dispatch categories sum to entered `advance` iterations, including early stops; they are not CPU proportions. The collection category counts service dispatches, including semantic maintenance, not physical collector CPU. Explicit binary choice births are not a count of independent programs. See `src/engine/diagnostics.rs` for event boundaries.

The `restriction_nodes` memory count reports live inspection-selection metadata. Shared expanded results belong to reader endpoints and active computation frontiers; the `conditions` count includes their Boolean representation. Ordinary execution without inspections has no restriction nodes.

Allocation JSON records process-wide successful allocation/reallocation counts, requested bytes allocated/freed, current live requested bytes and the lifetime high-water mark. A successful realloc counts an old-size free and a new-size allocation. These quantities exclude allocator metadata, fragmentation and internal transient realloc storage; they are not RSS. Categories describe the code executing each allocation or free, not the object retaining it. An object allocated during execution can be freed during cleanup, so per-category allocated-minus-freed is not retained ownership. `other` remains explicit for unscoped work, including report creation and portions of lifecycle/API handling. Inspection work is separate where scoped; source and inspection may otherwise share an engine service turn.

The final `allocations=` JSON is sampled after the workload function returns, before serializing that report and before process shutdown. Earlier reports contribute allocation traffic to subsequent checkpoints. Snapshots are consistent at quiescent single-thread checkpoints; atomics make counting safe across threads but do not make simultaneous counter reads an atomic snapshot. Engine object-count checkpoints remain available alongside byte accounting.

The `diagnostics` feature compiles engine counters and the measure allocator in. Default builds contain neither; these instrumented timings require observer-overhead calibration and must not be substituted for baseline timing. CPU and heap profiles provide source attribution, while scoped lifetime workloads check resource release. Calibration across rewriting, synthesis and runtime sessions found workload-dependent observer effects; use baseline runs for timings and instrumented runs for attribution.

Typed `diagnostics` records also expose existing `store_mutations` counters as
three [unique, copied] pairs for graph, propagation history and pending indexes,
and cumulative `graph_allocations`. A mutation count measures visited mutable
paths, including ownership checks and metadata refresh, not semantic rule work.
Batching can reduce these counts without changing applications or allocation;
charge batch normalization, finalization and release work separately.

The `graph_pages` diagnostic array reports cumulative page record allocations,
copied entries (COW and packing), entry writes, sparse-boundary splits, packed
merges, and entries returned by page cursors. In-place leaf-to-page conversions
count as merges, not new record allocations. Cursor node visits count a page
once; its returned entries are separate work and must also be charged. Sorted
updates additionally shift at most seven entries per write. Pages contain at
most eight entries; their vectors use ordinary bounded capacity growth.

## External limits and process resources

Build the desired binary first, then run it directly:

```sh
python3 examples/supervise.py --measure --seconds 10 --out /tmp/chr-run -- target/release/examples/measure rewrite 128 5000000 5
python3 examples/supervise.py --seconds 10 --out /tmp/chr-cli-run -- target/release/chr examples/proofs.chr --query 'edge(A,B,AB),edge(B,A,BA)'
python3 -m unittest discover -s tests -p supervise_test.py
```

The new output directory contains stdout/stderr and `run.json`: exact command, working directory, host, limits, status, elapsed time, user/system CPU, peak resident KiB, minor/major faults, voluntary/involuntary context switches and filesystem block counts. Campaign metadata records the build command and local environment.

Linux `pidfd` readiness enforces the wall deadline without busy polling. After the deadline, the process group receives an interrupt and then a hard kill after a one-second grace; CPU profiling uses five seconds to flush sampling data. A run remains censored even if it handles the interrupt and exits zero. Live children in the original process group are killed before reaping the leader; termination is checked for up to one second and reported separately. A normal exit with leftover group members is failed; a deadline remains censored with its cleanup result attached. The supervisor manages native CHR/profiler processes that stay in their launch group; it is not containment for commands that detach into another group/session. Launch/pre-exec and kernel-uninterruptible termination are not made preemptible by a user-space deadline. Address-space limits are per process, file-size limits per file, and CPU limits per process; these are not aggregate process-tree or disk quotas. SIGXCPU/SIGXFSZ are identified; other failures retain their status and stderr without guessing whether a cap caused them. `--measure` recognizes that harness's incomplete exit code; arbitrary CLI exit 2 is an error.

Resource accounting comes from Linux `wait4` for the launched process and descendants it waited for. It includes pre-exec/launcher work and, for profiling, perf itself. Peak RSS is a high-water mark, not summed process-tree memory or live allocation bytes. Fork/pre-exec memory can impose a floor on small workloads; `launch_parent_rss_kib` exposes the launch context. Filesystem blocks are kernel accounting, not bytes or syscall counts. These metrics complement engine/allocation measurements; subtracting CPU from wall time does not identify a particular blocking cause.

## Repeated typed measurements

```sh
python3 examples/perf.py --out /tmp/chr-repeat --repeat 5 --seconds 10 --total-seconds 120 -- rewrite 128 5000000 5
python3 examples/perf.py --diagnostics --out /tmp/chr-work --repeat 3 -- partial-join 32 5000000 5 --rows 16
cargo build --offline --release --example measure
python3 -m unittest discover -s tests -p perf_test.py
```

The repeat command builds before timing unless `--binary` selects an existing binary. It records the command, toolchain, host, build environment, revision and dirty-tree status. Prebuilt binary build flags remain unavailable. One warmup is the default; every warmup, unsuccessful sample and completed sample retains stdout/stderr plus an atomically published typed `<index>.json` record. `campaign.json` records outcome counts and per-metric median, range, median absolute deviation, observed sample count and incomplete-or-missing count. Unavailable values remain null. Each observation records a value and whether its measurement interval finished. A censored search can still supply process resource usage and completed cleanup measurements; unfinished source latency stays incomplete. Raw records preserve progress and limits.

`measurement=` lines are versioned JSON emitted directly from measured values. Configuration, result, diagnostics, allocations and lifecycle phase records are separate; named memory gauges replace positional interpretation. The runner does not scrape display text and rejects missing/duplicate/inconsistent final results. Per-phase and per-rule diagnostic summaries retain scope; cumulative checkpoints must not be summed as exclusive work. The aggregate sample budget reserves interrupt/cleanup grace and can end before the requested repeat count, reported as aggregate censoring. A campaign alarm also bounds parsing and summarization; processing interrupted by that deadline retains a censored record and raw output. Prebuilt-binary toolchain/source details are explicitly unverified local context. Build has a separate five-minute cap. This repeat command is infrastructure for routine/deep comparisons; the separate suite and regression commands below provide campaign selection and calibrated comparisons; this command reports regression assessment as not performed.

Shared matching arrangements report `shared_restrictions` alongside engine work in diagnostic measurements and CLI checkpoints. `producer_candidates` counts raw rows inspected while building shared partitions; `indexed_candidate_visits` counts candidates presented to matchers. Include both when assessing row inspection work. Partition lookups, projected ports, reuse, and current/peak retained rows and partitions expose the construction and retention costs. These counters are diagnostics-only.

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

The comparator consumes raw campaign samples with matching workload/observation configurations. It reports mean changes, descriptive medians, ranges and counts, using a two-sided permutation test of absolute mean differences with Holm correction across the requested metrics. Up to 9,999 assignments are enumerated exactly; larger tests use 9,999 seeded permutations with the conservative plus-one estimator. Fewer than five observations per side means insufficient evidence; missing values remain unavailable. The default processing budget is 30 seconds (`--seconds`). Failures, censoring, invalid records and comparison timeouts have distinct outcomes; completed subsets cannot establish the outcome of an incomplete campaign. Raw sample outcomes determine counts.

An increase/decrease is statistical change evidence, not a material regression verdict. Exchangeability/independence assumptions can fail under machine drift or ordered execution; uncertainty does not establish equivalence. Materiality, scaling policy and broad control calibration remain required suite work. Exit 0 means the comparison was calculated, not that performance passed an acceptance gate; exit 1 means failed/invalid evidence and exit 2 means censoring or a processing deadline.

The native result includes `native_peak_rss_estimate_kib`, read once at final-result emission from Linux `/proc/self/status` VmHWM. This covers the native harness address space through that point, including parsing, validation, allocator overhead and reporting so far; it excludes the launcher address space before exec. It remains null when unavailable. [Linux documents VmHWM as approximate](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html); the estimate can move slightly downward as accounting updates. A subprocess calibration touches 32 MiB and verifies both held and released observations expose at least 30 MiB above baseline. This tests a material signal, not exact byte accuracy. Wait4 RSS remains separate launcher-inclusive evidence. Requested-allocation peaks provide a different, precisely defined measurement under diagnostics.

Two unchanged seven-sample `rewrite 512` campaigns gave native peak medians 6,936 and 6,932 KiB, with ranges 6,888–7,084 and 6,816–7,084 KiB. No default metric showed a statistically qualified change; source-delivery medians were 24.84 and 25.68 ms. This is one local benign control, not full false-positive calibration or a universal noise bound.

## Preparation shape, reuse and direct destruction

Round 022 packs Prepared operands, heads and trigger records into shared flat
arenas. The measure allocator now separates `prepare`, `engine_drop` and
`prepared_drop` from setup/cleanup. `prepare` excludes parsing and the caller's
outer Arc allocation. Lifecycle archive/inspection/pending probes report
`final_drop` with separate drop clocks and the final Prepared ownership check.
Diagnostic `representation_bytes` in preparation results (and
`plan_representation_bytes` in engine checkpoints) includes the changed backing
buffers, their capacities/padding and the complete Prepared header; unchanged
name/signature and constructor/update backing is excluded from that gauge but
included in allocator measurements. See the
[Round 022 evidence](optimization-evidence/rehearse/round022-prepared-arena/README.md).

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

Defaults are twelve measured repeats per point without warmup, a five-second external sample limit, and a 120-second routine or 600-second deep execution/analysis budget. Each workload also has a declared three-second native phase limit. Flags can set these external budgets and repeats before execution. Builds occur once with a separate five-minute limit; `--binary` uses an existing executable, and `--diagnostics` builds the instrumented variant. Each point delegates to the same `perf.py` measurement/supervision path. Samples run in rounds: every selected point gets its first measured observation before any point gets its second. The standalone runner still defaults to one warmup. Nested reporting remains subject to the suite deadline, and known outcomes are published before report I/O. The final small suite record is written after the execution/analysis alarm is disarmed.

`suite.json` contains the selected points, commands, risk questions, outcomes, raw campaign paths and adjacent-size scaling. Scaling selects execution time, residency, allocation and work-growth measurements; configuration values and incidental numeric fields remain raw context. Ratios retain both median operands; log-log exponents are descriptive, not complexity proofs. Censored or missing points do not connect a scaling slope. Necessary derivation/output growth remains visible alongside cost. A point interrupted during processing is distinct from a point never admitted. Completed failures remain failures even when later reporting is interrupted. Exit 0 means all requested observations completed; it is **not a regression acceptance gate**. Exit 1 records failed evidence, exit 2 bounded incomplete execution. Use the calibrated comparison command below for material-change signals; use raw observations and profiles to investigate any signal.

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

Use two independently executed unchanged suite campaigns as controls, with the same selected workloads, parameters and observation mode as the comparison. Controls describe observed variation on that host; they are not a confidence bound on future noise. Resolution uses median drift and normal-scaled median absolute deviation; extremes remain visible in raw ranges. The command reads raw validated samples, retains suite and campaign failure states separately, and tests both per-point costs and adjacent-size cost ratios. Scaling compares ratios of means using independent resampling of the four before/after and low/high samples. Sorted inputs make results invariant to record order; resamples are not extra observations. Default metrics cover source/first-answer/cleanup or the corresponding preparation/session phases, native residency and whole-process wall time. Explicit `--metric` selections use the same detection path.

A regression signal requires an increase beyond robust control resolution and qualification within one Holm family covering mean, distribution and scaling tests. Mean costs use permutation tests. Scaling uses a centered independent bootstrap; its inference and the combined family error control are approximate. Means express average cost; medians and raw ranges remain descriptive evidence. Mean direction and ratio use the same statistic as the test. Each hypothesis uses at most 9,999 resamples; coverage size does not increase this work. A family whose threshold exceeds that resolution is reported as resolution-limited. Exact equal-size two-sided tests account for complementary assignments. Mathematically insufficient resolution, fewer than five observations, missing calibration, censoring and unavailable observations stay explicit and cannot establish a clean comparison. Twelve repeats are the suite default because five-sample comparisons cannot resolve many material changes after family correction; repeat count alone does not guarantee sensitivity under noisy data.

The comparison has a declared 300-second processing budget, adjustable with `--seconds`. Exit 1 indicates a regression signal or failed/invalid evidence; exit 2 indicates incomplete/insufficient evidence or timeout; exit 0 means no qualified regression was detected at the reported resolution. It does not establish equivalence, optimal complexity, broad coverage, or acceptable baseline speed. A tie-aware distribution test exposes changes that preserve the mean. A distribution change does not imply adverse direction: observed maxima and exceedance counts do not establish rare-event rates. Small or heavy-tailed samples, drift and near-zero scaling denominators can undermine inference; inspect raw observations when these assumptions are doubtful.

Native calibration used six points across rewriting, partial-hit matching and preparation with nine measured repeats. The held-out unchanged campaign had no qualified regression. An 80 ms startup delay applied outside the native executable produced six process-wall regression signals (observed mean increases approximately 80–82 ms), while native source timing remained separately reported. A large-family check verifies that insufficient resampling resolution is explicit. Native latency, memory and coupled interventions also produced qualified signals while an unchanged control did not; evidence is linked from the current goal state.

## Runtime session lifetime and replay

```sh
target/release/examples/measure runtime-sessions 8 100000 5 --closed 8 --retained 4 --rows 8 --batch 1 --replay-every 2 --work 32
python3 examples/perf_suite.py deep --only sessions-history --only sessions-retained --only sessions-batch --only sessions-replay --out /tmp/session-sweeps
```

This runs the native Runtime API and scheduler directly, without a browser or HTTP server. SIZE controls finite answer count; rows controls residual width. Closed sessions are completed, drained, closed and retired before the measured pair of a main query and a tiny query. Retained sessions remain owned after completion. Batch size counts scalar output events (1–4096). Replay cadence is measured in nonempty batches; zero disables replay. Work is a lower bound on continuing background applications; zero omits that source.

Each finite result has an independent residual/application oracle. Replays repeat an unacknowledged frozen batch after background progress, require identical contents/metadata, and verify that output reads themselves advance neither source. Reports separate setup, source, tiny-answer latency and work position, cleanup and Runtime destruction. Source includes fresh admissions, API handling and validation; detailed read/replay/validator clocks are opt-in. Requests plus scheduler turns form this case's work budget; they are not engine ticks. Setup/source share a budget; cleanup receives a separate equal budget, bounded externally by the sample deadline.

Linux spool evidence counts descriptors and unique logical file lengths belonging to this Runtime. History cleanup, retained-file count, closure/owner retirement and post-drop descriptor release are checked; these observations do not claim full heap reclamation. Other platforms report null spool evidence. Initial deadline exhaustion remains incomplete, and validated answer progress is published before another budgeted operation can exit. The four-dimensional selected sweep completed three samples at all 13 points in 1.82 seconds. Its progress/resource checks and censoring boundaries have dedicated tests.

## Native heap allocation and peak attribution

```sh
python3 examples/heap_profile.py --out /tmp/chr-heap -- notebook-behavior-i 1 50000000 5
python3 examples/heap_profile.py --cli --out /tmp/chr-cli-heap -- examples/proofs.chr --query 'edge(A,B,AB),edge(B,A,BA)'
```

The command uses [heaptrack](https://github.com/KDE/heaptrack) and inferno with a symbolized optimized build. `--binary` selects an existing executable. `--prefix` selects the heaptrack installation prefix, normally `/usr`. On this host the distribution packages are installed locally, so the runnable form is:

```sh
LD_LIBRARY_PATH=/home/ahart/.local/share/heaptrack/usr/lib/x86_64-linux-gnu python3 examples/heap_profile.py --prefix /home/ahart/.local/share/heaptrack/usr --out /tmp/chr-heap-local -- notebook-behavior-i 1 50000000 5
CHR_HEAPTRACK_PREFIX=/home/ahart/.local/share/heaptrack/usr LD_LIBRARY_PATH=/home/ahart/.local/share/heaptrack/usr/lib/x86_64-linux-gnu python3 -m unittest discover -s tests -p heap_profile_test.py
```

Outputs include readable `allocations.svg` and `peak.svg`, their folded stacks, source-attributed text reports, raw heaptrack data and a Massif-compatible live-heap timeline plus `timeline.json`. Allocation flame weights count intercepted native allocation calls; peak weights count requested bytes live at the observed global peak. They include native harness, validation and library work. They are not engine-exclusive ownership, allocator arena size, or RSS. Timeline snapshots can miss short peaks. The native target's actual exit status is retained independently of interpretation/rendering; partially captured traces remain censored even when readable.

Recording and the entire analysis stage each use the shared external supervisor, including Python parsing and renderers. Defaults are 40 seconds for recording and 120 seconds for analysis, with per-process address-space and per-file size limits; these are not aggregate tree quotas. Analysis limits have separate outcomes, and failed process-group cleanup takes precedence over ordinary censoring. Builds have a separate five-minute limit. No allocation tracing occurs in normal execution.

Native calibration checks three known allocation calls, exactly 2.5 MiB live at that call stack's peak, sampled timeline units, actual target exit 7, analysis deadline enforcement and interrupted malformed output. The behavior-I profile in `/tmp/chrimp-heap-final` validated 252,115 allocation calls and a 4,400,971-byte global requested-heap peak. Condition-store allocation and cursor traversal appear among major call contributors. These are instrumented diagnostic observations, not baseline timing or a requirement to optimize those functions.

## Hardware instruction, cache and branch counters

```sh
python3 examples/profile.py --counters --out /tmp/chr-counters -- rewrite 128 5000000 5
python3 examples/profile.py --counters --cli --out /tmp/chr-cli-counters -- examples/proofs.chr --query 'edge(A,B,AB),edge(B,A,BA)'
```

This uses the same native build/supervision path as CPU sampling, with perf instruction, cycle, cache-miss, branch and branch-miss events. Raw perf JSON and typed counter records retain event/PMU names, event runtime and running percentage. Unsupported or uncounted events remain null; they are not zero. Values may be scaled by perf. In particular, this host has separate atom/core PMUs: migration can split exposure, so the report preserves those domains and does not invent an aggregate IPC or sum their scaled counts. Counts include native setup, validation, delivery and cleanup; they are not logical engine work. The `rewrite 128` native check completed with available hardware events.

## Saved notebook coverage

```sh
python3 examples/perf_suite.py deep --notebook-inventory
python3 examples/perf_suite.py deep --only notebook-type-s --only notebook-behavior-m --out /tmp/saved-synthesis
```

The inventory reads all 85 saved queries directly from the four notebook files. It maps all 17 unrestricted synthesis queries to exact native measurement cases: eight type-directed targets and nine behavior-directed targets. `i`, `k`, `ki`, `s`, `b`, `c`, `w`, `t`, and behavior-only `m` denote identity, constant, second argument, substitution, composition, swap, duplication, application to a function, and self-application. Each case executes the saved query without supplying a witness. Independent SK reduction checks each delivered program; type cases additionally use independent finite-simple-type inference. Size requests an answer prefix, with budget exhaustion kept censored.

Other saved queries remain visibly classified as `saved_query`, with no claim of an exact performance case. Arithmetic magnitude/direction and nested lambda identities have separate generated performance cases. Saved arithmetic, lambda, and supplied-program synthesis checks live in `tests/notebook_arithmetic.rs`, `tests/notebook_lambda.rs`, and `tests/notebook_synthesis.rs`; those semantic tests are not substituted for unrestricted-search timing.

## Comparing allocation costs

Diagnostics campaigns expose `allocations.total_allocated_bytes`, `allocations.process_peak_requested_bytes`, and `allocations.phases.<phase>.allocated_bytes` (plus allocations, deallocations and freed bytes). Phase names remain stable if emission order changes. Totals sum exclusive executing-code categories. Live/peak values cover Rust global-allocator requests across the process, including the harness; direct foreign allocations, allocator arena overhead and retaining-owner identity are outside this counter. Cumulative checkpoints remain separate observations and are not summed into traffic totals.

```sh
python3 examples/perf_regress.py /tmp/control-a /tmp/control-b /tmp/before /tmp/after --metric allocations.total_allocated_bytes --metric allocations.process_peak_requested_bytes --out /tmp/alloc-regressions.json
```

Default regression comparisons include total requested allocation traffic and the requested-byte peak when the campaigns use the diagnostics feature. Baseline campaigns do not claim these observations. The raw-record control checks that a fourfold allocation increase reaches both explicit and default detection paths, including phase reordering and malformed duplicate records.

## Work diagnostics for arbitrary native queries

```sh
cargo build --offline --release --features diagnostics --bin chr
target/release/chr examples/reachability.chr --query 'edge(A,B),edge(B,C)' --diagnostics
```

Answers use the ordinary stdout graph stream. The optional stderr JSON report contains source and after-cancellation work/memory checkpoints, rule names, source completion, and phase durations. Work is cumulative over this one Engine; subtract checkpoints for cleanup work. Source elapsed includes output writes and flushing; load/prepare/init includes file reading. Diagnostic serialization and final Rust value destruction are outside these phase clocks. Cancellation finishes before diagnostic output is written, including when the answer sink failed. A still-live Engine retains its current coordinate epoch after cancellation; this is visible separately from released execution objects.

This report is emitted when execution returns, not periodically. An externally terminated continuing query can therefore lack it; CPU/heap profiling still provides bounded evidence for those inputs. Builds without the diagnostics feature reject the flag before evaluation. Ordinary execution requests no diagnostic clocks or report.

## Independent native detection challenge

The existing runtime-session workload detected independently introduced spool-read delay, a touched 64 MiB mapping retained through execution, and their combination. Twelve measured samples per variant preserved the workload's exact output/progress/resource checks. Tiny-answer median latency increased from 3.62 to 16.66 ms; process peak RSS from 4,496 to 70,002 KiB. Each intervention signaled regression, and the bracketing unchanged control signaled none. All 79 observations including feasibility and warmups completed in 6.26 seconds of summed native execution.

The temporary interposer affected real native reads and preserved returned data. These observations establish latency/residency sensitivity and a coupled control, not an engine leak, allocator-accounting calibration, scaling proof, or rare-event rate. Raw local evidence is in `/tmp/chr-native-challenge`; routine use requires only the maintained commands.

Shared-service diagnostics separate entered collection phases, compaction-owned Boolean/transform work, persistent-index substitution steps, coordinate publications/retirements, and search/completion coordinate transport. Counters include cancellation and explicit maintenance. Nested operation work is a subset of the enclosing service and cannot be summed as exclusive CPU time. Conditional work outside those measured responsibilities remains explicitly unavailable; CPU and heap stacks provide broader source attribution. Calibration checks actual physical maintenance, conditional compaction and coordinate retirement. Observer checks require the measured graph-pruning/arena traversal difference to account for the permitted collection-iteration difference.

Coordinate diagnostics now separate `transform_starts` from exact
`epochs_crossed`: a composed or cached prefix can cross several epochs with
one or zero transforms. `coordinates.segments` reports segment creation/release,
composition eligibility probes, copied entries, builds, hits and invalidations,
and exact input/result prefix probes, publications, hits and invalidations.
Gauges report retained source maps, segments, composition maps/assignments and
prefix results. The engine retains at most one composition of eight constant
images and one completed prefix result; live transforms separately own any
images still in use. The public `coordinate_records` count preserves original
source-map, assignment and epoch records and adds run/cache metadata; it is
neither bytes nor a count of Boolean nodes reachable through a cached root.
Composition construction/release processes at most eight entries; prefix reuse
compares exact source/target boundaries and input identity. Functional images
and nonsingleton deltas use the existing per-epoch transformation.

The diagnostic unit test
`segmented_transport_work_scales_at_equal_publications_and_reader_cutoffs`
compares actual per-epoch transformations with segmented transport at 8, 64
and 512 publications, validates three identical endpoints, prints raw work and
retention counters, and checks final map release. It is a mechanism probe,
not an end-to-end notebook or allocation benchmark.

Observer costs can be compared explicitly while holding the workload fixed:

```sh
python3 examples/perf_compare.py /tmp/baseline /tmp/detailed --observer-overhead --metric workload.times_ms.source_delivery --out /tmp/observer-cost.json
```

Only `detailed` and `diagnostics_feature` may differ in this mode; query size, inputs, budgets and other configuration still must match. The report lists observation changes and retains raw distributions. Ordinary comparisons reject mode changes. An observed ratio describes those workloads and samples, not a universal instrumentation surcharge.

The all-target synthesis selection completed in 67.58 seconds with one warmup and one measured sample per query: seven validated first-answer prefixes and ten censored samples at the declared limits. These establish execution/oracle coverage and preserve expensive searches, not stable timing estimates. Local results are in `/tmp/chrimp-all-synthesis-queries`.

## Conditional turnover with retained readers

```sh
python3 examples/perf_suite.py deep --only life-archive-rotate-conditional-owners --only life-archive-rotate-conditional-duration --out /tmp/conditional-readers
```

The four lifecycle interaction cases also accept `-conditional`: held output, fixed archive, rotating archive, and partly unread inspectors. Owner-count and continuing-work sweeps vary independently. Snapshot/inspection cases keep a fixed relational payload while a nullary rewrite creates explicit choices with a known surviving arm. Projection selects that surviving prefix; it does not enumerate unfinished failing histories. Archives overlap capture and release, while inspector cases release the snapshot handle so inspectors themselves must retain the old state.

The oracles verify exact committed payloads, newly born choices, retained choice pins through collection, reader completion, and release while the source is still live. Projection-only draining after the declared source-work prefix must perform zero additional source applications. The report separates admitted, held, resumed, and unpinned phases, followed by cancellation and final reclamation. Work and snapshot lifetimes can legitimately amplify inspection/reclamation cost; larger cases retain honest censoring at their declared limits.

## Constructor source probe

`constructor_probe` uses the same automatically prepared execution plan as the CLI and notebook. It accepts a source file and query, or the saved behavior-I query:

```sh
cargo build --offline --release --example constructor_probe
target/release/examples/constructor_probe --behavior-i --first --steps 5000000
target/release/examples/constructor_probe program.chr --query 'app(R,A,B),app(R,C,D)' --steps 500000
```

The probe reports source progress and bounded cleanup separately. A cleanup limit is a censored release, not a completed timing. Its normalization counters describe admitted work, not CPU shares. Preparation-reuse measurements use the shared immutable prepared plan; source recognition is not repeated for each engine. Historical comparison modes remain on the research branches.

## Pending syntax promotion probes

`life-pending-snapshot` and `life-pending-cancel` stop at an exact unposted
prefix: SIZE scheduled tasks, no posted facts or applications, and SIZE
relations of arity SIZE, all using the one query identity. SIZE must be at
least two. These probes separate preparation, source admission, first and
second capture, cancellation, projection/validation, release, and final
Engine/Prepared destruction. Snapshot mode validates both retained views
after cancellation: exact relation multiplicity, each ordered port, and zero
source applications. Cancellation mode measures the same prefix without a view.

```sh
cargo build --offline --release --features diagnostics --example measure
python3 examples/perf.py --binary target/release/examples/measure --out /tmp/pending-syntax --repeat 5 --warmup 0 --seconds 10 --total-seconds 60 -- life-pending-snapshot 64 50000000 3 --detail
```

Before the first syntax view, ordinary execution owns remaining expressions in
live bodies. Completion certificates remain persistent. First capture scans
queued/parked tasks and materializes live body descriptors synchronously;
subsequent captures retain persistent roots. Cancellation performs this barrier
one task per service turn, preserving views requested during discard. The
diagnostic counters `syntax_promotions`, `syntax_promotion_tasks`, and
`syntax_descriptors_materialized` distinguish this displaced work from ordinary
execution. History-enabled engines start on the persistent path.

The raw `pending_*` phase clocks describe completed probe boundaries; the
general comparator may keep their latency summaries unavailable because the
overall source is intentionally unfinished. Do not interpret that as zero cost.

## Certified structural storage probes

The four retained-reader interaction cases also accept `-structural`. They use
cyclic ternary payloads with source-defined structural consistency rules and
validate every ordered field identity. Archive/inspector payloads remain static
beside a separate live rewrite driver; draining projections must perform zero
additional source applications. For example:

```sh
python3 examples/perf.py --binary target/release/examples/measure \
  --out /tmp/structural-retention --repeat 5 --warmup 0 --seconds 10 --total-seconds 60 -- \
  life-archive-fixed-structural 4 50000000 3 --rows 64 --work 64 --cadence 4
```

Notebook result records include `times_ms.engine_and_prepared_drop` after the
existing cancellation/collection phase and `prepared_released`, a check that
final destruction released the last strong prepared-program owner. With
`--detail`, validator time remains a subset of source-to-End time, not an
additional phase to add twice. Diagnostic records expose preparation recognition
and sparse-update plan costs plus executed/avoided field-index writes and raw
lookup fallback work. See the [Rehearse 001 evaluation](optimization-evidence/rehearse/round001-theory-derived/README.md)
for scope, common-baseline harness patch, retention evidence and limitations.
