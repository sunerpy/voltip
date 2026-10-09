"""Tests for scripts/release/macos-signature.py: the release app and every Mach-O in it carry the
project's one self-signed certificate; an ad-hoc build, another certificate or a longer chain fail.
The ``codesign`` reports below have the shape codesign prints.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]

spec = importlib.util.spec_from_file_location("macos_signature", HERE / "macos-signature.py")
assert spec is not None and spec.loader is not None
signature = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signature)

HASH = "3571B3B4C8596B9731392EEFC280664E66234288"
EXPECTED = f'identifier "dev.voltip.desktop" and certificate leaf = H"{HASH}"'
SIGNED = f"""Executable=/tmp/Voltip.app/Contents/MacOS/voltip-desktop
Identifier=dev.voltip.desktop
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20400 size=12345 flags=0x0(none) hashes=380+7 location=embedded
Signature size=1700
Authority=Voltip Code Signing
Signed Time=29 Sep 2026 at 13:30:00
Info.plist entries=24
TeamIdentifier=not set
"""
ADHOC = """Executable=/tmp/Voltip.app/Contents/MacOS/voltip-desktop
Identifier=dev.voltip.desktop
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20400 size=12345 flags=0x2(adhoc) hashes=380+7 location=embedded
Signature=adhoc
Info.plist entries=24
TeamIdentifier=not set
"""


def requirements(requirement: str) -> str:
    return f"Executable=/tmp/Voltip.app/Contents/MacOS/voltip-desktop\ndesignated => {requirement}\n"


class AppTest(unittest.TestCase):
    def test_the_release_certificate_passes_in_either_spelling(self) -> None:
        self.assertEqual(signature.check_app(SIGNED, requirements(EXPECTED), EXPECTED), EXPECTED)
        root = f'identifier "dev.voltip.desktop" and certificate root = H"{HASH.lower()}"'
        self.assertEqual(signature.check_app(SIGNED, requirements(root), EXPECTED), root)

    def test_an_ad_hoc_app_fails(self) -> None:
        # What every local and ordinary CI build is: never a release.
        with self.assertRaisesRegex(SystemExit, "signed ad hoc"):
            signature.check_app(ADHOC, requirements('cdhash H"0123456789abcdef0123456789abcdef01234567"'), EXPECTED)

    def test_another_certificate_or_identifier_fails(self) -> None:
        other = EXPECTED.replace(HASH, "0" * 40)
        with self.assertRaisesRegex(SystemExit, "designated requirement is"):
            signature.check_app(SIGNED, requirements(other), EXPECTED)
        renamed = EXPECTED.replace("dev.voltip.desktop", "dev.voltip.other")
        with self.assertRaisesRegex(SystemExit, "designated requirement is"):
            signature.check_app(SIGNED, requirements(renamed), EXPECTED)

    def test_a_chain_is_not_the_self_signed_certificate(self) -> None:
        chained = SIGNED.replace("Authority=Voltip Code Signing\n", "Authority=Voltip Code Signing\nAuthority=Some CA\n")
        with self.assertRaisesRegex(SystemExit, "one self-signed authority"):
            signature.check_app(chained, requirements(EXPECTED), EXPECTED)

    def test_unsigned_code_has_no_requirement(self) -> None:
        with self.assertRaisesRegex(SystemExit, "unsigned"):
            signature.designated("Executable=/tmp/x\n")


class CodeTest(unittest.TestCase):
    def test_a_library_signed_by_the_same_certificate_passes(self) -> None:
        dylib = f'identifier "libsherpa-onnx-c-api" and certificate leaf = H"{HASH.lower()}"'
        self.assertEqual(signature.check_code(requirements(dylib), EXPECTED), HASH)

    def test_an_ad_hoc_or_foreign_library_fails(self) -> None:
        with self.assertRaisesRegex(SystemExit, "pins no certificate"):
            signature.check_code(requirements('cdhash H"0123456789abcdef0123456789abcdef01234567"'), EXPECTED)
        foreign = f'identifier "libonnxruntime" and certificate leaf = H"{"1" * 40}"'
        with self.assertRaisesRegex(SystemExit, "signed by certificate"):
            signature.check_code(requirements(foreign), EXPECTED)


class TargetsTest(unittest.TestCase):
    def test_the_repository_names_the_release_certificate(self) -> None:
        # One canonical requirement for both macOS legs; its hash is the certificate's SHA-1.
        targets = json.loads((ROOT / ".github/release-targets.json").read_text(encoding="utf-8"))
        requirement = targets["macos_signing"]["designated_requirement"]
        self.assertEqual(signature.canonical(requirement), requirement)
        self.assertTrue(requirement.startswith('identifier "dev.voltip.desktop" and certificate leaf = H"'))
        self.assertEqual(signature.certificate(requirement), targets["macos_signing"]["certificate_sha1"])


if __name__ == "__main__":
    unittest.main()
