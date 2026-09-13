# Architectural hypotheses and first selection

Five differently prioritized investigations have returned. They used primary research and local evidence, with no supplied conversation history or named preferred techniques. Two reported accidentally reading ARCHITECTURE.md; their selection is therefore not fully blind. The other three reported respecting the proposal-reading boundary.

## Candidate frontier

| Hypothesis | Work avoided | Decisive uncertainty |
|---|---|---|
| Compile interacting rules into resumable procedures | Transient store occurrences and their activation, matching, commitment and lifetime machinery | How much useful computation can remain procedural when relations interact and identities change? |
| Whole-head query execution with reusable query evidence | Repeated unsuccessful partial joins and repeated establishment of unchanged subquery results | Does saved work outweigh maintenance through consumption and identity changes? |
| Reusable execution summaries under identity renaming | Separately reached but equivalent derivations | Are sufficient contexts small and recurrent enough to repay recognition and instantiation? |
| Selective joint/concrete execution of alternatives | Symbolic support and identity maintenance where applications have already diverged | Can local savings survive boundary coordination, migration and lost common computation? |

These are hypotheses, not four mandatory implementations. Compilation can operate with any of the other foundations. Reusable match evidence is narrower than general execution memoization. Failure summaries are a special case of reusable finite execution summaries; incremental trace repair is a possible extension when exact reuse fails. Neither is independently a deliverable.

Evidence precedents: [optimizing CHR compilation](https://arxiv.org/abs/cs/0408025), [CHR unfolding](https://arxiv.org/abs/0807.3979), [query fusion](https://vldb.org/pvldb/vol4/p539-neumann.pdf), [worst-case optimal joins](https://arxiv.org/abs/1203.1952), [certificate-based joins](https://arxiv.org/abs/1302.0914), [nominal incremental computation](https://arxiv.org/abs/1503.07792), [selective state merging](https://dslab.epfl.ch/pubs/stateMerging.pdf). These establish mechanisms in their own settings; CHR transfer costs remain to be tested.

## Decisions already narrowed

A direct scan of all saved notebook ASTs found no exclusive, unconditional single-head control predicate in behavior synthesis (59 rules). Arithmetic has add/sub, lambda has step/lamEq, type synthesis has infer. This is a narrow sufficient-condition scan, not evidence that stronger compilation is impossible. It rules out treating simple private-control fusion as an established synthesis solution.

A private-control traversal through immutable relations supplies a clean compiler experiment with actual relational joins, but excludes dynamic identity effects and interacting controls. A win would establish an execution opportunity in that class, not general-language superiority. It should not receive repeated refinement merely because a small result is easy to improve.

The whole-head proposal supplies a contrasting witness: r(X,Y),s(Y,Z),t(U),v(U) ==> hit(X,Z,U), where r/s admit a product while t/v have no matching identity. Maintaining the latter component's emptiness can avoid exploring the former product. The baseline's actual scaling and scheduling have not yet been measured on this witness, so the predicted amplification must be checked before a runtime implementation is commissioned.

Absence of successful answers does not establish finite exhaustion. The search-reduction investigator corrected its abstract-viability proposal after a targeted challenge. Safe failure summaries need finite legal operational coverage, including source alternatives they generate. Schedule freedom permits choosing a legal failing derivation even when competing schedules differ; all-schedule equivalence is not a requirement. This narrows learning to an experimentally useful operational mechanism without inventing a semantic restriction.

## First experiment: establish the value of whole-head execution

The next decision is whether the disconnected-head witness exposes a substantial algorithmic cost worth replacing, beyond a changed local join-order heuristic. First measure the baseline on increasing sizes, empty and one-hit variants, using total completion time and the existing diagnostics only as explanatory evidence. Include a cyclic rejected join as a structurally different check and a successful case to distinguish avoidable work from required output. No new general perf subsystem.

Budget for this initial discriminator: at most 30 minutes implementation/validation effort for a source-generated measurement case if the maintained harness cannot already express it; at most 120 seconds total measurement time, 3 seconds per native observation, 5 seconds external deadline, five observations only at sizes needed to establish the trend. Reuse compiled baseline. Do not run concurrent timing campaigns. Stop at a clear scaling difference or honest censoring; do not repeatedly adjust sizes for a nicer graph.

Advance to a bounded whole-head prototype if the baseline demonstrates substantial avoidable scaling (superlinear work while the empty component needs linear inspection) and a source-driven alternative can include its own setup/maintenance costs. Compare against an improved on-demand plan as well, so a simple plan improvement is not misattributed to a new architecture. If the baseline avoids the proposed excess, reject this witness as grounds for that prototype and select the next high-value uncertainty. Either result changes the decision.

A complete prototype comparison must subsequently include invalidation by explicit equality and consuming rules before claiming a robust foundation. Those become required only if this hypothesis advances, not because they appeared in research notes.

## Stronger compiler region established during T003

A follow-up inspection of all 59 behavior-synthesis rules identified a materially stronger opportunity than exclusive control fusion. The first 45 rules are nine consistency rules and all 36 pairwise clashes for nine structural-description relations. Their source rules enforce at most one consistent description per root identity within each alternative. The other 14 rules retain these descriptions, except a consuming interaction that fails. A legal chosen schedule normalizes descriptions after posting or merging, before evaluator use. Each consistency step consumes an occurrence and introduces only finitely many field equalities, so finite-store normalization terminates even with cyclic fields; it still needs fair suspension.

A compiler can derive conditional attachments and same-description field merging directly from this complete rule subsystem. This would avoid generic multihead discovery and commitment for those 45 rules. Conditional support, merge maintenance, consumer wakeups, residual descriptions and evaluator search remain real costs. Equal shapes do not merge their root identities. This is a program-derived representation opportunity, not built-in structural syntax.

Existing evidence was reused rather than running another campaign: /tmp/chrimp-observer-calibration/diagnostics-behavior/1.stdout, the source checkpoint for unrestricted identity synthesis. Its 59 rule records align with the current notebook ordering. The first 45 rules account for:

| Counter | Subsystem | All rules | Share |
|---|---:|---:|---:|
| Matching dispatches | 147570 | 215115 | 68.6% |
| Indexed candidate visits | 4221 | 6037 | 69.9% |
| Tasks created | 3350 | 4776 | 70.1% |
| Committed applications | 35 | 383 | 9.1% |
| Commitment dispatches | 1978 | 16543 | 12.0% |

These are work counts, not CPU proportions or predicted savings. They establish that most matching effort in this observation concerns the lowerable subsystem despite relatively few successful applications. The next representation experiment should include conditional field merging that enables evaluator matching, cycles, and the actual unrestricted synthesis query. A general scheduler with normalization priority is the attribution control; otherwise early-failure scheduling gains could be mistaken for representation savings. This is a promising next candidate, not an added mandatory implementation independent of the exploration decision.

## Selective-execution discriminator and compiler interaction

The candidate comparison must survive ordinary program simplification: repeated tests over unchanged identity/relations can be hoisted, and private transient rule chains can be fused. Fixed physical application counts cannot be used to prevent that. Compilation may subsume selective grouping on such a workload; a timing win over an executor forced to retain those operations would not establish an architectural advantage.

The targeted study identified a stronger bounded slice: two explicit alternatives execute runtime-supplied allocation plans. Each plan interleaves equality merges with consuming cell lookups and emits assigned(Request,Payload). Different merge/allocation orders can reach the same final equivalence classes while producing different assignments. A shared suffix computes a data-dependent composition of finite relations and emits value(X,Z). Both the assignments and composition are variable-sized observable outputs, so necessary input/output work survives fusion. Any input-dependent specialization is charged, even if it performs work before execution.

For the fixed program family, a whole-rule boundary certificate separates the local allocation effects from unchanged composition inputs. Shared input records and the composition remain shared; local equality/liveness and assignments remain alternative-specific. A disconnected head coupling assigned with scan invalidates the certificate. Compare joint conditional execution against concrete local execution with shared composition, allowing equivalent compilation and including setup, boundary publication, full residual delivery and release. No fixed rule-application count is required; reuse evidence checks common computation without requiring literal source execution.

This is a candidate experiment, not an activated implementation obligation. If compilation makes joint execution equally efficient, that is a useful conclusion in favor of compiler-selected representation rather than a dynamic grouping policy. The current normalization prototype's results will inform whether and how to commission this comparison.
