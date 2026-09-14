# Round 018 execution notes

Accepted baseline: `9f5fd4a3d334b62253304256a88776861cd08878`.
Worktree: `/tmp/chrimp-opt-round018-batch-seen`.
Branch: `codex/opt/round018-batch-seen`.

Scope: only Store batch scratch duplicate elimination, plus shared diagnostics,
semantic/lifecycle oracles and evidence. No rwLog runs or landing changes.

Plan:
1. Build/check accepted baseline; commit common diagnostics and ordered-map probes.
2. Measure baseline and reverse exact-key seen-set implementation on the same probes.
3. Charge scratch allocations and unique/small controls; preserve bounded graph updates.
4. Run maintained wide/update and lifecycle workloads, audit exact semantic and work
   vectors, validate ordinary/diagnostic builds, and commit a KEEP/DISCARD report.

Builds and test execution are separate. Every test process is guarded by
`timeout --kill-after=5s 60s`. Raw commands/output and reports will accompany
the final evidence. No runtime-based acceptance or new campaign candidates.
