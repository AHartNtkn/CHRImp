# r2_c1_unused_port_index_omission — round 2 candidate 1

Baseline worktree HEAD bcb215e82a9b1279e0858840c13359d39123a2bf differs
from campaign 505a107 only in campaign documentation. No source difference.
The local baseline build will include the same diagnostic counters as candidate.

Source diagnosis: Graph::Update writes FACT + RELATION + one PORT and one
INCIDENCE per argument, plus an optional whole-tuple PORT. Persistent updates
copy paths; collection/pruning and deferred release traverse those nodes.
Prepared source rules already enumerate every kept/removed head before
constructor trigger lowering. Absence there certifies that normal source
matching cannot query a relation. Output/inspection uses retained occurrences
and identity, not PORT. Record this certificate before lowering (lowered
triggers alone cannot certify absence). No relation-name assumptions.

Hypothesis: omit only those certified absent-head PORT entries, retaining all
other occurrence, support, identity, pending-body and history representations.
Standalone Graph constructors keep every port index. Certified graphs use
pinned relation cursors filtered by immutable tuple arguments for public port
lookup and exact fallback counts. No shared restriction may use an absent PORT
subtree as a version witness.

Decision observations: maintained measure fresh-contract groups64/256,
copies4/depth4; fresh-unmerged64, fresh-contract64/copies1/depth4 and
copies4/depth0; rewrite128, partial-join128/rows8; retained archive and late
snapshot/stepping/cancellation fixtures. One diagnostic observation per side,
same source/oracles; repeat only to resolve a concrete uncertainty. Work counts
and requested bytes are descriptive mechanism evidence, not runtime inference.
No timing significance helper or runtime acceptance gate applies: user and
repository architectural rules supersede archived timing-worker defaults.

Charge preparation (existing head enumeration + new signature flag pass and
per-engine configuration), all graph write categories, node allocation/copying,
collection/pruning, graph release pairs, allocation phases, exact fallback rows,
live/peak memory and final reclamation. Falsifier: material avoided PORT work is
replaced by preparation, scanning, retained state or maintenance growth, or any
semantic mismatch. A reduced counter alone is not acceptance. Full semantic
checks and explicit limitations accompany the verdict. No variants, landing
checkout edits, or shared baseline mutation. Initial scope: 20 active minutes
for executable mechanism, then bounded verification within campaign budget;
120 seconds maximum serialized workload execution (builds separate).
