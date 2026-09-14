# Rehearse for CHRImp

Run from the repository root. `AGENTS.md` holds project rules;
`docs/rehearse-goal.md` is the complete campaign prompt to paste into a Luna task.
The helper performs no LLM calls and starts no experiments.

## Setup

```sh
bash tools/rehearse/setup.sh
```

This uses `uv` to create an isolated `.rehearse/venv`, installs CPU embedding
dependencies, and caches the
pinned all-MiniLM-L6-v2 model locally. The encoder runs on CPU. `.rehearse/` is
ignored by Git and persists across rounds and campaign restarts. Keep the history
with this CHRImp campaign; other tasks start their own empty history.

## One round

Write `.rehearse/candidates.json`: an array of five objects with exactly these
nonempty string fields (repeat the shape for five distinct proposals):

```json
{
  "description": "Concrete proposed change",
  "hypothesis": "Mechanism of gain and comparison that would establish it",
  "implementation": "What to change in the current code",
  "type": "CODE"
}
```

Write `.rehearse/context.md` with the shared accepted baseline, relevant evidence,
objective, and constraints described in the goal. Each round uses a new directory:

```sh
.rehearse/venv/bin/python tools/rehearse/rehearse.py prepare \
  .rehearse/candidates.json .rehearse/context.md .rehearse/round-001
```

This snapshots the proposals/context and writes `judge-0-1.md` through all twenty
ordered pairs. Indices are zero-based. For each file, use a fresh native Codex
subagent with `model="gpt-5.6-luna"`, `fork_turns="none"`, and that file's complete
text as its message. Each agent returns one JSON verdict; save it unchanged as
`verdict-0-1.json` beside the corresponding prompt, and similarly for every pair.
Use a fresh context for each judgment, including when reusing an available slot.
These are the campaign's real selection calls, not helper tests.

```sh
.rehearse/venv/bin/python tools/rehearse/rehearse.py select .rehearse/round-001
```

`selected` is the winning zero-based index; `candidate` is the full proposal.
Implement that candidate with one fresh Astra agent (`gpt-6-astra`), as described
in the goal. After evaluating and integrating or discarding the patch:

```sh
.rehearse/venv/bin/python tools/rehearse/rehearse.py record \
  .rehearse/round-001 KEEP --evidence 'Result and path to the relevant measurements'
```

Use `DISCARD` for an evaluated patch that was not accepted. If the implemented
change differs from the selected description, supply `--description 'Actual
evaluated change'`. History is `.rehearse/history.jsonl`. Repeating an identical
record command is harmless; a conflicting outcome for the same evaluated change
in the same round is an error. A materially different follow-up competes in the
next proposal round; preserve negative results as well as wins in the history.
Unfinished work remains in the working notes, with no invented outcome.

## Method and adaptation

The [Rehearse paper, Appendices B–C](https://arxiv.org/html/2607.27687v1#A2)
specifies five proposals, two orders per pair, a vote only on agreement, and ties
resolved by mean confidence of won comparisons then proposal order. The helper
averages the two confidences of each agreed pair. All prior descriptions with
normalized embedding cosine at least 0.40 are retrieved; only description and
outcome reach the judge. The encoder revision is pinned to the paper's reference.

CHRImp replaces the training metric with its semantic cost objective and retains
the simplification exception. KEEP/DISCARD express this acceptance rule. Native
Luna judgments and Astra implementation replace the paper's execution backend;
the campaign has no training-run budget. These are adaptations, not a claim that
the paper validated this project or model pairing.

## Verification

```sh
timeout --kill-after=5s 60s python3 -m unittest discover -s tools/rehearse -p 'test_*.py'
```

The tests use synthetic verdicts and vectors to check mechanics without agent
calls. They do not establish long-run model behavior.
