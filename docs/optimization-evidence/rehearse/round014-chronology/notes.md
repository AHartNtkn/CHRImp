# Round 014 working notes

Baseline: 225fe302fa45c7667684107ba96cba357cf103dd. Isolated worktree
/tmp/chrimp-opt-round014-chronology, branch codex/opt/round014-chronology.
Landing must remain unchanged.

Design: eight-node monotonic epochs indexed by a hash directory of live blocks.
Each block links the previous and next live epoch. The frozen sweep saves its
next live ID before deleting its current node; reset and unique passes do not
delete payloads. No per-slot tombstones or reused identity generations. Full
64-bit chronological IDs preserve baseline sift ordering and exhaustion behavior.
Empty blocks unlink immediately; directory contraction occurs at collection end.

Plan: implement layout and diagnostics; check sparse high-slot retirement,
cursor interruption, serial boundaries, archive/Job/cancellation ownership;
run built semantic binaries under individual 60-second limits; compare saved
hash-verified baseline binaries through maintained perf.py against candidate at
the Round 013 matrix's equal semantic frontiers; audit gains AND regressions;
commit implementation, evidence/data and final report separately. No integration.

Initial full-file apply_patch attempt was rejected before changes because it
combined delete/add for the same path. Retried as Update File. No source loss.
Two diagnostic builds overlap only in time: the first completes initial code,
the second compiles added counters/tests; Cargo serializes its target lock.
Only the final artifacts are evaluated.

The diagnostic native suite initially had two failures: notebook/CLI child
servers could not bind loopback (Operation not permitted). Both complete test
binaries passed with loopback permission; original failures and retries remain.
There were no native semantic failures or timeouts. Library tests were later
strengthened to cross the 32-bit serial boundary, check full 64-bit exhaustion,
and interrupt both ordinary and archive sweeps. Only cfg(test) changed after
the diagnostic measurement binary was built; production implementation stayed
identical. The final library test log includes the sparse gauges.

Matched diagnostic runs overlapped some semantic tests at their beginning;
allocations and engine counters are process-local. Runtime is diagnostic only.
Initial ordinary controls overlapped tests and an example-test build, so their
30 observations remain under raw/ordinary-overlap-* and are excluded from the
reported ordinary comparison. Five ordinary controls were rerun after the
build/test processes finished, under raw/ordinary-*.

Saved baseline binary hashes match Round 012/013 committed evidence. Baseline
instrumentation ca8a55d8bed1835c2174c87f8bc40c78128f8053 wraps the original
BTreeMap and supplies the same observation-only dispatch stopping boundary.
No baseline engine code was replaced or integrated.

An optional ad-hoc result inspection assumed every result.work was a dictionary;
the lifecycle schema also uses an integer, so it raised AttributeError before
writing anything. The maintained schema-aware audit and full native oracles
passed. No observations were changed or removed.
