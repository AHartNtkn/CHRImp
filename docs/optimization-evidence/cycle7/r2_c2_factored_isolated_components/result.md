## OPTIMIZATION RESULT

**Slug / round:** r2_c2_factored_isolated_components / cycle 7 round 2 candidate 2

**Kind:** performance

**Verdict:** BLOCKED

**Reason:** Trial stopped at the user's bound. No executable shared-component
mechanism or source certificate was completed. Only a symmetric semantic boundary
fixture and incumbent diagnostic observations were produced. No optimization
code changed, no candidate comparison exists, and no gain or rejection of the
architectural hypothesis is established. No variants were started.

**Worktree / branch / commit:** `/tmp/chr-cycle7-opt-factored-isolated-components`,
`codex/opt/factored-isolated-components`. Starting HEAD was
`3f5bb819cb0f496d5fb261bade0660277caf2693`; the closeout commit containing this
file preserves evidence/tests only, not an executable optimization.

**Baseline:** `bcb215e82a9b1279e0858840c13359d39123a2bf`; source, examples, tests and Cargo files were identical
to starting HEAD. The difference consists of campaign documentation and prior
candidate evidence. Native baseline built locally using
`cargo build --offline --release --features diagnostics --example measure -j 2`.
Build succeeded. No shared binary or landing checkout was changed.
Baseline binary SHA256: `ccfe488d44f70a5bcfb6a3062226446843ff53b5f8c31fb04e020a8f9a436b1e`.
Maintained perf helper SHA256: `c423132beb577d42d2555d394e2b753c9b5b22b35e037f10009edb9cdcf14c8c`.
Archived protocol SHA256: `51c48a61afd54a188f2a907232c6b0ba6ca7711c02bb6d832b0e4f843e28e798`.

**Protocol:** `docs/optimization-loop.md`, `docs/performance.md`, explicit user
diagnostic acceptance rules. Archived worker and generic measurement protocol
were read; runtime-ratio gates do not apply. Maintained `examples/perf.py` and
`examples/supervise.py` captured typed records and cleanup outcomes.

**Evidence directory:** this directory. `hypothesis.md` declares the trial;
`baseline-*/0.json`, `0.stdout`, `0.stderr`, and `campaign.json` retain all native
records/configurations. `check-diagnostics-direct/` records the clean direct test
execution. Fixture: `tests/factored_components_boundary.rs`.

### Checks

- Diagnostics release boundary fixture: 5 passed, 0 failed; direct executable
  rerun under the maintained supervisor completed with no leftover descendants.
- Ordinary release boundary fixture: 5 passed, 0 failed, cargo exit 0.
- `cargo fmt --all` completed. Full suite and Clippy were not run; no broad
  correctness or candidate correctness claim is made.
- First cargo invocation under the supervisor passed all 5 tests, but the
  supervisor classified it failed because a descendant remained after cargo
  exited. That record is preserved under `check-diagnostics/`; the direct test
  rerun resolved the check without weakening the supervisor.
- An initial perf invocation incorrectly combined `--diagnostics` and
  `--binary`; argument parsing rejected it before native execution. Subsequent
  runs used the prebuilt binary; every configuration reports diagnostics true.

### Workloads and diagnostics

Nine baseline observations completed, one per configuration, no warmups:
fresh-contract groups 64/256, copies 4, depth 4 in grouped/interleaved order;
fresh-unmerged64 and one-copy64 interleaved; rewrite128; held-output and held
snapshot lifecycle controls. All used maintained semantic/resource oracles.
Non-equivalent components and late observation were checked in the fixture.
**Candidate observations: zero.** No statistical or runtime acceptance gate ran.

Grouped baseline observations (source checkpoints unless otherwise specified):

| Diagnostic | groups64 | groups256 |
|---|---:|---:|
| Logical applications | 2,048 | 8,192 |
| Body posts | 3,648 | 14,592 |
| Scalar output events | 6,658 | 26,626 |
| Graph collection phase dispatches | 52,298 | 213,193 |
| Graph pruning phase dispatches | 229,175 | 691,530 |
| Store release dispatches | 250,618 | 1,176,650 |
| Sampled peak graph nodes | 23,040 | 92,160 |
| Total requested allocation bytes | 76,592,341 | 328,114,178 |
| Process peak requested bytes | 5,494,811 | 21,896,832 |

Collection/pruning/release counts are maintained service work, not CPU time or
exact graph-write counts. Allocation includes harness/validation; phase records
are retained. After cleanup graph nodes, occurrences, pending tasks/descriptors,
history, snapshots, inspectors and release batches are zero; the live Engine
retains its current coordinate epoch. Canonical/logical sharing counts and
candidate graph-write/expansion costs are unavailable, not zero savings.

### Introduced costs and semantic obligations

No production costs were introduced: changes are tests/evidence only. A real
candidate must charge isolation certification, alpha-normalization, template
preparation, instance identity/occurrence/event maps, versioned frontiers,
per-instance scheduling, observation expansion, retained versions and complete
release. None of those costs has been measured for a working candidate.

The fixture establishes ten distinct logical application events and 22 distinct
post boundaries for two equivalent groups; late stepping pauses after one
application with its body still pending. Snapshots at applications 1 and 5 retain
their exact facts, identities and pending expression streams through cancellation
with history disabled. Duplicate tuples retain distinct occurrences/fresh
witnesses; within-group contraction does not alias fresh values across groups.
Variable-disjoint multiheads can still consume across groups, and source equality
can enable later keyed joins. A safe certificate must exclude those interactions.

Source barrier: `Engine::facts` returns borrowed logical argument slices from
`Graph`; snapshot capture owns both the graph root and pending obligation root.
`CommitStatus::Applied` publishes consumption before body posts. Factoring must
represent heterogeneous instance frontiers at these boundaries, not simply share
one current canonical root or clone final output. The trial did not implement
that representation, fallback integration, or fair round-robin scheduling, so
their obligations remain unverified. This is an implementation boundary, not a
proof that factoring is impossible or unprofitable.

**Strongest next test:** two certified equivalent groups, two copies each, with
one canonical transition and two logical frontiers. Step only one logical
application, inspect and retain both committed views, advance the other instance,
then cancel and release. Require distinct source/occurrence/fresh IDs and frozen
pending syntax while charging canonical writes, mapping/expansion allocations,
version retention and reclamation. Pass that executable test before any further
64/256 comparison. This is a recorded next test, not an active variant.
