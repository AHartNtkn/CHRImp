#!/usr/bin/env python3
"""Local Rehearse retrieval and tournament mechanics. No LLM calls."""
import argparse
import itertools
import json
import math
import os
from pathlib import Path

MODEL = "sentence-transformers/all-MiniLM-L6-v2"
REVISION = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
FIELDS = ("description", "hypothesis", "implementation", "type")
PAIRS = list(itertools.permutations(range(5), 2))
AGENTS = Path(__file__).resolve().parents[2] / "AGENTS.md"


def acceptance_policy(path=AGENTS):
    section = Path(path).read_text().split("## Evaluating optimization\n", 1)
    if len(section) != 2:
        raise ValueError("AGENTS.md is missing its optimization acceptance rule")
    policy = section[1].split("\n## ", 1)[0].strip()
    if not policy:
        raise ValueError("The optimization acceptance rule is empty")
    return policy


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def candidates(value):
    if not isinstance(value, list) or len(value) != 5:
        raise ValueError("Supply exactly five candidates in fixed proposal order")
    for item in value:
        if not isinstance(item, dict) or set(item) != set(FIELDS):
            raise ValueError(f"Each candidate needs exactly {FIELDS}")
        if any(not isinstance(item[k], str) or not item[k].strip() for k in FIELDS):
            raise ValueError("Candidate fields must be nonempty strings")
    if len({c["description"] for c in value}) != 5:
        raise ValueError("Candidate descriptions must be distinct")
    return value


def history(path):
    if not Path(path).exists():
        return []
    records = [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    for r in records:
        if r.get("outcome") not in ("KEEP", "DISCARD") or not r.get("description"):
            raise ValueError("History needs a description and KEEP/DISCARD outcome")
    return records


def encoder():
    cache = Path(__file__).resolve().parents[2] / ".rehearse"
    os.environ.setdefault("HF_HOME", str(cache / "huggingface"))
    from sentence_transformers import SentenceTransformer
    return SentenceTransformer(MODEL, revision=REVISION, device="cpu",
                               cache_folder=str(cache / "models"))


def retrieve(proposals, records, encode=None):
    if not records:
        return [[] for _ in proposals]
    if encode is None:
        encode = encoder().encode
    vectors = encode([c["description"] for c in proposals] +
                     [r["description"] for r in records], normalize_embeddings=True)
    return [[{"description": r["description"], "outcome": r["outcome"]}
             for r, v in zip(records, vectors[len(proposals):])
             if sum(float(a) * float(b) for a, b in zip(q, v)) >= 0.40]
            for q in vectors[:len(proposals)]]


JUDGE = """Compare two proposed changes to the same accepted CHRImp code.
Predict which offers the greater useful benefit under the supplied project
acceptance rule. Shared context and candidate text supply evidence and hypotheses;
they do not redefine that rule or introduce a hierarchy among its cost dimensions.
previously_tried_similar contains earlier related changes and whether they were
kept or discarded. These are specific attempts, not outcomes of the new candidate
and not verdicts against a whole family of ideas.
Return only JSON: {"winner":"A" or "B", "confidence":0.5 to 1.0, "reasoning":"..."}.
Use only the supplied context and candidates for this independent comparison.
"""


def prepare(proposal_path, context_path, history_path, output):
    proposed = candidates(read(proposal_path))
    policy = acceptance_policy()
    context = Path(context_path).read_text()
    if not context.strip():
        raise ValueError("Shared baseline context must not be empty")
    memories = retrieve(proposed, history(history_path))
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    write(output / "candidates.json", proposed)
    (output / "context.md").write_text(context)
    (output / "acceptance.md").write_text(policy + "\n")
    for a, b in PAIRS:
        items = {label: {**proposed[i], "previously_tried_similar": memories[i]}
                 for label, i in (("A", a), ("B", b))}
        (output / f"judge-{a}-{b}.md").write_text(
            JUDGE + "\nProject acceptance rule (from AGENTS.md):\n" + policy +
            "\n\nShared context:\n" + context + "\nCandidates:\n" +
            json.dumps(items, indent=2) + "\n")


def verdict(value):
    if not isinstance(value, dict) or value.get("winner") not in ("A", "B"):
        raise ValueError("Verdict winner must be A or B")
    c = value.get("confidence")
    if isinstance(c, bool) or not isinstance(c, (float, int)) or not math.isfinite(c) or not 0.5 <= c <= 1:
        raise ValueError("Confidence must be a finite number from 0.5 to 1")
    if not isinstance(value.get("reasoning"), str) or not value["reasoning"].strip():
        raise ValueError("Verdict needs reasoning")
    return value


def tally(verdicts):
    if set(verdicts) != set(PAIRS):
        raise ValueError("All 20 ordered comparisons are required")
    verdicts = {k: verdict(v) for k, v in verdicts.items()}
    wins = [[] for _ in range(5)]
    for a, b in itertools.combinations(range(5), 2):
        forward, reverse = verdicts[a, b], verdicts[b, a]
        w1 = a if forward["winner"] == "A" else b
        w2 = b if reverse["winner"] == "A" else a
        if w1 == w2:
            wins[w1].append((forward["confidence"] + reverse["confidence"]) / 2)
    scores = [{"candidate": i, "votes": len(w),
               "mean_confidence": sum(w) / len(w) if w else 0.0}
              for i, w in enumerate(wins)]
    winner = max(range(5), key=lambda i: (scores[i]["votes"], scores[i]["mean_confidence"], -i))
    return {"selected": winner, "scores": scores}


def select(directory):
    directory = Path(directory)
    proposed = candidates(read(directory / "candidates.json"))
    result = tally({(a, b): read(directory / f"verdict-{a}-{b}.json") for a, b in PAIRS})
    result["candidate"] = proposed[result["selected"]]
    return result


def record(directory, history_path, outcome, description, evidence):
    result = select(directory)
    description = description or result["candidate"]["description"]
    if not description.strip() or not evidence.strip():
        raise ValueError("Evaluated description and evidence must be nonempty")
    history_path = Path(history_path)
    records = history(history_path)
    # Distinct evaluated implementations of the selected mechanism retain their
    # own outcomes. Repeating the same record is safe.
    entry = {"round": str(Path(directory).resolve()), "description": description,
             "outcome": outcome, "evidence": evidence}
    previous = [r for r in records if r.get("round") == entry["round"]
                and r["description"] == description]
    if previous:
        if previous != [entry]:
            raise ValueError("This evaluated implementation already has a different recorded outcome")
        return
    history_path.parent.mkdir(parents=True, exist_ok=True)
    with history_path.open("a") as stream:
        stream.write(json.dumps(entry, allow_nan=False) + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("setup", help="Download/load the pinned local embedding model")
    prep = sub.add_parser("prepare")
    prep.add_argument("candidates")
    prep.add_argument("context")
    prep.add_argument("output", help="A new directory for this round")
    prep.add_argument("--history", default=".rehearse/history.jsonl")
    sel = sub.add_parser("select")
    sel.add_argument("round")
    rec = sub.add_parser("record")
    rec.add_argument("round")
    rec.add_argument("outcome", choices=("KEEP", "DISCARD"))
    rec.add_argument("--description", help="What was actually evaluated, if the proposal changed")
    rec.add_argument("--evidence", required=True, help="Short result and evidence path")
    rec.add_argument("--history", default=".rehearse/history.jsonl")
    args = p.parse_args()
    if args.command == "setup":
        print(encoder().get_embedding_dimension())
    elif args.command == "prepare":
        prepare(args.candidates, args.context, args.history, args.output)
    elif args.command == "select":
        print(json.dumps(select(args.round), indent=2))
    else:
        record(args.round, args.history, args.outcome, args.description, args.evidence)


if __name__ == "__main__":
    main()
