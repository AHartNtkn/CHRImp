# Validation

The implemented language includes variable-only relation arguments, explicit equality, nonbinding multihead matching, all three CHR head modes, fresh body variables, explicit disjunction, committed rule scheduling and residual answers without deduplication. The local notebook supports text and graph editing, ordered-port diagrams, execution controls, alternative inspection and optional history.

## Evidence

| Requirement | Evidence |
|---|---|
| Parsing, variable scope and common editor model | `tests/syntax.rs`, `tests/program.rs`, `web/notebook.test.mjs` |
| Nonbinding matching, equality, consumption and propagation | `tests/matching.rs`, `tests/identity.rs`, `tests/commit.rs`, `tests/semantics.rs` |
| Explicit search, shared execution and exact answer multiplicity | `tests/semantics.rs`, `tests/balanced.rs`, `tests/output.rs` |
| Finite progress beside divergence and bounded delivery | `tests/progress.rs`, `tests/step.rs`, `tests/output.rs` |
| Continuing reclamation and retained-state ownership | `tests/lifecycle.rs`, `tests/compaction.rs`, `tests/cancel.rs`, `tests/arc_ownership.rs` |
| Notebook protocol, editing, recovery and recording | `tests/notebook.rs`, `tests/inspection.rs`, `tests/pending_body.rs`, `web/*.test.mjs` |
| Usable examples | `examples/reachability.chr`, `examples/proofs.chr`, `examples/synthesis.chr`, tested in `tests/semantics.rs` |

At `20db986`, all 295 Rust tests and strict Clippy passed. The notebook session suite passed 103 tests; all four web test files passed. Real-browser checks exercised editing, execution, stepping, pause/resume, alternative selection, history and release. Retained `keep(A),p(A)` inspection survived a subsequent application and cancellation with the release-queue implementation.

Fair service includes source work, completion, observation and reclamation. A frozen pending-work view excludes unfinished scopes; children are admitted before their parent obligation retires. The FIFO mutation lane revalidates against current state before publication. Immutable readers and explicit snapshots retain their dependencies. Tests exercise these obligations under divergence, conditional updates, collection and cancellation; they are not a formal proof over every program.

## Performance and limits

Measurements cover sparse/dense joins, aliases, high-degree merges, cycles, common and distinct alternatives, failure, continuing execution and retained views. The executable `examples/measure.rs` checks exact tuples, identities and multiplicity and reports preparation, execution/delivery, disposal and sampled storage separately.

Historical equivalent-control measurements at `294c010` versus CHRLang `4ed9c045` found size-128 runtime ratios of 13.29 for aliases, 8.87 for sparse joins, 3.10 for dense joins, 0.323 for cycles and 0.520 for 128-way common work. Ratios below one favor this engine. Two-way common work and distinct work also had substantial overhead. These are historical results, not timings of the current revision or a universal performance prediction. Later changes reduced redundant work and allocation, with modest or mixed timing gains. No numerical performance-parity target was specified.

Sharing avoids repeated common execution, not necessary answer enumeration. Output and explicitly retained history can grow. Continuing-memory tests distinguish bounded live workloads from growing frontiers; sampled object counts are not exact byte peaks or comparative RSS measurements. The older CHRLang checkout is currently unavailable at its recorded path, so its historical comparison cannot be rerun there.

## Running checks

```sh
cargo test --offline --all-targets
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
node web/notebook.test.mjs
node --test web/*.test.mjs
cargo run --release --offline --example measure -- sparse 128
cargo run --release --offline -- --notebook
```

Socket tests require loopback access. Choose validation according to the behavior and ownership affected by a change. A commit alone does not require another browser session, performance matrix or complete test run.
