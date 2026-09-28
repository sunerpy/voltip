"""Tests for .github/scripts/changed-code.sh (the CI change classifier), with a fake `gh`.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github/scripts/changed-code.sh"

RELEASE_FILES = [
    {"filename": "CHANGELOG.md", "patch": "@@ -1,3 +1,9 @@\n+## [0.0.4]\n+\n+* feature"},
    {
        "filename": "package.json",
        "patch": '@@ -1,6 +1,6 @@\n {\n   "name": "voltip-workspace",\n-  "version": "0.0.3",\n+  "version": "0.0.4",\n   "private": true,',
    },
    {
        "filename": "apps/desktop/package.json",
        "patch": '@@ -1,4 +1,4 @@\n-  "version": "0.0.3",\n+  "version": "0.0.4",',
    },
    {
        "filename": ".release-please-manifest.json",
        "patch": '@@ -1,3 +1,3 @@\n {\n-  ".": "0.0.3"\n+  ".": "0.0.4"\n }',
    },
]


class ChangedCode(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def run_script(self, event: str, files: list[dict], **env: str) -> str:
        """Run the classifier with a fake `gh` answering `files`; returns its `code` output."""
        payload = self.root / "files.json"
        payload.write_text(json.dumps(files), encoding="utf-8")
        gh = self.root / "gh"
        # compare: the push API's object; pulls/N/files: one {filename, patch} per line, as
        # `gh api --paginate --jq '.[] | {filename, patch}'` prints them.
        gh.write_text(
            "#!/usr/bin/env bash\n"
            f'payload="{payload}"\n'
            'case "$2" in\n'
            '  */compare/*) jq \'{files: .}\' "$payload" ;;\n'
            "  */files) jq -c '.[] | {filename, patch}' \"$payload\" ;;\n"
            "  *) exit 1 ;;\n"
            "esac\n",
            encoding="utf-8",
        )
        gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
        output = self.root / "output"
        output.write_text("", encoding="utf-8")
        environment = {
            **os.environ,
            "PATH": f"{self.root}:{os.environ['PATH']}",
            "GITHUB_OUTPUT": str(output),
            "EVENT": event,
            "REPO": "example/voltip",
            "PR": "10",
            "BEFORE": "a" * 40,
            "AFTER": "b" * 40,
            **env,
        }
        subprocess.run(["bash", str(SCRIPT)], env=environment, check=True, capture_output=True)
        lines = output.read_text(encoding="utf-8").split()
        self.assertEqual(len(lines), 1, lines)
        return lines[0]

    def test_the_pushed_release_commit_runs_no_build(self) -> None:
        self.assertEqual(self.run_script("push", RELEASE_FILES), "code=false")

    def test_the_release_pull_request_still_runs_the_full_ci(self) -> None:
        self.assertEqual(self.run_script("pull_request", RELEASE_FILES), "code=true")

    def test_a_package_json_that_changes_more_than_the_version_is_code(self) -> None:
        files = [
            {
                "filename": "package.json",
                "patch": '@@ -1,4 +1,4 @@\n-  "version": "0.0.3",\n+  "version": "0.0.4",\n-  "vitest": "5.0.0"\n+  "vitest": "5.0.1"',
            }
        ]
        self.assertEqual(self.run_script("push", files), "code=true")

    def test_a_version_file_without_a_patch_is_code(self) -> None:
        self.assertEqual(self.run_script("push", [{"filename": "package.json"}]), "code=true")

    def test_documentation_only_and_code(self) -> None:
        docs = [{"filename": "docs/feedback.md", "patch": "@@\n+x"}, {"filename": "LICENSE"}]
        self.assertEqual(self.run_script("push", docs), "code=false")
        self.assertEqual(self.run_script("pull_request", docs), "code=false")
        code = [*RELEASE_FILES, {"filename": "apps/desktop/src/App.tsx", "patch": "@@\n+x"}]
        self.assertEqual(self.run_script("push", code), "code=true")

    def test_a_new_branch_and_other_events_run_everything(self) -> None:
        self.assertEqual(self.run_script("push", RELEASE_FILES, BEFORE="0" * 40), "code=true")
        self.assertEqual(self.run_script("workflow_dispatch", RELEASE_FILES), "code=true")
        self.assertEqual(self.run_script("merge_group", RELEASE_FILES), "code=true")


if __name__ == "__main__":
    unittest.main()
