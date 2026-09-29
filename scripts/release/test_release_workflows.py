"""Contract tests for the release workflows (build once in release-candidate.yml, promote in
release.yml) and the CI hooks they depend on. The workflow files are read as text, block by
indentation, so the test needs nothing beyond the standard library.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github/workflows"


def code_lines(text: str) -> list[str]:
    """The lines of a YAML block without comment-only lines."""
    return [line for line in text.splitlines() if not line.lstrip().startswith("#")]


def top_block(text: str, key: str) -> str:
    """The lines under a top-level key (`on:`, `jobs:`) up to the next top-level key."""
    lines = text.splitlines()
    start = lines.index(f"{key}:")
    block = []
    for line in lines[start + 1 :]:
        if line and not line.startswith((" ", "#")):
            break
        block.append(line)
    return "\n".join(block)


def jobs(path: Path) -> dict[str, str]:
    """Each job's text (comments included), by job id."""
    found: dict[str, list[str]] = {}
    current = None
    for line in top_block(path.read_text(encoding="utf-8"), "jobs").splitlines():
        match = re.fullmatch(r"  ([A-Za-z0-9_-]+):", line)
        if match:
            current = match.group(1)
            found[current] = []
        elif current is not None:
            found[current].append(line)
    return {name: "\n".join(body) for name, body in found.items()}


class Promote(unittest.TestCase):
    """release.yml publishes what the candidate built; it never builds."""

    def setUp(self) -> None:
        self.jobs = jobs(WORKFLOWS / "release.yml")

    def test_the_promote_job_installs_no_toolchain_and_runs_no_build(self) -> None:
        body = "\n".join(code_lines(self.jobs["promote"]))
        forbidden = [
            r"\bcargo\b",
            r"\brustup\b",
            r"rust-toolchain",
            r"rust-cache",
            r"\b(pnpm|npm|npx|yarn|bun)\b",
            r"\bnode\b",
            r"setup-(node|python|go|java)",
            r"install-action",
            r"actions/cache",
            r"\bmake\b",
            r"\bpip3? install\b",
            r"\bbrew\b",
            r"tauri (build|bundle|dev)",
        ]
        for pattern in forbidden:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, body), f"promote matches {pattern}")
        # Its one package: minisign, for verify-signatures.
        installs = re.findall(r"apt-get install ([^\n]*)", body)
        self.assertEqual(installs, ["--yes --no-install-recommends minisign"])
        actions = set(re.findall(r"uses: ([^@\s]+)@", body))
        self.assertEqual(
            actions, {"actions/checkout", "actions/download-artifact", "actions/attest"}
        )

    def test_the_promote_job_publishes_only_the_verified_candidate(self) -> None:
        body = self.jobs["promote"]
        self.assertIn("candidate.py verify", body)
        self.assertIn("--print-files", body)
        self.assertIn("gh attestation verify", body)
        self.assertIn("--deny-self-hosted-runners", body)
        self.assertIn("run-id: ${{ needs.resolve_release.outputs.candidate_run_id }}", body)
        self.assertIn("publish-release-draft.sh --publish", body)

    def test_nothing_listens_on_tags_or_releases(self) -> None:
        for path in sorted(WORKFLOWS.glob("*.yml")):
            triggers = top_block(path.read_text(encoding="utf-8"), "on")
            with self.subTest(workflow=path.name):
                self.assertNotRegex(triggers, r"(?m)^  release:")
                self.assertNotRegex(triggers, r"(?m)^    tags:")


class CandidateBuild(unittest.TestCase):
    """release-candidate.yml builds from the source commit with tooling from protected main."""

    LEGS = ("bundle-windows", "bundle-linux", "bundle-macos")

    def setUp(self) -> None:
        self.path = WORKFLOWS / "release-candidate.yml"
        self.text = self.path.read_text(encoding="utf-8")
        self.jobs = jobs(self.path)

    def test_the_delta_proof_has_no_escape(self) -> None:
        self.assertNotRegex(self.text, r"allow[_-]unproven")
        self.assertIn("check-release-delta.py", self.jobs["prepare"])

    def test_only_the_aggregate_job_can_mint_attestations(self) -> None:
        holders = sorted(
            name for name, body in self.jobs.items() if "id-token: write" in "\n".join(code_lines(body))
        )
        self.assertEqual(holders, ["aggregate"])
        self.assertNotRegex("\n".join(code_lines(self.jobs["aggregate"])), r"\b(cargo|pnpm|make)\b")

    def test_each_leg_builds_the_source_with_tooling_from_the_workflow_commit(self) -> None:
        for leg in self.LEGS:
            with self.subTest(leg=leg):
                body = self.jobs[leg]
                source = body.index("- name: Checkout exact source")
                tooling = body.index("- name: Checkout trusted release tooling")
                # The source is checked out at the root first: a later root checkout would clean
                # .release-tooling away.
                self.assertLess(source, tooling)
                self.assertIn("ref: ${{ inputs.expected_head_sha }}", body[source:tooling])
                self.assertNotIn("path:", body[source:tooling])
                self.assertIn("ref: ${{ github.workflow_sha }}", body[tooling:])
                self.assertIn("path: .release-tooling", body[tooling:])
                self.assertIn(".release-tooling/scripts/release/updater-json.py collect", body)

    def test_the_linux_leg_refuses_a_library_its_packages_do_not_depend_on(self) -> None:
        self.assertIn("voltip_linux_sonames_accounted ../../target/release/voltip-desktop", self.jobs["bundle-linux"])

    def test_the_sherpa_fingerprint_is_cleared_in_the_source_target(self) -> None:
        # forget-sherpa-onnx-build.sh cds to the checkout it sits in and clears that target/.
        for leg in self.LEGS:
            with self.subTest(leg=leg):
                body = self.jobs[leg]
                self.assertIn("run: .github/scripts/forget-sherpa-onnx-build.sh", body)
                self.assertNotIn(".release-tooling/.github/scripts/forget-sherpa-onnx-build.sh", body)

    def test_the_macos_legs_sign_with_the_release_certificate(self) -> None:
        # docs/runbook.md 发布 · macOS 签名: one fixed self-signed certificate, never ad hoc, checked on
        # the app and every Mach-O inside it against the one requirement release-targets.json names.
        body = self.jobs["bundle-macos"]
        imported = body.index("macos-signing-keychain.sh import --expect-sha1")
        bundled = body.index("- name: Bundle and sign (app, dmg)")
        checked = body.index("--expect-requirement")
        self.assertLess(imported, bundled)
        self.assertLess(bundled, checked)
        self.assertIn("APPLE_SIGNING_IDENTITY: ${{ secrets.MACOS_SIGNING_IDENTITY }}", body[bundled:checked])
        self.assertIn(".macos_signing.designated_requirement .release-tooling/.github/release-targets.json", body)
        self.assertIn("/.github/release-targets.json", body)
        removal = body.index("- name: Remove the release signing keychain")
        self.assertIn("if: always()", body[removal:])
        self.assertIn("macos-signing-keychain.sh remove", body[removal:])
        self.assertIn("MACOS_CERTIFICATE_PRESENT", self.jobs["prepare"])
        # Local and ordinary CI builds stay ad hoc; the hardened runtime stays off (a certificate
        # without a Team ID would make library validation refuse the embedded dylibs).
        config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.macos.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(config["bundle"]["macOS"]["signingIdentity"], "-")
        self.assertFalse(config["bundle"]["macOS"]["hardenedRuntime"])

    def test_the_gate_status_is_written_only_in_automatic_mode(self) -> None:
        gate = self.jobs["gate"]
        self.assertIn("if: always()", gate)
        self.assertIn('if [ "$MODE" = automatic ]; then', gate)


class ContinuousIntegration(unittest.TestCase):
    def setUp(self) -> None:
        self.path = WORKFLOWS / "ci.yml"
        self.jobs = jobs(self.path)

    def test_no_merge_queue_entry_without_a_candidate_status(self) -> None:
        # The ruleset requires `Release candidate`, which nothing writes for a merge group.
        triggers = top_block(self.path.read_text(encoding="utf-8"), "on")
        self.assertNotRegex(triggers, r"(?m)^  merge_group:")

    def test_pull_requests_that_are_not_releases_get_the_candidate_status(self) -> None:
        body = self.jobs["candidate-status"]
        self.assertIn("github.event_name == 'pull_request'", body)
        self.assertIn("statuses: write", body)
        self.assertIn("context='Release candidate'", body)
        # release-please's PR is recognised without its label: the label arrives after `opened`.
        self.assertIn("github-actions[bot]", body)
        self.assertIn("release-please--branches--main--", body)
        self.assertNotIn("autorelease", body)
        self.assertNotIn("candidate-status", self.jobs["ci-success"])


if __name__ == "__main__":
    unittest.main()
