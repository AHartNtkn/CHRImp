# Identify the strongest CHR architecture

Find and experimentally substantiate major architectural improvements for CHR with explicit disjunction. Explore different foundations and compatible combinations, then recommend the best supported configuration or tradeoff frontier. The outcome is an architectural decision backed by working experiments, not a catalogue of ideas or production implementation of every candidate.

## What real progress means

Progress changes a consequential decision: establishing or rejecting a proposed source of large gains; showing why a complete configuration wins or loses; finding the workload boundary of a gain; or resolving whether two promising changes reinforce or undermine each other. An informative negative result counts. Source-backed reasoning can reject an incompatible idea without implementing it.

Code volume, experiment count, better internal counters, marginal speedups, additional instrumentation, and newly discovered possibilities do not independently count as progress. No architecture or counter from the current engine is privileged. A bottleneck can disappear through a different model instead of receiving a faster implementation.

## Exploration strategy

1. **Independent discovery.** Use the five independent investigations with different priorities: cheap necessary execution, computational sharing, less unnecessary search, compilation, and robust behavior under adverse workloads. Give agents requirements and evidence, not preferred techniques or another agent's proposal. Compare relevant primary literature; distinguish demonstrated results from speculative transfers.
2. **Build competing hypotheses.** Group findings by genuinely different architectural decisions. Separate alternative foundations from compatible transformations. For each promising configuration explain what work disappears, what replaces it, where it should win, where it should lose, and what evidence could refute it. Include the existing engine as a candidate. Select a small frontier of distinct, credible contenders, rather than one worktree per named technique.
3. **Resolve the highest-value uncertainty first.** Before each experiment record the decision it resolves, comparable input/output obligations, bounded implementation scope, wall-time/memory limits, and advance/reject/inconclusive criteria. Prefer experiments capable of changing the ranking. Reject unsupported assumptions analytically when possible. Use a minimal executable vertical slice when runtime evidence is needed; it must include the costs central to its hypothesis.
4. **Compare configurations fairly.** Use isolated worktrees from a common baseline. Run independent implementation work in parallel when scopes permit; serialize timing campaigns to avoid resource contention. Reuse existing evidence when comparable. Keep an unchanged control, comparable optimization settings, equivalent requested answers and honest censoring. Measure preparation/compilation, execution, output and retained resources separately where needed, as well as total cost. A capped run is not a completed runtime. Do not rerun rwLog benchmarks.
5. **Challenge the winners.** Start with a small set of workloads discriminating the hypothesis, then challenge promising configurations on materially different and adverse regimes. Supplied synthesis queries demonstrate practical value but do not define coverage. Include low and high reuse, independent and correlated choices, consuming interactions, identity changes, cycles, propagation, long-running growth and release when relevant to the architectural decision. Add a case only when it tests a plausible weakness or changes confidence in the recommendation; no Cartesian product of every dimension.
6. **Test consequential combinations.** Test complementary winners together when there is a credible interaction. Use comparisons that distinguish individual from combined gains. Do not assume gains add, all worktrees merge, or every subset deserves an implementation. Revisit a rejected candidate only when new evidence changes its premises.
7. **Decide and stop.** Recommend a configuration or explicit frontier with mechanisms, practical gains, losses, applicability limits and the next concrete implementation action. A universal winner is not required; retaining the baseline is legitimate if alternatives fail. Finish once credible leading alternatives and consequential interactions have been resolved well enough to make that decision. Further ideas, microoptimizations, production polish and stronger statistical inference are not unfinished delivery.

## Experiment discipline

An experiment's budget is chosen before execution from its decision value and expected cost. A budget expiration yields a bounded result, not automatic renewal. Extend only when new evidence identifies a specific remaining uncertainty worth the additional cost. Do not turn an incomplete prototype into evidence that the architecture itself is slow or impossible.

Advance a candidate for a substantial, repeatable end-to-end improvement, a demonstrated scaling advantage, or removal of a dominant cost with credible total-cost evidence. What is material depends on the hypothesis and measurement noise; declare it before measurement rather than inventing a universal percentage. Do not polish a weak candidate through a series of small optimizations. Fix correctness and experiment defects needed for a fair comparison, then reassess its architectural promise.

Use the maintained perf suite. Introduce a measurement only when an actual unresolved comparison cannot be made honestly without it. No broad perf-suite rewrite, provenance machinery, exhaustive audits, or obligation to implement every suggestion. Keep evidence sufficient to reproduce the architectural conclusion, not a second maintained research subsystem.

## Required behavior and open implementation choices

Arguments contain variables only. Heads test existing variable identity; body equality may merge identities. Only explicit disjunction creates semantic search. Competing rule applications commit to a legal schedule, with schedule freedom. Preserve simplification, simpagation, propagation, distinct occurrences, freshness, residual normal forms and fair answer progress beside continuing siblings. Cycles are allowed unless rejected by the program. Arbitrary heads can interact across graph fragments, even without shared variables.

The baseline already shares storage and common computation and avoids eagerly expanding independent choice products. A replacement must respect that requirement; a narrow prototype may leave other features outside its experiment, but must label them and cannot claim full-language superiority from a weaker implementation. Assess unresolved integration costs before recommending adoption.

Storage, support representation, matching, scheduling, compilation, reclamation and completion algorithms are open. No browser dependency is essential to evaluation. Optional inspection/history need source correspondence but need not dictate normal execution machinery. Prototype and benchmark changes are authorized here; replacing the production engine or integrating a proposed architecture into the notebook is a subsequent implementation decision.

## Evidence and completion

Use existing semantics/progress/observation/lifecycle tests as applicable and architecture-independent outputs as the oracle, not incumbent implementation counters. Existing sharing tests can establish behavior; their internal counters need not become requirements for a new representation. Verification follows changed behavior and risks, not commits.

Completion requires a concise comparative result supported by executable experiments for the leading runtime claims, defensible rejection of other serious contenders, examination of consequential combinations, and an explicit account of uncertainty that could change the recommendation. There is no claim to global optimality and no requirement to exhaust the literature. The PM checks this outcome directly; bookkeeping is not a deliverable. Commit validated task-owned work.
