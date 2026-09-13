# r2_c3_single_world_flat_occurrences — declared experiment

Round 2 candidate 3; sole worker; no variants. Assigned checkout
`/tmp/chr-cycle7-opt-single-world-flat-occurrences`, branch
`codex/opt/single-world-flat-occurrences`. Starting HEAD `43b4a0699345238e097c6b6ad9d705291c7b81ad`;
immutable campaign baseline `3f5bb81` differs only in campaign documentation/evidence.
No landing checkout writes. The archived worker and measurement protocol in
`../r1_c2_central_occurrence_support/protocol/` apply, except the explicit user
requirement to judge maintained diagnostics supersedes timing-ratio gates.

Diagnosis reused from candidate 2's maintained fresh observations: grouped
64/256 groups execute 2,048/8,192 applications, 3,648/14,592 body posts, and
sample 23,040/92,160 graph nodes. `Graph::insert`/updates create occurrence,
incidence and lookup records in the persistent store; `Store::insert_node`
retains/releases tree nodes and collection traverses/prunes those roots.
Fresh markers deliberately retain each producer witness. This same source and
oracle makes the prior work attribution applicable. The unresolved question is
whether flat physical storage avoids those costs after charging its own work.

One bounded executable prototype, opt-in through the maintained measure example.
Recognize source structure, not names: a finite linear chain of binary consuming
producers, each posting a fresh binary cell, passive binary witness, and next
control; a terminal producer posts a passive swapped endpoint. Cell consistency
is optional exact `cell(K,V) \\ cell(K,W) <=> V=W`. Distinct signatures per level,
no additional consumers, no propagation, explicit choice, query equality,
intermediate-cell admission, cycles or cross-family rules. Uncertain source and
unsupported history/choice observations use the incumbent from initial admission.

Flat records preserve occurrence IDs, raw fresh IDs, separate application and
post events, and dead slots. Union-find canonicalizes identity; per-root family
tables track live cells. Equality migrates losing-root attachments and queues
rechecks. FIFO activation plus finite producer rank establishes eventual finite
progress. Snapshots pin the entire store; mutation uses whole-store COW. Outputs
expand one occurrence/port at a time. Pending bodies retain source rule, PC,
event and bound/fresh slots. Cancellation discards source queues and ownership;
held readers retain their exact old states until release.

Primary: fresh-contract groups64/256, copies4, depth4, grouped/interleaved.
Controls: unmerged64, onecopy64, depthzero64, fresh held-snapshot/output,
mid-body cancellation, and an ineligible propagation source with fallback.
Same maintained generator, exact oracle, allocation counter and incumbent work
diagnostics; shared new adapter fixture on both modes. Single observations are
causal work/allocation evidence, not runtime or statistical acceptance claims.
Serialize measurements; 5M operations per phase, 15s external/2GiB per process,
120s total native measurement ceiling subject to remaining campaign budget.

Charge parse/prepare, recognition/opcode emission separately; record writes,
fresh/parent work, family lookups/writes/migrations, body frames/opcodes, queue
push/pop/cancel, retained live/dead/capacity, snapshot/COW copies, observation
scans/canonicalization/scalars, and cancellation/destruction. Reuse maintained
phase allocator including preparation, delivery, validation and cleanup.

KEEP requires an executable, correct mechanism plus accounted avoided work
without larger preparation/execution/observation/cleanup costs. Zero incumbent
counters cannot establish gain. Whole-store COW and tombstone retention are
predeclared adverse cases. Incomplete obligations imply BLOCKED; measured mixed
tradeoffs imply INCONCLUSIVE or scoped DISCARD. No variant will be started.
