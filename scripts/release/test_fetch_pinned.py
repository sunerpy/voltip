"""Tests for .github/scripts/fetch-pinned.sh, the retried and digest-checked download CI's GitHub
release downloads go through, against a fake server on 127.0.0.1 (no other host is contacted):
it retries through 504s and a transfer cut short, gives up after the bounded number of attempts,
and refuses bytes whose SHA-256 is not the pinned one. Also the Gradle wrapper's distribution
directory that prepare-gradle.sh fills, checked against directories the wrapper itself made.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import hashlib
import http.server
import os
import stat
import subprocess
import tempfile
import threading
import unittest
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github/scripts/fetch-pinned.sh"
PREPARE_GRADLE = ROOT / ".github/scripts/prepare-gradle.sh"

BODY = b"pinned release asset\n" * 4096
BODY_SHA256 = hashlib.sha256(BODY).hexdigest()
OTHER_SHA256 = hashlib.sha256(b"something else").hexdigest()


class FakeReleases(http.server.BaseHTTPRequestHandler):
    """`/flaky/<n>` answers 504 n times, then BODY; `/cut-once` sends half of BODY with BODY's
    length and closes once, then BODY; `/always-504` never stops; `/body` is BODY."""

    hits: Counter[str] = Counter()
    lock = threading.Lock()

    def do_GET(self) -> None:  # noqa: N802 (http.server's name)
        with self.lock:
            self.hits[self.path] += 1
            n = self.hits[self.path]
        if self.path == "/always-504" or (self.path.startswith("/flaky/") and n <= int(self.path.split("/")[2])):
            self.send_response(504)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(BODY)))
        self.end_headers()
        if self.path == "/cut-once" and n == 1:
            self.wfile.write(BODY[: len(BODY) // 2])
            self.close_connection = True
            return
        self.wfile.write(BODY)

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002 (http.server's name)
        pass


class FetchPinned(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeReleases)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=10)

    def setUp(self) -> None:
        FakeReleases.hits.clear()
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.dest = self.root / "downloads" / "asset.tgz"
        # No proxy and no ~/.curlrc of the machine's: the requests go straight to the fake server.
        self.env = {k: v for k, v in os.environ.items() if k.lower() not in ("http_proxy", "https_proxy", "all_proxy", "no_proxy")}
        self.env.update(
            NO_PROXY="*",
            HOME=str(self.root),
            FETCH_PINNED_RETRY_DELAY="1",
            FETCH_PINNED_RETRIES="5",
            FETCH_PINNED_RETRY_MAX_TIME="60",
        )

    def fetch(self, path: str, sha256: str = BODY_SHA256, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(SCRIPT), self.base + path, sha256, str(self.dest)],
            env={**self.env, **env},
            capture_output=True,
            text=True,
            timeout=120,
        )

    def leftovers(self) -> list[str]:
        return sorted(p.name for p in self.dest.parent.glob("*.part.*")) if self.dest.parent.exists() else []

    def test_retries_through_504s_and_keeps_the_pinned_bytes(self) -> None:
        run = self.fetch("/flaky/3")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(FakeReleases.hits["/flaky/3"], 4, "three 504s, then the asset")
        self.assertEqual(self.dest.read_bytes(), BODY)
        self.assertEqual(self.leftovers(), [])

    def test_retries_a_transfer_cut_short(self) -> None:
        # curl counts a short body (exit 18) as no transient error: only --retry-all-errors retries it.
        run = self.fetch("/cut-once")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(FakeReleases.hits["/cut-once"], 2)
        self.assertEqual(self.dest.read_bytes(), BODY)

    def test_gives_up_after_the_bounded_attempts(self) -> None:
        run = self.fetch("/always-504", FETCH_PINNED_RETRIES="2")
        self.assertEqual(run.returncode, 1)
        self.assertEqual(FakeReleases.hits["/always-504"], 3, "the first attempt and two retries")
        self.assertIn("still failed after the retries", run.stderr)
        self.assertFalse(self.dest.exists())
        self.assertEqual(self.leftovers(), [])

    def test_refuses_bytes_with_another_digest(self) -> None:
        run = self.fetch("/body", OTHER_SHA256)
        self.assertEqual(run.returncode, 1)
        self.assertEqual(FakeReleases.hits["/body"], 1, "a wrong digest is not retried away")
        self.assertIn(f"has SHA-256 {BODY_SHA256}", run.stderr)
        self.assertIn(f"pinned {OTHER_SHA256}", run.stderr)
        self.assertFalse(self.dest.exists())
        self.assertEqual(self.leftovers(), [])

    def test_keeps_a_destination_that_already_holds_the_pinned_bytes(self) -> None:
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(BODY)
        run = self.fetch("/always-504")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(FakeReleases.hits["/always-504"], 0)
        self.assertIn("already holds", run.stdout)

    def test_replaces_a_destination_with_other_bytes(self) -> None:
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(b"stale")
        run = self.fetch("/body")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(self.dest.read_bytes(), BODY)

    def test_refuses_a_curl_without_retry_all_errors(self) -> None:
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        curl = bin_dir / "curl"
        curl.write_text("#!/usr/bin/env bash\necho 'curl 7.68.0 (x86_64-pc-linux-gnu) libcurl/7.68.0'\n", encoding="utf-8")
        curl.chmod(curl.stat().st_mode | stat.S_IXUSR)
        run = self.fetch("/body", PATH=f"{bin_dir}{os.pathsep}{self.env['PATH']}")
        self.assertEqual(run.returncode, 2)
        self.assertIn("curl 7.68.0 has no --retry-all-errors", run.stderr)
        self.assertEqual(FakeReleases.hits["/body"], 0)

    def test_refuses_a_digest_that_is_no_sha256(self) -> None:
        run = self.fetch("/body", BODY_SHA256.upper())
        self.assertEqual(run.returncode, 2)
        self.assertEqual(FakeReleases.hits["/body"], 0)


class GradleDistributionDirectory(unittest.TestCase):
    """prepare-gradle.sh puts the distribution where the wrapper looks for it: wrapper/dists/<name>/
    <base-36 MD5 of the URL>/<zip> (Gradle's PathAssembler). The two names below are directories
    Gradle 8.14.3's and 9.4.1's wrappers created on a developer machine."""

    def dist_dir(self, url: str) -> str:
        run = subprocess.run(["bash", str(PREPARE_GRADLE), "--dist-dir", url], capture_output=True, text=True, timeout=60)
        self.assertEqual(run.returncode, 0, run.stderr)
        return run.stdout.strip()

    def test_matches_the_wrappers_own_directories(self) -> None:
        self.assertEqual(
            self.dist_dir("https://services.gradle.org/distributions/gradle-8.14.3-bin.zip"),
            "gradle-8.14.3-bin/cv11ve7ro1n3o1j4so8xd9n66",
        )
        self.assertEqual(
            self.dist_dir("https://services.gradle.org/distributions/gradle-9.4.1-bin.zip"),
            "gradle-9.4.1-bin/arn2x92ynaizyzdaamcbpbhtj",
        )


if __name__ == "__main__":
    unittest.main()
