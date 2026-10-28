"""Tests for .github/scripts/sign-android-package.sh and check-android-package.sh: the Android app's
APK and AAB (the React Native app since 0.0.50, under the package dev.voltip.mobile), with
stand-ins for the Android build tools: the packages are real zip files, the tools print what the
real ones print.

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import json
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
  # match must not fail the check. The app's activity follows the device (-1 = unspecified);
  # FAKE_ACTIVITY adds the attributes of another one.
  echo "        A: http://schemas.android.com/apk/res/android:extractNativeLibs(0x010104ea)=$FAKE_EXTRACT"
  echo "          E: activity (line=85)"
  echo "            A: http://schemas.android.com/apk/res/android:screenOrientation(0x0101001e)=-1"
  if [ -n "${FAKE_ACTIVITY:-}" ]; then
    echo "          E: activity (line=99)"
    echo "            A: http://schemas.android.com/apk/res/android:$FAKE_ACTIVITY"
  fi
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

    def package(
        self,
        name: str,
        prefix: str,
        library: str,
        r8: dict | None = None,
        mapping: bytes | None = b"# compiler: R8\n",
    ) -> Path:
        """An unsigned package: the native library and this version's licence texts; an AAB
        (prefix `base/`) also R8's report (`r8`, or the shares a real build of 0.2.0 left) and its
        map, as AGP writes them into BUNDLE-METADATA."""
        path = self.root / name
        with zipfile.ZipFile(path, "w") as z:
            z.writestr(f"{prefix}lib/arm64-v8a/{library}", b"\x7fELF native code")
            z.writestr(f"{prefix}assets/THIRD-PARTY-NOTICES.txt", f"Voltip {VERSION} — third-party notices\n")
            if prefix == "base/":
                stats = r8 if r8 is not None else {
                    "noObfuscationPercentage": 19.22,
                    "noOptimizationPercentage": 19.57,
                    "noShrinkingPercentage": 19.01,
                }
                if stats:
                    z.writestr("BUNDLE-METADATA/com.android.tools/r8.json", json.dumps({"stats": stats, "version": "8.12.14"}))
                if mapping is not None:
                    z.writestr("BUNDLE-METADATA/com.android.tools.build.obfuscation/proguard.map", mapping)
        return path

    def run_script(self, script: Path, *args: str, package: str = "dev.voltip.mobile") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(script), *map(str, args)],
            env={**self.env, "FAKE_PACKAGE": package},
            capture_output=True,
            text=True,
            timeout=60,
        )

    def signed(
        self,
        library: str = "libvoltip_rn.so",
        r8: dict | None = None,
        mapping: bytes | None = b"# compiler: R8\n",
        out: str = "android-bundle",
    ) -> tuple[Path, Path]:
        """The app's APK and AAB with `library` (and the AAB's R8 report and map), signed into
        `out`/."""
        apk = self.package(f"{out}-unsigned.apk", "", library)
        aab = self.package(f"{out}-unsigned.aab", "base/", library, r8=r8, mapping=mapping)
        out_dir = self.root / out
        out = out_dir
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
        self.assertIn("R8 obfuscation 80.78 %, optimization 80.43 %, shrinking 80.99 %, its map in the AAB", checked.stdout)
        self.assertIn("no orientation or size limit", checked.stdout)

    def test_r8_must_have_run_and_covered_a_quarter_of_the_code_each_way(self) -> None:
        # Google Play's floor from 2027-02 (help answer 17492799): obfuscation, optimization and
        # shrinking each at least 25 %. 0.1.8 had 1 % (R8 never ran: enableMinifyInReleaseBuilds was
        # off), and only shrinking with the template's proguard-android.txt (-dontoptimize).
        for what, stats, message in (
            ("no report", {}, "has no BUNDLE-METADATA/com.android.tools/r8.json"),
            (
                "unoptimized",
                {"noObfuscationPercentage": 20, "noOptimizationPercentage": 99, "noShrinkingPercentage": 20},
                "R8's optimization covers 1.00 % of the code, below the 25 %",
            ),
            (
                "just below",
                {"noObfuscationPercentage": 75.01, "noOptimizationPercentage": 20, "noShrinkingPercentage": 20},
                "R8's obfuscation covers 24.99 % of the code",
            ),
        ):
            with self.subTest(what=what):
                apk, aab = self.signed(r8=stats, out=what.replace(" ", "-"))
                result = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(message, result.stderr)
        apk, aab = self.signed(
            r8={"noObfuscationPercentage": 75, "noOptimizationPercentage": 75, "noShrinkingPercentage": 75},
            out="at-the-floor",
        )
        floor = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
        self.assertEqual(floor.returncode, 0, floor.stderr)
        self.assertIn("R8 obfuscation 25.00 %, optimization 25.00 %, shrinking 25.00 %", floor.stdout)

    def test_the_aab_must_carry_r8s_map(self) -> None:
        for what, mapping in (("none", None), ("empty", b"")):
            with self.subTest(what=what):
                apk, aab = self.signed(mapping=mapping, out=f"map-{what}")
                result = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn("the AAB has no R8 map", result.stderr)

    def test_no_activity_may_hold_an_orientation_or_limit_its_size(self) -> None:
        # Play's Android 16 notice (2026-10-28): the app's own activity was portrait (app.json) and
        # ML Kit's scanner activity declared portrait too; on large screens Android 16 ignores both.
        apk, aab = self.signed()
        for attribute, message in (
            ("screenOrientation(0x0101001e)=1", "held to one orientation (screenOrientation 1 )"),
            ("screenOrientation(0x0101001e)=7", "held to one orientation (screenOrientation 7 )"),
            ("screenOrientation(0x0101001e)=12", "held to one orientation (screenOrientation 12 )"),
            ("resizeableActivity(0x010104f6)=false", "is not resizable"),
            ("maxAspectRatio(0x01010530)=2.1", "limits its aspect ratio"),
            ("minAspectRatio(0x01010591)=1.33", "limits its aspect ratio"),
        ):
            with self.subTest(attribute=attribute):
                self.env["FAKE_ACTIVITY"] = attribute
                result = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(message, result.stderr)
        # Following the device is fine: user, behind, sensor, fullSensor, fullUser.
        for value in (2, 3, 4, 10, 13):
            with self.subTest(value=value):
                self.env["FAKE_ACTIVITY"] = f"screenOrientation(0x0101001e)={value}"
                result = self.run_script(CHECK, apk, aab, DIGEST, VERSION)
                self.assertEqual(result.returncode, 0, result.stderr)

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
