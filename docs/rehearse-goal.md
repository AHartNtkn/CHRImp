/goal Continuously improve CHRImp in /home/ahart/Documents/Codex/CHRImp using the Rehearse loop below. Use Luna (gpt-5.6-luna) as orchestrator and proposal/judge model, and Astra (gpt-6-astra) for code implementation. Continue until I stop or redirect the campaign; do not create campaign or candidate time budgets.

Read AGENTS.md, README.md, relevant architecture and performance documentation,
and tools/rehearse/README.md. The objective is substantial reduction of semantic
work, storage, representation cost, or scaling cost while preserving language
semantics and existing sharing. Useful simplification with no material regression
also qualifies. Runtime is diagnostic, not the architectural acceptance metric.
Use the maintained diagnostics and semantic oracles, not a new benchmark system.

Start from the current accepted code and its existing measurements. Locate the
evidence needed for the next question; do not rebuild or remeasure the baseline
every round. Start Rehearse history empty on the first run. On continuation, reuse
the existing history and resume an unfinished experiment before starting another.
Keep a short .rehearse/current.md with the accepted revision, baseline evidence,
active experiment/worktree, and next action so continuation preserves actual work.
Use the current branch as the campaign's landing branch. Run the helper and keep
campaign history in this repository; use experiment worktrees for candidate code.

Repeat:

1. Inspect the current code and relevant evidence. Propose five distinct mechanisms
   targeting substantial opportunities, not five small variations of one idea.
   Consider algorithms, representations, execution foundations, and simplification
   on their merits. Write candidates.json as documented by the helper. Each
   hypothesis should explain what work or complexity disappears, what replaces
   it, and which semantic comparison would establish the gain. Use concise shared
   context describing the accepted baseline, relevant workloads and their current
   costly operations or retained data, priorities among those costs, and semantic
   constraints. Use the same priorities for judging and acceptance. State the
   named workload, completed answer or work interval, and work/storage measurements
   that would test each hypothesis; place this in the existing hypothesis and
   implementation fields. Do not fill judge context with the campaign transcript.
2. Run the helper's prepare command. It retrieves similar executed attempts and
   creates 20 prompts: every candidate pair in both orders. Dispatch each prompt
   to a fresh Luna subagent with no inherited conversation, one independent
   judgment per agent. Collect its JSON into the corresponding verdict file.
   Use available agent slots in batches as needed. Do not share judgments across
   agents or let a judge see the reversed comparison. Run select to choose the
   candidate once every judgment is present. Obtain a fresh independent judgment
   when a completed judge returns a malformed verdict.
3. Give the selected candidate, shared context, relevant code/evidence paths, and
   AGENTS.md to one fresh Astra implementor in an isolated worktree from the
   accepted code. Have Astra implement and debug the mechanism through a meaningful
   semantic evaluation. Only one candidate is implemented at a time. Polling
   timeouts mean keep waiting; they do not end an implementation. If interrupted,
   resume its work rather than recording an untested outcome.
4. Check correctness and compare the affected work/storage/scaling at equivalent
   semantic progress, including consequential costs displaced elsewhere. Build
   separately; guard each test invocation at 60 seconds and diagnose failures.
   Repair coding and measurement defects so the comparison actually tests the
   mechanism. A correctness evaluation showing that the chosen mechanism violates
   required semantics is a negative result, not unfinished implementation.
   KEEP a demonstrated substantial overall gain under the priorities used for
   selection, weighing displaced costs, affected workloads, and added complexity.
   Explain consequential tradeoffs; a small residual win does not justify a
   complicated mechanism whose intended gain failed. Useful simplification with
   no material regression also qualifies. Otherwise DISCARD the evaluated patch.
   Preserve the evaluated implementation as a Git commit and include its revision
   with the evidence. For KEEP, integrate and commit the validated change on the
   landing branch and update the accepted revision in current.md. For DISCARD,
   leave accepted code unchanged; retain the experiment commit for later inspection.
5. Record the concrete evaluated implementation, KEEP or DISCARD, and a short
   evidence reference with record. Describe what was actually evaluated; ordinary
   debugging iterations are not separate experiments. If a negative result points
   to a materially different promising implementation, include that proposal in
   the next selection round alongside competing ideas. A negative result for one
   implementation does not reject the underlying architectural family.
   An unfinished implementation has no outcome to record.
   Keep brief working notes for unfinished work; the helper supplies only relevant
   descriptions and outcomes to judges. After integration, the accepted code and
   its measurements become the next baseline. Continue with a fresh proposal round.

At each new round and after context recovery, return to the objective: substantial
work/storage/scaling improvements or useful simplification. Do not drift into
microoptimization because the current implementation is familiar. Let measured
mechanisms guide the next idea; similar failed attempts are context, not a ban on
an architectural direction. Keep records sufficient to resume and interpret the
experiments, without adding reporting rituals. When useful ideas run thin, inspect
another substantial cost, research relevant algorithms, revisit near-misses in
light of their measured failure, combine complementary mechanisms, or reconsider
the execution architecture. Do not fill the five slots with cosmetic variations
or stop because the obvious ideas are exhausted.

Profiling, research, isolated implementation, tests, and local integration of
validated improvements are authorized without per-round approval. Preserve user
changes. Keep campaign control instructions fixed while optimizing the engine.
Add diagnostics or regression coverage when the experiment needs them, preserving
the program semantics and expected answers being tested. Report meaningful results
concisely and continue working.
