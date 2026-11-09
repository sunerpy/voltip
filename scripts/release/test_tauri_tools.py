"""scripts/lib/tauri-tools.sh: the NSIS files tauri-bundler checks in its tools directory, placed
there before a Windows installer is bundled off Windows (scripts/build-windows-x64.sh), run through
bash with `set -euo pipefail` the way the build script runs them.

Nothing here reaches the network: downloads come from a server on 127.0.0.1, every other proxied
request goes to a closed local port, and HOME and XDG_CACHE_HOME point into a temporary directory.
fetch-pinned.sh retries nothing here (FETCH_PINNED_RETRIES=0); its retries have their own tests in
test_fetch_pinned.py.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import hashlib
import http.server
import io
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import tempfile
import threading
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "scripts/lib/tauri-tools.sh"
MEMBER = "nsis-3.13/NSIS.exe"
EXE = b"MZ an NSIS.exe for the tests\n"
DLL = b"MZ an nsis_tauri_utils.dll for the tests\n"
OTHER = b"MZ something else\n"
PROXY_VARIABLES = ("http_proxy", "https_proxy", "all_proxy", "no_proxy", "ftp_proxy")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha1(data: bytes) -> str:
    return hashlib.sha1(data).hexdigest()


def a_zip(members: dict[str, bytes]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in members.items():
            archive.writestr(name, data)
    return buffer.getvalue()


ZIP = a_zip({MEMBER: EXE, "nsis-3.13/makensis.exe": b"MZ makensis\n", "nsis-3.13/Include/MUI2.nsh": b"; MUI2\n"})


def closed_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run(script: str, home: str, env: dict[str, str] | None = None, cwd: str | None = None) -> subprocess.CompletedProcess:
    """`script` after sourcing the library, with HOME at <home>, XDG_CACHE_HOME at <home>/xdg, a
    dead proxy for everything but 127.0.0.1 and no download retries."""
    environ = {k: v for k, v in os.environ.items() if k.lower() not in PROXY_VARIABLES}
    dead = f"http://127.0.0.1:{closed_port()}"
    environ.update(
        HOME=home,
        XDG_CACHE_HOME=os.path.join(home, "xdg"),
        http_proxy=dead,
        https_proxy=dead,
        HTTPS_PROXY=dead,
        all_proxy=dead,
        no_proxy="127.0.0.1",
        FETCH_PINNED_RETRIES="0",
        FETCH_PINNED_MAX_TIME="60",
    )
    environ.update(env or {})
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


def read(path: str) -> bytes:
    with open(path, "rb") as f:
        return f.read()


def write(path: str, data: bytes) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def files_under(path: str) -> list[str]:
    found = []
    for directory, _, names in os.walk(path):
        found.extend(os.path.relpath(os.path.join(directory, name), path) for name in names)
    return sorted(found)


class NsisToolset(unittest.TestCase):
    """voltip_tauri_nsis_tools: NSIS.exe and nsis_tauri_utils.dll in <tools>/NSIS, the two files
    tauri-bundler 2.10.4 checks there off Windows, from a local server standing in for GitHub."""

    def setUp(self) -> None:
        self.home = tempfile.mkdtemp(prefix="voltip-tauri-tools-test-")
        self.addCleanup(shutil.rmtree, self.home)
        self.tools = os.path.join(self.home, "xdg", "tauri")
        self.nsis = os.path.join(self.tools, "NSIS")
        self.exe = os.path.join(self.nsis, "NSIS.exe")
        self.dll = os.path.join(self.nsis, "Plugins", "x86-unicode", "additional", "nsis_tauri_utils.dll")

    def place(self, server: Server, **pins: str) -> subprocess.CompletedProcess:
        values = {
            "VOLTIP_NSIS_ZIP_URL": server.url("nsis.zip"),
            "VOLTIP_NSIS_ZIP_SHA256": sha256(ZIP),
            "VOLTIP_NSIS_ZIP_SHA1": sha1(ZIP),
            "VOLTIP_NSIS_EXE_MEMBER": MEMBER,
            "VOLTIP_NSIS_EXE_SHA256": sha256(EXE),
            "VOLTIP_NSIS_EXE_SHA1": sha1(EXE),
            "VOLTIP_NSIS_TAURI_UTILS_URL": server.url("nsis_tauri_utils.dll"),
            "VOLTIP_NSIS_TAURI_UTILS_SHA256": sha256(DLL),
            "VOLTIP_NSIS_TAURI_UTILS_SHA1": sha1(DLL),
        }
        values.update(pins)
        # The library's own values, replaced after it is sourced; `|| exit 1`, the way the build
        # script tests it: errexit does not apply inside the function then.
        assignments = "; ".join(f"{name}='{value}'" for name, value in values.items())
        return run(f"{assignments}; voltip_tauri_nsis_tools || exit 1", self.home)

    def test_regression_the_toolset_holds_nsis_exe_so_the_bundler_downloads_nothing(self) -> None:
        # Run 42567630080 (main, 2026-11-09): the cross-build stopped at the bundler's own single
        # download of nsis_tauri_utils.dll (`http status: 504`). Off Windows the bundler never puts
        # NSIS.exe into <tools>/NSIS but checks its SHA-1 before every NSIS bundle, so it removed
        # the directory and downloaded the dll again each time ("NSIS.exe is missing. Recreating
        # it.", the release candidate's second Windows bundle in run 42514933736).
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(server.requests, ["/nsis.zip", "/nsis_tauri_utils.dll"])
        self.assertEqual(read(self.exe), EXE)
        self.assertEqual(read(self.dll), DLL)
        self.assertEqual(files_under(self.nsis), ["NSIS.exe", "Plugins/x86-unicode/additional/nsis_tauri_utils.dll"], "only the two files, nothing partial")
        self.assertEqual(files_under(self.tools), ["NSIS/NSIS.exe", "NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll"])
        for path in (self.exe, self.dll):
            self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o644)

    def test_a_toolset_already_in_place_is_kept_without_a_download(self) -> None:
        write(self.exe, EXE)
        write(self.dll, DLL)
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(server.requests, [])
        self.assertEqual((read(self.exe), read(self.dll)), (EXE, DLL))

    def test_regression_the_dll_the_bundler_left_stays_and_only_nsis_exe_is_added(self) -> None:
        # What a bundle off Windows left in <tools>/NSIS before this script: the dll alone.
        write(self.dll, DLL)
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(server.requests, ["/nsis.zip"])
        self.assertEqual((read(self.exe), read(self.dll)), (EXE, DLL))

    def test_files_with_other_bytes_are_replaced_by_the_pinned_ones(self) -> None:
        # As the bundler itself replaces them: it would remove an NSIS.exe it does not know and
        # download a dll whose SHA-1 is not its own again.
        write(self.exe, OTHER)
        write(self.dll, OTHER)
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(server.requests, ["/nsis.zip", "/nsis_tauri_utils.dll"])
        self.assertEqual((read(self.exe), read(self.dll)), (EXE, DLL))

    def test_a_zip_member_with_other_bytes_fails_and_places_nothing(self) -> None:
        other_zip = a_zip({MEMBER: OTHER})
        with Server({"nsis.zip": other_zip, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server, VOLTIP_NSIS_ZIP_SHA256=sha256(other_zip), VOLTIP_NSIS_ZIP_SHA1=sha1(other_zip))
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"has SHA-256 {sha256(OTHER)}, pinned {sha256(EXE)}", result.stderr)
        self.assertFalse(os.path.exists(self.exe))
        self.assertEqual(files_under(self.tools), [], "nothing partial is left behind")

    def test_a_zip_without_the_member_fails(self) -> None:
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server, VOLTIP_NSIS_EXE_MEMBER="nsis-3.14/NSIS.exe")
        self.assertEqual(result.returncode, 1)
        self.assertIn("has no nsis-3.14/NSIS.exe", result.stderr)
        self.assertFalse(os.path.exists(self.exe))

    def test_a_dll_whose_sha1_is_not_the_one_the_bundler_checks_fails(self) -> None:
        # The SHA-256 pinned here and the SHA-1 the bundler compares name the same bytes; when
        # they do not, the pins are wrong and the bundler would download the file itself again.
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server, VOLTIP_NSIS_TAURI_UTILS_SHA1=sha1(OTHER))
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"has SHA-1 {sha1(DLL)}, not the {sha1(OTHER)} tauri-bundler checks", result.stderr)
        self.assertFalse(os.path.exists(self.dll))

    def test_a_zip_whose_sha1_is_not_the_one_the_bundler_checks_fails(self) -> None:
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            result = self.place(server, VOLTIP_NSIS_ZIP_SHA1=sha1(OTHER))
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"has SHA-1 {sha1(ZIP)}, not the {sha1(OTHER)} tauri-bundler checks", result.stderr)
        self.assertEqual(files_under(self.tools), [])

    def test_a_failed_download_fails_the_caller_and_leaves_nothing_behind(self) -> None:
        with Server({}) as server:
            result = self.place(server)
        self.assertEqual(result.returncode, 1)
        self.assertIn("still failed after the retries above", result.stderr)
        self.assertEqual(files_under(self.tools), [])

    def test_fetch_pinned_is_found_wherever_the_library_was_sourced_from(self) -> None:
        # The candidate's legs source the scripts of .release-tooling/ by a relative path and the
        # build script changes directory afterwards.
        values = (
            f"VOLTIP_NSIS_ZIP_URL='{{zip}}'; VOLTIP_NSIS_ZIP_SHA256={sha256(ZIP)}; VOLTIP_NSIS_ZIP_SHA1={sha1(ZIP)}; "
            f"VOLTIP_NSIS_EXE_MEMBER={MEMBER}; VOLTIP_NSIS_EXE_SHA256={sha256(EXE)}; VOLTIP_NSIS_EXE_SHA1={sha1(EXE)}; "
            f"VOLTIP_NSIS_TAURI_UTILS_URL='{{dll}}'; VOLTIP_NSIS_TAURI_UTILS_SHA256={sha256(DLL)}; VOLTIP_NSIS_TAURI_UTILS_SHA1={sha1(DLL)}"
        )
        with Server({"nsis.zip": ZIP, "nsis_tauri_utils.dll": DLL}) as server:
            script = (
                f"cd {ROOT}; . scripts/lib/tauri-tools.sh; cd /; "
                + values.format(zip=server.url("nsis.zip"), dll=server.url("nsis_tauri_utils.dll"))
                + "; voltip_tauri_nsis_tools || exit 1"
            )
            result = run(script, self.home, cwd="/")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((read(self.exe), read(self.dll)), (EXE, DLL))


class Pins(unittest.TestCase):
    """The values tauri-tools.sh pins belong to the tauri-cli CI installs."""

    def setUp(self) -> None:
        self.lib = LIB.read_text(encoding="utf-8")

    def value(self, text: str, name: str) -> str:
        match = re.search(rf'(?m)^{name}="?([^"\s]+)"?$', text)
        self.assertIsNotNone(match, f"{name} is not assigned")
        return match.group(1)

    def test_the_pins_were_read_from_the_tauri_cli_ci_installs(self) -> None:
        # The bundler's files and digests change with its version: bumping the Tauri CLI in
        # install-cargo-tauri.sh fails here until they are read again from the new bundler's
        # source and the version below is updated with them.
        installed = self.value((ROOT / ".github/scripts/install-cargo-tauri.sh").read_text(encoding="utf-8"), "TAURI_CLI_VERSION")
        self.assertEqual(self.value(self.lib, "VOLTIP_TAURI_TOOLS_CLI_VERSION"), installed)

    def test_the_pins_are_the_files_tauri_bundler_2_10_4_downloads(self) -> None:
        # Its constants (bundle/windows/nsis/mod.rs): NSIS_URL, NSIS_SHA1, NSIS_EXE_SHA1,
        # NSIS_TAURI_UTILS_URL, NSIS_TAURI_UTILS_SHA1.
        self.assertEqual(self.value(self.lib, "VOLTIP_NSIS_ZIP_URL"), "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip")
        self.assertEqual(self.value(self.lib, "VOLTIP_NSIS_ZIP_SHA1"), "db14f8af2ea346b786c6ae19343f61aaf5e09b8e")
        self.assertEqual(self.value(self.lib, "VOLTIP_NSIS_EXE_SHA1"), "6d45d4e8ccf18fe1fdac9164058f356102a2ecf2")
        self.assertEqual(
            self.value(self.lib, "VOLTIP_NSIS_TAURI_UTILS_URL"),
            "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll",
        )
        self.assertEqual(self.value(self.lib, "VOLTIP_NSIS_TAURI_UTILS_SHA1"), "75197fee3c6a814fe035788d1c34ead39349b860")
        for name in ("VOLTIP_NSIS_ZIP_SHA256", "VOLTIP_NSIS_EXE_SHA256", "VOLTIP_NSIS_TAURI_UTILS_SHA256"):
            with self.subTest(name=name):
                self.assertRegex(self.value(self.lib, name), r"^[0-9a-f]{64}$")

    def test_no_config_moves_the_bundlers_tools_directory(self) -> None:
        # bundle.useLocalToolsDir would make tauri-cli give the bundler <target>/.tauri instead of
        # dirs::cache_dir()/tauri, where these files go.
        for config in sorted((ROOT / "apps/desktop/src-tauri").glob("tauri*.conf.json")):
            with self.subTest(config=config.name):
                self.assertNotIn("useLocalToolsDir", json.dumps(json.loads(config.read_text(encoding="utf-8"))))


if __name__ == "__main__":
    unittest.main()
