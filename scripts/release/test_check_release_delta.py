"""Tests for .github/scripts/check-release-delta.py (the release-PR delta proof) on real Git repos.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github/scripts/check-release-delta.py"

# The files release-please rewrites on a Voltip release PR (release-please-config.json), exactly as
# release-candidate.yml passes them.
ALLOW_GLOBS = "CHANGELOG.md .release-please-manifest.json"
VERSION_ONLY_GLOBS = "package.json apps/desktop/package.json apps/mobile/package.json"


def package_json(name: str, version: str, extra: str = "") -> str:
    return f'{{\n  "name": "{name}",\n  "version": "{version}",\n  "private": true{extra}\n}}\n'


class ReleaseDelta(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.repo = Path(self._tmp.name)
        self.git("init", "-q", "-b", "main")
        self.write_release_files("0.0.3", changelog="# Changelog\n")
        self.write("apps/desktop/src/App.tsx", "export const app = 1;\n")
        self.base = self.commit("feat: the base")

    # ---------------------------------------------------------------------------- helpers

    def git(self, *args: str) -> str:
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.test",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.test",
        }
        done = subprocess.run(
            ["git", "-C", str(self.repo), *args], env=env, check=True, capture_output=True, text=True
        )
        return done.stdout.strip()

    def write(self, path: str, text: str) -> None:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def write_release_files(self, version: str, changelog: str) -> None:
        self.write("package.json", package_json("voltip-workspace", version))
        self.write("apps/desktop/package.json", package_json("@voltip/desktop", version))
        self.write("apps/mobile/package.json", package_json("@voltip/mobile", version))
        self.write(".release-please-manifest.json", f'{{\n  ".": "{version}"\n}}\n')
        self.write("CHANGELOG.md", changelog)

    def commit(self, message: str) -> str:
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")

    def release_commit(self, version: str = "0.0.4") -> str:
        self.write_release_files(version, changelog=f"# Changelog\n\n## [{version}]\n\n* a feature\n")
        return self.commit(f"chore: release {version}")

    def prove(self, head: str, base: str, mode: str = "automatic", *extra: str) -> tuple[int, dict]:
        self.git("checkout", "-q", "--detach", head)
        done = subprocess.run(
            [
                "python3",
                str(SCRIPT),
                "--repo",
                str(self.repo),
                "--head",
                head,
                "--base",
                base,
                "--allow-globs",
                ALLOW_GLOBS,
                "--version-only-globs",
                VERSION_ONLY_GLOBS,
                "--mode",
                mode,
                *extra,
            ],
            capture_output=True,
            text=True,
        )
        result = json.loads(done.stdout) if done.stdout.strip() else {}
        return done.returncode, result

    # ------------------------------------------------------------------------------ tests

    def test_a_release_please_commit_on_the_base_is_proven(self) -> None:
        head = self.release_commit()
        code, result = self.prove(head, self.base)
        self.assertEqual(code, 0, result)
        self.assertTrue(result["ok"])
        self.assertEqual(result["shape"], "single-parent")
        self.assertEqual(
            sorted(result["files"]),
            sorted(
                [
                    ".release-please-manifest.json",
                    "CHANGELOG.md",
                    "apps/desktop/package.json",
                    "apps/mobile/package.json",
                    "package.json",
                ]
            ),
        )

    def test_an_update_branch_merge_of_the_moved_base_is_proven(self) -> None:
        old_head = self.release_commit()
        self.git("checkout", "-q", "-b", "later", self.base)
        self.write("docs/notes.md", "a documentation change\n")
        new_base = self.commit("docs: a note")
        self.git("checkout", "-q", "--detach", old_head)
        self.git("merge", "-q", "--no-ff", "-m", "Merge branch 'main' into the release PR", new_base)
        merged = self.git("rev-parse", "HEAD")
        code, result = self.prove(merged, new_base)
        self.assertEqual(code, 0, result)
        self.assertEqual(result["shape"], "update-branch")

    def test_code_in_the_release_pr_is_refused_in_every_mode(self) -> None:
        self.write_release_files("0.0.4", changelog="# Changelog\n\n## [0.0.4]\n")
        self.write("apps/desktop/src/App.tsx", "export const app = 2;\n")
        head = self.commit("chore: release 0.0.4")
        for mode in ("automatic", "dry-run"):
            with self.subTest(mode=mode):
                code, result = self.prove(head, self.base, mode)
                self.assertEqual(code, 1, result)
                self.assertFalse(result["ok"])
                self.assertIn(
                    "apps/desktop/src/App.tsx: not a release-please owned file", result["violations"]
                )

    def test_a_package_json_that_changes_more_than_the_version_is_refused(self) -> None:
        self.write_release_files("0.0.4", changelog="# Changelog\n\n## [0.0.4]\n")
        self.write(
            "package.json",
            package_json("voltip-workspace", "0.0.4", ',\n  "scripts": {"postinstall": "curl x"}'),
        )
        head = self.commit("chore: release 0.0.4")
        code, result = self.prove(head, self.base)
        self.assertEqual(code, 1, result)
        self.assertTrue(any(v.startswith("package.json:") for v in result["violations"]), result)

    def test_a_head_that_does_not_sit_on_the_base_is_refused(self) -> None:
        head = self.release_commit()
        self.git("checkout", "-q", "-b", "later", self.base)
        self.write("docs/notes.md", "main moved on\n")
        moved = self.commit("docs: main moved")
        code, result = self.prove(head, moved)
        self.assertEqual(code, 1, result)
        self.assertEqual(result["shape"], "unknown")

    def test_the_dry_run_escape_of_the_template_does_not_exist(self) -> None:
        head = self.release_commit()
        code, _ = self.prove(head, self.base, "dry-run", "--allow-unproven")
        self.assertEqual(code, 2, "argparse must reject --allow-unproven")


if __name__ == "__main__":
    unittest.main()
