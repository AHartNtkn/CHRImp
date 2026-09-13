# Factored isolated components — round 2 candidate 2

Assigned worktree: `/tmp/chr-cycle7-opt-factored-isolated-components`, branch
`codex/opt/factored-isolated-components`. Starting revision `3f5bb819cb0f496d5fb261bade0660277caf2693`.
Engine baseline `bcb215e`: `git diff bcb215e HEAD -- src examples tests Cargo.toml Cargo.lock`
is empty. The intervening commit contains only the previous candidate's evidence
and campaign records. No shared binary or landing checkout will be changed.

## Diagnosis and experiment

The maintained fresh generator posts one independent group plus four producer
chains per group. At depth four, each group requires 20 producer applications,
12 contractions, 57 body posts, and 104 scalar output events including query
variables, plus two answer framing events per query. Recheck exact accounting
against the native reports rather than treating this prediction as a measurement.
`src/engine.rs` posts every occurrence through `Graph::post`; each producer gets
its own direct-anchor search/commit and fresh variable vector. Normalization
already avoids contraction pair enumeration. Graph/store insertion, collection,
and release still operate on the logical copies. Prepared instructions are
already shared; sharing those again would not test this hypothesis.

Hypothesis: a certified isolated alpha-equivalent component can execute once,
with template-local occurrences/variables/events and separate logical instance
renamings and frontiers. This should avoid repeated matching and graph mutations,
while retaining logical application/post boundaries and exact residual witnesses.

Only finite deterministic consuming producers and same-key contractions are in
scope. Certification must exclude disjunction, propagation, all additional
consumers, disconnected multiheads, equality that can join ownership domains,
and uncertain cases. Variable-disjoint query syntax alone is insufficient.
Reachable fresh values and equality edges require an inductive ownership proof;
initial connected-component discovery does not supply it. All unsupported
programs must continue on the incumbent engine.

The implementation gate is the committed frontier: late stepping may leave
equivalent instances at different events; snapshots freeze both their graph and
pending syntax, even without history. Sharing one mutable current canonical root
or replicating just a final answer fails that gate. A safe mechanism needs a
versioned template frontier plus instance/event/occurrence mappings, observation
expansion and bounded release integrated with the public Engine APIs.

Use at most a 20-minute local feasibility slice within the existing campaign
budget, no variants, no renewed cycle budget. Stop BLOCKED if that executable
mechanism cannot be completed safely. A boundary fixture or certificate alone
does not qualify as an optimization. Do not build a second ordinary executor and
count that as factoring.

## Evidence and gates

Primary: maintained fresh-contract groups 64/256, copies 4, depth 4, both grouped
and interleaved. Controls: fresh-unmerged, one copy, rewrite128, non-equivalent
components and unsafe cross-component rules; held/late observation with history
disabled. Use single diagnostic observations for work attribution, not timing
acceptance. Serialize native runs; cap at 5M dispatches/5 seconds per phase,
15 seconds external/2 GiB and at most 120 seconds total native measurement from
the existing budget. No baseline/candidate comparison is meaningful until an
executable candidate exists.

Charge canonical and logical applications/posts, preparation of templates and
instances, frontier versions and storage, graph mutations/nodes, matching,
collection/pruning, release, phase allocations, observation expansion and peak
retention. Check complete cleanup after retained readers release their ownership.
Maintain every source event, occurrence and fresh identity, per-instance
round-robin progress, snapshot/stepping semantics and exact residual multiplicity.
KEEP requires correctness and avoided work exceeding all replacement costs;
missing costs or an incomplete executable path mean unassessed/BLOCKED.

The archived repository worker/protocol documents under
`../r1_c2_central_occurrence_support/protocol/` were read. The user's maintained
diagnostic policy supersedes their generic timing-ratio acceptance protocol;
no timing hypothesis test, alpha allocation or timing acceptance helper applies.
