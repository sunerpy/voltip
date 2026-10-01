"""Tests for third-party-notices.py (release-helpers gate)."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("third_party_notices", HERE / "third-party-notices.py")
assert SPEC and SPEC.loader
notices = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notices)

MIT = "MIT License\n\nCopyright (c) The authors\n\nPermission is hereby granted..."


def about_json() -> dict:
    crate = lambda name, version: {"name": name, "version": version}  # noqa: E731
    return {
        "overview": [],
        "crates": [
            {"package": {"name": "serde", "version": "1.0.229", "license": "MIT OR Apache-2.0"}, "license": "MIT OR Apache-2.0"},
            {"package": {"name": "webpki-roots", "version": "1.0.9", "license": "CDLA-Permissive-2.0"}, "license": "CDLA-Permissive-2.0"},
        ],
        "licenses": [
            {"id": "MIT", "name": "MIT License", "text": MIT, "used_by": [{"crate": crate("serde", "1.0.229")}]},
            # The same text a second time (cargo-about lists one entry per source): printed once.
            {"id": "MIT", "name": "MIT License", "text": MIT, "used_by": [{"crate": crate("serde", "1.0.229")}]},
            {"id": "CDLA-Permissive-2.0", "name": "Community Data License Agreement Permissive 2.0", "text": "CDLA text", "used_by": [{"crate": crate("webpki-roots", "1.0.9")}]},
        ],
    }


class Notices(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        font = self.root / "node_modules" / "@fontsource-variable" / "instrument-sans"
        font.mkdir(parents=True)
        (font / "LICENSE").write_text("SIL OPEN FONT LICENSE Version 1.1\n", encoding="utf-8")
        bare = self.root / "node_modules" / "bare"
        bare.mkdir(parents=True)
        self.pnpm = {
            "OFL-1.1": [{"name": "@fontsource-variable/instrument-sans", "versions": ["5.3.0"], "paths": [str(font)], "homepage": "https://fontsource.org"}],
            "MIT": [{"name": "bare", "versions": ["1.0.0"], "paths": [str(bare)]}],
        }
        (self.root / "about.json").write_text(json.dumps(about_json()), encoding="utf-8")
        (self.root / "pnpm.json").write_text(json.dumps(self.pnpm), encoding="utf-8")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def generate(self) -> str:
        out = self.root / "THIRD-PARTY-NOTICES.txt"
        code = notices.main(["--out", str(out), "--version", "0.0.1", "--about-json", str(self.root / "about.json"), "--pnpm-json", str(self.root / "pnpm.json"), "--no-crate-sources"])
        self.assertEqual(code, 0)
        return out.read_text(encoding="utf-8")

    def test_every_source_is_listed_with_its_licence_text(self) -> None:
        text = self.generate()
        self.assertTrue(text.startswith("Voltip 0.0.1 — third-party notices"))
        for heading in ("Native libraries", "Rust crates (2)", "Web frontend packages (2)"):
            self.assertIn(heading, text)
        self.assertIn("serde 1.0.229 — MIT OR Apache-2.0", text)
        # The root store's data licence asks for its text next to the binary (deny.toml).
        self.assertIn("CDLA text", text)
        self.assertIn("Used by: webpki-roots 1.0.9", text)
        self.assertEqual(text.count(MIT), 1, "an identical licence text is printed once")
        self.assertIn("SIL OPEN FONT LICENSE Version 1.1", text)
        self.assertIn("(no licence file in the package; the licence is MIT)", text)

    def test_regression_the_notices_carry_voltips_own_licence_and_where_its_source_is(self) -> None:
        # User decision 2026-10-01: AGPL-3.0-or-later after 0.0.20. AGPL-3.0 s. 4 and 6 ask for the
        # licence text with every copy of the program and for the Corresponding Source; the notices
        # file is what each package ships, so both are in it.
        text = self.generate()
        licence = (notices.ROOT / "LICENSE").read_text(encoding="utf-8").strip()
        self.assertIn("GNU AFFERO GENERAL PUBLIC LICENSE", licence)
        self.assertEqual(text.count(licence), 1)
        self.assertIn("either version 3\nof the License, or (at your option) any later version", text)
        self.assertIn("https://github.com/sunerpy/voltip/tree/v0.0.1", text)
        self.assertLess(text.index(licence), text.index("Rust crates (2)"), "Voltip's own licence comes first")

    def test_the_native_libraries_carry_their_own_texts(self) -> None:
        text = self.generate()
        self.assertIn("ONNX Runtime 1.28.2", text)
        self.assertIn("Copyright (c) Microsoft Corporation", text)
        self.assertIn("ONNX Runtime's own third-party notices:", text)
        self.assertIn("Khronos Vulkan loader", text)
        self.assertIn("Apache License", (HERE / "licenses" / "vulkan-loader-LICENSE.txt").read_text(encoding="utf-8"))
        self.assertIn("sherpa-onnx", text)

    def test_the_android_app_lists_its_crates_and_packages_and_no_desktop_library(self) -> None:
        out = self.root / "android" / "THIRD-PARTY-NOTICES.txt"
        code = notices.main(["--out", str(out), "--app", "mobile", "--version", "0.0.1", "--about-json", str(self.root / "about.json"), "--pnpm-json", str(self.root / "pnpm.json")])
        self.assertEqual(code, 0)
        text = out.read_text(encoding="utf-8")
        self.assertIn("Rust crates (2)", text)
        self.assertIn("Web frontend packages (2)", text)
        for desktop_only in ("Native libraries", "ONNX Runtime", "Vulkan", "WebView2", "transcribe.cpp"):
            self.assertNotIn(desktop_only, text)

    def test_the_android_app_is_scanned_for_its_own_target(self) -> None:
        calls = []
        original = notices.run_json
        notices.run_json = lambda argv, cwd: calls.append((argv, cwd)) or {}
        try:
            notices.cargo_about("mobile")
            notices.pnpm_licenses("mobile")
        finally:
            notices.run_json = original
        (about, _), (pnpm, cwd) = calls
        self.assertIn(str(notices.MOBILE_MANIFEST), about)
        self.assertEqual(about[about.index("--target") + 1], "aarch64-linux-android")
        self.assertNotIn("gpu-vulkan", about)
        self.assertEqual(cwd, notices.ROOT / "apps" / "mobile")

    def test_a_tool_that_fails_is_an_error_not_an_empty_notice(self) -> None:
        with self.assertRaises(notices.Failure):
            notices.run_json(["false"], self.root)
        with self.assertRaises(notices.Failure):
            notices.run_json(["voltip-no-such-tool"], self.root)


if __name__ == "__main__":
    unittest.main()
