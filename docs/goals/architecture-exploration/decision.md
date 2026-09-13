# Architectural decision

The strongest demonstrated direction is **compiling source-proven rule interactions into direct operations, then avoiding alternatives that those same rules already prove impossible**. Whole-head feasibility supplies a separate substantial gain for a different class of relational programs. These are the supported components of a compiler architecture; the experiments do not select a universal replacement for the engine's conditional-state foundation.

## Recommended components and their limits

| Component | What changes | Evidence and boundary |
|---|---|---|
| Source-derived structural normalization | A recognized consistency/clash subsystem becomes direct conditional description operations. | 3.57x faster than priority alone on 256 equivalent descriptions. Actual identity synthesis also improves. Ordinary evaluator execution and structural output remain. |
| Conditional dispatch after normalization | Existing supported descriptions select the source arm before fresh alternatives are allocated; unknown regions remain generative. | Adds 2.71x on a repeated known-description workload and 1.42x on paired identity synthesis. The controlled case uses about 15.5% more peak allocation, so direct normalization alone remains a lower-peak option there. |
| Whole-head feasibility for suitable joins | Establish whether every join component can contribute before enumerating an expensive product. | About 142x versus on-demand matching in the same static executor on the empty-component witness. Connected and dense controls show no such benefit. Dynamic identity/consumption integration is unproved. |

The meaningful combination—normalization plus dispatch—was implemented and compared directly. The evaluator's remaining heads are connected constructor/control pairs, so there is no demonstrated benefit from inserting the static disconnected-component technique there. A benchmark composed of independent workloads would not establish an interaction.

The measured compiler package preserves source-derived admission, variable-only arguments, conditional identity, explicit alternative multiplicity, residual answers, common computation and fair finite-sibling progress within its tested scope. Selected arms retain their original effects. Unrecognized bodies execute normally. Historical stepping inside fused operations remains outside the experimental commands and must be addressed during product integration.

## What remains unknown

Composition and duplicator synthesis returned no answer in five three-second observations per configuration. Their completion times and the architecture needed to close the hard-synthesis gap remain unknown. Some runs also exceeded the bounded cleanup allowance. The successful short and finite workloads do not establish sustained hard-search scalability.

A direct-normalization W profile identifies substantial conditional-state maintenance, including coordinate compaction. Choice-local continuation lifetimes could avoid more temporary global state; stable support handles or lazy interpretation could change its maintenance costs. None yet has a demonstrated total-cost advantage or sufficiently established general interaction boundary here. They remain hypotheses, not defeated alternatives or new required deliverables.

Memoization likewise lacks identified recurring CHR contexts with enough semantic information and avoided work to justify a prototype. rwLog context-table behavior does not establish that evidence for this language. No rwLog benchmarks were rerun.

## Next implementation action

Integrate source recognition, direct normalization and conditional dispatch as one scoped compiler capability. Preserve general relational execution outside proven transformations and source correspondence for optional inspection. Apply whole-head planning separately where query structure justifies it; do not mechanically combine experimental branches or generalize the static executor's admission contract.

A broader support/continuation replacement would be a new architectural investigation, justified by a concrete executable boundary and total-cost comparison. It is not necessary to establish the compiler components already supported by this exploration.

## Why the exploration stops here

The strategy was to use independent priorities to produce competing hypotheses, select bounded experiments that could change the decision, challenge gains on different and adverse inputs, and test consequential combinations. Informative negative results and explicit deferrals narrowed the frontier. Improving every remaining hotspot was never the completion criterion.

The comparison now supports an implementation direction, its applicability boundaries, a measured speed/memory tradeoff and the major unresolved foundation question. A targeted independent completion review agreed that these establish a defensible frontier. Further prototypes might improve it, but no specific unperformed comparison prevents the scoped recommendation above. There is no global-optimality claim.

Detailed evidence: [whole-head comparison](notes/T003-comparison.md), [normalization comparison](notes/T005-comparison.md), [conditional-dispatch comparison](notes/T007-comparison.md). Experimental commits: `c95cc9f6` (whole-head), `f2cf1ce` (normalization), `e5dcb37` (combined dispatch). Production execution is unchanged.
