# Round 009 implementation and evaluation plan

Accepted source: `8e025bc`. Candidate 0 and the round acceptance contract are
supplied by the user; execution is authorized in this isolated worktree.

- [x] Establish baseline coordinate tests and preserved maintained measurements.
- [x] Add failing tests for bounded segmented metadata and exact reusable
  transport of adjacent singleton constant deltas, including interior endpoints.
- [x] Implement append-only eight-epoch segments in `src/engine/coordinates.rs`.
  Keep every epoch lease; retire each delta at its own cutoff. Cache at most one
  immutable recent exact interval composition across all segments. Constant images compose by
  first assignment per key; all other maps fall back to the existing transform.
- [x] Test repeated keys, functional fallback, publication while transporting,
  tracing during retirement, cancellation at each suspension, and final release.
- [x] Extend maintained coordinate diagnostics for segments, cache construction,
  reuse/invalidation, transform starts, crossings, and retained composition maps.
- [x] Run native and diagnostic semantic/progress/lifecycle suites, building
  separately and guarding every test invocation with
  `timeout --kill-after=5s 60s`.
- [x] Compare maintained behavior I/S, native controls, history choices,
  conditional archive rotation, inspections, held output, and runtime replay at
  their validated endpoints. Include all phases and raw evidence. Reuse existing
  workload limits; do not introduce a candidate budget or run rwLog.
- [x] Review actual gains and displaced costs under the verbatim acceptance
  rule; commit the evaluated implementation and complete evidence; recommend
  KEEP or DISCARD without modifying the campaign landing branch.

The selected bounded composition is exact because each admitted image is a
Boolean constant: subsequent substitutions cannot change it. A cached image is
valid only for its exact source and target IDs. The transport's source lease
pins all intervening maps, and its target is frozen. Segment capacity bounds
construction, invalidation, and cache release; general functional composition
is outside this slice and retains the proven per-epoch path.

Completed implementation: `6970a96f62cdb22a142a89a14fe79a24c8145206`.
The final refinement also retains one exact completed input/result prefix, and
caps compositions globally rather than per segment. The final report recommends
KEEP, with all measured storage/allocation costs and lambda limits explicit.
