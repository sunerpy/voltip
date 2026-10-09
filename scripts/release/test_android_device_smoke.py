"""Tests for .github/scripts/android-device-smoke.sh: which APK it installs and which package it
starts, with a fake `adb` that records every call and refuses the install, so the script stops
there.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github/scripts/android-device-smoke.sh"


class DeviceSmoke(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        self.calls = self.root / "adb-calls.txt"
        adb = bin_dir / "adb"
        # `get-state` answers `offline` FAKE_OFFLINE_FOR times first, and `wait-for-device` fails
        # while it does, as adb did on CI's emulator.
        polls = self.root / "get-state-calls"
        adb.write_text(
            "#!/usr/bin/env bash\n"
            f'echo "$*" >>"{self.calls}"\n'
            'case "$1" in\n'
            "  get-state)\n"
            f'    n=$(cat "{polls}" 2>/dev/null || echo 0); echo $((n + 1)) >"{polls}"\n'
            '    if [ "$n" -lt "${FAKE_OFFLINE_FOR:-0}" ]; then echo offline; else echo device; fi ;;\n'
            '  wait-for-device) [ "${FAKE_OFFLINE_FOR:-0}" -gt 0 ] && { echo "adb: device offline" >&2; exit 1; } ;;\n'
            '  install) echo "Failure [fake adb]"; exit 1 ;;\n'
            "esac\n"
            "exit 0\n",
            encoding="utf-8",
        )
        adb.chmod(adb.stat().st_mode | stat.S_IXUSR)
        # The hosted runners' locale (C.UTF-8): `sort` orders by code point there, so `-` comes before
        # `_`. A locale such as en_US orders the two names the other way round.
        self.env = {**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}", "LC_ALL": "C.UTF-8"}
        # The Android leg of a release candidate: the APK and the AAB, nothing else.
        self.leg = self.root / "dist"
        self.leg.mkdir()
        for name in ("Voltip_0.0.50_android_arm64.apk", "Voltip_0.0.50_android_arm64.aab"):
            (self.leg / name).write_bytes(b"package")

    def smoke(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(SCRIPT), *args, str(self.root / "out")],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def adb_calls(self) -> list[str]:
        return self.calls.read_text(encoding="utf-8").splitlines() if self.calls.exists() else []

    def test_the_app_is_installed_from_the_leg_under_the_package_it_took_over(self) -> None:
        # Since 0.0.50 the React Native app is the Android app, under the Tauri phone app's package
        # (user decision 2026-10-09), so an installed Tauri app updates to it.
        result = self.smoke(str(self.leg))
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("the APK did not install", result.stderr)
        calls = self.adb_calls()
        self.assertIn("uninstall dev.voltip.mobile", calls)
        self.assertIn(f"install -r -g {self.leg}/Voltip_0.0.50_android_arm64.apk", calls)

    def test_regression_a_device_that_drops_offline_after_boot_is_waited_for(self) -> None:
        # main CI 2026-10-07 (run 37688703262): `adb: device offline` the moment the smoke began,
        # after the emulator had booted, and the smoke stopped there without a word.
        self.env["FAKE_OFFLINE_FOR"] = "2"
        result = self.smoke(str(self.leg))
        self.assertIn(f"install -r -g {self.leg}/Voltip_0.0.50_android_arm64.apk", self.adb_calls(), result.stderr)
        self.assertIn("the APK did not install", result.stderr)

    def test_an_apk_named_on_the_command_line_is_installed_as_it_is(self) -> None:
        apk = self.leg / "Voltip_0.0.50_android_arm64.apk"
        self.smoke(str(apk))
        self.assertIn(f"install -r -g {apk}", self.adb_calls())

    def test_a_directory_without_the_app_or_with_two_of_it_is_refused(self) -> None:
        (self.leg / "Voltip_0.0.50_android_arm64.apk").unlink()
        result = self.smoke(str(self.leg))
        self.assertEqual(result.returncode, 2)
        self.assertIn("no APK for dev.voltip.mobile", result.stderr)
        (self.leg / "Voltip_0.0.50_android_arm64.apk").write_bytes(b"package")
        (self.leg / "Voltip_0.0.49_android_arm64.apk").write_bytes(b"older")
        result = self.smoke(str(self.leg))
        self.assertEqual(result.returncode, 2)
        self.assertIn("more than one Voltip_*.apk", result.stderr)
        self.assertEqual(self.adb_calls(), [])

    def test_the_second_apps_option_is_gone(self) -> None:
        # `--app mobile-rn` chose the React Native app while the Tauri app was the default; there is
        # one Android app now.
        result = self.smoke("--app", "mobile-rn", str(self.leg))
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)
        self.assertEqual(self.adb_calls(), [])


if __name__ == "__main__":
    unittest.main()
