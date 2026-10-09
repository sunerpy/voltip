"""Tests for .github/scripts/sign-android-package.sh and check-android-package.sh: the Android app's
APK and AAB (the React Native app since 0.0.50, under the package dev.voltip.mobile), with
stand-ins for the Android build tools: the packages are real zip files, the tools print what the
real ones print.

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
if [ "$1 $2" = "dump xmltree" ]; then
  # A manifest as long as a real one prints, the attribute early: a reader that stops at the first
  # match must not fail the check.
  echo "        A: http://schemas.android.com/apk/res/android:extractNativeLibs(0x010104ea)=$FAKE_EXTRACT"
  for i in $(seq 1 100000); do echo "        E: meta-data (line=$i)"; done
  exit 0
fi
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
            "FAKE_EXTRACT": "true",
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

    def signed(self, library: str = "libvoltip_rn.so") -> tuple[Path, Path]:
        """The app's APK and AAB with `library`, signed into android-bundle/."""
        apk = self.package("unsigned.apk", "", library)
        aab = self.package("unsigned.aab", "base/", library)
        out = self.root / "android-bundle"
        result = self.run_script(SIGN, apk, aab, out, VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)
        return out / f"apk/Voltip_{VERSION}_android_arm64.apk", out / f"aab/Voltip_{VERSION}_android_arm64.aab"

    def test_the_app_is_signed_and_checked_as_an_apk_and_an_aab(self) -> None:
        apk, aab = self.signed()
        out = self.root / "android-bundle"
        self.assertEqual(sorted(p.name for p in out.rglob("*") if p.is_file()), [aab.name, apk.name])
        self.assertIn(b"STAND-IN-SIGNATURE", apk.read_bytes())
        self.assertIn(b"STAND-IN-SIGNATURE", aab.read_bytes())
        checked = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertIn("dev.voltip.mobile 0.0.46 (code 46, target SDK 36); APK and AAB signed by", checked.stdout)

    def test_regression_the_package_is_the_one_the_tauri_app_had_and_carries_the_apps_library(self) -> None:
        # Since 0.0.50 the React Native app is the Android app under dev.voltip.mobile (user decision
        # 2026-10-09): its own earlier package, or a package without its library, is not it. The
        # library loop of the check names its variable `library`; a first version kept the app's
        # library in the same variable and then looked for lib/arm64-v8a/ with no name.
        apk, aab = self.signed()
        old = self.run_script(CHECK, apk, aab, DIGEST, VERSION, package="dev.voltip.mobile.rn")
        self.assertEqual(old.returncode, 1)
        self.assertIn("package name dev.voltip.mobile.rn, expected dev.voltip.mobile", old.stderr)
        tauri_apk, tauri_aab = self.signed("libvoltip_mobile_lib.so")
        missing = self.run_script(CHECK, tauri_apk, tauri_aab, DIGEST, VERSION)
        self.assertEqual(missing.returncode, 1)
        self.assertIn("the APK has no arm64-v8a libvoltip_rn.so", missing.stderr)

    def test_regression_the_app_must_extract_its_native_libraries(self) -> None:
        # PR #123's emulator run: with the libraries left in the APK, React Native's SoLoader looked
        # for libreactnative.so under lib/x86_64 on the x86_64 emulator that runs the arm64 app, and
        # the app closed on start.
        apk, aab = self.signed()
        self.env["FAKE_EXTRACT"] = "false"
        kept = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
        self.assertEqual(kept.returncode, 1)
        self.assertIn("does not extract its native libraries", kept.stderr)

    def test_a_signed_input_another_certificate_or_another_version_is_refused(self) -> None:
        apk, aab = self.signed()
        again = self.run_script(SIGN, apk, aab, self.root / "twice", VERSION)
        self.assertEqual(again.returncode, 1)
        self.assertIn("is signed already", again.stderr)
        other = self.run_script(CHECK, apk, aab, "cd" * 32, VERSION)
        self.assertEqual(other.returncode, 1)
        self.assertIn(f"the APK is signed with {DIGEST}", other.stderr)
        newer = self.run_script(CHECK, apk, aab, DIGEST, "0.0.47")
        self.assertEqual(newer.returncode, 1)
        self.assertIn("version name 0.0.46, expected 0.0.47", newer.stderr)

    def test_a_missing_aab_the_second_apps_option_or_a_missing_argument_is_refused(self) -> None:
        apk = self.package("unsigned.apk", "", "libvoltip_rn.so")
        no_aab = self.run_script(SIGN, apk, self.root / "none.aab", self.root / "out", VERSION)
        self.assertEqual(no_aab.returncode, 1)
        self.assertIn("no AAB at", no_aab.stderr)
        for script, args in (
            (SIGN, ("--app", "mobile-rn", apk, self.root / "out", VERSION)),
            (SIGN, (apk, self.root / "out", VERSION)),
            (CHECK, ("--app", "mobile-rn", apk, DIGEST, VERSION)),
            (CHECK, (apk, DIGEST, VERSION)),
        ):
            with self.subTest(script=script.name, args=args[:2]):
                result = self.run_script(script, *args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
