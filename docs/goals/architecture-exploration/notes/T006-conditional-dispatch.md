# Avoid creating temporary choice coordinates

## Decision and evidence

After direct structural normalization, the fixed W checkpoint still spends substantial time maintaining conditional state. A frame-pointer CPU profile of 18,639,360 source steps plus the existing one-million-step capped cleanup yielded 1,773 validated samples: collection appears in 62.0%, coordinate compaction in 37.0%, compaction with condition transformation in 11.6%, and compaction without that frame in 25.4%. Graph/pending substitution appears beneath compaction in 5.6%. These inclusive categories overlap. The remainder outside transformation is not proven to be pure dependence discovery. Evidence: `/tmp/chr-structural-cpu-w/{stacks.folded,flame.svg,stderr,process.json}`; one bounded run, using maintained profiling stack normalization/sample validation and supervisor. The process reports the already-diagnosed cleanup-cap exhaustion, not successful complete execution.

The independent sharing and adverse-workload investigators favor avoiding global ownership of temporary choices before experimenting with lazy rebasing or stable support handles. Those narrower mechanisms retain dependence discovery and may principally reduce a smaller cost. A general continuation-boundary runtime is plausible but its boundary costs and interaction coverage remain unestablished.

The selected smaller executable discriminator is program-derived conditional dispatch. Both actual evaluator rules start their alternatives with distinct structural descriptions of the same input identity. The admitted consistency/clash subsystem proves that an incompatible leading description fails in finitely many legal source steps. Existing supported descriptions can therefore determine the surviving arm before any fresh choice coordinate is exported. W has two initially known application roots; later frequency and performance benefit are unmeasured.

This combines structural representation lowering with a compiler transformation that prevents avoidable search-state creation. It is not a consumer-matcher tuning exercise, a promise to implement all continuation mechanisms, or evidence that this source pattern solves global conditional-state lifetime.

## Bounded prototype

Worktree `/tmp/chr-arch-conditional-dispatch`, branch `codex/arch-conditional-dispatch`, starting at structural prototype `f2cf1ceb094ebd7f1386992563c9089ea711c478`. One worker, 45-minute initial implementation/validation budget. Compare direct normalization alone with direct normalization plus conditional dispatch in the same experimental command. Preserve the original baseline/priority controls.

Recognize the source shape from prepared bodies and the existing whole-program structural invariant, independently of predicate/rule names and ordering. On a disjunction whose alternatives begin with distinct recognized tags at the same key, resolve the key under the actual application support and inspect symbolic attachment partitions. Admit the corresponding original arm under each compatible known partition without introducing another coordinate. Known unsupported tags fail only on their supported region. Uncovered support executes the original generative alternatives. Duplicate leading tags and other unsupported shapes retain their exact ordinary behavior; do not collapse their multiplicity.

Keep the selected original arm, including its leading post and direct normalization, in the first comparison. That preserves field identity effects while isolating avoided choice creation; a separate field-binding optimization is not part of this experiment. Do not enumerate concrete worlds. Long classification and admission work must yield, keep completion obligations, and remain traceable/cancelable through existing ownership. Do not change consumer scheduling or inner loops merely to improve timings.

Validate source renaming/reordering, conditional known/uncovered support, distinct field bindings in different alternatives, unsupported known tags, duplicate alternatives, cycles/divergent continuations with finite siblings, shared independent choices, exact residuals and affected cancellation/collection behavior. Use the full behavior program and architecture-independent answer oracles. Unsupported program-shape specialization is an experiment boundary, not a new language restriction.

## Comparative experiment and stopping

After focused validation, use at most 120 seconds total measurement time, serialized. Reuse the direct-normalization results when the command and input remain comparable; new paired comparisons should use the same binary/build settings. Measure setup, requested answer/complete output, cleanup, and peak/retained memory where the outcome requires them. Start with unrestricted I and previously censored B/W, plus a finite repeated known-description choice case demonstrating whether state creation scales with useful alternatives or repeated dispatch. Include a genuinely unknown/duplicate-alternative control so avoided work cannot come from changing the source search.

Five observations only at useful points; three-second source observations, bounded external execution, honest source/cleanup censoring. Do not extend caps to obtain a favorable answer. Add no general replay system or instrumentation framework.

Advance for a substantial end-to-end gain or scaling improvement with exact alternatives, shared computation and release preserved. If only the specially constructed recurrence improves, report that scope and reassess broader continuation lifetime before recommending it for synthesis. If overhead, rarity or escaping effects erase the gain, reject this as the leading synthesis mechanism. Either result informs whether a more fundamental continuation/support redesign merits implementation. No loop-tuning campaign follows a weak result.

## Other candidates

Memoization is deferred: the targeted source study found no concrete recurring CHR context with demonstrated avoided work and a sufficient reusable boundary. Repeated rule application counts and rwLog context hits do not establish that evidence. Stable support handles and lazy interpretation remain alternatives if upstream choice prevention fails to address the measured problem. No candidate is independently a required deliverable.
