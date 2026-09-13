# Complete the CHR performance suite

Build and validate a maintained performance suite that proactively exposes plausible performance failures across the CHR language and runtime, including failures not yet observed, and detects regressions as the implementation evolves. Discover the risks from the system's semantics, algorithms, data structures, resource ownership and their interactions; do not let current slow examples or existing benchmark cases define the scope.

## User outcome

The user requested: “Create a FULL /goal which would COMPLETE the perf suite. Use brainstorming.” They clarified: “There will be other failures in the future; your goal is to anticipate THOSE!”

The completed suite must exercise how the system can become unexpectedly expensive under valid programs, queries, input shapes, execution histories and resource lifetimes. It must detect material failures, supply reliable evidence about them, and remain practical to run and extend. A larger benchmark list or an explanation of the present synthesis slowdown is insufficient.

This is a recovery goal for the earlier broad benchmark-blindspot work. Five independent brainstorming agents inform its design; the assessment is in `notes/brainstorm.md`. Implementation must carry the full outcome through discovery, construction, adversarial validation and final review.

## 1. Discover prospective performance risks

Start with independent examination of the language/runtime and existing suite. Follow public capabilities through execution and resource lifetimes, then ask how legal inputs and operation sequences could make time, memory, latency, throughput, preparation, delivery or cleanup degrade. Examine algorithms and shared services as well as domain-level examples. Do not use the earlier audit's finding table as the discovery checklist.

For each meaningful risk, identify:

- The valid trigger and independently variable dimensions that could expose it.
- Why the implementation could exhibit the cost: repeated work, multiplicative interactions, growing retained state, representation expansion, adverse data distribution, scheduling interference, or another evidenced mechanism.
- What distinguishes necessary problem/output size from avoidable amplification, and what observation would reveal the failure.
- Existing coverage that actually exercises the trigger, or the missing workload/control/measurement needed to do so.

Use independent brainstorming and adversarial review to challenge this model. Inspect neglected paths, boundaries and interactions; seek risks beyond both known incidents and the first proposed catalog. Retain a compact risk-to-evidence map. Every benchmark must answer a meaningful risk question; every material identified risk must have runnable detection evidence or a concrete, evidence-backed reason it cannot arise under the supported model. An unimplemented detector is an open gap, not an exclusion.

Examples of discovery dimensions, not an exhaustive specification: data size and skew; graph topology and sharing; rule/head shape and selectivity; equality and invalidation; condition correlations; search depth, breadth and failure placement; productive versus nonproductive work; program preparation and repeated use; output volume and consumer pace; cancellation and retained ownership; session lifetime and competition. Discover additional dimensions from the code rather than stopping at these examples.

## 2. Turn risks into a general workload strategy

Extend the maintained suite and its reusable generators/oracles. Combine small discriminating controls, parameterized size/shape sweeps, selected interaction stresses, fixed-seed generated variations, long-running lifecycle workloads, and end-to-end programs. Use relation/graph/rewrite/search properties as organizing dimensions so coverage extends to mathematical domains without ready-made user examples.

Exercise relational rewriting, algebraic and graph computation, proof/program synthesis and evaluation through their relevant cost mechanisms, not merely named examples. Include supported rule forms, identities, cycles, sharing, explicit alternatives, normal forms, answer multiplicity, partial structures and continuing computation. Vary independent dimensions separately and combine them where coupling could conceal failures. A full Cartesian product is neither required nor a substitute for reasoning about interactions.

Existing arithmetic, type-synthesis, behavior-synthesis and lambda notebooks remain end-to-end regressions. Inventory their saved queries, but do not equate query count with coverage. Keep unrestricted generation distinct from supplied-witness validation. Budget expensive searches honestly; neither timeouts nor easier witness queries discharge the original performance question.

Use independent semantic oracles and metamorphic properties where justified by actual language semantics. Do not assume confluence, a particular answer order, deduplication or a particular synthesized witness. Generated/perturbed cases must remain legal and have checkable outcomes or progress/resource properties. Capture a small reproducible case when a variation reveals a failure; avoid making an experimental archive a maintained subsystem.

## 3. Make failures detectable and evidence trustworthy

Derive instrumentation from the prospective risks and the decisions the suite must support, not from a fixed inventory of current counters. Provide enough information to distinguish workload growth, intrinsic output/search growth, repeated implementation work, resource retention and observer effects. Existing gaps in search, matching, completion, scheduling, representation and allocation measurements must be covered, but these are required examples, not the boundary of the goal.

Required capabilities include:

- End-to-end latency, first complete answer, useful throughput/prefixes, finite exhaustion, continuing progress, cancellation and cleanup; preparation/runtime/output/validation boundaries must be explicit.
- Work volume and where it occurs: candidates and failed attempts as well as commits/results; per-rule attribution where meaningful; repeated scans, retries, invalidations, dispatches and shared-service work. Define search quantities correctly for a shared conditional graph rather than counting jobs as independent branches.
- A maintained native CPU flame-graph command with readable Rust stacks, raw sampling data, workload/build metadata and validated sample accounting; support registered workloads and arbitrary CLI program/query execution.
- Actual CPU attribution and memory/allocation/ownership evidence sufficient to localize material growth. State exclusive/inclusive attribution and uncertainty. Do not infer time from tick fractions or collector execution from collection-active status. Distinguish live bytes, allocation traffic, object counts, sampled peaks and process residency.
- Scaling and comparison across sizes, shapes, repetitions, lifetimes and revisions, including distributions and censored observations. Preserve raw numerators and denominators; zero-denominator ratios are undefined.
- Diagnostics for ordinary native program/query inputs as well as registered benchmark families. Engine/CLI/runtime execution must stand independently of browser requests. Optional history and inspection costs require separate cases.

Use minimally instrumented native timing and opt-in detailed measurement through the same maintained workload and validation paths. Quantify observer overhead and check semantic consistency. Do not require default tracing, retained execution history, or per-tick clocks. Metric definitions must specify units, event boundaries, scope and lifetime; unclassified or unavailable measurements must remain visible.

The runner must record enough input/build/toolchain/environment/limit information to reproduce results, emit machine-readable data, and provide a concise comparison. It must distinguish a correctness error, a performance regression, uncertainty/noise, a censored run, and a proved completed workload. Preserve existing rwLog measurements and their provenance; do not rerun rwLog.

## 4. Make the suite useful before the next failure

Provide a bounded routine regression run and a deeper exploratory/scaling run using the same infrastructure. Select cases based on risk coverage and detection power, not a case-count target. Declare per-case and aggregate execution budgets before campaigns. External supervision must bound stuck calls, cleanup and process resource exhaustion as well as ordinary in-engine limits. Do not increase budgets merely to turn incomplete results into successes.

Implement actionable regression detection using deterministic work/resource checks where appropriate and statistically qualified timing/scaling comparisons where needed. Establish thresholds from measured noise, expected scaling and material effects; do not invent universal speed targets or freeze incidental implementation counts as language requirements. Known slow baselines must remain visible and must not disable detection of further deterioration.

Demonstrate sensitivity to regressions and resistance to false alarms. Run comparisons serially without accidental build/test/benchmark contention; preserve unsuccessful samples and honest limits. A changing implementation should be able to use the maintained commands without ad hoc source patches or temporary audit directories.

Keep extension cheap: document how a new semantic capability, execution responsibility, data representation or ownership path changes the risk model, generators, oracles and detection checks. This is a short contributor procedure tied to affected behavior, not blanket requalification after every commit.

## 5. Prove coverage with adversarial challenges

Test the instrument independently. Use small calculable programs, independently observed events, scoped allocation controls and profiler corroboration to establish measurement validity. Checks must catch missing/doubled/misattributed measurements and invalid or prematurely reported answers, not merely validate report fields.

Have an independent reviewer select performance hazards and legal workload variations after the initial suite exists, without limiting them to the builders' calibration cases or historical incidents. Include previously unobserved failure mechanisms and coupled stresses. Use safe test-only perturbations or controlled variants where necessary to demonstrate that the suite detects representative time, memory, scaling, latency/fairness and lifecycle regressions while preserving semantic outcomes. Do not add production slow paths or commit optimized-away behavior solely to manufacture proof.

A challenge is successful only when the maintained suite notices the material problem, produces usable evidence, and classifies it correctly. Exercise established generators and detectors before adding a dedicated reproducer for the challenge; otherwise the exercise only proves that a known defect can be tested. Merely timing a hand-picked slow case is not a successful challenge. Repair general gaps revealed by misses, then use another independently selected variation to check that the repair generalizes. Predeclare material effect sizes and noise tolerances from measured feasibility, not arbitrary universal targets. Include unchanged/equivalent controls to test false positives and measurement perturbation.

## Completion oracle

Finish only when all of these are demonstrated by runnable repository commands and independently reviewed results:

1. Prospective risk discovery covers the supported language/runtime responsibilities and plausible interactions, including risks not drawn from historical failures. Material discovered gaps have executable witnesses, not only documentation.
2. The suite's workload generation, scaling, lifecycle and interaction coverage exercises those risks with independent correctness/progress/resource oracles.
3. Measurements are calibrated and observer effects quantified; reports distinguish resource/work growth, uncertainty and incomplete execution accurately.
4. Routine regression detection and deeper exploration work within declared bounds and demonstrate detection of representative independently chosen regressions, including previously unseen risks, without systematic false positives on controls.
5. Current end-to-end workloads produce validated timings or honest censored results and useful diagnoses using maintained tooling. Existing temporary probes are not required to explain material cost.
6. An independent final review maps the original forward-looking outcome to running evidence, challenges untested material paths, and confirms no required coverage, measurement, runner or detection work remains. Record `full_outcome_complete: true` only then.

This does not promise detection of every possible future performance defect. Adequacy is established by the source-backed risk argument, generative and interaction coverage, independently chosen challenges, calibrated detection and practical extension path together. Neither a frozen checklist nor the assertion that testing can never be exhaustive is a reason to stop short of the full outcome.

## Execution boundaries and stopping

Complete coherent packages: prospective risk/coverage discovery; maintained workload and measurement implementation; bounded runner and regression detection; independent challenges and closure. Adjust the decomposition as evidence requires without narrowing the outcome. `state.yaml` records active work and compact evidence. Implementation choices belong to the assistant; this suite work needs no new architecture approval.

Preserve language behavior, variable-only relation arguments, nonbinding head matching, explicit-disjunction-only search, sharing, fairness, reclamation, permitted cyclic behavior-synthesis structures and default history-off execution. Do not infer new semantic restrictions from incumbent internals or fixtures.

Fix suite and measurement defects end to end. Engine instrumentation and directly necessary correctness repairs are in scope; broad engine redesign, unrelated UI work, performance-parity targets and an open-ended optimization campaign are not. Record newly exposed engine bottlenecks with evidence without making their optimization a prerequisite for suite completion.

Reuse existing infrastructure and checks. Keep supporting documentation/results compact; do not build a separate dashboard, benchmark archive or administrative subsystem. Testing follows changed behavior and risk. Commit validated task-owned work and preserve unrelated changes.

Planning, more cases, counters, ordinary passing tests, a report or a speedup cannot satisfy the oracle by themselves. Continue until the full oracle passes, then stop. Further possible engine optimizations do not keep this goal open.
