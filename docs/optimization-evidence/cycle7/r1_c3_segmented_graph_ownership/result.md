## OPTIMIZATION RESULT

**Slug / round:** r1_c3_segmented_graph_ownership / cycle 7, round 1, candidate 3
**Kind:** performance
**Verdict:** BLOCKED
**Reason:** User stopped the bounded trial during implementation. Instrumentation and a partial region type conversion exist; no executable ownership mechanism, candidate validation, or candidate comparison was completed. The hypothesis remains unresolved.
**Worktree / branch / candidate commit:** /tmp/chr-cycle7-opt-segmented-graph-ownership / codex/opt/segmented-graph-ownership / none. HEAD 5f91f1e16b211c5ab093f3359b2f582a39dabf91 contains only instrumentation and experiment declaration. Partial implementation remains uncommitted in src/store.rs.
**Baseline manifest / source commit:** manifest.json and baseline-build/run.json in this evidence directory / 16e55baddf9bbd72cf315dd85e7a47221c814bd4. The preserved baseline binary includes the separately committed instrumentation; its build record correctly reports the earlier dirty baseline revision.
**Protocol / helper identity:** SHA-256 51c48a61afd54a188f2a907232c6b0ba6ca7711c02bb6d832b0e4f843e28e798 / b384ab842efb90cf915938bae7ee029f7ac886c6ad55c207402ea7c2f3c0e97c. Helper not run; architecture verdicts use diagnostics, never timing.
**Evidence directory:** /tmp/chr-cycle7-opt-segmented-graph-ownership/docs/optimization-evidence/cycle7/r1_c3_segmented_graph_ownership
**Correctness:** Instrumented baseline: `cargo build --offline --release --features diagnostics --example measure` passed. `cargo test --offline --release --features diagnostics --test store --test graph --test graph_prune --test arc_ownership --test inspection --test history --test cancel --test matching --test collection_work` passed 72 tests, covering persistence, conditional support, multiplicity, retained readers, cancellation and release. Raw stdout/stderr and exact commands are in baseline-build/ and baseline-focused/. These checks predate the partial implementation and establish no candidate correctness. Candidate build/tests, full suites, docs, formatter, Clippy and Python checks were not run. Closeout `git diff --check` passed (whitespace only).

### Workloads

Declared in hypothesis.md: fresh-contract64/256, fresh-unmerged64/256 (all rows4/depth4/interleaved), rewrite128, life-inspections-conditional2/rows3/work24, and partial-join512/rows128. All have zero observations. Planned units were ownership operations, records, bytes and semantic work over preparation through reclamation; two diagnostic observations per configuration, zero demonstrated adverse ownership/retention tolerance for KEEP. No timing inference or allocated alpha, no effect estimate or uncertainty, and no acceptance gate evaluated.

### Files Changed

Committed instrumentation: src/store.rs, src/engine.rs, examples/measure.rs. Committed experiment artifacts: hypothesis.md, run.py in this evidence directory. Uncommitted implementation: src/store.rs, partial Region/NodeRef/LocalBranch conversion. Untracked durable artifacts: manifest.json, result.md, baseline-build/, baseline-focused/, binaries/baseline-measure. Campaign state and parent checkout unchanged.

### Insights

Source inspection establishes that public roots, cursor roots and weak subtree witnesses participate in identity and retention obligations. The proposed three-record region would replace internal owning links with scalar slots while preserving external region/slot identity. Allocation, update and traversal integration was not completed; grouped reclamation was not exercised. There is no measured avoided ownership work, survivor-copy cost, retained-reader cost or release reduction. This stop does not reject segmented ownership generally. No variant was started; nothing is eligible for integration.
