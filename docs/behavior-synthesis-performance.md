# Behavior synthesis performance — 13 September 2026

Four of nine unrestricted behavior-synthesis queries find an answer within three seconds. The other five spend substantial CPU on conditional-state maintenance and matching, while their live memory grows. The profiles locate those costs; they do not establish how much is necessary for the search.

Five baseline observations per saved query, round-robin, no warmup, optimized release build without diagnostics or detailed observation. Each query uses its actual synthesis program without supplied witnesses. The native phase limit is three seconds, external sample limit five seconds, and suite budget 150 seconds. All 45 observations ran in 83.15 seconds. Completed timings cover source delivery and validation of the first answer; they are not search-exhaustion times. Capped results are not completion-time estimates.

| Query | Answer time: median (range), ms | Median peak RSS, MiB | Median cleanup, ms | Profile |
|---|---:|---:|---:|---|
| I | 130.7 (117.6–154.9) | 8.6 | 1.0 | [CPU flame graph](/tmp/chr-behavior-cpu-i/flame.svg) |
| K | 17.6 (17.2–19.3) | 5.6 | 0.1 | [CPU flame graph](/tmp/chr-behavior-cpu-k/flame.svg) |
| KI | 117.2 (110.1–122.4) | 8.0 | 1.1 | [CPU flame graph](/tmp/chr-behavior-cpu-ki/flame.svg) |
| S | 500.6 (475.4–566.7) | 16.7 | 5.5 | [CPU flame graph](/tmp/chr-behavior-cpu-s/flame.svg) |
| B | No answer within 3,000 ms (5/5) | 95.9 | 208.0 | [CPU flame graph](/tmp/chr-behavior-cpu-b/flame.svg) |
| C | No answer within 3,000 ms (5/5) | 67.6 | 47.0 | [CPU flame graph](/tmp/chr-behavior-cpu-c/flame.svg) |
| W | No answer within 3,000 ms (5/5) | 85.0 | 274.4 | [CPU flame graph](/tmp/chr-behavior-cpu-w/flame.svg) |
| T | No answer within 3,000 ms (5/5) | 79.3 | 40.1 | [CPU flame graph](/tmp/chr-behavior-cpu-t/flame.svg) |
| M | No answer within 3,000 ms (5/5) | 76.9 | 21.2 | [CPU flame graph](/tmp/chr-behavior-cpu-m/flame.svg) |

All 45 runs completed cancellation within its limit and returned engine ownership counts to zero except the current engine's one coordinate record. RSS is the whole harness peak, not exclusively retained engine storage.

The five capped searches have similar CPU costs. These are inclusive percentages of sampled user-space CPU across the whole process, including cleanup; nested rows overlap and must not be added.

| Query | Samples | Collection/compaction (`collect_heap_mode`) | Boolean evaluation (`condition::Job::tick`) | Matching (`Matches::tick`) |
|---|---:|---:|---:|---:|
| B | 1,494 | 34.7% | 25.6% | 19.0% |
| C | 1,489 | 35.3% | 26.3% | 19.4% |
| W | 1,496 | 31.7% | 28.1% | 21.2% |
| T | 1,477 | 37.5% | 27.2% | 17.5% |
| M | 1,481 | 38.6% | 26.0% | 18.8% |

The collection path includes representation compaction and condition transformations, beyond physical reclamation. Boolean evaluation repeatedly reads and constructs condition nodes. `Arena::node` maintains a node map and a uniqueness table (`src/condition.rs:253`); these structures also appear prominently in heap profiles. Matching repeatedly traverses relation membership and store cursors. Owner-set lookups alone occupy 6.6–8.6% of self samples in these five runs. This is evidence of substantial representation and bookkeeping cost, not evidence that every such operation is avoidable.

W and M have separate heap profiles, each using a three-second native limit, eight-second external recording limit and forty-second analysis limit. Instrumentation changes the amount of search reached, so their heap values must not be compared directly with baseline RSS or treated as baseline timing.

| Query | Allocation calls | Peak requested live heap | Allocation calls under matching | Peak bytes allocated through condition-node construction |
|---|---:|---:|---:|---:|
| W | 2,459,854 | 38.8 MiB | 49.0% | 55.6% |
| M | 2,539,554 | 41.5 MiB | 44.0% | 49.4% |

W's sampled live heap grows from 7.6 MiB near 0.5 seconds to 14.8 MiB near one second and 25.7 MiB near two seconds. M grows from 8.7 to 13.2 to 15.7 MiB at those times. Growth is not monotonic: collection releases memory during execution. Both timelines end at 544 bytes after shutdown. Peak allocation stacks identify where live memory was allocated, not its exclusive current owner.

[W allocation flame graph](/tmp/chr-behavior-heap-w/allocations.svg), [W peak heap](/tmp/chr-behavior-heap-w/peak.svg), [W timeline](/tmp/chr-behavior-heap-w/timeline.json).
[M allocation flame graph](/tmp/chr-behavior-heap-m/allocations.svg), [M peak heap](/tmp/chr-behavior-heap-m/peak.svg), [M timeline](/tmp/chr-behavior-heap-m/timeline.json).

Remaining uncertainty: these bounded observations cannot predict when B/C/W/T/M will produce an answer, count distinct candidate programs explored, or separate logically necessary search from avoidable representation amplification. CPU shares vary with collection phase and address-dependent execution details. K's nine CPU samples are too sparse for attribution; its repeated baseline timing is usable. I and KI have 66 and 67 samples, S has 313. No engine or program changes were made.

Reproduce the baseline:

```sh
python3 examples/perf_suite.py deep --out /tmp/behavior-baseline --repeat 5 --seconds 150 --sample-seconds 5 --only notebook-behavior-i --only notebook-behavior-k --only notebook-behavior-ki --only notebook-behavior-s --only notebook-behavior-b --only notebook-behavior-c --only notebook-behavior-w --only notebook-behavior-t --only notebook-behavior-m
```

Profile an individual query (substitute its letter):

```sh
python3 examples/profile.py --out /tmp/behavior-w-cpu --seconds 8 -- notebook-behavior-w 1 50000000 3
LD_LIBRARY_PATH=/home/ahart/.local/share/heaptrack/usr/lib/x86_64-linux-gnu python3 examples/heap_profile.py --prefix /home/ahart/.local/share/heaptrack/usr --binary target/profiling/examples/measure --out /tmp/behavior-w-heap --seconds 8 --analysis-seconds 40 -- notebook-behavior-w 1 50000000 3
```

[Raw baseline campaigns](/tmp/chr-behavior-perf/suite.json) preserve every sample, its work and memory counters, outcome and command. Each profile directory preserves native logs, folded stacks and validated metadata. Profiles use an optimized build with debug symbols and frame pointers; build and postprocessing have the maintained tools' separate deadlines.
