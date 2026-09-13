# Current optimization cycle

- Status: cycle 2 complete; awaiting next hourly wake. The usage-limit interruption did not start a new cycle or renew its budget.
- Automation: `chr-architecture-optimization`, active hourly heartbeat in this thread; confirmed through the native automation view.
- Repository: `/home/ahart/Documents/CHRImp`; landing branch `codex/shared-relational-engine`.
- Production engine: structural normalization plus conditional dispatch, integration `0ded0c4` / implementation `23208ad`. CLI and notebook share this executor, including optional history and source stepping.
- Latest decision: [cycle 2](optimization-cycle-2.md). Direct projected overlap and scope-relative completion pass targeted validation but offer no demonstrated substantial gain. Neither is integrated. Duplicator synthesis remains censored without an answer.
- Clean, committed experiments: `/tmp/chr-loop2-overlap`, branch `codex/spike-projected-overlap`, commit `950d747`; `/tmp/chr-loop2-completion`, branch `codex/spike-relative-completion`, commit `ec1822f`. No experiment implementation remains in progress.
- Cycle 2 measurement: 160 observations, approximately 35.55 seconds serialized execution against 120 seconds allowed. Evidence under `/tmp/chr-loop2/`; see the cycle report for methods and validation. Do not repeat this comparison without a changed premise.
- Next action: choose a broader representation/execution experiment capable of avoiding eager condition construction, with exact semantic reasoning, reclamation and fair execution included in its cost. Shared circuits with resumable semantic queries remain a candidate, not an assumed winner. Use independent priorities and retain broad graph/rewrite, correlation and observation controls.
- Earlier evidence: [cycle 1](optimization-cycle-1.md). Ownership `861dc54` gives modest reader gains with unchanged retained growth; reverse order `737090f` gives a large reader gain but severe wide-delivery and synthesis regressions. Retained prefixes and conditional-selector aggregation must be evaluated together in future representation experiments.
- Earlier architectural decision: `goals/architecture-exploration/notes/decision.md`. Static whole-head feasibility remains separate and unintegrated.
