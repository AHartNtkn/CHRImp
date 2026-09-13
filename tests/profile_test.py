"""Checks for the native flame-graph pipeline; run with python3 -m unittest discover -s tests -p profile_test.py."""
import importlib.util
from pathlib import Path
import unittest
import sys
sys.path.insert(0, str(Path(__file__).parents[1] / "examples"))

spec = importlib.util.spec_from_file_location("chr_profile", Path(__file__).parents[1] / "examples/profile.py")
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


class ProfileTests(unittest.TestCase):
    def test_perf_path_normalization_preserves_frames_and_samples(self):
        raw = "measure 1 2.0: 100 cpu-clock:u:\n\t ab _Rfunction.llvm.123+0x10 (/a path/measure)\n\t cd symbol (/lib/libc.so)\n\n"
        actual = profile.normalize_stacks(raw)
        self.assertIn("100 cpu-clock:u:", actual)
        self.assertIn("_Rfunction+0x10", actual)
        self.assertIn("(/a%20path/measure)", actual)
        self.assertIn("symbol (/lib/libc.so)", actual)
        self.assertEqual(actual.count("\n"), raw.count("\n"))

    def test_sample_accounting_catches_lost_or_doubled_samples(self):
        raw = "measure 1/1 2.0: cpu-clock:u: \n\t ab leaf (/binary)\n\n"
        self.assertEqual(profile.validate_samples(raw, "root;leaf 1\n"), 1)
        with self.assertRaises(ValueError):
            profile.validate_samples(raw, "root;leaf 2\n")

    def test_cli_errors_are_not_benchmark_limit_results(self):
        self.assertEqual(profile.outcome(2, False, True), "failed")
        self.assertEqual(profile.outcome(2, False, False), "censored")
        self.assertEqual(profile.outcome(130, True, True), "censored")
        self.assertEqual(profile.outcome(0, False, True), "completed")

    def test_empty_or_malformed_folded_profile_is_not_success(self):
        self.assertEqual(profile.folded_weight("root;leaf 12\nroot;other 5\n"), 17)
        for text in ["", "root;leaf 0\n", "root;leaf wat\n", "root;leaf -1\n"]:
            with self.assertRaises(ValueError):
                profile.folded_weight(text)


if __name__ == "__main__":
    unittest.main()
