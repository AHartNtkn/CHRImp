#!/usr/bin/env python3
"""Exact sign-test analysis of independent randomized baseline/candidate pairs.

Only Python's standard library is required. See references/measurement-protocol.md
for the sampling assumptions, input format, and acceptance policy.
"""

import argparse
import json
import math
from pathlib import Path


def number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a finite number")
    try:
        finite = math.isfinite(value)
    except OverflowError as exc:
        raise ValueError(f"{name} is outside the supported numeric range") from exc
    if not finite:
        raise ValueError(f"{name} must be a finite number")
    return value


def samples(value, name):
    if not isinstance(value, list) or not value:
        raise ValueError(f"{name} must be a nonempty array")
    for item in value:
        if number(item, name) <= 0:
            raise ValueError(f"{name} timings must be positive")
    return value


def median(values):
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    low, high = ordered[mid - 1:mid + 1]
    return low + (high - low) / 2


def analyze(data):
    required = {"design", "unit", "baseline", "candidate", "alpha",
                "min_improvement", "max_regression"}
    if not isinstance(data, dict) or required - data.keys():
        raise ValueError("required fields: " + ", ".join(sorted(required)))
    if data["design"] != "paired":
        raise ValueError("this helper requires independent randomized pairs")
    if not isinstance(data["unit"], str) or not data["unit"].strip():
        raise ValueError("unit must be a nonempty string shared by all timings")
    baseline = samples(data["baseline"], "baseline")
    candidate = samples(data["candidate"], "candidate")
    if len(baseline) != len(candidate):
        raise ValueError("baseline and candidate arrays must contain matched pairs")
    alpha = number(data["alpha"], "alpha")
    improvement = number(data["min_improvement"], "min_improvement")
    regression = number(data["max_regression"], "max_regression")
    if not 0 < alpha < 0.5:
        raise ValueError("alpha must be between 0 and 0.5, exclusively")
    if not 0 <= improvement < 1:
        raise ValueError("min_improvement must be in [0, 1)")
    if regression < 0 or not math.isfinite(1 + regression):
        raise ValueError("max_regression must be nonnegative with a finite bound")
    ratios = [c / b for b, c in zip(baseline, candidate)]
    if any(not math.isfinite(r) or r <= 0 for r in ratios):
        raise ValueError("ratios must be finite and positive; rescale timing units")
    n = len(ratios)
    denominator = 2**n
    alpha_num, alpha_den = alpha.as_integer_ratio()

    def significant(tail_count):
        return tail_count * alpha_den <= alpha_num * denominator

    def tail_count(count):
        return sum(math.comb(n, j) for j in range(count, n + 1))
    # Select the narrowest one-sided order-statistic bound with coverage >= 1-alpha.
    # For continuous data, coverage at X_(k) is P[Binomial(n, 1/2) <= k-1].
    # Atoms/ties at a population median make this bound conservative.
    cumulative = 0
    order = None
    for k in range(1, n + 1):
        cumulative += math.comb(n, k - 1)
        if significant(denominator - cumulative):
            order = k
            break
    ordered = sorted(ratios)
    upper = ordered[order - 1] if order is not None else None
    lower = ordered[n - order] if order is not None else None
    improvement_limit = 1 - improvement
    regression_limit = 1 + regression
    improvement_tail = tail_count(sum(r < improvement_limit for r in ratios))
    noninferiority_tail = tail_count(sum(r < regression_limit for r in ratios))
    regression_tail = tail_count(sum(r > regression_limit for r in ratios))
    return {
        "method": "exact_paired_sign_v1",
        "estimand": "population median of candidate/baseline paired runtime ratios",
        "pairs": n,
        "unit": data["unit"],
        "alpha": alpha,
        "baseline_median": median(baseline),
        "candidate_median": median(candidate),
        "paired_ratio_median": median(ratios),
        "ratios": ratios,
        "min_improvement": improvement,
        "max_regression": regression,
        "upper_order_statistic": order,
        "ratio_upper_bound": upper,
        "ratio_lower_bound": lower,
        "bound_note": "Each bound is one-sided at 1-alpha; null means unbounded.",
        "improvement_p": improvement_tail / denominator,
        "noninferiority_p": noninferiority_tail / denominator,
        "regression_p": regression_tail / denominator,
        "p_exact": {
            "denominator": denominator,
            "improvement_numerator": improvement_tail,
            "noninferiority_numerator": noninferiority_tail,
            "regression_numerator": regression_tail,
        },
        "improvement_supported": significant(improvement_tail),
        "noninferiority_supported": significant(noninferiority_tail),
        "regression_supported": significant(regression_tail),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="JSON file of paired timings and declared bounds")
    args = parser.parse_args()
    try:
        data = json.loads(args.input.read_text())
        result = analyze(data)
    except (OSError, ValueError) as exc:
        parser.exit(2, f"measurement error: {exc}\n")
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
