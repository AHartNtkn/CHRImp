## OPTIMIZATION RESULT

**Slug / round:** r2_c1_unused_port_index_omission / cycle 7, round 2, candidate 1
**Kind:** simplification
**Verdict:** BLOCKED
**Reason:** User bounded the trial. An executable omission mechanism and focused
oracles exist, but required correctness is unresolved and there are zero
baseline/candidate workload observations. No gain, regression, or integration
eligibility is established. No runtime or timeout informs this verdict.
**Worktree / branch / candidate commit:** /tmp/chr-cycle7-opt-unused-port-index /
codex/opt/unused-port-index / no candidate implementation commit. HEAD is
cc6ce6a78fbc83b283b2ddce1ad7594d65d8ac92 (symmetric instrumentation and hypothesis
only). Implementation and added tests remain uncommitted and preserved.
**Baseline manifest / source commit:** initial HEAD
bcb215e82a9b1279e0858840c13359d39123a2bf; its source is identical to campaign
505a107 (only two campaign documents differ). Instrumented baseline source is
cc6ce6a78fbc83b283b2ddce1ad7594d65d8ac92. Binary target/baseline-measure,
SHA-256 313512110f8d74ec85a8f1e694a36c525db4ebb9c3bb5bc8986e470bdcd81c07.
No workload manifest was generated because no workloads ran.
**Protocol / helper identity:** repository docs/optimization-loop.md and user
architectural requirements supersede timing gates in the archived worker and
measurement protocol at
docs/optimization-evidence/cycle7/r1_c2_central_occurrence_support/protocol/.
No timing analysis helper executed. Local run.py wraps maintained
examples/supervise.py; no new performance framework.
**Evidence directory:**
/tmp/chr-cycle7-opt-unused-port-index/docs/optimization-evidence/cycle7/r2_c1_unused_port_index_omission/

### Correctness / commands

- `cargo build --offline --release --features diagnostics --example measure`
  passed for instrumented baseline; preserved as target/baseline-measure.
- `cargo test --offline --release --features diagnostics --test graph --test program --test matching --test normalization_observation --test shared_restrictions`
  passed 42 tests on the initial omission implementation. Output is in the task
  transcript; no durable log for that invocation. A preceding compile attempt
  found a missing new cursor field initializer, corrected before this pass.
- `python3 docs/optimization-evidence/cycle7/r2_c1_unused_port_index_omission/run.py check focused cargo test --offline --release --features diagnostics --lib --test unused_port_observation --test graph --test graph_prune --test matching --test normalization_observation --test shared_restrictions`
  failed during linking with LLVM thread-resource errors. Logs and exact command
  are in focused/. Supervisor completed process-group cleanup.
- The same command with label `focused-j2` and `cargo test -j 2` built
  successfully. Library tests: **94 passed, 1 failed**. The failure is
  `engine::collection::tests::routine_pressure_preserves_fifo_and_emergency_or_explicit_gc_bounds_writer_growth`,
  src/engine/collection.rs:991, assertion `deferred && deferred_ticks > 0`.
  The integration-test executables were not run because the library test failed.
  Logs and exact command are in focused-j2/. Exit 101, no descendants remained.
- Both new library tests passed: source-head certificate across lowering and
  exact signatures; filtered lookup with ordered duplicate occurrences,
  conditional liveness/removal, pinned roots through collection, and zero final
  graph nodes/occurrences. The latter calibrates 260 omitted writes, 6 fallback
  lookups and 641 scanned rows in the omitted-index fixture, versus zero fallback
  work in its standalone indexed control. These are unit calibration counts,
  **not campaign workload measurements**.
- `cargo fmt` ran after source changes. `git diff --check` passed on the earlier
  instrumentation revision. No final full-suite, doc-test, Clippy, Python-check,
  or candidate measure build/measurement claim. The new late-observation test
  is copied from the existing round-1 semantic fixture but was not executed in
  the last validation command, which stopped at the library failure.

### Workloads

**Zero observations on both sides.** Declared fresh-contract groups64/256,
copies4/depth4; fresh-unmerged64; one-copy/depth-zero controls; rewrite128;
partial-join128/rows8; retained archive were not run. Thus setup allocation,
total/phase bytes, omitted/retained workload writes/nodes, collection/pruning,
release work, fallback scans, live/peak retention and cleanup comparisons are
unavailable. Statistical alpha, runtime effect bounds and timing gates are not
applicable to this architectural diagnostic trial.

### Files Changed

Committed instrumentation: src/graph.rs, src/store.rs, src/engine.rs,
examples/measure.rs, and hypothesis.md. Uncommitted mechanism: src/program.rs,
src/graph.rs, src/graph/restriction.rs, src/engine.rs. Added uncommitted semantic
fixture: tests/unused_port_observation.rs. Local evidence: run.py, result.md,
focused/ and focused-j2/. No landing checkout or shared baseline was modified;
no variant was started. Worktree and binary are retained.

### Insights / introduced costs / obligations

The source certificate is feasible: reuse every kept/removed source-head
activation entry before constructor lowering, then create one presence flag per
signature. Prepared plans are immutable. All source-head relations remain
indexed, including normalized heads. Standalone Graph constructors retain their
existing port indexes. Certified graphs omit only PORT writes for absent-head
relations, while keeping FACT, RELATION, INCIDENCE, immutable tuples and IDs.
Fallback filters a pinned relation cursor and returns its root-specific support;
exact port_count scans through that fallback. Shared restrictions reject absent
PORT indexes so they cannot use an empty subtree as a false version witness.

Introduced costs: per-signature preparation pass/flags; per-engine flag cloning;
per-update policy branch; larger Occurrences cursor with optional filter;
per-return filter branch for all cursors; O(relation rows) omitted-port lookup
and exact count, with payload lookups and possible many rows per next() call.
Diagnostics additionally charge graph writes, atomic fallback counters and graph
release pairs. Allocation and displaced scheduling/maintenance costs remain
unmeasured. The pressure assertion failure may concern changed pressure rather
than an outcome violation, but this was **not diagnosed or waived**.

Required obligations remain unchanged: equal-tuple multiplicity and fresh
identities; all committed application/control boundaries; ordinary facts;
late snapshots and stepping; retained/history inspection across collection;
cancellation without further source applications; final reclamation. Partial
passes do not establish the full set on the current candidate.

**Strongest next test:** after renewed authorization, reproduce the failed
collection-pressure test on the immutable baseline and current candidate, then
determine whether omitted nodes change its pressure premise or violate FIFO/
emergency collection behavior. Preserve the obligation rather than weakening
the assertion to pass. Only after resolving correctness run the declared
maintained fresh64/256 diagnostic comparison and controls to assess net avoided
work, allocation, observation and release. No continuation or variant is owed
under this bounded result.
