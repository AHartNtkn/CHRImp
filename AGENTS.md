# CHRImp

CHRImp is a Rust relational rewriting engine, library, CLI, and notebook.
`README.md` explains the language syntax. `ARCHITECTURE.md`, under “Agreed
language and environment,” specifies its semantics; its proposed engine design
is not a restriction on architectural changes.

## Behavior to preserve

- Relation arguments are variables. In a rule body or query, `X=Y` makes two
  variables the same identity. Rule heads only check existing relationships:
  `p(X,X)` matches only when its arguments already share an identity; matching
  must not merge them. Cyclic relational structures remain valid unless the
  program's own rules reject them.
- Separate occurrences of a relation remain separate even when their arguments
  are identical. A `==>` rule keeps its matched relations and fires once per
  ordered combination of matching occurrences in each alternative; merging variables must not make
  that combination eligible again. Variables appearing only in a rule body
  must be fresh for every application of that rule.
- Only an explicit `;` creates search alternatives. When several rule applications
  compete, the engine commits to a permitted application rather than exploring
  every possible rule order. There is no required order within an alternative.
- An alternative returns the relations and variable connections left when no
  rule application or body work remains. Keep separate answers from separate
  successful alternatives even when those answers are identical. An unfinished
  search is neither failure nor proof that no more answers exist.
- Continuing work in one alternative must not prevent another alternative from
  reaching and delivering an answer that requires finite work, assuming sufficient
  resources. Preserve this progress through execution, answer delivery, and cleanup.
- Preserve existing sharing of both data and computation across alternatives.
  Representing more alternatives must not itself cause shared work to be repeated
  for every alternative. Reclaim data when execution, retained answers, snapshots,
  and requested history no longer need it.
- Ordinary execution must not retain a history of past states unless recording
  is enabled. CLI and notebook must use the same language core, and execution
  must continue without browser requests.

## Evaluating optimization

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
or regression if it outweighs a demonstrated benefit. Disclosure does not make
a regression acceptable: being temporary, bounded, or outside the favored
dimension does not establish that its cost is insignificant.

Demonstrate what work or storage a change removes: for example, fewer repeated
joins, less retained data, or slower growth of work as inputs increase. Compare
runs that fulfill the same semantic obligations, including answer multiplicity
and progress. Fewer internal ticks are not by themselves a gain when the meaning
of a tick or the amount of completed work differs between implementations.

Establish the claimed benefit after accounting for the work or storage that
replaces it. Fewer operations of one kind do not establish less work when other
operations are added. Use measurements or a supported algorithmic cost argument
to resolve consequential replacement costs; if they could overturn the claimed
benefit, continue evaluating or refining the mechanism before accepting it.

Assess gains and regressions at consistent workload and component scales, with
absolute costs and growth as well as percentages. A small whole-process change
does not dismiss a substantial cost in the affected component. Targeted probes
can establish useful gains, but their adverse cases count equally; unchanged
workloads that do not exercise the changed path cannot clear its regressions.

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

## Validation and campaign instructions

Use `docs/validation.md` to find the tests for the behavior being changed, and
`docs/performance.md` for maintained measurement tools. Build separately from
running tests. Each test invocation must finish within 60 seconds; use
`timeout --kill-after=5s 60s` and investigate a timeout as a test failure. That
limit does not end an implementation task.

For a Rehearse campaign, follow `docs/rehearse-goal.md` and the helper instructions
in `tools/rehearse/README.md`. They replace the cycle procedure in
`docs/optimization-loop.md`. Older experiment reports supply evidence, not
additional campaign instructions.
