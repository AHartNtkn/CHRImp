# Cycle 7: first optimize-skill campaign

## Round 1, candidate 1: transient control lowering

**Status:** BLOCKED. No engine implementation or executable lowering prototype was completed; nothing was eligible for integration.

The candidate targeted the diagnosed cost in `fresh-contract`: transient control rows are posted into the graph and later collected/released. The worker established a semantic boundary instead of weakening the API. Committed controls are observable through `Engine::facts`, late snapshots, and late stepping even with history disabled. A pending-syntax substitute would therefore change observable state, snapshot retention, or fresh occurrence multiplicity.

The focused fixture passed 2 ordinary and 2 diagnostics release tests, including late snapshot retention through cancellation, distinct fresh witnesses for equal control tuples, late stepping, and complete reclamation. Broad validation was not claimed because an existing notebook startup assertion stopped the broader run. No candidate comparison was made, and no runtime or timeout was used as a verdict.

Baseline evidence and worker artifacts are preserved under [cycle-7 candidate evidence](optimization-evidence/cycle7/r1_c1_transient_control_lowering/). The worker branch/commit was `codex/opt/transient-control-lowering` / `981302f`; its strongest untested follow-up is a virtual committed-occurrence representation with explicit promotion to the general graph on observation/escape. That is a materially different representation question, not an automatic continuation follow-up.

The next candidate is the independent `central_occurrence_support` simplification: test whether one versioned occurrence-support authority can remove duplicated support rewrites and their release work without increasing dead-posting scans or retained snapshot state. A separate `segmented_graph_ownership` candidate remains in this round for breadth.
