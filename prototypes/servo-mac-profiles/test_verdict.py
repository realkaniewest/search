"""Regression check: each independent probe failure must make the run fail."""

import importlib.util
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location("servo_profile_probe", Path(__file__).with_name("run.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class VerdictTest(unittest.TestCase):
    def test_every_criterion_is_required(self):
        names = (
            "A_B_cookie_checks_pass",
            "A_B_localStorage_checks_pass",
            "temporary_storage_meets_probe_criterion",
        )
        passing = dict.fromkeys(names, True)
        self.assertTrue(MODULE.all_probe_criteria_pass(passing))
        for name in names:
            with self.subTest(failing=name):
                verdict = {**passing, name: False}
                self.assertFalse(MODULE.all_probe_criteria_pass(verdict))


if __name__ == "__main__":
    unittest.main()
