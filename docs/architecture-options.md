# Architectural alternatives for CHR with disjunction

My strongest direction is to compile the submitted relational theory and query into a specialized graph program. Its physical representation and search machinery should follow the computation, rather than requiring every program to use the current conditional graph engine. This is a research recommendation, not a demonstrated speedup or an implementation plan.

## Problem

Find substantial architectural changes that make expensive work unnecessary across general mathematical relational programming. The behavior-synthesis profiles locate current costs; they do not specify the responsibilities a replacement must contain.

Language obligations remain variable-only relational arguments, identity-testing heads, explicit equality, explicit-disjunction-only search, committed rule scheduling, occurrence-sensitive rewriting, fresh variables, propagation eligibility, fair search and residual normal-form answers. Physical layouts, intermediate representations, matching mechanisms, scheduler jobs, condition managers, completion algorithms and internal metrics are open choices. Optional inspection/history requires source correspondence, not identical machinery during normal execution.

## Candidate solutions

### 1. Derive a machine from the relational theory

Treat the whole rule system as a specification from which the compiler derives both storage and execution code. Analyze which relations define structural consistency, mutually exclusive shapes, dependencies and deterministic transformations. Generate native representations and operations for eligible subsystems, with general compiled relational operations where those properties cannot be established.

The behavior notebook supplies a concrete opportunity:

```text
app(N,A,B) \ app(N,C,D) <=> A=C,B=D.
k(N),s(N) <=> fail.
```

Together with the rest of its consistency and clash rules, these can justify a shape description with ordered fields at each variable identity. Adding a second application description can perform the required child equalities directly. A contradictory shape can fail immediately. Generic relation posting, index activation and clash-rule discovery need not occur for those certified operations.

This is inferred from the program, not built into the name `app`. Distinct identities carrying `k` remain distinct: structural agreement does not imply wire identity. Cycles remain legal unless the program rules reject them. Additional consumers or rules observing duplicate occurrences can invalidate a proposed representation; whole-program interference matters.

**Work avoided:** interpreting the invariant-maintenance theory and constructing its transient administrative objects, even when every search branch is different.

**Replacement cost/risk:** compiler analysis, generated code and relational reconstruction for observation. The important uncertainty is how much of diverse programs admits useful lowering without expensive analysis. Standard CHR compilation is evidence that program information can guide implementation; automatic representation derivation proposed here goes beyond selecting a better join order. [CHR compilation research](https://arxiv.org/abs/cs/0408025).

### 2. Compile a query into a new search program

Use partial deduction, supercompilation or staging to transform the rules together with the known query into a residual program. Unfold operations justified by known information, fuse intermediate graph construction with its consumers, and fold recurring symbolic configurations into residual recursion. Unknown choices remain explicit choices in generated code.

For behavior synthesis, combine evaluation, structural consistency, target observations and `no_c` rather than repeatedly interpreting those relations against candidate fragments. The goal is a generator embodying the requested behavior, with failures exposed before unnecessary candidate structure is materialized. It can also apply to recognizers, proof searches and relational arithmetic; success does not require a special combinator backend.

**Work avoided:** repeated interpretation and intermediate representations that a specialized derivation can prove unnecessary. Some failing families may never become runtime branches.

**Replacement cost/risk:** specialization time and code growth. Unrestricted synthesis exposes less static information than synthesis from a mostly known sketch. Folding must preserve fresh identities, multiplicities, allowed committed scheduling and finite-answer progress; logical equivalence alone does not establish equivalent search behavior. A compiler can bound specialization and emit residual code for what remains unknown.

This is a credible research direction rather than a promised speedup: [multi-stage relational programming](https://namin.seas.harvard.edu/pubs/pldi-staged-mk.pdf) demonstrates substantial improvements for staged relational interpreters, while [partial deduction for miniKanren](https://arxiv.org/abs/2109.02814) documents the difficulty of adapting specialization to relational search. Neither establishes the result for this CHR language.

### 3. Execute a factored expression of possible stores

Represent execution as an expression with explicit alternatives and coexisting relational fragments. Rewrite the expression itself. A common transformation executes once at its shared location; a choice spreads into other fragments only when an interaction requires it.

For example, after two independent explicit choices, retain the internal expression:

```text
Together(Choice(c1, left(A), right(A)),
         Choice(c2, left(B), right(B)),
         work(C))
```

If the program rewrites `work(C)` through a long chain to `done(C)`, execute that chain once. Enumerate the four answers only when requested. A later consuming rule involving a chosen fact and `done(C)` distributes the affected occurrence into the relevant alternatives. Coexistence does not assert independence: arbitrary multihead rules, including heads without shared variables, can cross these fragments.

**Work avoided:** premature Cartesian expansion and repeated execution of already shared computation. This benefits common work without requiring recurrent equivalent contexts.

**Replacement cost/risk:** circuit-aware matching, local distribution and occurrence bookkeeping. Choice-dependent equality and consuming joins can force extensive distribution. Settled alternatives must be emitted without waiting for divergent siblings. Choice correlation, per-world freshness and propagation eligibility must survive sharing.

The proposal changes the primary state from a globally condition-annotated graph to a scoped algebraic expression. It does not guarantee that support tracking becomes trivial. [Factorised relational representations](https://fdbresearch.github.io/principles.html) support the representation principle; extending it to CHR's mutable, consuming, identity-sensitive execution is the research problem.

### 4. Run shared producers for reusable graph transformations

Let a reusable computation have explicit relational dependencies and effects. Equivalent contexts subscribe to one producer's progress. Results are effect templates: each subscriber instantiates the appropriate fresh identities and consumes its own occurrences. When a computation needs information outside its validated context, it suspends at that boundary.

This targets repeated derivations reached through different search paths, including unsuccessful work. It differs from candidate 3, which exploits common computation already represented in a shared expression.

The inspected rwLog checkout has actual context tabling: [Fix call keys and producers](/tmp/chr-rwlog-research-20260906/src/work/fix.rs:90) include recursive relation identity, binding identity and boundary normal forms. Its [Or rotation](/tmp/chr-rwlog-research-20260906/src/node.rs:70) is separately a scheduling mechanism. Rotation itself establishes no computed-work reuse.

A CHR producer cannot be keyed merely by `eval(T,Sp,O)`. Adding `no_c(T)` or another `app` occurrence can affect the result. Relevant surrounding facts, aliases, consumption permissions and propagation eligibility must be represented, or the producer must stop before those interactions.

**Work avoided:** re-deriving context-equivalent transitions, even when callers were reached independently.

**Replacement cost/risk:** dependency validation, context comparison, result instantiation and table retention. If a sufficient context approaches the entire store, the mechanism can lose its advantage. rwLog's answer deduplication and fixed-point completion policy are not CHR semantics: repeated states do not certify normal forms, and independent source alternatives must not become correlated.

### 5. A compiled branch machine with shared versions

A contrasting execution foundation stores ordinary branch-local identities and occurrences behind persistent roots. Explicit disjunction creates resumable continuations sharing unchanged storage. Compiled activation code runs the branch until a fair yield point; an empty, dependency-complete agenda establishes its normal form.

**Work avoided:** global Boolean-support calculations and global coordinate transport for ordinary branch-local operations. Completion becomes local quiescence rather than a query over symbolic alternatives. Generated loops still yield during long matching or rewriting; fairness does not require today's object-per-tick scheduler.

**Replacement cost/risk:** branch updates, frontier memory and repeated work after branching. Persistent storage shares data, not computations. Alone this design can perform badly on many independent choices with a common expensive suffix. It is a serious execution alternative when combined with specialization or producer sharing, not an assumption that cheap snapshots solve search.

### 6. Learn explanations that exclude families of failing choices

A more speculative direction equips compiled reactions with explanations of failure in terms of the explicit choices and context that caused it. The search engine learns a reusable forbidden combination. Future exploration can bypass all alternatives covered by that explanation instead of rediscovering each contradiction.

For example, a clash dependent on three choices among many could exclude their combination regardless of the irrelevant choices, provided its context dependencies are valid. This changes the explored search itself. Learned failures may be combined with bounded SAT/SMT encodings for suitable finite regions; a bounded unsatisfiable result must not mean global exhaustion.

**Work avoided:** repeated failed exploration, potentially across exponentially many irrelevant combinations.

**Replacement cost/risk:** explanation construction, clause management and soundness under mutable occurrence consumption. Timeouts and divergence are not failure proofs. Internal solver branching must implement the source alternatives, not invent variable assignments or rule-schedule search. General unbounded CHR is not automatically a finite solver problem.

[Lazy clause generation](https://link.springer.com/article/10.1007/s10601-026-09390-9) provides the explanatory-search precedent. Its transfer to arbitrary CHR is substantially more speculative than the compiler approaches.

## Comparative assessment

| Direction | Distinct source of gain | Main adverse regime |
|---|---|---|
| Theory-derived machine | Required theory operations become native representation operations | Weak inferable invariants; interfering consumers |
| Query specialization | Interpretation and intermediate derivations disappear | Mostly unknown inputs; specialization explosion |
| Factored rewrite expression | Shared work runs once before alternatives interact | Entangled consuming joins and equality |
| Shared producers | Recurrent computations run once across different paths | Broad mutable context; low recurrence |
| Branch machine | Operations no longer maintain global symbolic support | Wide frontier; duplicated suffix computation |
| Explanatory learning | Proven failing families need not be explored | Expensive explanations; context-sensitive failures |

The first two can support either execution foundation. Factoring and producer reuse exploit different kinds of sharing and can coexist, but combining everything at once would obscure whether each mechanism earns its cost.

## Recommendation

Make **compiler-derived execution** the primary bet: compile the rule-defined representation and query together, aiming at specialized graph transformations rather than a faster implementation of the current interpreter. Treat supercompilation as a serious route to that outcome, not a mandatory technique or a universal answer.

Compare two genuine runtime foundations: factored expression rewriting for work shared across large families, and compiled branch-local execution for inexpensive individual operations. Use context-based producer sharing where it can obtain genuinely small, sufficient boundaries. Do not make either unconditional BDD sharing or unconditional branch expansion the default architectural axiom.

My strongest architectural experiment is whether the compiler can turn a relational structural theory plus a consumer into direct graph operations, preserving a legal source execution. The strongest runtime discriminator is a common long derivation beside many choices, followed by progressively entangled equality and consumption. A producer experiment should separately vary irrelevant surroundings, relevant aliases and consuming competitors. These are proposed discriminators, not new completion obligations.

Judge alternatives by semantic behavior, answer latency and throughput, live/peak memory, compilation cost and reclamation under continued use. Current condition-job counts or coordinate-maintenance counters have no authority over replacement designs.

## Open questions

- Which rule-defined invariants and computation boundaries can be established cheaply across programs beyond the synthesis examples?
- Does selective distribution or branch-local execution have the better total cost as correlation and consumption increase?
- How much useful specialization remains when both the synthesized graph and much of its relational context are unknown?

These require architectural experiments, not new language restrictions from the user. No engine changes or benchmark reruns were made for this brainstorm.

## Agent inputs

Five independent agents examined the same problem with different inspiration words. Their strongest grounded proposals concerned compiler-derived representations, context-sensitive producers and algebraic factoring. Specialization and explanatory learning are additional directions in the lead assessment, supported by the linked research. The recommendation is not a vote count.

| Agent | Five inspiration words |
|---|---|
| Bacon | estuary, loom, quartz, migration, fold |
| Aquinas | orchard, prism, turbine, echo, weave |
| Turing | glacier, mosaic, ferment, orbit, copper |
| Pasteur | mycelium, lens, avalanche, tessellate, harbor |
| McClintock | crystal, delta, kiln, relay, germinate |
