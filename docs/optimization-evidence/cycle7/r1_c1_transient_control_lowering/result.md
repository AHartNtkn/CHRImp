# Bounded result: BLOCKED

No engine implementation or executable lowering prototype was completed. The
only code change is the symmetric semantic fixture
`tests/transient_control_boundary.rs`. It is suitable for unchanged baseline and
any later candidate; it contains no alternate execution mode or weakened oracle.
The original hypothesis remains in `hypothesis.md`.

## Precise boundary

The finite producer shape is source-visible in `examples/measure/fresh.rs` and
`Prepared.rules/triggers/instructions`. However, source recognition cannot
certify that an ordinary Engine will never be observed later. `Engine::facts`
(`src/engine.rs:398`) borrows `&self` and reads current committed occurrences;
`capture_snapshot` (`src/engine/inspection.rs:362`) accepts late captures even
with history disabled; `request_step` (`src/engine/step.rs:109`) accepts late
stepping. Snapshots freeze both the graph root and remaining source syntax.

The fixture demonstrates that a committed step0 occurrence is observable with
zero applications and zero prior snapshots. A capture retains that Fact, rather
than pending Post syntax, through source cancellation. Inspection performs zero
source applications. Releasing the capture reclaims all graph/occurrence,
pending/descriptor, history, condition, restriction and release storage. The
still-live engine retains its one current coordinate epoch, as on baseline.

The second fixture uses two identical step0(A,I) tuples. A late stepping request
observes exactly four separate source applications (two per rule), and the final
two marker occurrences carry distinct fresh witnesses. Cancellation and cleanup
pass. Thus neither tuple deduplication nor eager chain execution is a substitute.

A history=false guard, an empty current snapshots map, or single-consumer source
recognition is insufficient for the proposed fallback. Merely leaving controls
as pending Body syntax changes observable committed state. The current Commit
path also requires real graph head occurrences. A lowering therefore needs a
compatible virtual committed-occurrence representation or an explicit execution
capability restriction. Neither was implemented or assumed authorized as a
weakened API. This is an implementation boundary, not a refutation of the idea.

Strongest untested follow-up: first prototype one virtual committed occurrence
per certified control, with original occurrence identity and argument ownership,
visible to the existing borrowed facts iterator and frozen snapshot roots.
Prove late observation, late stepping, retention and incremental cancellation
against this fixture before routing execution through it. Then test resumable
promotion to general execution for escapes and charge overlay/capture/promotion
allocation and release against the avoided graph/index work. The key unresolved
question is whether that representation preserves the gain without duplicating
the general store. No follow-up implementation is included here.

## Validation

- `check-boundary/`: release/offline focused tests, 2 passed.
- `check-boundary-diagnostics/`: release/offline diagnostics focused tests,
  2 passed. Both retain stdout.log, stderr.log and run.json with command, cwd,
  resource limits and exit status.
- `cargo +nightly fmt --all -- --check` and `git diff --check`: passed.
- Earlier `check-release/`: broad release run stopped at
  `notebook_default_origin_is_stable_and_port_override_is_honored`, which received
  empty startup stdout instead of its expected loopback URL. Its preceding unit
  and integration targets passed. Sandbox loopback restrictions were suspected,
  not established from this assertion alone. The requested unrestricted retry
  was interrupted by the user before completion; no retry pass is claimed.
- Per the close-out instruction, no further broad suite, Clippy or Python checks
  were run. This is not an integration-ready correctness result.

## Reused diagnosis, not a candidate comparison

Copied raw campaigns (including warmups, stdout/stderr, typed records, build logs
and manifests) are retained in `baseline-fresh64/` and `baseline-fresh256/`.
Their source is cee3c256c1639546c0a35d02a7b19690d45b4809; Git confirms the assigned
b7882b00f2f439812af1dfc4f8990f83a83f03e7 differs only in the campaign-state document.
Their recorded commands use rows=4, depth=4, grouped order. Source/build metadata
is inherited from these original manifests; binary hashes were not recorded by
those campaigns and cannot be reconstructed as historical build proof.

The following are the first measured records (`1.json`), in operations/counts
or requested bytes. They are descriptive baseline diagnosis only.

| Quantity | fresh-contract64 | fresh-contract256 |
|---|---:|---:|
| Total source applications (sum of per-rule applied) | 2,048 | 8,192 |
| Normalization applications only | 768 | 3,072 |
| Producer applications / potential control occurrences | 1,280 | 5,120 |
| Source live graph nodes | 23,038 | 92,158 |
| Source live occurrences | 1,600 | 6,400 |
| Source collection dispatches | 326,235 | 1,050,696 |
| Source store-release dispatches | 250,618 | 1,176,650 |
| Cleanup additional collection dispatches | 1,641 | 6,441 |
| Cleanup additional store-release dispatches | 11,519 | 46,079 |
| Setup allocated bytes | 344,966 | 1,299,658 |
| Engine allocated bytes | 75,482,936 | 324,501,056 |
| Validator allocated bytes | 465,054 | 1,906,802 |
| Cleanup allocated bytes | 4,672 | 4,672 |
| Cleanup freed bytes | 3,221,751 | 12,844,803 |
| Other allocated bytes | 294,023 | 399,684 |
| Process peak requested bytes | 5,494,750 | 21,896,771 |

Inspection/delivery/calibration allocator phases are zero in these ordinary
campaigns; this does not estimate the cost of a future fallback. Setup includes
preparation; the raw workload record retains finer phase clocks, but clocks have
no role in this verdict. Allocator categories describe executing code, not
retaining ownership. At after_cancel both cases have zero graph nodes,
occurrences, pending tasks/nodes/descriptors, condition/history/restriction nodes,
release batches, snapshots and inspections, with one coordinate epoch retained.
Final process-live requests of 1,804/1,805 bytes include harness/lifecycle costs
and do not contradict the engine object-count cleanup oracle.

fresh64 has 3 measured observations plus 1 warmup; fresh256 has 1 measured
observation plus 1 warmup. No new timing campaign was run, no measurement
ownership was held, and the paired timing helper was not invoked. Runtime
effects, confidence bounds and alpha allocation are N/A. No candidate storage,
work/allocation effect, noninferiority or architectural-gain gate was measured.
fresh-unmerged64/256 and rewrite128 were planned but not collected because no
candidate exists; the observation control alone was executed as a semantic test.
All candidate comparison gates remain unassessed.

No parent-checkout source, global skills or docs/optimization-state.md was
modified. The assigned branch and worktree, scoped fixture, hypothesis and all
raw evidence are preserved. No app task was created.
