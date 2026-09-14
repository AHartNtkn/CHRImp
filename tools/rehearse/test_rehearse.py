import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import rehearse as r


def judgments():
    # Lower index wins, independent of presentation order.
    return {(a, b): {"winner": "A" if a < b else "B", "confidence": 0.8,
                     "reasoning": "Synthetic fixture"} for a, b in r.PAIRS}


class RehearseTest(unittest.TestCase):
    def test_project_policy_is_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "AGENTS.md"
            for text in ("# Project\n", "## Evaluating optimization\n\n## Other\n"):
                path.write_text(text)
                with self.assertRaises(ValueError):
                    r.acceptance_policy(path)

    def test_reverse_order_consensus(self):
        result = r.tally(judgments())
        self.assertEqual(result["selected"], 0)
        self.assertEqual([s["votes"] for s in result["scores"]], [4, 3, 2, 1, 0])

    def test_disagreement_gives_neither_a_vote(self):
        data = judgments()
        data[1, 0]["winner"] = "A"
        result = r.tally(data)
        self.assertEqual([s["votes"] for s in result["scores"]], [3, 3, 2, 1, 0])
        self.assertEqual(result["selected"], 0)

    def test_confidence_breaks_vote_tie(self):
        data = judgments()
        data[1, 0]["winner"] = "A"
        for a, b in [(1, 2), (2, 1), (1, 3), (3, 1), (1, 4), (4, 1)]:
            data[a, b]["confidence"] = 0.9
        self.assertEqual(r.tally(data)["selected"], 1)

    def test_all_disagree_uses_proposal_order(self):
        data = judgments()
        for v in data.values():
            v["winner"] = "A"
        result = r.tally(data)
        self.assertEqual(result["selected"], 0)
        self.assertEqual(sum(s["votes"] for s in result["scores"]), 0)

    def test_missing_or_invalid_judgment_cannot_select(self):
        data = judgments()
        del data[0, 1]
        with self.assertRaises(ValueError):
            r.tally(data)
        for invalid in (True, float("nan"), float("inf"), 0.49, 1.01, "0.9"):
            data = judgments()
            data[0, 1]["confidence"] = invalid
            with self.assertRaises(ValueError):
                r.tally(data)

    def test_retrieval_threshold_all_matches_and_minimal_surface(self):
        records = [{"description": str(i), "outcome": outcome, "evidence": "secret"}
                   for i, outcome in enumerate(("KEEP", "DISCARD", "KEEP"))]
        def encode(texts, normalize_embeddings):
            self.assertTrue(normalize_embeddings)
            self.assertEqual(texts, ["new", "0", "1", "2"])
            return [[1, 0], [0.4, 0.916515139], [1, 0], [0.39, 0.920815]]
        found = r.retrieve([{"description": "new"}], records, encode)[0]
        self.assertEqual(found, [{"description": "0", "outcome": "KEEP"},
                                 {"description": "1", "outcome": "DISCARD"}])

    def test_empty_history_needs_no_encoder(self):
        with patch.object(r, "encoder", side_effect=AssertionError):
            self.assertEqual(r.retrieve([{}] * 5, []), [[]] * 5)

    def test_round_flow_and_resume(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            proposed = [{k: f"{k}-{i}" for k in r.FIELDS} for i in range(5)]
            r.write(base / "proposals.json", proposed)
            (base / "context.md").write_text("Accepted baseline context")
            hist, trial = base / "history.jsonl", base / "round-001"
            r.prepare(base / "proposals.json", base / "context.md", hist, trial)
            self.assertEqual(len(list(trial.glob("judge-*.md"))), 20)
            prompt = (trial / "judge-1-0.md").read_text()
            policy = r.acceptance_policy()
            self.assertEqual((trial / "acceptance.md").read_text(), policy + "\n")
            for file in trial.glob("judge-*.md"):
                self.assertIn(policy, file.read_text())
            self.assertLess(prompt.index("description-1"), prompt.index("description-0"))
            with self.assertRaises(FileExistsError):
                r.prepare(base / "proposals.json", base / "context.md", hist, trial)
            with self.assertRaises(FileNotFoundError):
                r.select(trial)
            for (a, b), value in judgments().items():
                r.write(trial / f"verdict-{a}-{b}.json", value)
            self.assertEqual(r.select(trial)["candidate"], proposed[0])
            for _ in range(2):
                r.record(trial, hist, "KEEP", "Actual change", "Measured evidence")
            stored = r.history(hist)
            self.assertEqual(len(stored), 1)
            self.assertEqual(stored[0]["description"], "Actual change")
            with self.assertRaises(ValueError):
                r.record(trial, hist, "DISCARD", "Actual change", "Different result")
            r.record(trial, hist, "DISCARD", "Different evaluated implementation", "Evidence")
            self.assertEqual(len(r.history(hist)), 2)


if __name__ == "__main__":
    unittest.main()
