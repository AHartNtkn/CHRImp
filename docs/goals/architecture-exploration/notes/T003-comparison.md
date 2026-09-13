# Whole-head comparison: a selective gain, not a universal runtime

Prototype commit c95cc9f6b9231c84f67564930327e198fdc79ddd, branch codex/arch-whole-head, worktree /tmp/chr-arch-whole-head. Only examples/arch_whole_head.rs is task-owned there. Nine release tests pass, including exact Engine agreement, fresh identity sharing, duplicates, repeated ports, cyclic/disconnected heads, all 24 witness head orders, and explicit unsupported-input rejection.

The source-driven prototype compares two plans in the same executor. On-demand uses greedy indexed lookup, choosing the smallest bucket. Whole-head first seeks one witness per component, retaining no component-result table; an empty component suppresses full enumeration. Same-signature heads belong to the same component because they compete for distinct occurrences even without shared variables.

A 23-second campaign used the maintained supervisor, five round-robin repetitions, rotated mode order, a three-second external deadline, 0.1-second interrupt grace and a 1 GiB address-space cap. Fifty-five completed observations passed exact ordered relational-multiset/query-identity checks; five Engine runs at empty-256 were censored. No engine changes or concurrent timing campaigns.

| Input | Existing Engine ms | On-demand ms | Whole-head ms |
|---|---:|---:|---:|
| Empty disconnected join, 64 rows/relation | 606.8 | 17.18 | 3.26 |
| Empty disconnected join, 256 rows/relation | All five capped at 3 seconds | 674.6 | 4.76 |
| Empty connected cyclic join, 64 rows/relation | 18.98 | 3.17 | 3.32 |
| Dense successful join, 32 rows/relation | 74.90 | 3.35 | 3.51 |

Numbers are medians of completed total process observations; no completed runtime is assigned to capped runs. They include startup, input preparation, evaluation, exact canonical output, serialization and teardown. All modes also pay the prototype's index/component setup. The Engine mode therefore is an attribution control inside this harness, not an unmodified CLI latency claim. Small prototype process times are largely launch/setup/output; no material advantage is claimed from small differences between them.

The stronger within-executor attribution is explicit work. At size 256, on-demand examines 16,843,008 candidates; whole-head examines 258 and performs no full enumeration. Median evaluation/collection time is 669.43 versus 0.0619 ms, while total process time differs by about 142x. At size 64 the counts are 266,304 versus 66. The cyclic case requires 64 candidates under either plan. The dense case must enumerate 1,024 successful tuples under either plan; whole-head adds two feasibility probes.

## Decision

Selective whole-head feasibility is supported for this adverse static regime. The cubic empty-product traversal is unnecessary, and a reasonable smallest-bucket greedy plan alone does not avoid it. Maintained result tables are not needed to obtain this gain on an immutable store. The experiment therefore does not justify adopting a general maintained-view subsystem.

Both compact executors also greatly outperform the existing engine on the controls where feasibility adds no gain. Those differences cannot be credited to whole-head planning: the prototype specializes a much narrower semantic class and avoids the generic incremental runtime. This is evidence for investigating execution/representation lowering, not evidence that its omissions are acceptable in a full replacement.

Scope: atom-only inputs, pure propagation, terminal atom outputs; no equality updates, consumption, recursive feedback or explicit search. Fresh variables are supported with an eight-variable-per-rule exact-output-check limit. Answers are materialized before delivery. Dynamic invalidation, fair search, common-computation sharing and sustained reclamation remain untested by this prototype. Any adoption recommendation must account for them, but productionizing this prototype is not an independent obligation.

Next prioritize the separately established structural-consistency lowering opportunity in actual synthesis. A targeted code-level feasibility study is active to define an experiment including conditional field merging and the remaining evaluator rules. Keep whole-head feasibility as a candidate execution-plan capability; assess dynamic integration when comparing the leading complete configurations rather than extending this static prototype by default.

Evidence: /tmp/chr-whole-head-comparison/results.json, per-observation stdout/stderr and generated inputs; exact campaign driver /tmp/chr-whole-head-compare.py. Build/test: cargo test --offline --release --example arch_whole_head; cargo build --offline --release --example arch_whole_head. CLI: target/release/examples/arch_whole_head MODE PROGRAM --query-file QUERY, where MODE is on-demand, whole-head, engine or compare. Compare mode verifies all three exact answers and is not used for timing.
