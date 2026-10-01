"""Tests for .github/scripts/ci-success.sh (the rule of ci.yml's `CI Success` aggregate).

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import json
import os
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github/scripts/ci-success.sh"

# `ci-success.needs` in ci.yml (scripts/release/test_release_workflows.py checks the workflow side).
JOBS = [
    "changes",
    "verify-rust",
    "verify-web",
    "windows-cross",
    "windows-native",
    "hooks-windows",
    "smoke-desktop",
    "macos",
]

# What `changes` skips on the documentation fast path: every job but itself and `verify-web`.
FAST_PATH_SKIPS = {job: "skipped" for job in JOBS if job not in ("changes", "verify-web")}


def needs(**results: str) -> dict:
    """`toJSON(needs)`: every job a success unless named (`verify_rust="failure"`)."""
    out = {job: {"result": "success", "outputs": {}} for job in JOBS}
    for name, result in results.items():
        out[name.replace("_", "-")]["result"] = result
    return out


class CiSuccess(unittest.TestCase):
    def run_script(self, needs_json: dict, code: str = "true", event: str = "push") -> subprocess.CompletedProcess:
        env = {**os.environ, "NEEDS_JSON": json.dumps(needs_json), "CODE": code, "EVENT": event}
        return subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, check=False)

    def assertGreen(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("every required job succeeded", result.stdout)

    def assertRed(self, result: subprocess.CompletedProcess, job: str) -> None:
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(job, result.stdout)

    def test_everything_green_passes_on_every_event(self) -> None:
        for event in ("push", "pull_request", "workflow_dispatch"):
            with self.subTest(event=event):
                self.assertGreen(self.run_script(needs(), event=event))

    def test_regression_a_red_mac_on_a_push_fails_ci_success(self) -> None:
        # v0.0.5: main's CI run 36461609860 on 06334e6 had macos-x64 red and CI Success green.
        self.assertRed(self.run_script(needs(macos="failure"), event="push"), "macos=failure")

    def test_regression_a_skipped_mac_on_a_push_with_code_fails(self) -> None:
        self.assertRed(self.run_script(needs(macos="skipped"), event="push"), "macos=skipped")

    def test_a_manual_run_needs_the_macs_too(self) -> None:
        self.assertRed(self.run_script(needs(macos="skipped"), event="workflow_dispatch"), "macos=skipped")
        self.assertRed(self.run_script(needs(macos="cancelled"), event="workflow_dispatch"), "macos=cancelled")

    def test_the_macs_may_be_skipped_on_a_pull_request_only(self) -> None:
        self.assertGreen(self.run_script(needs(macos="skipped"), event="pull_request"))
        # Skipped is the only result a pull request may report for them, and only for them.
        self.assertRed(self.run_script(needs(macos="failure"), event="pull_request"), "macos=failure")
        self.assertRed(self.run_script(needs(smoke_desktop="skipped"), event="pull_request"), "smoke-desktop=skipped")

    def test_the_documentation_fast_path_skips_all_but_its_two_jobs(self) -> None:
        for event in ("push", "pull_request"):
            with self.subTest(event=event):
                self.assertGreen(self.run_script(needs(**{k.replace("-", "_"): v for k, v in FAST_PATH_SKIPS.items()}), code="false", event=event))

    def test_the_fast_path_still_needs_changes_and_verify_web(self) -> None:
        skips = {k.replace("-", "_"): v for k, v in FAST_PATH_SKIPS.items()}
        self.assertRed(self.run_script(needs(**skips, verify_web="skipped"), code="false"), "verify-web=skipped")
        self.assertRed(self.run_script(needs(**skips, changes="failure"), code=""), "changes=failure")

    def test_a_failed_or_cancelled_job_fails_and_is_named(self) -> None:
        result = self.run_script(needs(verify_rust="cancelled", windows_native="failure"))
        self.assertRed(result, "verify-rust=cancelled")
        self.assertIn("windows-native=failure", result.stdout)

    def test_an_aggregate_without_its_two_fixed_jobs_fails(self) -> None:
        partial = needs()
        del partial["verify-web"]
        self.assertRed(self.run_script(partial), "verify-web")


if __name__ == "__main__":
    unittest.main()
