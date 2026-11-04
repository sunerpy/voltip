"""scripts/lib/appimage.sh: the pinned AppRun placed in tauri-bundler's tools directory before an
AppImage is bundled, and the check that every user can read and run what an AppImage carries, run
through bash with `set -euo pipefail` the way scripts/build-linux-x64.sh and release-candidate.yml
run them.

Nothing here reaches the network: downloads come from a server on 127.0.0.1, every other proxied
request goes to a closed local port, and HOME and XDG_CACHE_HOME point into a temporary directory.
The permission check builds small squashfs images: it needs squashfs-tools (mksquashfs,
unsquashfs), which CI's web job installs; elsewhere those tests are skipped without it.
"""

import hashlib
import http.server
import os
import re
import shutil
import socket
import stat
import subprocess
import tempfile
import threading
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LIB = os.path.join(ROOT, "scripts", "lib", "appimage.sh")
# What tauri-bundler 2.9.4 downloads when its tools directory has no AppRun-x86_64.
PINNED_URL = "https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64"
PINNED_SHA256 = "f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f"
CONTENT = b"\x7fELF an AppRun for the tests\n"
OTHER = b"\x7fELF something else\n"
PROXY_VARIABLES = ("http_proxy", "https_proxy", "all_proxy", "no_proxy", "ftp_proxy")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def closed_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run(script: str, home: str, env: dict[str, str | None] | None = None, cwd: str | None = None) -> subprocess.CompletedProcess:
    """`script` after sourcing the library, with HOME at <home>, XDG_CACHE_HOME unset unless <env>
    sets it, and a dead proxy for everything but 127.0.0.1."""
    environ = {k: v for k, v in os.environ.items() if k.lower() not in PROXY_VARIABLES and k != "XDG_CACHE_HOME"}
    dead = f"http://127.0.0.1:{closed_port()}"
    environ.update(HOME=home, http_proxy=dead, https_proxy=dead, HTTPS_PROXY=dead, all_proxy=dead, no_proxy="127.0.0.1")
    for key, value in (env or {}).items():
        if value is None:
            environ.pop(key, None)
        else:
            environ[key] = value
    return subprocess.run(
        ["bash", "-c", f"set -euo pipefail; . {LIB}; {script}"], capture_output=True, text=True, check=False, env=environ, cwd=cwd
    )


class Server:
    """An HTTP server on 127.0.0.1 serving <files> by name and recording the paths asked for."""

    def __init__(self, files: dict[str, bytes]):
        self.requests: list[str] = []
        server = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802 (the standard library's name)
                server.requests.append(self.path)
                body = files.get(self.path.lstrip("/"))
                if body is None:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args) -> None:
                pass

        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def __enter__(self) -> "Server":
        self.thread.start()
        return self

    def __exit__(self, *exc) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join()

    def url(self, name: str) -> str:
        return f"http://127.0.0.1:{self.httpd.server_address[1]}/{name}"


def mode(path: str) -> int:
    return stat.S_IMODE(os.lstat(path).st_mode)


def read(path: str) -> bytes:
    with open(path, "rb") as f:
        return f.read()


def write(path: str, data: bytes, permissions: int) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    os.chmod(path, permissions)


class PlacePinnedExecutable(unittest.TestCase):
    """voltip_place_pinned_executable <url> <sha256> <file>, against a local server."""

    def setUp(self) -> None:
        self.home = tempfile.mkdtemp(prefix="voltip-appimage-test-")
        self.addCleanup(shutil.rmtree, self.home)
        self.tools = os.path.join(self.home, "cache", "tauri")
        self.file = os.path.join(self.tools, "AppRun-x86_64")

    def place(self, server: Server, sha: str = sha256(CONTENT)) -> subprocess.CompletedProcess:
        # `|| exit 1`, the way a caller tests it: errexit does not apply inside the function then.
        return run(f"voltip_place_pinned_executable '{server.url('AppRun-x86_64')}' {sha} '{self.file}' || exit 1", self.home)

    def test_a_missing_file_is_downloaded_checked_and_made_0755(self) -> None:
        with Server({"AppRun-x86_64": CONTENT}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(server.requests, ["/AppRun-x86_64"])
        self.assertEqual(read(self.file), CONTENT)
        self.assertEqual(mode(self.file), 0o755)
        self.assertEqual(os.listdir(self.tools), ["AppRun-x86_64"], "no partial download is left behind")

    def test_regression_a_cached_file_with_mode_0770_becomes_0755_without_a_download(self) -> None:
        # tauri-bundler 2.9.4 writes the AppRun it downloads with mode 0770, and packaged it so.
        write(self.file, CONTENT, 0o770)
        with Server({"AppRun-x86_64": OTHER}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(server.requests, [])
        self.assertEqual(read(self.file), CONTENT)
        self.assertEqual(mode(self.file), 0o755)

    def test_a_cached_file_with_other_contents_fails_and_stays_as_it_is(self) -> None:
        write(self.file, OTHER, 0o770)
        with Server({"AppRun-x86_64": CONTENT}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"has SHA-256 {sha256(OTHER)}, not the pinned {sha256(CONTENT)}", result.stderr)
        self.assertEqual(server.requests, [], "nothing replaces it quietly")
        self.assertEqual(read(self.file), OTHER)
        self.assertEqual(mode(self.file), 0o770)

    def test_a_download_with_other_contents_leaves_nothing_behind(self) -> None:
        with Server({"AppRun-x86_64": OTHER}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 1)
        self.assertIn("not the pinned", result.stderr)
        self.assertEqual(os.listdir(self.tools), [])

    def test_a_failed_download_leaves_nothing_behind(self) -> None:
        with Server({}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 1)
        self.assertIn("download failed", result.stderr)
        self.assertEqual(os.listdir(self.tools), [])

    def test_what_is_not_a_regular_file_is_refused(self) -> None:
        target = os.path.join(self.home, "elsewhere")
        write(target, CONTENT, 0o770)
        os.makedirs(self.tools)
        os.symlink(target, self.file)
        with Server({"AppRun-x86_64": CONTENT}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 1)
        self.assertIn("is not a regular file", result.stderr)
        self.assertEqual(mode(target), 0o770, "a symlink's target is not touched")
        os.remove(self.file)
        os.mkdir(self.file)
        with Server({"AppRun-x86_64": CONTENT}) as server:
            self.assertEqual(self.place(server).returncode, 1)


class PinnedAppRun(unittest.TestCase):
    """voltip_appimage_apprun: the pinned AppRun in the directory tauri-bundler reads it from."""

    def setUp(self) -> None:
        self.home = tempfile.mkdtemp(prefix="voltip-appimage-test-")
        self.addCleanup(shutil.rmtree, self.home)

    def test_the_tools_directory_is_the_one_dirs_cache_dir_gives_the_bundler(self) -> None:
        # dirs 6 on Linux: XDG_CACHE_HOME when it is an absolute path, else $HOME/.cache.
        cache = os.path.join(self.home, "xdg")
        for value, expected in (
            (cache, f"{cache}/tauri"),
            (None, f"{self.home}/.cache/tauri"),
            ("", f"{self.home}/.cache/tauri"),
            ("relative/cache", f"{self.home}/.cache/tauri"),
        ):
            with self.subTest(XDG_CACHE_HOME=value):
                result = run("voltip_tauri_tools_dir", self.home, {"XDG_CACHE_HOME": value})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, expected + "\n")

    def test_the_pinned_apprun_goes_into_the_tools_directory(self) -> None:
        cache = os.path.join(self.home, "xdg")
        result = run(
            "voltip_place_pinned_executable() { printf '%s\\n' \"$@\"; }; voltip_appimage_apprun", self.home, {"XDG_CACHE_HOME": cache}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[:3], [PINNED_URL, PINNED_SHA256, f"{cache}/tauri/AppRun-x86_64"])

    def test_a_cached_apprun_that_is_not_the_pinned_one_is_refused_and_kept(self) -> None:
        cached = os.path.join(self.home, ".cache", "tauri", "AppRun-x86_64")
        write(cached, OTHER, 0o770)
        result = run("voltip_appimage_apprun || exit 1", self.home)
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"{cached} has SHA-256 {sha256(OTHER)}, not the pinned {PINNED_SHA256}", result.stderr)
        self.assertEqual((read(cached), mode(cached)), (OTHER, 0o770))

    def test_a_failure_to_place_it_fails_the_caller(self) -> None:
        # In a condition errexit does not apply inside the function: it has to return the failure.
        result = run("voltip_place_pinned_executable() { return 1; }; if voltip_appimage_apprun; then echo placed; else echo refused; fi", self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "refused\n")


def squashfs_tools() -> bool:
    return shutil.which("mksquashfs") is not None and shutil.which("unsquashfs") is not None


# The tree of a small AppImage: AppRun -> AppRun.wrapped as linuxdeploy leaves it, the executable,
# a document and the directories, with the modes a correct build gives them.
TREE = {
    "AppRun.wrapped": 0o755,
    "usr": 0o755,
    "usr/bin": 0o755,
    "usr/bin/voltip-desktop": 0o755,
    "usr/lib": 0o755,
    "usr/share": 0o755,
    "usr/share/doc": 0o755,
    "usr/share/doc/voltip": 0o755,
    "usr/share/doc/voltip/notes.txt": 0o644,
}
DIRECTORIES = {"usr", "usr/bin", "usr/lib", "usr/share", "usr/share/doc", "usr/share/doc/voltip"}


def appimage(dir_: str, modes: dict[str, int] | None = None, offset: int = 4096, reported: str | None = None, runtime_exit: int = 0) -> str:
    """An 'AppImage': a shell runtime that answers --appimage-offset with <reported> (default
    <offset>), padded to <offset> bytes, followed by a squashfs of TREE with <modes> applied."""
    src = os.path.join(dir_, "tree")
    for path in sorted(TREE):
        full = os.path.join(src, path)
        if path in DIRECTORIES:
            os.makedirs(full, exist_ok=True)
        else:
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "wb") as f:
                f.write(b"#!/bin/sh\n" if path != "usr/share/doc/voltip/notes.txt" else b"notes\n")
    os.symlink("AppRun.wrapped", os.path.join(src, "AppRun"))
    for path in TREE:
        os.chmod(os.path.join(src, path), (modes or {}).get(path, TREE[path]))
    image = os.path.join(dir_, "fs.squashfs")
    subprocess.run(["mksquashfs", src, image, "-noappend", "-no-progress", "-all-root"], check=True, capture_output=True)
    runtime = (
        "#!/bin/sh\n"
        f'case "$1" in --appimage-offset) echo {reported if reported is not None else offset}; exit {runtime_exit} ;; esac\n'
        "echo 'not an AppImage runtime' >&2\n"
        "exit 1\n"
    ).encode()
    path = os.path.join(dir_, "Voltip_0.0.0_amd64.AppImage")
    with open(path, "wb") as f:
        f.write(runtime.ljust(offset, b"\n"))
        f.write(read(image))
    os.chmod(path, 0o755)
    return path


@unittest.skipUnless(os.environ.get("CI") or squashfs_tools(), "needs squashfs-tools (mksquashfs, unsquashfs)")
class AppImagePermissions(unittest.TestCase):
    """voltip_check_appimage_permissions <AppImage> <label>."""

    def setUp(self) -> None:
        self.dir = tempfile.mkdtemp(prefix="voltip-appimage-test-")
        self.addCleanup(shutil.rmtree, self.dir)

    def check(self, path: str, cwd: str | None = None) -> subprocess.CompletedProcess:
        # As scripts/build-linux-x64.sh calls it: errexit does not apply inside the function.
        return run(f"voltip_check_appimage_permissions '{path}' test || exit 1", self.dir, cwd=cwd)

    def test_regression_an_apprun_wrapped_with_mode_0770_stops_the_build(self) -> None:
        # 0.2.1 in the AppImage catalogue's firejail test: "AppRun.wrapped: Permission denied".
        result = self.check(appimage(self.dir, {"AppRun.wrapped": 0o770}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("cannot read or run — refusing to ship", result.stderr)
        offenders = [line for line in result.stderr.splitlines() if line.startswith("  ")]
        self.assertEqual(len(offenders), 1, result.stderr)
        self.assertRegex(offenders[0], r"^  -rwxrwx--- 0/0 .* squashfs-root/AppRun\.wrapped$")

    def test_regression_a_filter_that_fails_fails_the_check(self) -> None:
        # Goal review, round 1: the awk filters' own failures went unchecked, and called as
        # `… || exit 1` (errexit off inside the function) an AppImage with a 0770 entry passed.
        # An awk that fails on one of the two programs stands in for any failure of a filter.
        path = appimage(self.dir, {"AppRun.wrapped": 0o770})
        fakes = os.path.join(self.dir, "bin")
        os.mkdir(fakes)
        with open(os.path.join(fakes, "awk"), "w", encoding="utf-8") as f:
            f.write(f'#!/bin/sh\ncase "$*" in *"$FAIL_AWK_ON"*) exit 2 ;; esac\nexec {shutil.which("awk")} "$@"\n')
        os.chmod(os.path.join(fakes, "awk"), 0o755)
        for program in ("$1 ~ /^[-dlbcps]", "substr($1, 8, 1)"):
            with self.subTest(failing=program):
                result = run(
                    f"voltip_check_appimage_permissions '{path}' test || exit 1",
                    self.dir,
                    {"PATH": f"{fakes}:{os.environ['PATH']}", "FAIL_AWK_ON": program},
                )
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn("could not", result.stderr)
                self.assertNotIn("all readable by every user", result.stdout)

    def test_an_appimage_every_user_can_read_and_run_passes(self) -> None:
        result = self.check(appimage(self.dir))
        self.assertEqual(result.returncode, 0, result.stderr)
        # The root, the AppRun symlink and the nine entries of TREE.
        self.assertIn(": 11 entries, all readable by every user", result.stdout)

    def test_an_entry_other_users_cannot_read_or_run_fails(self) -> None:
        for path, permissions in (
            ("usr/share/doc/voltip/notes.txt", 0o640),
            ("usr/bin/voltip-desktop", 0o744),
            ("usr/bin/voltip-desktop", 0o4754),
            ("usr/lib", 0o750),
            ("usr/lib", 0o754),
        ):
            with self.subTest(path=path, mode=oct(permissions)), tempfile.TemporaryDirectory(dir=self.dir) as d:
                result = self.check(appimage(d, {path: permissions}))
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertRegex(result.stderr, rf"(?m)^  \S+ 0/0 .* squashfs-root/{re.escape(path)}$")

    def test_a_file_nobody_runs_needs_only_to_be_readable(self) -> None:
        for permissions in (0o444, 0o604, 0o1755):
            with self.subTest(mode=oct(permissions)), tempfile.TemporaryDirectory(dir=self.dir) as d:
                result = self.check(appimage(d, {"usr/share/doc/voltip/notes.txt": permissions}))
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_an_appimage_that_does_not_report_its_offset_fails(self) -> None:
        for reported, code in (("4096", 1), ("not-a-number", 0), ("", 0)):
            with self.subTest(reported=reported, exit=code), tempfile.TemporaryDirectory(dir=self.dir) as d:
                result = self.check(appimage(d, reported=reported, runtime_exit=code))
                self.assertEqual(result.returncode, 1)
                self.assertIn("did not report where its file system starts", result.stderr)

    def test_an_offset_that_is_not_the_file_system_fails(self) -> None:
        result = self.check(appimage(self.dir, reported="4097"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("unsquashfs could not list", result.stderr)

    def test_a_missing_appimage_fails(self) -> None:
        result = self.check(os.path.join(self.dir, "Voltip_0.0.0_amd64.AppImage"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("is missing — nothing to check", result.stderr)

    def test_an_appimage_named_without_a_directory_runs_from_the_current_one(self) -> None:
        # Not looked up on PATH: `Voltip_….AppImage` alone is the file in the current directory.
        path = appimage(self.dir)
        result = self.check(os.path.basename(path), cwd=self.dir)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
