# Current optimization cycle

- Status: architectural decisions require reassessment under the user’s explicit prohibition on using program runtimes to determine gains. No implementation is in progress.
- Automation: `chr-architecture-optimization`, hourly heartbeat in this thread.
- Repository: `/home/ahart/Documents/CHRImp`; landing branch `codex/shared-relational-engine`.
- Production: structural normalization plus conditional dispatch, integration `0ded0c4` / implementation `23208ad`. CLI and notebook share the executor, optional history and source stepping.
- Latest decision status: the timing-based hold/rejection conclusions in cycle reports 1–3 are not valid selection decisions. Retain their observations and correctness evidence, but reassess architectural merit without runtime gates. Neither cycle 3 spike is integrated; neither has an established architectural verdict.
- Clean committed experiments: `/tmp/chr-loop3-producer-consumer`, `codex/spike-producer-consumer`, `f5c8648`; `/tmp/chr-loop3-demand-support`, `codex/spike-demand-support`, `f108957`. The latter has explicitly incomplete validation; do not treat completing every experimental path as outstanding delivery.
- Cycle 3 used about 35.48 seconds measured execution of 120 allowed: 78 observations and one CPU profile. Raw evidence under `/tmp/chr-loop3/` and `/tmp/chr-loop3-support-profile/`. Its budget/experiment is closed; do not repeat without a changed premise.
- Next action: reassess existing candidate mechanisms using source and causal evidence of computation, representation, applicability, scaling and introduced costs. Do not run another timing comparison to settle their merit. Profile attribution alone does not establish whether a design improves those properties. Reuse the existing experiments; no new broad audit or automatic implementation obligation.
- Earlier experiment records: [cycle 2](optimization-cycle-2.md), projected overlap `950d747` and relative completion `ec1822f`; [cycle 1](optimization-cycle-1.md), ownership `861dc54` and reverse order `737090f`. Their runtime-based rankings must not control subsequent work.
- Earlier architectural decision: `docs/goals/architecture-exploration/notes/decision.md`. Static whole-head feasibility remains separate and unintegrated. Hard behavior synthesis remains unresolved; use “timed out without an answer” in user-facing results.
