KEEP a correct change that materially improves any relevant measure of work,
storage, representation, or scaling without significant regression elsewhere.
These dimensions are independently valuable: a storage improvement does not
require a semantic-work improvement, and total allocation is not a summary gate
for other measures. Useful simplification without material regression also qualifies.

Assess the actual implemented change and its measured benefits. A useful partial
improvement can be kept even when it falls short of the original proposal. Pursuing
ambitious candidates does not raise the acceptance bar for gains already achieved.
Weigh concrete added complexity against those benefits; patch size or a narrower
result alone is not a reason to discard it. Identify a specific significant cost
or regression if it outweighs a demonstrated benefit.

Demonstrate what work or storage a change removes: for example, fewer repeated
joins, less retained data, or slower growth of work as inputs increase. Compare
runs that fulfill the same semantic obligations, including answer multiplicity
and progress. Fewer internal ticks are not by themselves a gain when the meaning
of a tick or the amount of completed work differs between implementations.

Include affected preparation, query execution, answer delivery, retention, and
cleanup costs so moving a cost between phases does not appear to remove it.
Runtime can point to a problem worth investigating, but must not determine
architectural gains, candidate ranking, or acceptance. Existing rwLog comparison
results are in `docs/validation.md`; do not rerun rwLog benchmarks without an
explicit request.

Evaluate all relevant measured dimensions before deciding. An inconclusive result
on one workload does not invalidate a demonstrated gain on another; limit the
claim to the evidence. Preserve uncertainty where a comparison is not meaningful.

Consider changes to algorithms, representations, and the execution architecture
when they can remove substantial work or complexity. Judge the proposed mechanism
and its evidence, not the size of the patch or familiarity of the current design.
A failed implementation does not establish that every implementation of its idea
will fail.
