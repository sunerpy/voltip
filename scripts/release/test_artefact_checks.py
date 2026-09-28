"""scripts/lib/artefact-checks.sh: the provider-key scan and the linkage check, run through bash
with `set -euo pipefail` the way the build scripts and release.yml run them."""

import os
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LIB = os.path.join(ROOT, "scripts", "lib", "artefact-checks.sh")
KEY = b"gsk_" + b"A" * 40


def binary(dir_: str, key_at: str | None) -> str:
    """A 'binary' with 200 000 printable strings (far more than a pipe buffer) and a key at the
    start, the end, or nowhere."""
    path = os.path.join(dir_, "bin")
    with open(path, "wb") as f:
        if key_at == "start":
            f.write(b"\0" + KEY + b"\0")
        for i in range(200_000):
            f.write(b"printable-string-number-%08d\0" % i)
        if key_at == "end":
            f.write(b"\0" + KEY + b"\0")
    return path


def run(script: str) -> subprocess.CompletedProcess:
    return subprocess.run(["bash", "-c", f"set -euo pipefail; . {LIB}; {script}"], capture_output=True, text=True, check=False)


class ProviderKeyScan(unittest.TestCase):
    def test_regression_a_key_early_in_a_large_binary_is_found(self):
        # `strings | grep -qE` under pipefail reported this binary clean (SIGPIPE on strings).
        with tempfile.TemporaryDirectory() as d:
            result = run(f"voltip_scan_provider_keys {binary(d, 'start')} test")
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("provider API key found", result.stderr)

    def test_a_missing_binary_is_not_clean(self):
        result = run("voltip_scan_provider_keys /nonexistent/voltip-desktop test")
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing or unreadable", result.stderr)

    def test_a_key_at_the_end_is_found_and_a_clean_binary_passes(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(run(f"voltip_scan_provider_keys {binary(d, 'end')} test").returncode, 1)
        with tempfile.TemporaryDirectory() as d:
            clean = run(f"voltip_scan_provider_keys {binary(d, None)} test")
            self.assertEqual(clean.returncode, 0, clean.stderr)


class OutputHas(unittest.TestCase):
    def test_an_early_match_in_long_output_counts(self):
        # `seq 1 2000000` writes ~15 MB; the match is on the first line.
        self.assertEqual(run("voltip_output_has '^1$' seq 1 2000000").returncode, 0)
        self.assertEqual(run("voltip_output_has '^nothing$' seq 1 2000").returncode, 1)


if __name__ == "__main__":
    unittest.main()
