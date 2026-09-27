"""Tests for scripts/release/updater-json.py (stdlib unittest, no network, no minisign).

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import base64
import importlib.util
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("updater_json", HERE / "updater-json.py")
assert SPEC is not None and SPEC.loader is not None
uj = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(uj)

# A syntactically valid minisign public key (Ed + 8-byte key id + 32-byte key), base64-wrapped
# the way tauri.conf.json stores it. Only the shape is checked; no signature is verified here.
PUBKEY_TEXT = "untrusted comment: minisign public key: TEST\n" + base64.b64encode(
    b"Ed" + b"\x01" * 8 + b"\x02" * 32
).decode() + "\n"
PUBKEY_B64 = base64.b64encode(PUBKEY_TEXT.encode()).decode()
SIG_TEXT = "untrusted comment: signature from tauri secret key\nRUQ=\ntrusted comment: timestamp:1\nAAA=\n"
SIG_B64 = base64.b64encode(SIG_TEXT.encode()).decode()


def run(*argv: str) -> str:
    out = io.StringIO()
    with redirect_stdout(out):
        uj.main(list(argv))
    return out.getvalue()


class Workspace(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def write(self, rel: str, content: str | bytes) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            path.write_bytes(content)
        else:
            path.write_text(content, encoding="utf-8")
        return path

    def write_json(self, rel: str, data: object) -> Path:
        return self.write(rel, json.dumps(data, indent=2) + "\n")

    def assertFails(self, fragment: str, *argv: str) -> None:
        with self.assertRaises(SystemExit) as ctx:
            run(*argv)
        self.assertIn(fragment, str(ctx.exception))


class CheckConfig(Workspace):
    """tauri.conf.json carries no updater key: update.rs injects plugins.updater at runtime from
    VOLTIP_UPDATE_URL / VOLTIP_UPDATE_PUBKEY and the release enables signing with --config."""

    def conf(self, **overrides: object) -> dict:
        data = {
            "productName": "Voltip",
            "version": "2.0.0-alpha.1",
            "bundle": {"active": True, "targets": "all"},
        }
        data.update(overrides)
        return data

    def setUp(self) -> None:
        super().setUp()
        self.write_json("package.json", {"name": "voltip-workspace", "version": "2.0.0-alpha.1"})

    def args(self, *extra: str, url: str = "false", pubkey: str = "") -> tuple[str, ...]:
        return (
            "check-config",
            "--project",
            str(self.root / "apps/desktop/src-tauri"),
            "--package-json",
            str(self.root / "package.json"),
            "--update-url-present",
            url,
            "--update-pubkey",
            pubkey,
            *extra,
        )

    def check(self, *extra: str, **kw: str) -> dict:
        return json.loads(run(*self.args(*extra, **kw)))

    def test_literal_version_matching_package_json_passes_without_updater(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf())
        out = self.root / "gh_output"
        summary = self.check("--expect-version", "2.0.0-alpha.1", "--github-output", str(out))
        self.assertEqual(summary["version"], "2.0.0-alpha.1")
        self.assertEqual(summary["version_source"], "literal")
        self.assertFalse(summary["updater_enabled"])
        lines = out.read_text().splitlines()
        self.assertEqual(
            lines, ["version=2.0.0-alpha.1", "updater_enabled=false", "product_name=Voltip"]
        )

    def test_package_json_pointer_is_accepted(self) -> None:
        self.write_json(
            "apps/desktop/src-tauri/tauri.conf.json", self.conf(version="../../../package.json")
        )
        self.assertEqual(self.check()["version_source"], "package.json")

    def test_literal_version_drift_fails(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf(version="2.0.0-alpha.2"))
        self.assertFails("differs from", *self.args())

    def test_expect_version_mismatch_fails(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf())
        self.assertFails("!= release version", *self.args("--expect-version", "2.0.0"))

    def test_both_updater_secrets_enable_the_updater(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf())
        out = self.root / "gh_output"
        summary = self.check("--github-output", str(out), url="true", pubkey=PUBKEY_B64)
        self.assertTrue(summary["updater_enabled"])
        self.assertIn("updater_enabled=true", out.read_text().splitlines())

    def test_lone_updater_secret_fails(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf())
        self.assertFails("must be set together", *self.args(url="true"))
        self.assertFails("must be set together", *self.args(pubkey=PUBKEY_B64))

    def test_malformed_pubkey_fails(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf())
        self.assertFails("minisign", *self.args(url="true", pubkey="bm90IGEga2V5"))

    def test_create_updater_artifacts_in_conf_is_rejected(self) -> None:
        # It would make `make windows-x64` demand a signing key on every developer machine.
        self.write_json(
            "apps/desktop/src-tauri/tauri.conf.json",
            self.conf(bundle={"active": True, "createUpdaterArtifacts": True}),
        )
        self.assertFails("keep bundle.createUpdaterArtifacts out", *self.args())
        self.assertFails(
            "keep bundle.createUpdaterArtifacts out", *self.args(url="true", pubkey=PUBKEY_B64)
        )

    def test_conf_updater_block_must_agree_with_the_baked_in_pubkey(self) -> None:
        other = base64.b64encode(PUBKEY_TEXT.replace("TEST", "OTHER").encode()).decode()
        self.write_json(
            "apps/desktop/src-tauri/tauri.conf.json",
            self.conf(plugins={"updater": {"pubkey": other}}),
        )
        self.assertFails("differs from VOLTIP_UPDATE_PUBKEY", *self.args(url="true", pubkey=PUBKEY_B64))
        self.write_json(
            "apps/desktop/src-tauri/tauri.conf.json",
            self.conf(plugins={"updater": {"pubkey": PUBKEY_B64, "endpoints": ["https://u/l.json"]}}),
        )
        self.assertTrue(self.check(url="true", pubkey=PUBKEY_B64)["updater_enabled"])

    def test_http_endpoint_in_conf_fails(self) -> None:
        self.write_json(
            "apps/desktop/src-tauri/tauri.conf.json",
            self.conf(plugins={"updater": {"endpoints": ["http://x/latest.json"]}}),
        )
        self.assertFails("must be https", *self.args())

    def test_bundle_inactive_fails(self) -> None:
        self.write_json("apps/desktop/src-tauri/tauri.conf.json", self.conf(bundle={"active": False}))
        self.assertFails("bundle.active", *self.args())


class Collect(Workspace):
    def make_windows_bundle(self, signed: bool = True) -> Path:
        root = self.root / "target/x86_64-pc-windows-msvc/release"
        self.write(
            "target/x86_64-pc-windows-msvc/release/bundle/nsis/Voltip_2.0.0-alpha.2_x64-setup.exe",
            b"installer-bytes",
        )
        if signed:
            self.write(
                "target/x86_64-pc-windows-msvc/release/bundle/nsis/Voltip_2.0.0-alpha.2_x64-setup.exe.sig",
                SIG_B64,
            )
        self.write("package/windows-x64/Voltip_2.0.0-alpha.2_x64-portable.zip", b"portable-bytes")
        return root

    def collect_args(self, updater: str, *extra: str) -> tuple[str, ...]:
        return (
            "collect",
            "--target",
            "x86_64-pc-windows-msvc",
            "--bundle-dir",
            str(self.root / "target/x86_64-pc-windows-msvc/release/bundle"),
            "--bundles",
            "nsis",
            "--updater-bundle",
            "nsis",
            "--updater",
            updater,
            "--out",
            str(self.root / "dist"),
            "--evidence",
            str(self.root / "evidence/x86_64-pc-windows-msvc.json"),
            *extra,
        )

    def test_collect_copies_bundle_signature_and_extra(self) -> None:
        self.make_windows_bundle()
        portable = self.root / "package/windows-x64/Voltip_2.0.0-alpha.2_x64-portable.zip"
        run(*self.collect_args("true", "--extra", f"Voltip_2.0.0-alpha.2_x64-portable.zip={portable}"))
        dist = sorted(p.name for p in (self.root / "dist").iterdir())
        self.assertEqual(
            dist,
            [
                "Voltip_2.0.0-alpha.2_x64-portable.zip",
                "Voltip_2.0.0-alpha.2_x64-setup.exe",
                "Voltip_2.0.0-alpha.2_x64-setup.exe.sig",
            ],
        )
        evidence = json.loads((self.root / "evidence/x86_64-pc-windows-msvc.json").read_text())
        self.assertEqual(evidence["schema_version"], 1)
        self.assertEqual(evidence["platform"], "windows")
        self.assertEqual(evidence["updater_platform"], "windows-x86_64")
        self.assertTrue(evidence["updater_enabled"])
        self.assertEqual(
            evidence["updater"],
            {
                "kind": "nsis",
                "name": "Voltip_2.0.0-alpha.2_x64-setup.exe",
                "signature": "Voltip_2.0.0-alpha.2_x64-setup.exe.sig",
            },
        )
        kinds = {item["name"]: item["kind"] for item in evidence["files"]}
        self.assertEqual(kinds["Voltip_2.0.0-alpha.2_x64-portable.zip"], "extra")
        self.assertEqual(kinds["Voltip_2.0.0-alpha.2_x64-setup.exe.sig"], "signature")
        setup = next(i for i in evidence["files"] if i["name"].endswith("_x64-setup.exe"))
        self.assertTrue(setup["signature"])
        self.assertEqual(setup["size"], len(b"installer-bytes"))

    def test_collect_fails_without_signature_when_updater_enabled(self) -> None:
        self.make_windows_bundle(signed=False)
        self.assertFails("exactly one signed nsis", *self.collect_args("true"))

    def test_collect_without_updater_records_no_updater_entry(self) -> None:
        self.make_windows_bundle(signed=False)
        run(*self.collect_args("false"))
        evidence = json.loads((self.root / "evidence/x86_64-pc-windows-msvc.json").read_text())
        self.assertIsNone(evidence["updater"])
        self.assertFalse(evidence["updater_enabled"])

    def test_collect_fails_when_bundle_missing(self) -> None:
        (self.root / "target/x86_64-pc-windows-msvc/release/bundle").mkdir(parents=True)
        self.assertFails("no nsis bundle", *self.collect_args("false"))

    def test_collect_rejects_bad_extra_and_unsafe_target(self) -> None:
        self.make_windows_bundle()
        self.assertFails("NAME=PATH", *self.collect_args("true", "--extra", "nope"))
        self.assertFails(
            "unsafe characters",
            "collect",
            "--target",
            "x86_64;rm",
            "--bundle-dir",
            str(self.root),
            "--bundles",
            "nsis",
            "--updater-bundle",
            "nsis",
            "--updater",
            "false",
            "--out",
            str(self.root / "dist"),
            "--evidence",
            str(self.root / "e.json"),
        )


class Write(Workspace):
    def seed(self, *, linux: bool = True, tamper: bool = False) -> None:
        dist = self.root / "dist"
        dist.mkdir()
        (dist / "Voltip_2.0.0-alpha.2_x64-setup.exe").write_bytes(b"installer")
        (dist / "Voltip_2.0.0-alpha.2_x64-setup.exe.sig").write_text(SIG_B64)
        (dist / "Voltip_2.0.0-alpha.2_x64-portable.zip").write_bytes(b"portable")
        windows = {
            "schema_version": 1,
            "target": "x86_64-pc-windows-msvc",
            "platform": "windows",
            "updater_platform": "windows-x86_64",
            "updater_enabled": True,
            "updater": {
                "kind": "nsis",
                "name": "Voltip_2.0.0-alpha.2_x64-setup.exe",
                "signature": "Voltip_2.0.0-alpha.2_x64-setup.exe.sig",
            },
            "files": [
                self.record(dist / "Voltip_2.0.0-alpha.2_x64-setup.exe", "nsis", True),
                self.record(dist / "Voltip_2.0.0-alpha.2_x64-setup.exe.sig", "signature", False),
                self.record(dist / "Voltip_2.0.0-alpha.2_x64-portable.zip", "extra", False),
            ],
        }
        self.write_json("evidence/x86_64-pc-windows-msvc.json", windows)
        if linux:
            (dist / "Voltip_2.0.0-alpha.2_amd64.AppImage").write_bytes(b"appimage")
            (dist / "Voltip_2.0.0-alpha.2_amd64.AppImage.sig").write_text(SIG_B64)
            (dist / "Voltip_2.0.0-alpha.2_amd64.deb").write_bytes(b"deb")
            self.write_json(
                "evidence/x86_64-unknown-linux-gnu.json",
                {
                    "schema_version": 1,
                    "target": "x86_64-unknown-linux-gnu",
                    "platform": "linux",
                    "updater_platform": "linux-x86_64",
                    "updater_enabled": True,
                    "updater": {
                        "kind": "appimage",
                        "name": "Voltip_2.0.0-alpha.2_amd64.AppImage",
                        "signature": "Voltip_2.0.0-alpha.2_amd64.AppImage.sig",
                    },
                    "files": [
                        self.record(dist / "Voltip_2.0.0-alpha.2_amd64.AppImage", "appimage", True),
                        self.record(dist / "Voltip_2.0.0-alpha.2_amd64.AppImage.sig", "signature", False),
                        self.record(dist / "Voltip_2.0.0-alpha.2_amd64.deb", "deb", False),
                    ],
                },
            )
        if tamper:
            (dist / "Voltip_2.0.0-alpha.2_x64-setup.exe").write_bytes(b"installer-tampered")

    def record(self, path: Path, kind: str, signature: bool) -> dict:
        return {
            "kind": kind,
            "name": path.name,
            "sha256": uj.sha256(path),
            "signature": signature,
            "size": path.stat().st_size,
        }

    def write_args(self, *extra: str) -> tuple[str, ...]:
        return (
            "write",
            "--dist",
            str(self.root / "dist"),
            "--evidence-dir",
            str(self.root / "evidence"),
            "--base-url",
            "https://github.com/example/voltip/releases/download/",
            "--tag",
            "v2.0.0-alpha.2",
            "--version",
            "2.0.0-alpha.2",
            "--out",
            str(self.root / "dist/latest.json"),
            *extra,
        )

    def test_writes_manifest_with_release_asset_urls(self) -> None:
        self.seed()
        notes = self.write("body.md", "## What's new\n\n- fix\n")
        run(
            *self.write_args(
                "--notes-file",
                str(notes),
                "--require-platform",
                "windows-x86_64",
                "--require-platform",
                "linux-x86_64",
            )
        )
        manifest = json.loads((self.root / "dist/latest.json").read_text())
        self.assertEqual(manifest["version"], "2.0.0-alpha.2")
        self.assertEqual(manifest["notes"], "## What's new\n\n- fix")
        self.assertRegex(manifest["pub_date"], r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
        self.assertEqual(sorted(manifest["platforms"]), ["linux-x86_64", "windows-x86_64"])
        win = manifest["platforms"]["windows-x86_64"]
        self.assertEqual(
            win["url"],
            "https://github.com/example/voltip/releases/download/v2.0.0-alpha.2/Voltip_2.0.0-alpha.2_x64-setup.exe",
        )
        self.assertEqual(win["signature"], SIG_B64)
        # Pinned to this release's assets, never the rolling `releases/latest` pointer.
        self.assertNotIn("/releases/latest/", json.dumps(manifest))

    def test_missing_required_platform_fails(self) -> None:
        self.seed(linux=False)
        self.assertFails(
            "lacks required platforms: ['linux-x86_64']",
            *self.write_args("--require-platform", "linux-x86_64"),
        )

    def test_tampered_dist_bytes_fail(self) -> None:
        self.seed(tamper=True)
        self.assertFails("bytes differ", *self.write_args())

    def test_rejects_non_https_base_url_and_tag_mismatch(self) -> None:
        self.seed()
        args = list(self.write_args())
        args[args.index("https://github.com/example/voltip/releases/download/")] = "http://github.com/example/voltip/releases/download"
        self.assertFails("must be an https URL", *args)
        args = list(self.write_args())
        args[args.index("v2.0.0-alpha.2")] = "v2.0.0"
        self.assertFails("must equal v2.0.0-alpha.2", *args)

    def test_updater_enabled_without_bundle_recorded_fails(self) -> None:
        self.seed(linux=False)
        path = self.root / "evidence/x86_64-pc-windows-msvc.json"
        data = json.loads(path.read_text())
        data["updater"] = None
        path.write_text(json.dumps(data))
        self.assertFails("no updater bundle recorded", *self.write_args())

    def test_no_platforms_at_all_fails(self) -> None:
        self.seed(linux=False)
        path = self.root / "evidence/x86_64-pc-windows-msvc.json"
        data = json.loads(path.read_text())
        data["updater"] = None
        data["updater_enabled"] = False
        path.write_text(json.dumps(data))
        self.assertFails("refusing to write an empty latest.json", *self.write_args())


class Helpers(unittest.TestCase):
    def test_updater_platform_keys(self) -> None:
        self.assertEqual(uj.updater_platform_for("x86_64-pc-windows-msvc"), "windows-x86_64")
        self.assertEqual(uj.updater_platform_for("x86_64-unknown-linux-gnu"), "linux-x86_64")
        self.assertEqual(uj.updater_platform_for("aarch64-apple-darwin"), "darwin-aarch64")
        with self.assertRaises(SystemExit):
            uj.updater_platform_for("wasm32-unknown-unknown")

    def test_pubkey_shape_validation(self) -> None:
        self.assertEqual(uj.decode_minisign_pubkey(PUBKEY_B64), PUBKEY_TEXT)
        with self.assertRaises(SystemExit):
            uj.decode_minisign_pubkey(base64.b64encode(b"not a key").decode())
        with self.assertRaises(SystemExit):
            uj.decode_minisign_pubkey("%%%")

    def test_script_is_executable(self) -> None:
        self.assertTrue(os.access(HERE / "updater-json.py", os.X_OK))


if __name__ == "__main__":
    unittest.main()
