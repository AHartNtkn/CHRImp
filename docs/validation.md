# Validation and measured limits

The language and notebook have executable semantic and lifecycle coverage. Historical comparisons show substantial remaining overhead in several regimes, alongside gains from shared execution. This record does not claim the overall goal complete; identity and root-transfer work remains active.

## Current validation, separated by revision

These results were reported by the integration owner in the implementation session on 2026-09-12. They are not measurements of the same revision as the historical performance tables.

| Revision | Evidence |
|---|---|
| `f924e45d8a59945d07baac5f782a643a7d28bbac` | 291 all-target Rust tests, Clippy and formatting passed after direct body-scope filtering. |
| `56e158a8246fad6616661e8036350e3fa32eada9` | 290 Rust tests, Clippy and release build passed. Native browser history inspection preserved State 3 with `keep` and `p` across Step and cancellation. This is the latest release/native evidence recorded here. |
| Browser session evidence supplied with this record | `node web/notebook.test.mjs`: 103 tests passed. `node --test web/*.test.mjs`: four test files passed; this is not a count of four individual assertions. |

Run current checks from the repository root:

```sh
cargo test --offline --all-targets
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
cargo build --offline --release
node web/notebook.test.mjs
node --test web/*.test.mjs
```

The HTTP integration test requires permission to bind a loopback socket. The native browser result is a manual integration observation, not implied by Node success. Subsequent commits need their own release/native qualification.

## Coverage and ownership argument

| Responsibility | Concrete evidence |
|---|---|
| Variable-only language and shared editor AST | [syntax tests](../tests/syntax.rs): head forms, precedence, names, exact nested-container round trips, full depth-128 source/JSON boundary and malformed raw input. [program tests](../tests/program.rs): prepared slots, scopes and instructions. |
| Nonbinding matching and legal commitment | [matching](../tests/matching.rs), [commit](../tests/commit.rs), [merge delta](../tests/merge_delta.rs): ordered distinct occurrences, repeated-port guards, backtracking/anchors, conditional aliases, stale kept/consumed heads, novel propagation support and fresh locals. |
| Explicit search and shared applications | [semantics](../tests/semantics.rs), [balanced choices](../tests/balanced.rs): duplicate answers, nested inactivity, failure, correlated identity, common work and interaction. [Proof](../examples/proofs.chr) and [synthesis](../examples/synthesis.chr) examples have executable semantic tests. |
| Finite progress and completion | [progress](../tests/progress.rs), [step](../tests/step.rs): finite siblings beside divergence, delivery backpressure, logical application boundaries and shared-event reporting. |
| Continuing reclamation and bounded disposal | [lifecycle](../tests/lifecycle.rs), [compaction](../tests/compaction.rs), [cancel](../tests/cancel.rs), [Arc ownership](../tests/arc_ownership.rs): suspended jobs, continuing aliases/choices, retained views, stale roots, bounded release and cancellation. CLI tests in [main](../src/main.rs) exercise cleanup on successful delivery and output errors. |
| Actual intermediate and recorded views | [inspection](../tests/inspection.rs), [pending bodies](../tests/pending_body.rs): committed roots, unposted bodies, exact nested structure, selected alternatives and opt-in failed-state history. |
| Notebook ownership and editing | [HTTP protocol](../tests/notebook.rs), [session tests](../web/notebook.test.mjs), [connection tests](../web/connection.test.mjs), [graph paging](../web/graph-window.test.mjs): shared text/diagram model, ordered ports, submitted-program ownership, step/pause/resume, replay, close/release and reload. |
| Bounded durable output | [answer tests](../web/answers.test.mjs) and [native IndexedDB harness](../tests/answers.browser.html): normalized scalar records, checkpoint transactions, partial-answer discard and archive ownership. |

Completion is per condition. A frozen pending-work root excludes the scopes of unfinished work; child scopes stay within their parent's recorded scope, and child admission precedes parent retirement. The FIFO mutation lane revalidates the candidate against current active support before publication. This lets an unrelated finite region finish without waiting for a divergent sibling. Resumable source, completion and observation work receive service shares; pinned readers retain enumeration progress.

Semantic tracing determines condition/payload liveness independently of physical Arc ownership. Snapshots and selected views deliberately pin their dependencies; compaction transports old-reader support at mutation boundaries. Queue blocks hold at most fixed-capacity batches and release two child references per service tick. Cooperative CLI and notebook close paths drain execution state before destruction. Arbitrary host-Rust terminal destruction remains stack-safe but is not a cooperative language-service API.

These are code arguments supported by adversarial tests, not a formal proof over every program. Budget limits mean unfinished execution. Output may be exponential, explicit recordings and saved answers may grow, and sampled memory maxima are not exact byte peaks.

## Historical cross-engine comparison

The retained comparison is **current `294c010ebab311cf652d282398a3a2e38f85d4a5` versus older CHRLang `4ed9c045dc4eccfca58fb25e39a4f13f46a1a223`**. Its ratios do not measure `56e158a`, `f924e45`, or later integrations.

At size 128, median paired reused-preparation full-cost ratios against older Active were:

| Regime | Current / older Active |
|---|---:|
| Alias | 13.29 |
| Degree | 18.06 (secondary pass 11.23) |
| Sparse | 8.87 |
| Dense | 3.10 (secondary pass 3.09) |
| Cyclic | 0.323 |
| Common, two alternatives | 8.37 |
| Distinct | 19.98 |
| Failure | 12.36 |
| Common-wide, 128 alternatives | 0.520 |

Below one favors current. There is no assumed average workload. The wide case preserves 128 duplicate answers and 16,384 facts while sharing source applications. Large absolute-time drift between passes, especially degree128, prevents small-percentage conclusions.

The [historical report](validation/results/REPORT.md), [all tables](validation/results/TABLES.md) and [compact summary](validation/results/summary.json) preserve the exact qualification. Timed execution has tracing and history off. Separate traced references check application work. Ordinary validation checks ordered tuples and query identity; search validates exact answer multisets through a variable-only query-carrier translation. Constructors, term theory and incompatible benchmark semantics are not proxies for this language.

Full reused cost includes initialization, execution/output and engine/output disposal. Ordinary validation is outside runtime; search constructs normalized answers inside runtime and verifies them afterward. Source exhaustion and delivery tail overlap source/observation differently from older observation timing. “Cold” columns add fresh parsing/preparation/translation/disposal samples; they are not measured process-startup or cache-cold executions. The historical driver uses terminal engine disposal, not the subsequent CLI cooperative-cleanup integration. Compilation, HTTP and browser persistence are outside these ratios. These controls do not establish the strongest possible competitor.

## Reproduction and evidence provenance

[reproduce.py](validation/reproduce.py) extracts both exact commits with `git archive`, copies only the comparison harness, and builds offline with lockfiles. It requires Git, tar, Python 3, Rust/Cargo and already cached dependencies. The recorded toolchain was rustc 1.97.1; the script records the actual toolchain rather than switching it. The older repository must be supplied explicitly. Output is a new directory, defaulting to the system temporary directory and honoring `TMPDIR`.

```sh
python3 docs/validation/reproduce.py --older-repo /path/to/CHRLang
# Full historical matrix; CPU10 is the historical affinity, not a portability requirement:
python3 docs/validation/reproduce.py --older-repo /path/to/CHRLang --cpu 10 --full
# Rebuild tables without any engine execution or older checkout:
python3 docs/validation/summarize.py --output /path/to/new-summary-directory
```

The default runs ordinary semantic fixtures, sparse8, and common-wide8 with both default and older arena-cow builds. The full option reproduces 21-pair ordinary Active, seven-repetition Active/Global, 21-repetition default/COW search and fresh preparation charges. It leaves preserved results untouched.

The evidence was copied from the frozen `chr-refresh-294c010` experiment. Harness source, Cargo manifests/locks, build script, raw compact CSV samples, qualification logs, metadata, report and summary are retained under [validation](validation/). No older runtime or raw profiler recording is vendored. [SHA256SUMS](validation/SHA256SUMS) identifies the bundled evidence. The archival report's machine-local paths describe original provenance; use the commands above for durable reproduction.

Packaging validation rebuilt the tables and summary byte-for-byte from CSV. Setup/build and small-run evidence is recorded in [packaging-checks.md](validation/packaging-checks.md), including the exact source-access boundary encountered during validation. No new full performance matrix was run for this documentation delivery.
