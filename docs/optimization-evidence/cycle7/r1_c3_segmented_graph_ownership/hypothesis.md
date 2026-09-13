# Declared experiment

Baseline: 16e55baddf9bbd72cf315dd85e7a47221c814bd4. Assigned worktree only.
The user-supplied cycle-7 diagnosis and Store::allocate/unique/Record::drop/
release_tick establish repeated persistent-path ownership and child release.

Falsifiable hypothesis: packing a branch and its two exclusively owned leaf
children into one immutable region with scalar internal links eliminates two
owning tree edges and one deferred child-release pair per packed region. The
saved ownership/release work must exceed packing/unpacking, promoted leaf copies,
root-handle resolution and additional retained records. Fewer scheduler ticks or
smaller individual allocations alone do not establish a gain.

Exact boundary: the generic persistent Store index, including graph indexes;
no condition BDD arena changes. A bounded three-record region is the first
configuration. Existing external roots retain (region,index) identity and epochs;
weak witnesses prevent in-place mutation. Existing cursors and archive marking
still expose each reachable scalar payload. Partial-region readers intentionally
retain the region; physical records and bytes must report that cost. Cross-region
children remain owning dependencies and use bounded deferred release. No global
root registry, compaction, moving public handles, or survivor evacuation is added.
Packing only exclusive standalone leaves avoids modifying any published root.

Primary: fresh-contract 64 and 256, rows4 depth4 interleaved. Controls:
fresh-unmerged64/256 with the same parameters, rewrite128, partly unread
conditional inspections (2 owners, rows3, continued work24), partial-join512
rows128. Maintained independent endpoint/multiplicity oracles remain enabled.
Compare prepare, source, observation, cancellation and full reclamation with
allocator phases and Store/engine work checkpoints. Two diagnostic observations
per configuration are planned; no timing inference or runtime gate, no alpha
allocation, no statistical helper invocation. Timings in native logs are merely
incidental. Zero tolerated demonstrated adverse ownership/retention work for a
KEEP claim; otherwise DISCARD or INCONCLUSIVE, with qualitative limits stated.
Native limit5s/5million dispatches, external10s/1GiB, cumulative native budget120s.
The bounded prototype has a 35-minute implementation/validation budget from this
declaration; inability to complete semantic/diagnostic assessment is BLOCKED or
INCONCLUSIVE, never a rejection of segmented ownership generally.

Focused correctness before workload diagnosis: Store, graph, graph prune,
conditional matching, occurrence multiplicity, stale/old/pinned roots, inspections,
archives/history, cancellation, concurrent root drops and zero full-release counts.
Then full release ordinary/diagnostics, docs, format and Clippy. Baseline failures
are preserved and checked separately; no semantic oracle is weakened.

Source changes are committed before final diagnostic runs. Fixture-only
instrumentation is committed separately and used by the baseline build too.
Campaign state and shared skills remain untouched.
