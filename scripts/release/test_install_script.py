"""Tests for scripts/install.sh on Linux with the .deb, against a fake release (fake `curl`, `uname`,
`apt-get` and `sudo` on PATH; nothing is downloaded or installed).

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import hashlib
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/install.sh"
DEB = "Voltip_0.0.4_amd64.deb"

FAKES = {
    "uname": '#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo x86_64 ;; *) echo Linux ;; esac\n',
    # Serves the release's files from $FIXTURES by the URL's last segment.
    "curl": """#!/bin/sh
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    http*) url=$1; shift ;;
    *) shift ;;
  esac
done
src="$FIXTURES/${url##*/}"
[ -f "$src" ] || exit 22
if [ -n "$out" ]; then cp "$src" "$out"; else cat "$src"; fi
""",
    "apt-get": """#!/bin/sh
echo "$*" >>"$APT_LOG"
if [ "$1" = update ]; then exit "${FAKE_APT_UPDATE_EXIT:-0}"; fi
""",
    "sudo": '#!/bin/sh\nexec "$@"\n',
}


class DebInstall(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, text in FAKES.items():
            path = self.bin / name
            path.write_text(text, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IEXEC)
        self.fixtures = self.root / "release"
        self.fixtures.mkdir()
        package = b"a Voltip deb"
        (self.fixtures / DEB).write_bytes(package)
        self.write_sums(hashlib.sha256(package).hexdigest())
        self.log = self.root / "apt.log"

    def write_sums(self, digest: str) -> None:
        (self.fixtures / "SHA256SUMS").write_text(
            f"{'0' * 64}  Voltip_0.0.4_amd64.AppImage\n{digest}  {DEB}\n", encoding="utf-8"
        )

    def install(self, **env: str) -> tuple[int, list[str], str]:
        done = subprocess.run(
            ["sh", str(SCRIPT)],
            env={
                **os.environ,
                "PATH": f"{self.bin}:{os.environ['PATH']}",
                "FIXTURES": str(self.fixtures),
                "APT_LOG": str(self.log),
                "VOLTIP_VERSION": "0.0.4",
                "VOLTIP_PACKAGE": "deb",
                **env,
            },
            capture_output=True,
            text=True,
        )
        calls = self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []
        return done.returncode, calls, done.stderr

    def test_the_package_lists_are_refreshed_before_the_install(self) -> None:
        # Regression (install-scripts.yml, 0.0.4): with lists older than the mirror, apt asked for
        # dependency versions the mirror no longer had and failed with 404.
        code, calls, stderr = self.install()
        self.assertEqual(code, 0, stderr)
        self.assertEqual(len(calls), 2, calls)
        self.assertEqual(calls[0], "update -qq")
        self.assertTrue(calls[1].startswith("install -y ") and calls[1].endswith(f"/{DEB}"), calls)

    def test_a_failed_refresh_still_installs(self) -> None:
        code, calls, stderr = self.install(FAKE_APT_UPDATE_EXIT="100")
        self.assertEqual(code, 0, stderr)
        self.assertIn("apt-get update failed", stderr)
        self.assertEqual([call.split()[0] for call in calls], ["update", "install"])

    def test_a_checksum_mismatch_installs_nothing(self) -> None:
        self.write_sums("f" * 64)
        code, calls, stderr = self.install()
        self.assertEqual(code, 1)
        self.assertIn("checksum mismatch", stderr)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
