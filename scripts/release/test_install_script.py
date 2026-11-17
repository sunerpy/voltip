"""Tests for scripts/install.sh on Linux with the .deb, against a fake release (fake `curl`, `uname`,
`apt-get` and `sudo` on PATH; nothing is downloaded or installed).

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import hashlib
import http.server
import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
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
if [ -n "$out" ] && [ "$out" != - ]; then cp "$src" "$out"; else cat "$src"; fi
""",
    "apt-get": """#!/bin/sh
echo "$*" >>"$APT_LOG"
if [ "$1" = update ]; then exit "${FAKE_APT_UPDATE_EXIT:-0}"; fi
""",
    "sudo": '#!/bin/sh\nexec "$@"\n',
}


class FakeRelease(unittest.TestCase):
    """A release with the .deb, served by fake `curl` (or by a real tool in Downloads), installed
    by fake `apt-get` and `sudo`."""

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
        with subprocess.Popen(
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
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        ) as process:
            try:
                _, stderr = process.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
                raise
        calls = self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []
        return process.returncode, calls, stderr


class DebInstall(FakeRelease):
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


class Downloads:
    """Exercise the real download tool against an owned, local fake release: only the tool's
    destination URL is replaced, its own timeouts, retries and error messages stay real. The
    installed package and apt stay fake, as in DebInstall. CurlDownloads and WgetDownloads run it."""

    tool: str

    def serve(self, asset: str = "", status: int = 0, failures: int = 99) -> list[str]:
        """Serve the release, failing `asset` with `status` the first `failures` times."""
        executable = shutil.which(self.tool)
        if executable is None:
            self.skipTest(f"{self.tool} is not installed")
        requests: list[str] = []
        fixtures = self.fixtures

        class Release(http.server.BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                name = self.path.rsplit("/", 1)[-1]
                requests.append(name)
                if name == asset and requests.count(name) <= failures:
                    self.send_error(status)
                    return
                body = b'{"tag_name":"v0.0.4"}' if name == "latest" else (fixtures / name).read_bytes()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args: object) -> None:
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Release)
        server.timeout = 0.1
        stopped = threading.Event()

        def run() -> None:
            deadline = time.monotonic() + 15
            while not stopped.is_set() and time.monotonic() < deadline:
                server.handle_request()

        worker = threading.Thread(target=run, daemon=True)
        worker.start()

        def stop() -> None:
            stopped.set()
            worker.join(timeout=2)
            server.server_close()

        self.addCleanup(stop)
        # The tool's arguments, one JSON list per run, before the URL is replaced.
        self.runs_log = self.root / "runs.log"
        (self.bin / self.tool).write_text(
            f"#!{sys.executable}\nimport json, os, sys\n"
            f"with open({str(self.runs_log)!r}, 'a', encoding='utf-8') as log:\n"
            "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            f"args = ['http://127.0.0.1:{server.server_port}/' + arg.rsplit('/', 1)[-1] "
            "if arg.startswith('https://') else arg for arg in sys.argv[1:]]\n"
            "os.environ['no_proxy'] = os.environ['NO_PROXY'] = '127.0.0.1'\n"
            f"os.execv({executable!r}, [{executable!r}, *args])\n",
            encoding="utf-8",
        )
        (self.bin / self.tool).chmod(0o755)
        if self.tool == "wget":
            (self.bin / "curl").unlink()
            # No system PATH directory: each contains curl on the developer and CI hosts.
            # Only the commands this fake .deb installation needs are made available.
            for command in ("sh", "sed", "head", "grep", "mktemp", "rm", "awk", "chmod", "id"):
                path = shutil.which(command)
                assert path is not None, command
                (self.bin / command).symlink_to(path)
            for command in ("sha256sum", "shasum"):
                if path := shutil.which(command):
                    (self.bin / command).symlink_to(path)
        return requests

    def download(self, **env: str) -> tuple[int, list[str], str]:
        return self.install(**({"PATH": str(self.bin)} if self.tool == "wget" else {}), **env)

    def runs(self) -> list[list[str]]:
        return [json.loads(line) for line in self.runs_log.read_text(encoding="utf-8").splitlines()]

    def test_regression_checksum_download_errors_keep_the_url_and_http_status(self) -> None:
        # User report 2026-11-17: v0.3.1 said "has no SHA256SUMS" even though the release
        # carried it. install.sh had the same blanket error as the Windows installer.
        requests = self.serve("SHA256SUMS", 403)
        code, calls, stderr = self.download()
        self.assertEqual(code, 1, stderr)
        self.assertIn("https://github.com/sunerpy/voltip/releases/download/v0.0.4/SHA256SUMS", stderr)
        self.assertIn("403", stderr)
        self.assertNotIn("has no SHA256SUMS", stderr)
        self.assertEqual(requests, ["SHA256SUMS"], "permanent errors are not retried")
        self.assertEqual(calls, [])

    def test_regression_missing_checksums_keep_the_404_response(self) -> None:
        requests = self.serve("SHA256SUMS", 404)
        code, calls, stderr = self.download()
        self.assertEqual(code, 1, stderr)
        self.assertIn("404", stderr)
        self.assertIn("/v0.0.4/SHA256SUMS", stderr)
        self.assertEqual(requests, ["SHA256SUMS"])
        self.assertEqual(calls, [])

    def test_regression_latest_lookup_does_not_hide_a_download_failure(self) -> None:
        requests = self.serve("latest", 403)
        code, calls, stderr = self.download(VOLTIP_VERSION="")
        self.assertEqual(code, 1, stderr)
        self.assertIn("https://api.github.com/repos/sunerpy/voltip/releases/latest", stderr)
        self.assertIn("403", stderr)
        self.assertEqual(requests, ["latest"])
        self.assertEqual(calls, [])

    def test_regression_package_download_errors_name_the_failed_url(self) -> None:
        self.serve(DEB, 404)
        code, calls, stderr = self.download()
        self.assertEqual(code, 1, stderr)
        self.assertIn(f"https://github.com/sunerpy/voltip/releases/download/v0.0.4/{DEB}", stderr)
        self.assertIn("404", stderr)
        self.assertEqual(calls, [])

    def test_regression_transient_downloads_retry_and_still_require_the_checksum(self) -> None:
        requests = self.serve("SHA256SUMS", 503, failures=1)
        self.write_sums("f" * 64)
        code, calls, stderr = self.download()
        self.assertEqual(code, 1, stderr)
        self.assertEqual(requests, ["SHA256SUMS", "SHA256SUMS", DEB])
        self.assertIn("checksum mismatch", stderr)
        self.assertEqual(calls, [])

    def test_regression_transient_downloads_stop_after_three_attempts(self) -> None:
        requests = self.serve("SHA256SUMS", 503)
        code, calls, stderr = self.download()
        self.assertEqual(code, 1, stderr)
        self.assertEqual(requests, ["SHA256SUMS"] * 3)
        self.assertIn("503", stderr)
        self.assertIn("/v0.0.4/SHA256SUMS", stderr)
        self.assertEqual(calls, [])

    def test_regression_a_slow_download_is_never_cut_off(self) -> None:
        # Review 2026-11-17: curl's `--max-time 600` ended every attempt at a package download
        # slower than about 200 KB/s for the 117 MiB AppImage (80 KB/s for the 47 MiB deb), and
        # each retry started over; with curl 8.5, 1 MB at 100 KB/s under `--max-time 5` stopped
        # three times at 500 000 bytes. Only a stall may end a download. Checked by the tool's
        # arguments: a real slow download would take ten minutes.
        self.serve()
        code, calls, stderr = self.download(VOLTIP_VERSION="")
        self.assertEqual(code, 0, stderr)
        runs = self.runs()
        urls = [next(arg for arg in run if arg.startswith("https://")) for run in runs]
        self.assertEqual([url.rsplit("/", 1)[-1] for url in urls], ["latest", "SHA256SUMS", DEB])
        for run in runs:
            with self.subTest(run=run):
                if self.tool == "curl":
                    self.assertEqual([a for a in run if a in ("-m", "--max-time") or a.startswith("--max-time=")], [])
                    self.assertEqual(run[run.index("--speed-time") + 1], "30")
                    self.assertEqual(run[run.index("--speed-limit") + 1], "1")
                else:
                    self.assertIn("--timeout=30", run)


class CurlDownloads(Downloads, FakeRelease):
    tool = "curl"


class WgetDownloads(Downloads, FakeRelease):
    tool = "wget"


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



SERVER = "voltip-server-0.0.4-linux-x64.tar.gz"


class ServerInstall(unittest.TestCase):
    """install.sh --server (docs/dictation.md §23.5): the tar.gz into ~/.local/share, the command
    linked into ~/.local/bin, missing system libraries named, a systemd user unit on request."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        fakes = {
            **FAKES,
            "ldd": '#!/bin/sh\nfor lib in $FAKE_MISSING; do printf "\\t%s => not found\\n" "$lib"; done\n',
            # `is-active` answers from FAKE_ACTIVE and is not logged; every other call is.
            "systemctl": '#!/bin/sh\nif [ "$2" = is-active ]; then [ -n "$FAKE_ACTIVE" ]; exit; fi\necho "$*" >>"$SYSTEMCTL_LOG"\n',
        }
        for name, text in fakes.items():
            path = self.bin / name
            path.write_text(text, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IEXEC)
        self.fixtures = self.root / "release"
        self.fixtures.mkdir()
        package = self.root / "package" / "voltip-server-0.0.4-linux-x64"
        (package / "bin").mkdir(parents=True)
        (package / "lib").mkdir()
        exe = package / "bin" / "voltip-server"
        # Like the real binary: it does not start while a library it links is missing.
        exe.write_text(
            '#!/bin/sh\nfor lib in $FAKE_MISSING; do echo "voltip-server: error while loading shared libraries: $lib: cannot open shared object file" >&2; exit 127; done\necho voltip-server 0.0.4\n',
            encoding="utf-8",
        )
        exe.chmod(0o755)
        subprocess.run(["tar", "-C", str(self.root / "package"), "-czf", str(self.fixtures / SERVER), "voltip-server-0.0.4-linux-x64"], check=True)
        digest = hashlib.sha256((self.fixtures / SERVER).read_bytes()).hexdigest()
        self.write_sums(digest)
        self.home = self.root / "home"
        self.home.mkdir()
        self.systemctl = self.root / "systemctl.log"

    def write_sums(self, digest: str) -> None:
        (self.fixtures / "SHA256SUMS").write_text(f"{'0' * 64}  {DEB}\n{digest}  {SERVER}\n", encoding="utf-8")

    def install(self, *args: str, missing: str = "", active: bool = False) -> tuple[int, str]:
        done = subprocess.run(
            ["sh", str(SCRIPT), *args],
            env={
                "PATH": f"{self.bin}:{os.environ['PATH']}",
                "HOME": str(self.home),
                "FIXTURES": str(self.fixtures),
                "APT_LOG": str(self.root / "apt.log"),
                "SYSTEMCTL_LOG": str(self.systemctl),
                "FAKE_MISSING": missing,
                "FAKE_ACTIVE": "1" if active else "",
                "VOLTIP_VERSION": "0.0.4",
            },
            capture_output=True,
            text=True,
        )
        return done.returncode, done.stderr

    def test_the_server_installs_without_root_and_links_its_command(self) -> None:
        code, stderr = self.install("--server")
        self.assertEqual(code, 0, stderr)
        installed = self.home / ".local/share/voltip-server/0.0.4/bin/voltip-server"
        self.assertTrue(installed.is_file())
        link = self.home / ".local/bin/voltip-server"
        self.assertEqual(Path(os.readlink(link)), installed)
        self.assertEqual(subprocess.run([str(link)], capture_output=True, text=True, check=True).stdout.strip(), "voltip-server 0.0.4")
        self.assertFalse((self.root / "apt.log").exists(), "apt is never called")
        self.assertNotIn("does not start", stderr)
        self.assertFalse(self.systemctl.exists(), "no unit unless asked")

    def test_missing_system_libraries_are_named_not_installed(self) -> None:
        code, stderr = self.install("--server", missing="libblas.so.3")
        self.assertEqual(code, 0, stderr)
        self.assertIn("does not start on this system: voltip-server: error while loading shared libraries: libblas.so.3", stderr)
        self.assertIn("these system libraries are missing: libblas.so.3\n", stderr)
        self.assertIn("sudo apt-get install libblas3\n", stderr)
        self.assertFalse((self.root / "apt.log").exists(), "named, never installed")

    def test_a_systemd_user_unit_is_written_and_started_on_request(self) -> None:
        code, stderr = self.install("--server", "--systemd-user", "--enable")
        self.assertEqual(code, 0, stderr)
        unit = (self.home / ".config/systemd/user/voltip-server.service").read_text(encoding="utf-8")
        self.assertIn(f"ExecStart={self.home}/.local/bin/voltip-server\n", unit)
        self.assertEqual(
            self.systemctl.read_text(encoding="utf-8").splitlines(),
            ["--user daemon-reload", "--user enable voltip-server", "--user restart voltip-server"],
        )
        self.assertIn("voltip-server 0.0.4 is running as a systemd user service", stderr)
        self.assertIn("loginctl enable-linger", stderr)

    def test_regression_an_upgrade_restarts_the_running_service_or_says_to(self) -> None:
        # `enable --now` started nothing when the service ran: after an upgrade it kept running
        # the earlier version while the script said it was running.
        code, stderr = self.install("--server", "--systemd-user", "--enable", active=True)
        self.assertEqual(code, 0, stderr)
        self.assertIn("--user restart voltip-server", self.systemctl.read_text(encoding="utf-8").splitlines())
        self.assertNotIn("still runs the earlier version", stderr)
        self.systemctl.unlink()
        code, stderr = self.install("--server", active=True)
        self.assertEqual(code, 0, stderr)
        self.assertIn("still runs the earlier version; restart it with: systemctl --user restart voltip-server", stderr)
        self.assertFalse(self.systemctl.exists(), "nothing is restarted unless asked")

    def test_a_checksum_mismatch_or_an_unknown_option_installs_nothing(self) -> None:
        self.write_sums("f" * 64)
        code, stderr = self.install("--server")
        self.assertEqual(code, 1)
        self.assertIn("checksum mismatch", stderr)
        self.assertFalse((self.home / ".local/share/voltip-server").exists())
        code, stderr = self.install("--srever")
        self.assertEqual(code, 1)
        self.assertIn("unknown option: --srever", stderr)


if __name__ == "__main__":
    unittest.main()
