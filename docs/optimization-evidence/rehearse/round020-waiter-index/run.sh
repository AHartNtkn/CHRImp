#!/usr/bin/env bash
set -euo pipefail
# Use prebuilt diagnostics binaries. No building or rwLog work occurs here.
# Run from the experiment root: bash docs/optimization-evidence/rehearse/round020-waiter-index/run.sh OUT BEFORE AFTER
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
for n in 16 64 128; do
    pair pending-cancel-$n life-pending-cancel "$n"
    pair pending-snapshot-$n life-pending-snapshot "$n"
done
# Productive no-choice rewrites grow the same parked FIFO tail checked by the
# parking_tail_tests invariant test, then validate every answer tuple and port.
for n in 1 16 64 256 1024; do pair parking-tail-$n rewrite "$n"; done
for n in 1 4 8; do pair fair-growth-$n fair-grow "$n" --rows 8; done
for n in 1 4 8; do
    pair inspection-$n life-inspections "$n" --rows 4 --work 64
    pair inspection-conditional-$n life-inspections-conditional "$n" --rows 4 --work 64
done
for n in 16 128; do pair no-choice-$n raw-probes 1 --rows "$n"; done
pair empty prepare-reuse 0 --empty --uses 8
pair multiplicity answers 64 --rows 0
