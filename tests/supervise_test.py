"""Exercise real processes, resource limits, CPU/sleep attribution and completion."""
import importlib.util
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('chr_supervise', Path(__file__).parents[1]/'examples/supervise.py')
supervise = importlib.util.module_from_spec(spec)
spec.loader.exec_module(supervise)


class SupervisionTests(unittest.TestCase):
    def run_python(self, source, **limits):
        with tempfile.TemporaryDirectory() as d:
            with open(Path(d)/'out', 'w') as out, open(Path(d)/'err', 'w') as err:
                return supervise.run([sys.executable, '-c', source], d, out, err, **limits)

    def test_completion_failure_and_nonzero_exit_are_distinct(self):
        self.assertEqual(self.run_python('print(42)')['status'], 'completed')
        for code in [1, 2]:
            result = self.run_python(f'raise SystemExit({code})')
            self.assertEqual(result['status'], 'failed')
            self.assertEqual(result['returncode'], code)
            self.assertIsNone(result['limit_reason'])

    def test_cpu_sleep_and_touched_memory_have_distinct_evidence(self):
        sleeper = self.run_python('import time; time.sleep(.25)')
        busy = self.run_python('import time\nt=time.process_time()\nwhile time.process_time()-t < .25: pass')
        memory = self.run_python('x=bytearray(32*1024*1024); print(x[-1])')
        self.assertGreaterEqual(sleeper['wall_seconds'], .25)
        self.assertGreater(busy['user_cpu_seconds']+busy['system_cpu_seconds'], .24)
        self.assertLess(sleeper['user_cpu_seconds']+sleeper['system_cpu_seconds'], .15)
        self.assertGreater(memory['max_rss_kib'], sleeper['max_rss_kib']+16*1024)
        self.assertGreater(memory['minor_faults'], sleeper['minor_faults']+4096)

    def test_deadline_cannot_be_reported_as_completion(self):
        source = 'import signal,time\nsignal.signal(signal.SIGINT, lambda *_: exit(0))\ntime.sleep(10)'
        result = self.run_python(source, seconds=.15, grace=.2)
        self.assertEqual(result['status'], 'censored')
        self.assertEqual(result['limit_reason'], 'wall_deadline')
        self.assertLess(result['wall_seconds'], 2)

    def test_ignored_interrupt_has_hard_stop(self):
        result = self.run_python('import signal,time; signal.signal(signal.SIGINT, signal.SIG_IGN); time.sleep(10)', seconds=.15, grace=.1)
        self.assertEqual(result['status'], 'censored')
        self.assertEqual(result['returncode'], -9)
        self.assertLess(result['wall_seconds'], 2)

    def test_child_left_after_leader_exit_is_cleaned_and_not_completed(self):
        source = 'import os,time\nif os.fork()==0: time.sleep(10)'
        result = self.run_python(source)
        self.assertEqual(result['status'], 'failed')
        self.assertTrue(result['descendants_signaled_after_exit'])
        self.assertTrue(result['group_cleanup_complete'])
        for pid in result['descendant_pids']:
            try:
                state = Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[0]
            except FileNotFoundError:
                continue
            self.assertIn(state, ['Z', 'X'])

    def test_file_limit_has_identified_censoring(self):
        source = 'import os,signal; signal.signal(signal.SIGXFSZ, signal.SIG_DFL)\nf=os.open("large",os.O_CREAT|os.O_WRONLY,0o600)\nwhile True: os.write(f,b"x"*65536)'
        result = self.run_python(source, file_mib=1)
        self.assertEqual(result['status'], 'censored')
        self.assertEqual(result['limit_reason'], 'file_size_limit')

    def test_cpu_limit_signal_is_distinct_from_wall_timeout(self):
        # Lower the child CPU cap so it fires before the external wall deadline.
        source = 'import resource; resource.setrlimit(resource.RLIMIT_CPU,(1,2))\nwhile True: pass'
        result = self.run_python(source, seconds=5)
        self.assertEqual(result['status'], 'censored')
        self.assertEqual(result['limit_reason'], 'cpu_limit')
        self.assertGreater(result['user_cpu_seconds']+result['system_cpu_seconds'], .9)

    def test_waited_child_cpu_is_included(self):
        child = 'import time\nt=time.process_time()\nwhile time.process_time()-t < .2: pass'
        source = f'import subprocess,sys; subprocess.run([sys.executable,"-c",{child!r}],check=True)'
        result = self.run_python(source)
        self.assertEqual(result['status'], 'completed')
        self.assertGreater(result['user_cpu_seconds']+result['system_cpu_seconds'], .19)

    def test_supervision_preserves_an_unrelated_child(self):
        other = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(10)'])
        try:
            result = self.run_python('import os,time\nif os.fork()==0: time.sleep(10)')
            self.assertTrue(result['group_cleanup_complete'])
            self.assertIsNone(other.poll())
        finally:
            other.kill()
            other.wait()

    def test_cli_exit_mapping_and_launch_error_reports_are_terminal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for flag, expected in [([], 'failed'), (['--measure'], 'censored')]:
                out = root/expected
                command = [sys.executable, str(Path(supervise.__file__)), '--out', str(out), *flag,
                           '--', sys.executable, '-c', 'raise SystemExit(2)']
                result = subprocess.run(command, capture_output=True, text=True, timeout=5)
                self.assertIn(result.returncode, [1, 2])
                self.assertEqual(json.loads((out/'run.json').read_text())['status'], expected)
            bad = root/'invalid-executable'
            bad.write_text('invalid executable format')
            bad.chmod(0o700)
            out = root/'launch-error'
            result = subprocess.run([sys.executable, str(Path(supervise.__file__)), '--out', str(out), '--', str(bad)], capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            report = json.loads((out/'run.json').read_text())
            self.assertEqual(report['status'], 'supervisor_error')
            self.assertIn('error', report)

    def test_address_space_limit_is_effective_without_guessing_failure_cause(self):
        result = self.run_python('x=bytearray(256*1024*1024)', memory_mib=64)
        self.assertEqual(result['status'], 'failed')
        self.assertIsNone(result['limit_reason'])
        self.assertEqual(result['limits']['address_space_mib_per_process'], 64)


if __name__ == '__main__':
    unittest.main()
