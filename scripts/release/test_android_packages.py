"""Tests for .github/scripts/sign-android-package.sh and check-android-package.sh, both apps (the
Tauri phone app's APK and AAB, and `--app mobile-rn`'s APK alone), with stand-ins for the Android
build tools: the packages are real zip files, the tools print what the real ones print.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SIGN = ROOT / ".github/scripts/sign-android-package.sh"
CHECK = ROOT / ".github/scripts/check-android-package.sh"
DIGEST = "ab" * 32
VERSION = "0.0.46"

# apksigner: `verify` says whether the file carries the stand-in signature; `verify --print-certs`
# prints the signer the way apksigner does; `sign` copies the input and adds the signature.
APKSIGNER = """#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  verify)
    file=${@: -1}
    grep -q STAND-IN-SIGNATURE "$file" || exit 1
    if [ "$2" = --print-certs ]; then
      echo "Signer #1 certificate DN: CN=Voltip"
      echo "Signer #1 certificate SHA-256 digest: $FAKE_DIGEST"
    fi
    ;;
  sign)
    out="" input=${@: -1}
    while [ "$#" -gt 1 ]; do [ "$1" = --out ] && out=$2; shift; done
    cp "$input" "$out" && printf 'STAND-IN-SIGNATURE' >>"$out"
    ;;
esac
"""
ZIPALIGN = """#!/usr/bin/env bash
# -c checks (always aligned here); otherwise copy <in> to <out>.
[ "$1" = -c ] && exit 0
cp "${@: -2:1}" "${@: -1}"
"""
AAPT2 = """#!/usr/bin/env bash
echo "package: name='$FAKE_PACKAGE' versionCode='$FAKE_CODE' versionName='$FAKE_VERSION' platformBuildVersionName='16'"
echo "targetSdkVersion:'36'"
"""
READELF = """#!/usr/bin/env bash
echo "  LOAD           0x000000 0x0000000000000000 0x0000000000000000 0x001000 0x001000 R   0x4000"
"""
JARSIGNER = """#!/usr/bin/env bash
if [ "$1" = -verify ]; then
  grep -q STAND-IN-SIGNATURE "$2" && echo "jar verified." || echo "jar is unsigned."
  exit 0
fi
file=${@: -2:1}
printf 'STAND-IN-SIGNATURE' >>"$file"
"""
KEYTOOL = """#!/usr/bin/env bash
echo "	 SHA256: $FAKE_DIGEST"
"""


def tool(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class AndroidPackages(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        tools = self.root / "build-tools"
        bin_dir = self.root / "bin"
        ndk = self.root / "ndk"
        tool(tools / "apksigner", APKSIGNER)
        tool(tools / "zipalign", ZIPALIGN)
        tool(tools / "aapt2", AAPT2)
        tool(ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf", READELF)
        tool(bin_dir / "jarsigner", JARSIGNER)
        tool(bin_dir / "keytool", KEYTOOL)
        keystore = self.root / "release.jks"
        keystore.write_bytes(b"keystore")
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "ANDROID_BUILD_TOOLS": str(tools),
            "NDK_HOME": str(ndk),
            "ANDROID_KEYSTORE": str(keystore),
            "ANDROID_KEYSTORE_PASSWORD": "store-password",
            "ANDROID_KEY_ALIAS": "voltip",
            "ANDROID_KEY_PASSWORD": "key-password",
            "FAKE_DIGEST": DIGEST,
            "FAKE_VERSION": VERSION,
            "FAKE_CODE": "46",
        }

    def package(self, name: str, prefix: str, library: str) -> Path:
        """An unsigned package: the native library and this version's licence texts."""
        path = self.root / name
        with zipfile.ZipFile(path, "w") as z:
            z.writestr(f"{prefix}lib/arm64-v8a/{library}", b"\x7fELF native code")
            z.writestr(f"{prefix}assets/THIRD-PARTY-NOTICES.txt", f"Voltip {VERSION} — third-party notices\n")
        return path

    def run_script(self, script: Path, *args: str, package: str = "dev.voltip.mobile") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(script), *map(str, args)],
            env={**self.env, "FAKE_PACKAGE": package},
            capture_output=True,
            text=True,
            timeout=60,
        )

    def test_the_tauri_app_is_signed_and_checked_as_an_apk_and_an_aab(self) -> None:
        apk = self.package("unsigned.apk", "", "libvoltip_mobile_lib.so")
        aab = self.package("unsigned.aab", "base/", "libvoltip_mobile_lib.so")
        out = self.root / "android-bundle"
        signed = self.run_script(SIGN, apk, aab, out, VERSION)
        self.assertEqual(signed.returncode, 0, signed.stderr)
        self.assertEqual(sorted(p.name for p in out.rglob("*") if p.is_file()), [f"Voltip_{VERSION}_android_arm64.aab", f"Voltip_{VERSION}_android_arm64.apk"])
        checked = self.run_script(CHECK, out / f"apk/Voltip_{VERSION}_android_arm64.apk", out / f"aab/Voltip_{VERSION}_android_arm64.aab", DIGEST, VERSION)
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertIn("dev.voltip.mobile 0.0.46 (code 46, target SDK 36); APK and AAB signed by", checked.stdout)

    def test_regression_the_react_native_app_is_signed_under_its_own_release_name(self) -> None:
        # The secrets loop of the script names its variable `name`: the first version of
        # `--app mobile-rn` kept the asset's name in the same variable and wrote
        # ANDROID_KEY_PASSWORD_0.0.46_android_arm64.apk.
        apk = self.package("unsigned.apk", "", "libvoltip_rn.so")
        out = self.root / "android-rn-bundle"
        signed = self.run_script(SIGN, "--app", "mobile-rn", apk, out, VERSION)
        self.assertEqual(signed.returncode, 0, signed.stderr)
        self.assertEqual([p.name for p in out.rglob("*") if p.is_file()], [f"Voltip-RN_{VERSION}_android_arm64.apk"])
        self.assertIn(b"STAND-IN-SIGNATURE", (out / f"apk/Voltip-RN_{VERSION}_android_arm64.apk").read_bytes())

    def test_regression_the_react_native_app_is_checked_for_its_own_package_and_library(self) -> None:
        # The library loop of the check names its variable `library`: the first version of
        # `--app mobile-rn` kept the app's library in the same variable and then looked for
        # lib/arm64-v8a/ with no name.
        apk = self.package("unsigned.apk", "", "libvoltip_rn.so")
        out = self.root / "android-rn-bundle"
        self.assertEqual(self.run_script(SIGN, "--app", "mobile-rn", apk, out, VERSION).returncode, 0)
        signed = out / f"apk/Voltip-RN_{VERSION}_android_arm64.apk"
        checked = self.run_script(CHECK, "--app", "mobile-rn", signed, DIGEST, VERSION, package="dev.voltip.mobile.rn")
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertIn("dev.voltip.mobile.rn 0.0.46 (code 46, target SDK 36); APK signed by", checked.stdout)
        # The Tauri app's package, or a package without the app's library, is not the React Native app.
        wrong = self.run_script(CHECK, "--app", "mobile-rn", signed, DIGEST, VERSION, package="dev.voltip.mobile")
        self.assertEqual(wrong.returncode, 1)
        self.assertIn("package name dev.voltip.mobile, expected dev.voltip.mobile.rn", wrong.stderr)
        tauri = self.package("tauri.apk", "", "libvoltip_mobile_lib.so")
        self.assertEqual(self.run_script(SIGN, "--app", "mobile-rn", tauri, self.root / "other", VERSION).returncode, 0)
        missing = self.run_script(CHECK, "--app", "mobile-rn", self.root / f"other/apk/Voltip-RN_{VERSION}_android_arm64.apk", DIGEST, VERSION, package="dev.voltip.mobile.rn")
        self.assertEqual(missing.returncode, 1)
        self.assertIn("the APK has no arm64-v8a libvoltip_rn.so", missing.stderr)

    def test_a_signed_input_another_certificate_or_another_version_is_refused(self) -> None:
        apk = self.package("unsigned.apk", "", "libvoltip_rn.so")
        out = self.root / "android-rn-bundle"
        self.assertEqual(self.run_script(SIGN, "--app", "mobile-rn", apk, out, VERSION).returncode, 0)
        signed = out / f"apk/Voltip-RN_{VERSION}_android_arm64.apk"
        again = self.run_script(SIGN, "--app", "mobile-rn", signed, self.root / "twice", VERSION)
        self.assertEqual(again.returncode, 1)
        self.assertIn("is signed already", again.stderr)
        other = self.run_script(CHECK, "--app", "mobile-rn", signed, "cd" * 32, VERSION, package="dev.voltip.mobile.rn")
        self.assertEqual(other.returncode, 1)
        self.assertIn(f"the APK is signed with {DIGEST}", other.stderr)
        newer = self.run_script(CHECK, "--app", "mobile-rn", signed, DIGEST, "0.0.47", package="dev.voltip.mobile.rn")
        self.assertEqual(newer.returncode, 1)
        self.assertIn("version name 0.0.46, expected 0.0.47", newer.stderr)

    def test_an_unknown_app_or_a_missing_argument_is_a_usage_error(self) -> None:
        apk = self.package("unsigned.apk", "", "libvoltip_rn.so")
        for script, args in (
            (SIGN, ("--app", "desktop", apk, self.root / "out", VERSION)),
            (SIGN, ("--app", "mobile-rn", apk, self.root / "out")),
            (CHECK, ("--app", "desktop", apk, DIGEST, VERSION)),
            (CHECK, ("--app", "mobile-rn", apk, DIGEST)),
        ):
            with self.subTest(script=script.name, args=args[:2]):
                result = self.run_script(script, *args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
