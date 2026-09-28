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


DMG = "Voltip_0.0.4_aarch64.dmg"

# A Mac on Apple silicon: hdiutil "mounts" a directory holding Voltip.app, ditto copies it, and
# the LaunchServices / Spotlight tools record their calls.
MAC_FAKES = {
    "uname": '#!/bin/sh\ncase "$1" in -s) echo Darwin ;; -m) echo arm64 ;; *) echo Darwin ;; esac\n',
    "sysctl": "#!/bin/sh\necho 1\n",
    "pgrep": "#!/bin/sh\nexit 1\n",
    "hdiutil": """#!/bin/sh
echo "hdiutil $*" >>"$MAC_LOG"
if [ "$1" = attach ]; then
  while [ $# -gt 0 ]; do [ "$1" = -mountpoint ] && mnt=$2; shift; done
  mkdir -p "$mnt/Voltip.app/Contents/MacOS"
  echo app >"$mnt/Voltip.app/Contents/MacOS/voltip-desktop"
fi
""",
    "ditto": '#!/bin/sh\ncp -R "$1" "$2"\n',
    "xattr": "#!/bin/sh\nexit 0\n",
    "lsregister": '#!/bin/sh\necho "lsregister $*" >>"$MAC_LOG"\n',
    "mdimport": '#!/bin/sh\necho "mdimport $*" >>"$MAC_LOG"\n',
}


class MacInstall(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, text in {"curl": FAKES["curl"], **MAC_FAKES}.items():
            path = self.bin / name
            path.write_text(text, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IEXEC)
        self.fixtures = self.root / "release"
        self.fixtures.mkdir()
        dmg = b"a Voltip dmg"
        (self.fixtures / DMG).write_bytes(dmg)
        (self.fixtures / "SHA256SUMS").write_text(
            f"{hashlib.sha256(dmg).hexdigest()}  {DMG}\n", encoding="utf-8"
        )
        self.apps = self.root / "Applications"
        self.log = self.root / "mac.log"

    def test_regression_the_installed_app_is_registered_for_launchpad_and_spotlight(self) -> None:
        # User feedback 2026-09-29: after the one-line install, Launchpad and Spotlight did not
        # find Voltip until `open -a Voltip` ran. Finder registers what it copies with
        # LaunchServices; a shell copy is only registered at its first launch.
        done = subprocess.run(
            ["sh", str(SCRIPT)],
            env={
                **os.environ,
                "PATH": f"{self.bin}:{os.environ['PATH']}",
                "FIXTURES": str(self.fixtures),
                "MAC_LOG": str(self.log),
                "VOLTIP_VERSION": "0.0.4",
                "VOLTIP_INSTALL_DIR": str(self.apps),
            },
            capture_output=True,
            text=True,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        app = self.apps / "Voltip.app"
        self.assertTrue((app / "Contents/MacOS/voltip-desktop").is_file())
        calls = self.log.read_text(encoding="utf-8").splitlines()
        self.assertIn(f"lsregister -f {app}", calls)
        self.assertIn(f"mdimport {app}", calls)
        # Registered after the copy is complete and the image is detached.
        self.assertLess(
            next(i for i, c in enumerate(calls) if c.startswith("hdiutil detach")),
            calls.index(f"lsregister -f {app}"),
        )


if __name__ == "__main__":
    unittest.main()
