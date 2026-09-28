"""scripts/lib/artefact-checks.sh: the provider-key scan and the linkage check, run through bash
with `set -euo pipefail` the way the build scripts and release-candidate.yml run them."""

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


# What 0.0.4's Linux voltip-desktop links (readelf -d of the released deb's binary).
NEEDED_0_0_4 = [
    "libgobject-2.0.so.0", "libwebkit2gtk-4.1.so.0", "libgtk-3.so.0", "libgdk-3.so.0", "libcairo.so.2",
    "libgdk_pixbuf-2.0.so.0", "libsoup-3.0.so.0", "libgio-2.0.so.0", "libjavascriptcoregtk-4.1.so.0",
    "libglib-2.0.so.0", "libxkbcommon.so.0", "libasound.so.2", "libsherpa-onnx-c-api.so", "libblas.so.3",
    "libstdc++.so.6", "libm.so.6", "libvulkan.so.1", "libgcc_s.so.1", "libc.so.6", "ld-linux-x86-64.so.2",
]


class LinuxSonames(unittest.TestCase):
    """voltip_linux_sonames_accounted, with a fake readelf that prints the dynamic section."""

    def check(self, needed: list[str]) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as d:
            exe = os.path.join(d, "voltip-desktop")
            open(exe, "wb").close()
            readelf = os.path.join(d, "readelf")
            lines = "".join(f" 0x0000000000000001 (NEEDED)             Shared library: [{n}]\\n" for n in needed)
            with open(readelf, "w", encoding="utf-8") as f:
                f.write(f"#!/bin/sh\nprintf '{lines}'\n")
            os.chmod(readelf, 0o755)
            return subprocess.run(
                ["bash", "-c", f"set -euo pipefail; . {LIB}; voltip_linux_sonames_accounted {exe} test"],
                capture_output=True, text=True, check=False, env={**os.environ, "PATH": f"{d}:{os.environ['PATH']}"},
            )

    def test_regression_what_0_0_4_links_is_accounted_for(self):
        # 0.0.4's deb did not depend on the BLAS its binary links; libblas.so.3 is on the list now,
        # next to the depends that name it.
        result = self.check(NEEDED_0_0_4)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_new_library_stops_the_build(self):
        result = self.check([*NEEDED_0_0_4, "libopenblas.so.0"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("links libopenblas.so.0, which no Linux package depends on", result.stderr)

    def test_a_binary_without_a_dynamic_section_is_not_accounted_for(self):
        result = self.check([])
        self.assertEqual(result.returncode, 1)
        self.assertIn("names no shared library", result.stderr)


if __name__ == "__main__":
    unittest.main()
