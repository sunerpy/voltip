"""Tests for .github/scripts/check-no-production-hosts.sh (the release preflight and CI run it).

The guard's words come from VOLTIP_PRODUCTION_HOSTS (environment or .env.build) and must never be
printed; every case runs the real script inside a throwaway git repository.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / ".github" / "scripts" / "check-no-production-hosts.sh"
WORD = "zzqqhostword"


class ProductionHostGuard(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = Path(tempfile.mkdtemp(prefix="voltip-guard-"))
        self.addCleanup(shutil.rmtree, self.repo)
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        (self.repo / ".gitignore").write_text(".env.build\n", encoding="utf-8")
        (self.repo / "README.md").write_text("Point the client at <asr-host>.\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True)

    def run_guard(self, *args: str, words: str | None = None) -> subprocess.CompletedProcess[str]:
        env = {key: value for key, value in os.environ.items() if key != "VOLTIP_PRODUCTION_HOSTS"}
        if words is not None:
            env["VOLTIP_PRODUCTION_HOSTS"] = words
        return subprocess.run(
            ["bash", str(SCRIPT), *args], cwd=self.repo, env=env, capture_output=True, text=True, check=False
        )

    def test_without_words_it_skips_and_require_refuses(self) -> None:
        skipped = self.run_guard()
        self.assertEqual(skipped.returncode, 0, skipped.stderr)
        self.assertIn("skipped", skipped.stdout)
        required = self.run_guard("--require")
        self.assertEqual(required.returncode, 2)
        self.assertIn("VOLTIP_PRODUCTION_HOSTS is empty", required.stderr)

    def test_a_hit_names_the_file_and_line_but_never_the_word(self) -> None:
        (self.repo / "docs.md").write_text(f"intro\nconnect to relay.{WORD.upper()}.example\n", encoding="utf-8")
        result = self.run_guard(words=f"other-word,{WORD}")
        self.assertEqual(result.returncode, 1)
        self.assertIn("docs.md:2", result.stderr)
        self.assertNotIn(WORD, (result.stdout + result.stderr).lower())

    def test_words_from_env_build_count_and_the_file_itself_is_not_scanned(self) -> None:
        (self.repo / ".env.build").write_text(f'VOLTIP_PRODUCTION_HOSTS="{WORD}"\n', encoding="utf-8")
        clean = self.run_guard("--require")
        self.assertEqual(clean.returncode, 0, clean.stderr)
        self.assertIn("1 configured word", clean.stdout)
        (self.repo / "new-untracked.txt").write_text(f"{WORD}\n", encoding="utf-8")
        caught = self.run_guard()
        self.assertEqual(caught.returncode, 1)
        self.assertIn("new-untracked.txt:1", caught.stderr)

    def test_short_or_glob_words_are_refused(self) -> None:
        for words in ("abc", f"* {WORD}"):
            with self.subTest(words=words):
                result = self.run_guard(words=words)
                self.assertEqual(result.returncode, 2)
                self.assertIn("at least 4 characters", result.stderr)

    def test_the_guard_itself_names_no_host(self) -> None:
        text = SCRIPT.read_text(encoding="utf-8")
        self.assertNotIn("keywords=(", text, "the words live in the build environment, not in the script")


if __name__ == "__main__":
    unittest.main()
