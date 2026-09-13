# Brainstorming assessment

## Problem

Complete a suite that anticipates and detects performance failures beyond those already encountered. Its scope comes from CHR's capabilities, execution responsibilities, resource lifetimes and interactions. Current synthesis measurements and audit findings are regression evidence within that scope.

## Agent inputs

Five independent agents considered the same problem and success criteria with different inspiration words. Each reassessed the controlling outcome after the user's clarification that future failure discovery is the purpose.

| Agent | Inspiration words |
|---|---|
| James | basalt, compass, weave, orchard, resonance |
| Huygens | lantern, delta, hinge, ceramic, migration |
| Cicero | prism, tide, forge, lichen, counterpoint |
| Bacon | anchor, quartz, braid, estuary, harvest |
| Banach | sieve, canopy, pulse, copper, orbit |

The agents used local source/context evidence and made no benchmark runs or implementation changes. Their proposals are design reasoning, not proof that the prospective suite works.

## Candidate solutions

**Capability and resource risk model.** Derive plausible growth, repeated-work, interference and retention failures from supported semantics, then independently from execution/ownership boundaries. Connect each hypothesis to an input dimension, oracle and detector. This supplies a reason for coverage beyond named workloads, but the argument itself can miss interactions.

**Generated and relational checks.** Vary topology, skew, ordering, sharing, history, concurrency and other justified dimensions. Use scaling and valid metamorphic relations alongside semantic oracles. This explores more than fixed examples, but unconstrained random inputs and assumed equivalences can generate meaningless evidence.

**Independent regression challenges.** Introduce representative performance faults or controlled variants after the initial detectors exist. Require existing exploration/detection to notice them, and check benign controls. This tests detection power beyond calibration examples, but known mutation families alone cannot establish complete coverage.

**Historical comparisons and incident regressions.** Keep reproducible baselines and minimized discoveries. These cheaply catch recurrence and support everyday use, but cannot identify all inefficiency already present in the baseline or unknown failure mechanisms.

## Comparative assessment

These approaches address different weaknesses and belong together. The risk model determines what to investigate; generators and controls exercise it; independent challenges test whether discovery and detection work; historical cases make discoveries durable. None alone is an adequate stopping oracle.

The proposals agree on separating semantic truth, justified operational/resource properties, and empirical performance expectations. Equivalent answers do not imply equal costs. Diagnostics explain signals; they are not the organizing objective. The main implementation tradeoffs are observer overhead, generator/oracle complexity, exploration cost and detector sensitivity versus false alarms.

The lead's choice is to extend maintained infrastructure with this combined method. Scope is bounded by the current supported system and material risks exposed by the capability audit and independent challenges. Use declared exploration budgets and affected-check reruns. Do not turn hypothetical optimal complexity, a fixed number of repetitions, specific instrumentation mechanisms, or comparator parity into requirements.

## Recommendation

Implement the prospective discovery-and-detection capability specified in `../goal.md`. Require a demonstration that established generators and detectors catch independently chosen hazards before a dedicated reproducer teaches them the answer. A missed challenge requires a general coverage/detection repair and a fresh variation, not merely another named benchmark.

Keep timing and accounting calibrated, known slow cases visible, censored observations truthful, and extension procedures small. Finish when the full executable oracle passes. Engine optimization findings remain evidence for subsequent work rather than prerequisites for this suite's completion.

## Open questions

No user decision is required to create or execute the goal. Exact generators, instrumentation facilities, quantitative tolerances and campaign budgets must be selected from repository and host evidence during implementation. They are engineering responsibilities, not reasons to ask for architecture approval.
