"""Run after cargo build --release --features diagnostics --example measure; exercises both real observer modes."""
import os
import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('CHR_MEASURE_BINARY', ROOT / 'target/release/examples/measure'))


def run(case, size, detail=False, ticks=5_000_000):
    result = subprocess.run([str(BINARY), case, str(size), str(ticks), '5', *(['--detail'] if detail else [])],
                            cwd=ROOT, capture_output=True, text=True, timeout=15)
    fields = {}
    for key, value in re.findall(r'(?<!\S)([a-zA-Z]\w*)=([^\s]+)', result.stdout):
        fields.setdefault(key, []).append(value)
    return result, fields


class ObservationTests(unittest.TestCase):
    def test_same_validated_results_and_work_in_both_modes(self):
        for case, size in [('rewrite', 8), ('partial-join', 8), ('graph-bits', 3),
                           ('notebook-arithmetic-decompose', 3), ('notebook-behavior-i', 1),
                           ('life-alias', 8), ('life-archive', 2), ('runtime', 2)]:
            with self.subTest(case=case):
                baseline, b = run(case, size)
                detailed, d = run(case, size, True)
                self.assertEqual(baseline.returncode, 0, baseline.stdout + baseline.stderr)
                self.assertEqual(detailed.returncode, 0, detailed.stdout + detailed.stderr)
                required = ['source_status', 'answers'] if case.startswith('life-') or case == 'runtime' else ['status', 'answers']
                for key in required:
                    self.assertIn(key, b)
                    self.assertIn(key, d)
                for key in ['status', 'source_status', 'applications', 'apps',
                            'answers', 'facts', 'ports', 'scalars', 'cleanup_done',
                            'source_goal_reached', 'continuation_apps', 'source_apps', 'cleanup_in_time']:
                    self.assertEqual(b.get(key), d.get(key), (case, key))
                self.assertEqual(b['diagnostics_feature'], ['true'], 'build with --features diagnostics for work attribution checks')
                def work(output):
                    return [json.loads(line.split('=', 1)[1])['work'] for line in output.splitlines()
                            if line.startswith('diagnostics=')]
                bw, dw = work(baseline.stdout), work(detailed.stdout)
                self.assertEqual(len(bw), len(dw))
                if case != 'runtime':
                    self.assertGreaterEqual(len(bw), 2)
                for before, after in zip(bw, dw):
                    # Collector traversal includes address-ordered shared variable
                    # groups. Repeated unchanged runs vary in collection dispatches.
                    # Keep all raw counts, and demand that they explain the entire
                    # iteration difference; every other work field must match.
                    self.assertEqual(before['advance_iterations'] - after['advance_iterations'],
                                     before['dispatch']['collection'] - after['dispatch']['collection'])
                    for category in before['dispatch']:
                        if category != 'collection':
                            self.assertEqual(before['dispatch'][category], after['dispatch'][category], category)
                    # Address-ordered collection can change graph-pruning and
                    # arena traversal work. Require these measured services to
                    # explain the same difference; all other shared work matches.
                    shared_before=before['shared'];shared_after=after['shared']
                    delta=before['advance_iterations']-after['advance_iterations']
                    collection_delta=sum(shared_before['collection'][phase]-shared_after['collection'][phase]
                                         for phase in ('prune_graph','arena'))
                    self.assertEqual(collection_delta,delta)
                    self.assertEqual(shared_before['coordinates']['cleanup_probes']-shared_after['coordinates']['cleanup_probes'],delta)
                    for shared in (shared_before,shared_after):
                        for phase in ('prune_graph','arena'):shared['collection'].pop(phase)
                        shared['coordinates'].pop('cleanup_probes')
                    for key in before:
                        if key not in ['advance_iterations', 'dispatch']:
                            self.assertEqual(before[key], after[key], key)
                self.assertEqual(b['measurement_mode'], ['baseline'])
                self.assertEqual(d['measurement_mode'], ['detailed'])
                for key in ['validator_ms', 'max_advance1_ms', 'max_step_ms', 'max_scheduler_tick_ms',
                            'collection_ticks', 'window_collection_ticks', 'source_delivery_without_validator_ms',
                            'cancel_max_step_ms', 'release_max_step_ms', 'close_max_tick_ms']:
                    if key in b:
                        self.assertEqual(len(b[key]), len(d[key]))
                        self.assertTrue(all(v == 'null' for v in b[key]), (case, key, b[key]))
                        self.assertTrue(all(float(v) >= 0 for v in d[key]), (case, key))

    def test_tick_censoring_remains_incomplete_in_both_modes(self):
        for detail in [False, True]:
            result, fields = run('rewrite', 8, detail, ticks=1)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertEqual(fields['status'], ['INCOMPLETE'])
            self.assertEqual(fields['source_goal_reached'], ['false'])


if __name__ == '__main__':
    unittest.main()
