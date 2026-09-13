# Architectural spike reassessment

The compiler and ownership spikes demonstrate useful work avoidance. Reverse ordering demonstrates a representation-scaling improvement with an adverse ordering tradeoff. Projected overlap avoids intermediate construction but replaces it with predicate traversal. Relative completion does not establish a general benefit. Shared expressions improve prefix storage but incur severe allocation growth during exact queries.

These are diagnostic conclusions, not program-runtime rankings. The subsequent integration combines compiler and ownership; its current measurements and production status are in [optimization state](optimization-state.md).

## Method and limits

Used the maintained release `measure` executable with `diagnostics`, its semantic validators, allocator phase accounting, and `supervise.py` resource limits. All 44 initial observations and 10 counter followups completed. No rwLog runs. Commands and typed output are in `/tmp/chr-reassessment/observations.json` and `followup.json`; these are local experimental evidence, not a maintained subsystem.

Allocation traffic below sums requested bytes in setup, engine, validation, delivery, cleanup, inspection and calibration, excluding `other`. It is neither retained memory nor RSS. MB means decimal MB. Logical operation counts are taken before cleanup; allocation traffic includes cleanup. Continuation calls are not node visits. Transform actions are comparable between BDD variants, but not between BDD and expression implementations. Counts with different meanings are not combined into a synthetic score.

Workloads use `measure FAMILY SIZE 50000000 3`, except readers use four seconds and `--rows 4 --work N`; empty alternatives use `--rows 0`; ring uses `--shape ring --seed 0`. Limits bound collection, never determine gains. Matching answer counts do not imply identical synthesis witnesses: schedule freedom remains, and source evidence of precluded impossible derivations is necessary for the compiler conclusion.

## Findings

### Source-derived producer/consumer compiler: real avoided execution

For `notebook-behavior-i`, baseline → spike:

| Validated answers | Choice births | Relation posts | Task creations | Allocation MB |
|---|---|---|---|---|
| 1 | 60 → 43 | 325 → 249 | 1,651 → 1,306 | 20.42 → 14.70 |
| 2 | 358 → 229 | 1,855 → 1,259 | 9,653 → 6,827 | 170.96 → 103.80 |
| 4 | 492 → 325 | 2,487 → 1,761 | 13,037 → 9,504 | 257.87 → 156.33 |

At four answers compaction transform actions fall from 372,581 to 62,378. The compiler excludes constructor alternatives that source-derived terminal consumers would reject. Its certificate requires a persistent exclusion witness, including the notebook's retained incompatible constructor. Surviving bodies retain their multiplicity.

Introduced costs are preparation certification, marker scans, equality checks and support routing; repeated subrange scans remain. Ordinary matching counters omit those scans, so they cannot establish total matching savings. Allocation totals include the added machinery and still fall substantially. The unchanged `common 16` control has 32 commits, 48 posts and 133 tasks in both versions; allocation is 0.580 → 0.582 MB. Existing ordinary/diagnostics test and semantic validation evidence supports this spike. This is a substantive improvement, not merely a faster schedule.

### Incremental archive ownership: real bookkeeping reuse

`life-archive-rotate-conditional 4 --rows 4 --work N`:

| N | Baseline allocation MB | Ownership allocation MB | Held conditions, both |
|---|---|---|---|
| 32 | 12.01 | 10.57 | 701 |
| 64 | 42.83 | 33.35 | 2,413 |
| 128 | 189.22 | 120.44 | 9,039 |

Semantic operation counts and compaction transform work match. Protection-transition propagation and a retained canonical table avoid repeated closure traversal/table reconstruction. Added root inventories and reference bookkeeping do not shrink the retained condition graph: that representation still grows rapidly. At N=128 peak requested process memory is essentially unchanged (~4.45 MB). The benefit is lower allocation and repeated maintenance, not improved retained-state scaling.

### Reverse decision order: real scaling improvement, conditional on graph shape

The same readers retain 72, 136 and 264 conditions at N=32,64,128, versus 701, 2,413 and 9,039. Allocation becomes 7.98, 23.84 and 80.89 MB. Newest-first decisions permit prefixes to share their existing representation; retained prefixes scale linearly here. Source/unit evidence identifies the opposite suffix tradeoff.

The adverse case is measurable: for 24 empty alternatives, allocation grows from 0.822 to 1.376 MB, with the same 24 answers and 23 choice births. At 8 and 16 alternatives it is slightly lower. The spike also changes sifting's ordering freedom, so this is evidence for the combined ordering implementation, not attribution to a single switch. It is not justified as an unconditional replacement.

### Projected overlap: intermediate construction avoided, traversal substituted

At `bits-star-delayed 32`, transform actions fall from 7,746 to 1,174, while the new predicate performs 4,867 pair-continuation actions (5,425 calls including completion/drain). Compaction Boolean actions remain 620. Allocation traffic is effectively unchanged: 11,756,467 → 11,759,435 bytes; allocation requests fall 63,959 → 62,471. Semantic operation counts match.

At `life-dependent 16`, transform actions fall 4,316 → 3,101, with 818 new predicate actions; allocation increases 4,472,312 → 4,488,072 bytes. Peak requested memory is unchanged in both cases.

The predicate decides overlap without constructing both projected images and their intersection; successful disjointness still requires the positive image for substitution. Pair memoization and traversal/release replace that construction. This is demonstrated construction avoidance, but these unlike action counts do not establish a net reduction in computational cost. No meaningful storage improvement is established.

### Relative completion: benefit remains special-case, not general

The direct fixture proves avoidance of aggregate conditions for out-of-scope obligations. Full-engine results do not establish a general improvement:

- `life-dependent 16`: scanned rows 2,681 → 2,424, but completion Boolean actions 1,162 → 1,584, compaction transform actions 4,316 → 4,578, and allocation 4.472 → 4.558 MB. Semantic operation counts match.
- `bits-star-delayed 32`: completion Boolean actions remain 210; `answers 24` remains 130. Allocation is essentially unchanged.
- One-answer identity followup: completion actions 13,049 → 12,602, but posts differ (325 → 326), allocation increases slightly, and the cause is not isolated.

Fewer scanned rows alone therefore do not establish improved execution. Captured-demand subtraction changes intermediate conditions and scheduling. This spike has a proven narrow construction property, but no demonstrated workload-level overall gain.

### Shared expression support: storage improvement offset by expensive exact queries

Prefix construction shares linearly in the direct test. In full execution, exact terminal classification eagerly explores cofactors; traversal memoization is local and structural interning does not establish semantic equivalence.

For 8, 16 and 24 empty alternatives, baseline allocation is 0.175, 0.423 and 0.822 MB; expressions allocate 0.272, 0.961 and 44.104 MB. At 24, requests rise from 1,749 to 128,124. Both deliver the same 24 answers and complete cleanup. The eight-node ring also increases allocation from 1.962 to 3.454 MB with matching semantic operation counts. This establishes an adverse allocation-scaling effect without using runtime or work limits as verdicts.

The spike's existing correctness validation is incomplete (completion/work-cap failures and an unmigrated structural-equality assertion). These do not negate the storage property, but preclude treating this implementation as ready for integration. This result applies to this eager classification implementation, not all expression representations.

## Consequence for selection

Ownership and the compiler merit integration consideration on demonstrated work/allocation savings. Reverse order offers a major representation improvement for one family, with a demonstrated adverse family; retain that distinction. Overlap is a narrower construction tradeoff. Relative completion has not demonstrated general benefit. The current expression backend does not justify replacement despite its real prefix-storage property. Combined benefits of separate spikes have not been measured and are not assumed additive.

Completion Boolean accounting is now available in maintained diagnostics; projected-overlap accounting lives with that experiment. All three instrumented variants passed the 11 diagnostics tests and release diagnostic builds. Current selection authority is this reassessment, with prior cycle records retained as experiment history.
