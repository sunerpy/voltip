"""Tests for scripts/release/candidate.py (seal / verify a release candidate), fed with the evidence
the real ``updater-json.py collect`` writes for the five legs of .github/release-targets.json.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import base64
import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
TARGETS = ROOT / ".github/release-targets.json"


def load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


candidate = load("candidate", "candidate.py")
updater_json = load("updater_json_for_candidate", "updater-json.py")

SIG = base64.b64encode(
    b"untrusted comment: signature from tauri secret key\nRUQ=\ntrusted comment: timestamp:1\nAAA=\n"
).decode()
VERSION = "0.0.4"
HEAD = "8cb920662b3d9bd040b71d01979aa2da89226f3c"
PR_HEAD = "f4294c74be8931dd86ba73145d710d58e9ea259e"
TREE = "03deefd1bad9dc577d0bb53a9432c231b737e530"
WORKFLOW_SHA = "a" * 40
GATE_SHA = HEAD
REPO = "sunerpy/voltip"
WORKFLOW_PATH = ".github/workflows/release-candidate.yml"

# What each leg of release-candidate.yml bundles, in the tauri-bundler layout (bundle-dir/kind/…).
LEGS = {
    "x86_64-unknown-linux-gnu": {
        "bundles": "deb,appimage",
        "updater": "appimage",
        "files": {
            "deb/Voltip_0.0.4_amd64.deb": b"deb",
            "appimage/Voltip_0.0.4_amd64.AppImage": b"appimage",
            "appimage/Voltip_0.0.4_amd64.AppImage.sig": SIG.encode(),
        },
        "rename": [],
        "extra": ["voltip-server-0.0.4-linux-x64.tar.gz"],
    },
    "x86_64-pc-windows-msvc": {
        "bundles": "nsis",
        "updater": "nsis",
        "files": {
            "nsis/Voltip_0.0.4_x64-setup.exe": b"setup",
            "nsis/Voltip_0.0.4_x64-setup.exe.sig": SIG.encode(),
        },
        "rename": [],
        "extra": ["Voltip_0.0.4_x64-portable.zip"],
    },
    "aarch64-apple-darwin": {
        "bundles": "dmg,app",
        "updater": "app",
        "files": {
            "dmg/Voltip_0.0.4_aarch64.dmg": b"dmg-arm",
            "macos/Voltip.app.tar.gz": b"app-arm",
            "macos/Voltip.app.tar.gz.sig": SIG.encode(),
        },
        "rename": ["Voltip.app.tar.gz=Voltip_0.0.4_aarch64.app.tar.gz"],
        "extra": [],
    },
    "x86_64-apple-darwin": {
        "bundles": "dmg,app",
        "updater": "app",
        "files": {
            "dmg/Voltip_0.0.4_x64.dmg": b"dmg-intel",
            "macos/Voltip.app.tar.gz": b"app-intel",
            "macos/Voltip.app.tar.gz.sig": SIG.encode(),
        },
        "rename": ["Voltip.app.tar.gz=Voltip_0.0.4_x64.app.tar.gz"],
        "extra": [],
    },
    # The updater serves no Android target (docs/runbook.md 发布 · Android).
    "aarch64-linux-android": {
        "bundles": "apk,aab",
        "updater": "none",
        "files": {
            "apk/Voltip_0.0.4_android_arm64.apk": b"apk",
            "aab/Voltip_0.0.4_android_arm64.aab": b"aab",
        },
        "rename": [],
        "extra": [],
    },
}


def run(module, *argv: str) -> str:
    out = io.StringIO()
    with redirect_stdout(out):
        module.main(list(argv))
    return out.getvalue()


class Candidate(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.root = self.tmp / "candidate"
        # Each leg collects into its own directory, as on its runner; the aggregate job's
        # download-artifact (merge-multiple) then lays them over one another.
        for target, leg in LEGS.items():
            self.collect(target, leg)

    def collect(self, target: str, leg: dict) -> None:
        work = self.tmp / "legs" / target
        bundle = work / "bundle"
        for rel, data in leg["files"].items():
            path = bundle / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        argv = [
            "collect",
            "--target",
            target,
            "--bundle-dir",
            str(bundle),
            "--bundles",
            leg["bundles"],
            "--updater-bundle",
            leg["updater"],
            "--updater",
            "false" if leg["updater"] == "none" else "true",
            "--out",
            str(self.root / "dist"),
            "--evidence",
            str(self.root / "evidence" / f"{target}.json"),
        ]
        for rename in leg["rename"]:
            argv += ["--rename", rename]
        for name in leg["extra"]:
            source = work / name
            source.write_bytes(b"portable")
            argv += ["--extra", f"{name}={source}"]
        run(updater_json, *argv)

    def seal(self, **overrides: str) -> str:
        values = {
            "--mode": "backfill",
            "--head-sha": HEAD,
            "--release-pr-head-sha": PR_HEAD,
            "--source-gate-conclusion": "success",
            **overrides,
        }
        argv = [
            "seal",
            "--candidate-root",
            str(self.root),
            "--targets-file",
            str(TARGETS),
            "--repository",
            REPO,
            "--workflow-ref",
            f"{REPO}/{WORKFLOW_PATH}@refs/heads/main",
            "--workflow-sha",
            WORKFLOW_SHA,
            "--run-id",
            "123",
            "--run-attempt",
            "1",
            "--release-pr-number",
            "10",
            "--tree-sha",
            TREE,
            "--version",
            VERSION,
            "--source-gate-sha",
            GATE_SHA,
            "--subjects-out",
            str(self.tmp / "subjects.sha256"),
        ]
        for key, value in values.items():
            argv += [key, value]
        return run(candidate, *argv)

    def verify(self, *extra: str, **overrides: str) -> str:
        values = {
            "--expected-repository": REPO,
            "--expected-workflow-path": WORKFLOW_PATH,
            "--expected-workflow-ref": "refs/heads/main",
            "--expected-workflow-sha": WORKFLOW_SHA,
            "--expected-run-id": "123",
            "--expected-run-attempt": "1",
            "--expected-pr-number": "10",
            "--expected-head-sha": HEAD,
            "--expected-pr-head-sha": PR_HEAD,
            "--expected-tree-sha": TREE,
            "--expected-version": VERSION,
            "--expected-tag": f"v{VERSION}",
            "--expected-updater": "true",
            **overrides,
        }
        argv = ["verify", "--candidate-root", str(self.root), "--targets-file", str(TARGETS)]
        for key, value in values.items():
            argv += [key, value]
        return run(candidate, *argv, *extra)

    def assertFails(self, fragment: str, call, *args, **kwargs) -> None:
        with self.assertRaises(SystemExit) as ctx:
            call(*args, **kwargs)
        self.assertIn(fragment, str(ctx.exception))

    # ------------------------------------------------------------------------------ tests

    def test_a_complete_candidate_seals_verifies_and_lists_only_its_files(self) -> None:
        self.seal()
        manifest = json.loads((self.root / "candidate-manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["tag"], "v0.0.4")
        self.assertEqual(manifest["mode"], "backfill")
        self.assertTrue(manifest["updater_enabled"])
        self.assertEqual(
            [leg["target"] for leg in manifest["targets"]],
            sorted(LEGS),
        )
        listed = self.verify("--print-files").split()
        expected = sorted(
            [
                "Voltip_0.0.4_aarch64.app.tar.gz",
                "Voltip_0.0.4_aarch64.app.tar.gz.sig",
                "Voltip_0.0.4_aarch64.dmg",
                "Voltip_0.0.4_amd64.AppImage",
                "Voltip_0.0.4_amd64.AppImage.sig",
                "Voltip_0.0.4_amd64.deb",
                "Voltip_0.0.4_android_arm64.aab",
                "Voltip_0.0.4_android_arm64.apk",
                "Voltip_0.0.4_x64-portable.zip",
                "Voltip_0.0.4_x64-setup.exe",
                "Voltip_0.0.4_x64-setup.exe.sig",
                "Voltip_0.0.4_x64.app.tar.gz",
                "Voltip_0.0.4_x64.app.tar.gz.sig",
                "Voltip_0.0.4_x64.dmg",
                "voltip-server-0.0.4-linux-x64.tar.gz",
            ]
        )
        self.assertEqual(listed, [f"dist/{name}" for name in expected] + ["candidate-manifest.json"])
        subjects = (self.tmp / "subjects.sha256").read_text(encoding="utf-8").splitlines()
        self.assertEqual([line.split("  ", 1)[1] for line in subjects], expected)
        for line in subjects:
            digest, name = line.split("  ", 1)
            self.assertEqual(digest, candidate.sha256(self.root / "dist" / name))

    def test_a_missing_leg_is_refused(self) -> None:
        evidence = self.root / "evidence" / "x86_64-apple-darwin.json"
        for item in json.loads(evidence.read_text(encoding="utf-8"))["files"]:
            (self.root / "dist" / item["name"]).unlink()
        evidence.unlink()
        self.assertFails("missing ['x86_64-apple-darwin.json']", self.seal)

    def test_a_file_no_leg_recorded_is_refused(self) -> None:
        (self.root / "dist" / "stray.txt").write_text("x", encoding="utf-8")
        self.assertFails("unlisted ['stray.txt']", self.seal)

    def test_a_leg_without_one_of_its_bundles_is_refused(self) -> None:
        evidence = self.root / "evidence" / "x86_64-unknown-linux-gnu.json"
        leg = json.loads(evidence.read_text(encoding="utf-8"))
        leg["files"] = [item for item in leg["files"] if item["kind"] != "deb"]
        evidence.write_text(json.dumps(leg), encoding="utf-8")
        (self.root / "dist" / "Voltip_0.0.4_amd64.deb").unlink()
        self.assertFails("x86_64-unknown-linux-gnu: no deb bundle", self.seal)

    def test_an_android_leg_that_claims_the_updater_is_refused(self) -> None:
        evidence = self.root / "evidence" / "aarch64-linux-android.json"
        leg = json.loads(evidence.read_text(encoding="utf-8"))
        leg["updater_enabled"] = True
        evidence.write_text(json.dumps(leg), encoding="utf-8")
        self.assertFails("aarch64-linux-android: the updater serves no such target", self.seal)

    def test_a_target_with_half_an_updater_is_refused(self) -> None:
        targets = json.loads(TARGETS.read_text(encoding="utf-8"))
        android = next(t for t in targets["targets"] if t["target"] == "aarch64-linux-android")
        android["updater_platform"] = "android-aarch64"
        path = self.tmp / "targets.json"
        path.write_text(json.dumps(targets), encoding="utf-8")
        self.assertFails("aarch64-linux-android: bundles, updater_bundle or platforms are invalid", candidate.target_specs, path)

    def test_bytes_changed_after_sealing_are_refused(self) -> None:
        self.seal()
        (self.root / "dist" / "Voltip_0.0.4_x64.dmg").write_bytes(b"dmg-intel-tampered")
        self.assertFails("Voltip_0.0.4_x64.dmg: bytes differ", self.verify)

    def test_a_file_planted_after_sealing_is_never_listed(self) -> None:
        self.seal()
        (self.root / "dist" / "Voltip_0.0.4_x64-evil.exe").write_bytes(b"evil")
        self.assertFails("unlisted ['Voltip_0.0.4_x64-evil.exe']", self.verify, "--print-files")
        (self.root / "dist" / "Voltip_0.0.4_x64-evil.exe").unlink()
        (self.root / "SHA256SUMS").write_text("x", encoding="utf-8")
        self.assertFails("candidate layout", self.verify, "--print-files")

    def test_the_identity_must_match_what_the_controller_resolved(self) -> None:
        self.seal()
        self.assertFails("tree_sha", self.verify, **{"--expected-tree-sha": "b" * 40})
        self.assertFails("tag", self.verify, **{"--expected-tag": "v0.0.5"})
        self.assertFails("run_id", self.verify, **{"--expected-run-id": "124"})
        self.assertFails("workflow_sha", self.verify, **{"--expected-workflow-sha": "c" * 40})
        self.assertFails("updater_enabled", self.verify, **{"--expected-updater": "false"})
        self.assertFails(
            "workflow_ref",
            self.verify,
            **{"--expected-workflow-path": ".github/workflows/release.yml"},
        )

    def test_a_dry_run_candidate_is_not_promotable(self) -> None:
        self.seal(**{"--mode": "dry-run"})
        self.assertFails("'dry-run' candidate is not promotable", self.verify)

    def test_an_automatic_candidate_must_be_the_release_pr_head(self) -> None:
        self.assertFails(
            "must build the release PR head",
            self.seal,
            **{"--mode": "automatic", "--head-sha": PR_HEAD, "--release-pr-head-sha": HEAD},
        )

    def test_a_failed_source_gate_cannot_be_sealed(self) -> None:
        self.assertFails("not success", self.seal, **{"--source-gate-conclusion": "failure"})


if __name__ == "__main__":
    unittest.main()
