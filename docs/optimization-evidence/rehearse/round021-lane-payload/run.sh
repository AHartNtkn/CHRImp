#!/usr/bin/env bash
set -euo pipefail
# Run from the experiment root with separately built diagnostics executables.
out=$1
before=$2
after=$3
mkdir -p "$out"
pair() {
    local label=$1
    shift
    for side in before after; do
        local binary=$before
        if [[ $side == after ]]; then binary=$after; fi
        timeout --kill-after=5s 60s python3 examples/perf.py \
            --binary "$binary" --out "$out/$side-$label" \
            --repeat 3 --warmup 0 --seconds 10 --total-seconds 50 -- "$@" 50000000 3 --detail
    done
}
for n in 1 16 64 256 1024; do pair rewrite-$n rewrite "$n"; done
for n in 1 4 8; do pair fair-growth-$n fair-grow "$n" --rows 8; done
for n in 16 64 128; do
    pair pending-cancel-$n life-pending-cancel "$n"
    pair pending-snapshot-$n life-pending-snapshot "$n"
done
for n in 1 4 8; do
    pair inspection-$n life-inspections "$n" --rows 4 --work 64
    pair inspection-conditional-$n life-inspections-conditional "$n" --rows 4 --work 64
    pair archive-fixed-$n life-archive-fixed "$n" --rows 4 --work 64
    pair archive-rotate-$n life-archive-rotate "$n" --rows 4 --work 64 --cadence 4
    pair archive-conditional-$n life-archive-rotate-conditional "$n" --rows 4 --work 64 --cadence 4
done
for n in 16 128; do pair no-choice-$n raw-probes 1 --rows "$n"; done
pair empty prepare-reuse 0 --empty --uses 8
pair multiplicity answers 64 --rows 0
pair held-output life-held-output 4 --rows 4 --work 64

# Independently repeat the variable-cost retained conditional archive controls.
for n in 1 4 8; do
    for side in before after; do
        binary=$before
        if [[ $side == after ]]; then binary=$after; fi
        timeout --kill-after=5s 60s python3 examples/perf.py \
            --binary "$binary" --out "$out-archive-audit/$side-archive-conditional-$n" \
            --repeat 12 --warmup 0 --seconds 10 --total-seconds 50 -- \
            life-archive-rotate-conditional "$n" --rows 4 --work 64 --cadence 4 50000000 3 --detail
    done
done
for side in before after; do
    binary=$before
    if [[ $side == after ]]; then binary=$after; fi
    timeout --kill-after=5s 60s python3 examples/perf_suite.py routine \
        --binary "$binary" --out "$out-suite-$side" --only archive --only fairness \
        --repeat 3 --seconds 50 --sample-seconds 5
done
