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

    DESKTOP_LEGS = ("bundle-windows", "bundle-linux", "bundle-macos")
    LEGS = (*DESKTOP_LEGS, "bundle-android")

    def setUp(self) -> None:
        self.path = WORKFLOWS / "release-candidate.yml"
        self.text = self.path.read_text(encoding="utf-8")
        self.jobs = jobs(self.path)

    def test_regression_the_legs_cache_no_target_directory(self) -> None:
        # 2026-10-05: the legs' target/ caches (release-*, about 4 GB) took the repository past
        # GitHub's 10 GB Actions cache limit, which evicts the least recently used entries first:
        # the macOS caches of ci.yml, which no pull request reads. The Intel Mac leg of main's CI
        # then built cold (47 min) and the v0.0.44 candidate's 45-minute source gate gave up on it.
        for leg in self.LEGS:
            body = "\n".join(code_lines(self.jobs[leg]))
            with self.subTest(leg=leg):
                self.assertIn("Swatinem/rust-cache@", body)
                self.assertIn("cache-targets: false", body)

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

    def test_the_linux_leg_ships_the_headless_server_built_before_any_signing_key(self) -> None:
        body = self.jobs["bundle-linux"]
        build = body.index("- name: Build and package voltip-server (tar.gz)")
        signing = body.index("- name: Bundle and sign (deb, AppImage)")
        collect = body.index("- name: Collect bundles and evidence")
        self.assertLess(build, signing, "nothing is compiled after the signing step")
        step = "\n".join(code_lines(body[build:signing]))
        # The source's packaging script, then the trusted copy of the checks on the same binary.
        self.assertIn("scripts/build-server-linux-x64.sh dist/server-linux-x64", step)
        self.assertNotIn(".release-tooling/scripts/build-server-linux-x64.sh", step)
        self.assertIn(". .release-tooling/scripts/lib/artefact-checks.sh", step)
        self.assertIn("voltip_scan_provider_keys target/release/voltip-server release", step)
        self.assertIn("voltip_linux_sonames_accounted target/release/voltip-server release VOLTIP_SERVER_SONAMES", step)
        self.assertNotIn("secrets.", step)
        gather = body[collect:]
        self.assertIn("VERSION: ${{ needs.prepare.outputs.version }}", gather)
        self.assertIn(
            '--extra "voltip-server-${VERSION}-linux-x64.tar.gz=dist/server-linux-x64/voltip-server-${VERSION}-linux-x64.tar.gz"',
            gather,
        )

    def test_the_sherpa_fingerprint_is_cleared_in_the_source_target(self) -> None:
        # forget-sherpa-onnx-build.sh cds to the checkout it sits in and clears that target/. The
        # Android app links no sherpa-onnx.
        for leg in self.DESKTOP_LEGS:
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
        # Regression (candidate 36550741255, 2026-09-29): removing the admin trust setting waited
        # for a dialog and hung the leg. The clean-up is bounded and never fails the leg, and the
        # script no longer touches the trust setting on the way out.
        self.assertIn("timeout-minutes: 2", body[removal:])
        self.assertIn("continue-on-error: true", body[removal:])
        script = (ROOT / ".github/scripts/macos-signing-keychain.sh").read_text(encoding="utf-8")
        self.assertNotIn("remove-trusted-cert", script)
        self.assertIn("MACOS_CERTIFICATE_PRESENT", self.jobs["prepare"])
        # Local and ordinary CI builds stay ad hoc; the hardened runtime stays off (a certificate
        # without a Team ID would make library validation refuse the embedded dylibs).
        config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.macos.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(config["bundle"]["macOS"]["signingIdentity"], "-")
        self.assertFalse(config["bundle"]["macOS"]["hardenedRuntime"])

    def test_every_leg_is_sealed_and_gated(self) -> None:
        for job in ("aggregate", "gate"):
            match = re.search(r"(?m)^    needs: \[([^\]]*)\]$", self.jobs[job])
            self.assertIsNotNone(match, f"{job} lists its needs on one line")
            needs = {name.strip() for name in match.group(1).split(",")}
            with self.subTest(job=job):
                self.assertEqual(sorted(set(self.LEGS) - needs), [])
        legs = {name for name in self.jobs if name.startswith("bundle-")}
        self.assertEqual(legs, set(self.LEGS))

    def test_the_android_leg_signs_outside_the_build_with_the_pinned_certificate(self) -> None:
        # docs/runbook.md 发布 · Android: one key for the GitHub APK and for Google Play. Gradle builds
        # unsigned with no signing material; the key exists in the signing step alone, which runs
        # tooling from the workflow commit; the checks require the certificate release-targets.json
        # pins on both packages.
        body = self.jobs["bundle-android"]
        built = body.index("- name: Build (unsigned APK and AAB)")
        signed = body.index("- name: Sign with the release key")
        checked = body.index("- name: Check the APKs and the AAB")
        collected = body.index("- name: Collect bundles and evidence")
        self.assertLess(built, signed)
        self.assertLess(signed, checked)
        self.assertLess(checked, collected)
        self.assertNotIn("secrets.ANDROID_", body[:signed])
        self.assertNotIn("secrets.ANDROID_", body[checked:])
        self.assertIn(".release-tooling/.github/scripts/sign-android-package.sh", body[signed:checked])
        self.assertIn("rm -f \"$ANDROID_KEYSTORE\"", body[signed:checked])
        self.assertIn(".android_signing.certificate_sha256 .release-tooling/.github/release-targets.json", body[checked:collected])
        self.assertIn(".release-tooling/.github/scripts/check-android-package.sh", body[checked:collected])
        self.assertIn("/.github/release-targets.json", body)
        self.assertIn("--updater-bundle none", body[collected:])
        self.assertIn("--updater false", body[collected:])
        self.assertIn("tauri.package-android.conf.json", body[built:signed])
        self.assertNotIn("VOLTIP_ANDROID_", self.text)
        # Every Android secret is required before a leg starts, and read nowhere but there and in
        # the signing step.
        prepare = self.jobs["prepare"]
        for name in ("ANDROID_KEYSTORE_BASE64", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_PASSWORD"):
            with self.subTest(secret=name):
                self.assertIn(f"{name}_PRESENT: ${{{{ secrets.{name} != '' }}}}", prepare)
                self.assertEqual(self.text.count(f"secrets.{name} "), 2)
        targets = json.loads((ROOT / ".github/release-targets.json").read_text(encoding="utf-8"))
        self.assertRegex(targets["android_signing"]["certificate_sha256"], r"^[0-9a-f]{64}$")

    def test_the_android_leg_ships_the_react_native_app_signed_like_the_tauri_one(self) -> None:
        # docs/mobile-rn.md §6: the React Native phone app comes from the source's build script,
        # unsigned and before the key exists, then the signing step signs it with the same key and
        # the trusted checks hold it to the pinned certificate. It ships as an extra file of the
        # Android leg, outside android-bundle/, so the leg keeps one apk bundle.
        body = self.jobs["bundle-android"]
        built = body.index("- name: Build the React Native app (unsigned APK)")
        signed = body.index("- name: Sign with the release key")
        checked = body.index("- name: Check the APKs and the AAB")
        collected = body.index("- name: Collect bundles and evidence")
        self.assertLess(body.index("- name: Build (unsigned APK and AAB)"), built)
        self.assertLess(built, signed, "nothing is compiled after the signing step")
        step = "\n".join(code_lines(body[built:signed]))
        self.assertIn("scripts/build-android-rn.sh --unsigned", step)
        self.assertNotIn(".release-tooling/scripts/build-android-rn.sh", step)
        self.assertNotIn("secrets.", step)
        self.assertIn(".release-tooling/.github/scripts/install-cargo-ndk.sh", body[:built])
        self.assertIn(
            '.release-tooling/.github/scripts/sign-android-package.sh --app mobile-rn \\\n'
            '            "dist/android-rn/Voltip-RN_${VERSION}_android_arm64-unsigned.apk" android-rn-bundle "$VERSION"',
            body[signed:checked],
        )
        self.assertIn(
            '.release-tooling/.github/scripts/check-android-package.sh --app mobile-rn \\\n'
            '            "android-rn-bundle/apk/Voltip-RN_${VERSION}_android_arm64.apk" "$certificate" "$VERSION"',
            body[checked:collected],
        )
        gather = body[collected:]
        self.assertIn("VERSION: ${{ needs.prepare.outputs.version }}", gather)
        self.assertIn(
            '--extra "Voltip-RN_${VERSION}_android_arm64.apk=android-rn-bundle/apk/Voltip-RN_${VERSION}_android_arm64.apk"',
            gather,
        )
        self.assertIn("--bundle-dir android-bundle", gather)
        # The app takes its version from the repository, which release-please writes.
        config = (ROOT / "apps/mobile-rn/app.config.js").read_text(encoding="utf-8")
        self.assertIn('require("../../package.json")', config)
        app = json.loads((ROOT / "apps/mobile-rn/app.json").read_text(encoding="utf-8"))["expo"]
        self.assertNotIn("version", app)
        self.assertNotIn("versionCode", app["android"])

    def test_regression_the_release_apk_starts_on_a_device_before_it_is_sealed(self) -> None:
        # 2026-10-01: 0.0.18 and 0.0.19 closed at once on the user's phone although every package
        # check passed; nothing had started the APK. The signed leg runs on an emulator in a job
        # with no secrets, and both the seal and the gate wait for it.
        body = self.jobs["device-android"]
        self.assertIn("needs: [prepare, bundle-android]", body)
        self.assertIn("name: candidate-aarch64-linux-android", body)
        self.assertIn(".release-tooling/.github/scripts/android-device-smoke.sh android-leg/dist device\n", body)
        self.assertIn(".release-tooling/.github/scripts/android-device-smoke.sh --app mobile-rn android-leg/dist device-rn", body)
        self.assertNotIn("secrets.", body)
        self.assertNotRegex(body, r"name: candidate-(?!aarch64-linux-android)")
        for job in ("aggregate", "gate"):
            with self.subTest(job=job):
                self.assertRegex(self.jobs[job], r"(?m)^    needs: \[[^\]]*\bdevice-android\b")

    def test_every_leg_bakes_in_the_feedback_endpoint(self) -> None:
        # The phone sends feedback of its own too (user decision 2026-10-01): a leg without the
        # endpoint would ship a 反馈 page that can only point to GitHub.
        for leg in self.LEGS:
            with self.subTest(leg=leg):
                body = self.jobs[leg]
                self.assertIn("VOLTIP_FEEDBACK_URL: ${{ secrets.VOLTIP_FEEDBACK_URL }}", body)
                self.assertIn("VOLTIP_FEEDBACK_TOKEN: ${{ secrets.VOLTIP_FEEDBACK_TOKEN }}", body)

    def test_regression_the_key_alias_is_no_secret(self) -> None:
        # Candidate 36844362994 (0.0.18, 2026-10-01): the alias was the secret ANDROID_KEY_ALIAS,
        # whose value is the project's name, so GitHub masked "voltip" in every log line of
        # `prepare` and the Android leg ("sunerpy/***", "dev.***.mobile"). The alias names the key
        # inside the keystore and reveals nothing; release-targets.json holds it.
        self.assertNotIn("ANDROID_KEY_ALIAS", "\n".join(code_lines(self.jobs["prepare"])))
        self.assertNotIn("secrets.ANDROID_KEY_ALIAS", self.text)
        body = self.jobs["bundle-android"]
        signed = body.index("- name: Sign with the release key")
        checked = body.index("- name: Check the APKs and the AAB")
        self.assertIn(".android_signing.key_alias .release-tooling/.github/release-targets.json", body[signed:checked])
        targets = json.loads((ROOT / ".github/release-targets.json").read_text(encoding="utf-8"))
        self.assertEqual(targets["android_signing"]["key_alias"], "voltip")

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

    def test_regression_ci_success_needs_every_job_but_the_advisory_ones(self) -> None:
        # v0.0.5 (2026-09-28): the candidate's gate commit 06334e6 had `macos-x64` red in main's CI
        # run 36461609860 while `CI Success` was green, because `macos` was not one of its needs, and
        # the source gate reads only `CI Success`. A job outside the aggregate is invisible to the
        # release. The only exceptions: `candidate-status` writes the ruleset's status and must not
        # wait for CI, and `codecov` is an upload to an outside service the file header leaves out.
        match = re.search(r"(?m)^    needs: \[([^\]]*)\]$", self.jobs["ci-success"])
        self.assertIsNotNone(match, "ci-success lists its needs on one line")
        needs = {name.strip() for name in match.group(1).split(",")}
        advisory = {"ci-success", "candidate-status", "codecov"}
        self.assertEqual(sorted(set(self.jobs) - advisory - needs), [], "jobs CI Success does not wait for")
        self.assertEqual(sorted(needs - set(self.jobs)), [], "needs that name no job")

    def test_the_android_job_signs_and_checks_as_the_candidate_does_with_a_key_of_its_own(self) -> None:
        body = self.jobs["android"]
        self.assertIn("if: needs.changes.outputs.code == 'true'", body)
        self.assertNotIn("secrets.", body)
        built = body.index("- name: Build the unsigned release APK and AAB (aarch64)")
        signed = body.index("- name: Sign with a key made for this run")
        checked = body.index("- name: Check the packages")
        self.assertLess(built, signed)
        self.assertLess(signed, checked)
        self.assertIn("tauri.package-android.conf.json", body[built:signed])
        self.assertIn(".github/scripts/sign-android-package.sh", body[signed:checked])
        self.assertIn('.github/scripts/check-android-package.sh', body[checked:])
        self.assertIn('"$CI_ANDROID_CERT_SHA256"', body[checked:])
        self.assertIn("make android-clippy", body)
        self.assertIn("third-party-notices.py --app mobile", body)

    def test_regression_ci_starts_the_android_app_on_a_device(self) -> None:
        # 2026-10-01: the packages passed every check and still closed on start on a phone.
        body = self.jobs["android-device"]
        self.assertIn("needs: [android]", body)
        self.assertIn(".github/scripts/android-device-smoke.sh android-apk device", body)
        self.assertIn("name: voltip-android-apk", self.jobs["android"])

    def test_the_react_native_app_is_built_signed_checked_and_started_as_the_candidate_does(self) -> None:
        body = self.jobs["android-rn"]
        self.assertIn("if: needs.changes.outputs.code == 'true'", body)
        self.assertNotIn("secrets.", body)
        built = body.index("- name: Build the unsigned release APK (scripts/build-android-rn.sh --unsigned)")
        signed = body.index("- name: Sign with a key made for this run")
        checked = body.index("- name: Check the package")
        self.assertLess(built, signed)
        self.assertLess(signed, checked)
        self.assertIn("scripts/build-android-rn.sh --unsigned", body[built:signed])
        self.assertIn('VOLTIP_ALLOW_NO_BUILTIN_ENGINES: "1"', body[built:signed])
        self.assertIn(".github/scripts/sign-android-package.sh --app mobile-rn", body[signed:checked])
        self.assertIn(".github/scripts/check-android-package.sh --app mobile-rn", body[checked:])
        self.assertIn('"$CI_ANDROID_CERT_SHA256"', body[checked:])
        self.assertIn(".github/scripts/install-cargo-ndk.sh", body[:built])
        # Read only: a target/ cache of its own would push the repository past the cache limit.
        cache = "\n".join(code_lines(body))
        self.assertIn("shared-key: ci-android", cache)
        self.assertIn("save-if: false", cache)
        device = self.jobs["android-rn-device"]
        self.assertIn("needs: [android-rn]", device)
        self.assertIn(".github/scripts/android-device-smoke.sh --app mobile-rn android-rn-apk device-rn", device)
        self.assertIn("name: voltip-android-rn-apk", body)
        self.assertIn("name: voltip-android-rn-apk", device)

    def test_regression_the_react_native_build_compiles_nothing_for_the_build_host(self) -> None:
        # PR #123's first CI run: the UniFFI bindings came from a debug build of the shell for the
        # build host, and on the Android runner, which has no ALSA headers, alsa-sys stopped it.
        # The bindings now come from the Android library itself, which keeps its symbols out of
        # cargo, and the script strips the packaged copy.
        script = (ROOT / "scripts/build-android-rn.sh").read_text(encoding="utf-8")
        code = "\n".join(line for line in script.splitlines() if not line.lstrip().startswith("#"))
        self.assertNotRegex(code, r"cargo build[^\n]*-p voltip-mobile-rn")
        self.assertIn('generate --library "$lib" --language kotlin', code)
        self.assertLess(code.index('generate --library "$lib"'), code.index('llvm-strip" --strip-all "$lib"'))
        self.assertLess(code.index('llvm-strip" --strip-all "$lib"'), code.index('voltip_scan_provider_keys "$lib"'))
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn('[profile.release.package.voltip-mobile-rn]\nstrip = false', cargo)

    def test_regression_the_device_smoke_starts_the_app_it_was_asked_for(self) -> None:
        # The candidate's Android leg holds Voltip_*.apk and Voltip-RN_*.apk, and `Voltip-RN_` sorts
        # first: the smoke took the first *.apk of the directory, so it would have installed the
        # React Native app and then failed to start dev.voltip.mobile. It now looks for the app's
        # own release name, and refuses a directory with more than one.
        script = (ROOT / ".github/scripts/android-device-smoke.sh").read_text(encoding="utf-8")
        self.assertNotIn("-name '*.apk'", script)
        self.assertIn("mobile) package=dev.voltip.mobile name='Voltip_*.apk' ;;", script)
        self.assertIn("mobile-rn) package=dev.voltip.mobile.rn name='Voltip-RN_*.apk' ;;", script)
        self.assertIn('found=$(find "$apk" -name "$name" | sort)', script)

    def test_ci_success_decides_with_the_tested_script_and_the_event(self) -> None:
        # `.github/scripts/ci-success.sh` holds the rule (scripts/release/test_ci_success.py): the
        # `macos` legs may be skipped on a pull request only, since they never run on one.
        body = "\n".join(code_lines(self.jobs["ci-success"]))
        self.assertIn("if: always()", body)
        self.assertIn(".github/scripts/ci-success.sh", body)
        self.assertIn("NEEDS_JSON: ${{ toJSON(needs) }}", body)
        self.assertIn("EVENT: ${{ github.event_name }}", body)
        self.assertIn("github.event_name == 'push' || github.event_name == 'workflow_dispatch'", self.jobs["macos"])

    def test_regression_the_keychain_harness_builds_outside_the_timed_check(self) -> None:
        # 2026-10-01 (main CI 36931331563): a cold release build of the harness took 14 min 32 s of
        # the check's 15 minutes on the Intel Mac, and the release candidate waited on the rerun.
        body = self.jobs["macos"]
        steps = re.split(r"\n      - name: ", body)
        build = next(s for s in steps if s.startswith("Build the keychain pre-install harness"))
        check = next(s for s in steps if s.startswith("Keychain pre-install hand-over"))
        self.assertIn("--bin keychain_preinstall", build)
        self.assertNotIn("cargo build", check)
        self.assertLess(body.index("Build the keychain pre-install harness"), body.index("Keychain pre-install hand-over"))

    def test_regression_the_smoke_models_come_from_the_actions_cache(self) -> None:
        # 2026-10-06 (main CI 37401697547): huggingface.co and hf-mirror.com were both unreachable
        # for a minute and the Intel Mac's headless run failed on the model download. Every job that
        # runs the headless smoke seeds the library from one cross-OS cache entry that main writes.
        key = "key: smoke-models-${{ hashFiles('crates/voltip-asr-local/src/catalogue.rs') }}"
        for name in ("windows-native", "smoke-desktop", "macos"):
            code = "\n".join(code_lines(self.jobs[name]))
            with self.subTest(job=name):
                runs = re.findall(r"smoke-native-cli\.ps1[^\n]*", code)
                self.assertTrue(runs)
                for run in runs:
                    self.assertIn("-ModelCache smoke-models", run)
                self.assertLess(code.index("actions/cache/restore@"), code.index("smoke-native-cli.ps1"))
                self.assertLess(code.rindex("smoke-native-cli.ps1"), code.index("actions/cache/save@"))
                self.assertIn("if: github.ref == 'refs/heads/main' && steps.smoke-models.outputs.cache-hit != 'true'", code)
                self.assertEqual(code.count(key), 2)
                self.assertEqual(code.count("enableCrossOsArchive: true"), 2)

    def test_regression_the_keychain_harness_builds_on_the_app_builds_crates(self) -> None:
        # 2026-10-05 (main CI 37293351063, Intel Mac): as an example the harness brought the
        # dev-dependencies, whose features (tauri's `test`, wiremock's hyper) recompiled some sixty
        # crates of the app build, 11 min 12 s of a 47-minute cold run that ran past the release
        # candidate's 45-minute source gate. A bin behind its own feature, with the feature the
        # Tauri CLI gives tauri, builds on the app's crates.
        steps = re.split(r"\n      - name: ", self.jobs["macos"])
        build = "\n".join(code_lines(next(s for s in steps if s.startswith("Build the keychain pre-install harness"))))
        self.assertIn("--bin keychain_preinstall", build)
        self.assertIn("--features keychain-harness,tauri/custom-protocol", build)
        self.assertNotIn("--example", build)
        check = next(s for s in steps if s.startswith("Keychain pre-install hand-over"))
        self.assertIn('"target/$MACOS_TARGET/release/keychain_preinstall"', check)

    def test_regression_every_macos_build_shares_the_apps_deployment_target(self) -> None:
        # 2026-10-02 (main CI 36982316309, Intel Mac): `cargo tauri build` exports the app's
        # minimumSystemVersion as MACOSX_DEPLOYMENT_TARGET and the harness build had none, so the C
        # crates' build scripts ran again and each build recompiled the tree (9 min 14 s and
        # 8 min 15 s with a warm cache); the cache kept whichever build ran last.
        body = self.jobs["macos"]
        steps = re.split(r"\n      - name: ", body)
        export = next(s for s in steps if "MACOSX_DEPLOYMENT_TARGET" in s and "GITHUB_ENV" in s)
        self.assertIn("apps/desktop/src-tauri/tauri.macos.conf.json", export)
        self.assertIn(".bundle.macOS.minimumSystemVersion", export)
        first_build = next(i for i, s in enumerate(steps) if re.search(r"\bcargo (test|build|tauri build)\b", "\n".join(code_lines(s))))
        self.assertLess(steps.index(export), first_build)

    def test_macos_exercises_the_preinstall_keychain_boundary(self) -> None:
        body = self.jobs["macos"]
        self.assertIn("--bin keychain_preinstall", body)
        self.assertIn(".github/scripts/check-keychain-preinstall.sh", body)
        self.assertNotIn("check-keychain-handoff.sh", body)
        script = (ROOT / ".github/scripts/check-keychain-preinstall.sh").read_text(encoding="utf-8")
        self.assertIn("USER=\"$user\" \"$work/harness\" \"$work/B.app.tar.gz\"", script)
        self.assertIn('make_tar "$work/good/Voltip.app" "$work/B.app.tar.gz"', script)
        self.assertIn('make_tar "$work/bad/Voltip.app" "$work/bad.app.tar.gz"', script)
        self.assertIn("wrong-signature staged app was stopped", script)
        self.assertIn("$user.signed.$b", script)
        self.assertIn("$bad_user.signed.$old", script)


class AptInstalls(unittest.TestCase):
    """2026-10-01: a slow apt mirror held the desktop smoke in its package step for 48 and 35
    minutes, twice in one day, and stalled the release candidates waiting for CI Success."""

    def test_regression_ci_and_the_candidate_install_packages_through_the_mirror_safe_script(self) -> None:
        for name, script in (
            ("ci.yml", ".github/scripts/apt-install.sh"),
            ("release-candidate.yml", ".release-tooling/.github/scripts/apt-install.sh"),
        ):
            with self.subTest(workflow=name):
                code = "\n".join(code_lines((WORKFLOWS / name).read_text(encoding="utf-8")))
                self.assertNotRegex(code, r"apt-get (update|install)")
                self.assertIn(script, code)
        body = (ROOT / ".github/scripts/apt-install.sh").read_text(encoding="utf-8")
        # Each download has a deadline and a later attempt leaves the Azure mirror; the install
        # from the downloaded files is never cut short.
        self.assertIn('sudo timeout "$deadline" apt-get "${opts[@]}" update', body)
        self.assertIn("--download-only", body)
        self.assertIn("s#azure\\.archive\\.ubuntu\\.com#archive.ubuntu.com#g", body)
        self.assertIn('sudo apt-get "${opts[@]}" install --yes --no-install-recommends "$@"', body)


if __name__ == "__main__":
    unittest.main()
