#!/usr/bin/env python3
"""Check that a macOS release build is signed with the project's fixed certificate.

Release builds are signed with one self-signed code signing certificate ("Voltip Code Signing",
docs/runbook.md, 发布 · macOS 签名), so a Mac sees every update as the same app: the microphone and
Accessibility grants and the keychain access stay. The canonical designated requirement lives in
``.github/release-targets.json`` (``macos_signing.designated_requirement``); the bundle guard
(``.github/scripts/check-macos-bundle.sh --expect-requirement``) runs these checks on the .app and
on every Mach-O inside it. An ad-hoc signature, another certificate or a chain of more than the one
certificate fails.

For a single self-signed certificate the leaf is the root, and codesign may write the requirement
with either word; both spellings pin the same certificate and are treated as one.

Stdlib only; the ``codesign`` output is the input, so the checks are tested on any host by
scripts/release/test_macos_signature.py.

    macos-signature.py app <Voltip.app> --expect-requirement REQ
    macos-signature.py code <Mach-O> --expect-requirement REQ
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

CERTIFICATE = re.compile(r'certificate (?:leaf|root) = H"([0-9A-Fa-f]{40})"')


class Failure(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(f"macos-signature: {message}")


def designated(requirements: str) -> str:
    """The designated requirement in ``codesign -d -r-`` output (what follows ``designated =>``)."""
    for line in requirements.splitlines():
        line = line.strip()
        if line.startswith("designated => "):
            return line[len("designated => ") :].strip()
    raise Failure("no designated requirement: the code is unsigned")


def certificate(requirement: str) -> str:
    """The certificate hash a requirement pins, upper case."""
    found = CERTIFICATE.search(requirement)
    if found is None:
        raise Failure(f"the requirement pins no certificate (signed ad hoc?): {requirement}")
    return found.group(1).upper()


def canonical(requirement: str) -> str:
    """The requirement with its one self-signed certificate written as the leaf, hash upper case."""
    return CERTIFICATE.sub(lambda m: f'certificate leaf = H"{m.group(1).upper()}"', requirement.strip())


def check_app(details: str, requirements: str, expected: str) -> str:
    """The .app: signed with a certificate (not ad hoc), by exactly one self-signed authority, and
    its designated requirement is the canonical one. Returns the requirement."""
    if any(line.strip() == "Signature=adhoc" for line in details.splitlines()):
        raise Failure("Voltip.app is signed ad hoc, not with the release certificate")
    authorities = [line.strip() for line in details.splitlines() if line.strip().startswith("Authority=")]
    if len(authorities) != 1:
        raise Failure(f"expected the one self-signed authority, found {authorities or 'none'}")
    actual = designated(requirements)
    if canonical(actual) != canonical(expected):
        raise Failure(f"designated requirement is `{actual}`, expected `{expected}`")
    return actual


def check_code(requirements: str, expected: str) -> str:
    """An executable or library inside the app: signed by the same certificate. Returns its hash."""
    want = certificate(expected)
    got = certificate(designated(requirements))
    if got != want:
        raise Failure(f"signed by certificate {got}, expected {want}")
    return got


def codesign(*args: str) -> str:
    """codesign's report (it writes most of it to stderr)."""
    done = subprocess.run(["codesign", *args], capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise Failure(f"codesign {' '.join(args)} failed: {done.stderr.strip() or done.stdout.strip()}")
    return done.stdout + done.stderr


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("kind", choices=["app", "code"])
    parser.add_argument("path")
    parser.add_argument("--expect-requirement", required=True)
    args = parser.parse_args(argv)
    requirements = codesign("-d", "-r-", args.path)
    if args.kind == "app":
        requirement = check_app(codesign("-dv", "--verbose=4", args.path), requirements, args.expect_requirement)
        print(f"macos-signature: {args.path}: {requirement}")
    else:
        print(f"macos-signature: {args.path}: certificate {check_code(requirements, args.expect_requirement)}")


if __name__ == "__main__":
    main(sys.argv[1:])
