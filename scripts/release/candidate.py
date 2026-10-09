#!/usr/bin/env python3
"""Seal and verify a Voltip release candidate (build once, promote the exact bytes).

``release-candidate.yml`` builds every package for one immutable source commit (a release PR head,
or an existing tag in backfill). Each build leg uploads what ``updater-json.py collect`` wrote:

    dist/<asset>                 the files a release ships (installers, bundles, .sig files)
    evidence/<target>.json       name, size, sha256 and kind of each file, plus the updater entry
                                 (none for Android, which the updater does not serve)

The aggregate job downloads every leg into one directory and runs ``seal``: the five legs of
``.github/release-targets.json`` must all be there, every file must match its evidence, and the
result is ``candidate-manifest.json`` binding those bytes to the source (head, tree, version),
the workflow run that built them and the CI gate the source passed. ``release.yml`` promotes a
candidate only after ``verify`` re-checks every identity, target, size and digest against what
the controller resolved on its own; ``--print-files`` then lists the promotable files from the
verified manifest, so promotion never globs the downloaded directory.

Stdlib only; every value arrives as an argument. Tested by scripts/release/test_candidate.py.

    candidate.py seal --candidate-root DIR --targets-file F --repository R --workflow-ref REF
        --workflow-sha SHA --run-id N --run-attempt N --release-pr-number N --mode MODE
        --head-sha SHA --release-pr-head-sha SHA --tree-sha SHA --version V
        --source-gate-sha SHA --source-gate-conclusion C [--subjects-out FILE]
    candidate.py verify --candidate-root DIR --targets-file F --expected-repository R
        --expected-workflow-path P --expected-workflow-ref REF --expected-workflow-sha SHA
        --expected-run-id N --expected-run-attempt N --expected-pr-number N
        --expected-head-sha SHA --expected-pr-head-sha SHA --expected-tree-sha SHA
        --expected-version V --expected-tag T --expected-updater true|false [--print-files]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"
)
# The asset-name rule of updater-json.py (SAFE_ASSET_NAME).
ASSET_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._+-]*$")
REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
MANIFEST_NAME = "candidate-manifest.json"
LAYOUT = {"dist", "evidence"}
MODES = ("automatic", "dry-run", "backfill")
# The only candidates release.yml may promote: dry runs prove the pipeline and are never shipped.
PROMOTABLE_MODES = {"automatic", "backfill"}
SOURCE_GATE_CHECK = "CI Success"
# File kinds updater-json.py collect records.
BUNDLE_KINDS = {"deb", "rpm", "appimage", "nsis", "msi", "dmg", "app", "apk", "aab"}
FILE_KINDS = BUNDLE_KINDS | {"signature", "extra"}
# Written only with bundle.createUpdaterArtifacts (updater-json.py UPDATER_ONLY).
UPDATER_ONLY = {"app"}
EVIDENCE_KEYS = {
    "files",
    "platform",
    "schema_version",
    "target",
    "updater",
    "updater_enabled",
    "updater_platform",
}
FILE_KEYS = {"kind", "name", "sha256", "signature", "size"}


class Failure(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(f"candidate: {message}")


# ------------------------------------------------------------------------------ helpers


def read_json(path: Path) -> object:
    try:
        with path.open(encoding="utf-8") as stream:
            return json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise Failure(f"{path}: {error}") from error


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_sha(label: str, value: object) -> str:
    if not isinstance(value, str) or not SHA.fullmatch(value):
        raise Failure(f"{label} must be a full lowercase Git SHA, got {value!r}")
    return value


def require_positive(label: str, value: int) -> int:
    if value < 1:
        raise Failure(f"{label} must be a positive integer, got {value}")
    return value


def target_specs(path: Path) -> dict[str, dict]:
    """The legs of .github/release-targets.json, by target triple."""
    data = read_json(path)
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        raise Failure(f"{path}: unsupported target manifest")
    targets = data.get("targets")
    if not isinstance(targets, list) or not targets:
        raise Failure(f"{path}: targets must be a non-empty array")
    specs: dict[str, dict] = {}
    for entry in targets:
        if not isinstance(entry, dict):
            raise Failure(f"{path}: every target must be an object")
        target = entry.get("target")
        bundles = entry.get("bundles")
        if not isinstance(target, str) or not ASSET_NAME.fullmatch(target):
            raise Failure(f"{path}: invalid target {target!r}")
        if target in specs:
            raise Failure(f"{path}: duplicate target {target}")
        # A target the updater does not serve (Android) names neither an updater bundle nor an
        # updater platform; every other target names both.
        updater_bundle = entry.get("updater_bundle")
        updater_platform = entry.get("updater_platform")
        served = updater_bundle is not None or updater_platform is not None
        if (
            not isinstance(bundles, list)
            or not bundles
            or any(kind not in BUNDLE_KINDS for kind in bundles)
            or (served and (updater_bundle not in bundles or not isinstance(updater_platform, str)))
            or not isinstance(entry.get("platform"), str)
        ):
            raise Failure(f"{path}: {target}: bundles, updater_bundle or platforms are invalid")
        specs[target] = entry
    return specs


def check_layout(root: Path, files: set[str]) -> None:
    """The candidate directory holds exactly dist/, evidence/ and the given top-level files."""
    if not root.is_dir():
        raise Failure(f"candidate root is not a directory: {root}")
    directories = {path.name for path in root.iterdir() if path.is_dir()}
    top_files = {path.name for path in root.iterdir() if not path.is_dir()}
    if directories != LAYOUT or top_files != files:
        raise Failure(
            f"candidate layout must be {sorted(LAYOUT)} plus {sorted(files)}; found directories "
            f"{sorted(directories)} and files {sorted(top_files)}"
        )


def check_evidence(root: Path, specs: dict[str, dict]) -> tuple[list[dict], list[dict], bool]:
    """Every leg's evidence against the target manifest and the bytes in dist/.

    Returns (evidence sorted by target, flat file list sorted by name, updater_enabled).
    """
    evidence_dir = root / "evidence"
    dist = root / "dist"
    evidence_names = {path.name for path in evidence_dir.iterdir()}
    expected = {f"{target}.json" for target in specs}
    if evidence_names != expected:
        raise Failure(
            f"evidence set differs from the target manifest: missing "
            f"{sorted(expected - evidence_names)}, unexpected {sorted(evidence_names - expected)}"
        )
    legs: list[dict] = []
    flat: list[dict] = []
    owner: dict[str, str] = {}
    updater_states: set[bool] = set()
    for target in sorted(specs):
        spec = specs[target]
        leg = read_json(evidence_dir / f"{target}.json")
        if not isinstance(leg, dict) or set(leg) != EVIDENCE_KEYS or leg["schema_version"] != 1:
            raise Failure(f"{target}: evidence fields are incomplete or unexpected")
        if leg["target"] != target:
            raise Failure(f"{target}: evidence names target {leg['target']!r}")
        if leg["platform"] != spec["platform"] or leg["updater_platform"] != spec["updater_platform"]:
            raise Failure(f"{target}: evidence platforms differ from the target manifest")
        enabled = leg["updater_enabled"]
        if not isinstance(enabled, bool):
            raise Failure(f"{target}: updater_enabled must be a boolean")
        if spec["updater_bundle"] is None:
            if enabled or leg["updater"] is not None:
                raise Failure(f"{target}: the updater serves no such target")
        else:
            updater_states.add(enabled)
        files = leg["files"]
        if not isinstance(files, list) or not files:
            raise Failure(f"{target}: evidence lists no files")
        by_name: dict[str, dict] = {}
        for item in files:
            if not isinstance(item, dict) or set(item) != FILE_KEYS:
                raise Failure(f"{target}: a file entry has incomplete or unexpected fields")
            name = item["name"]
            if not isinstance(name, str) or not ASSET_NAME.fullmatch(name) or name == MANIFEST_NAME:
                raise Failure(f"{target}: unsafe or reserved asset name {name!r}")
            if name in owner:
                raise Failure(f"{name}: produced by both {owner[name]} and {target}")
            if (
                item["kind"] not in FILE_KINDS
                or not isinstance(item["signature"], bool)
                or not isinstance(item["size"], int)
                or item["size"] < 1
                or not isinstance(item["sha256"], str)
                or not SHA256.fullmatch(item["sha256"])
            ):
                raise Failure(f"{target}: {name}: kind, size, digest or signature flag is invalid")
            path = dist / name
            if not path.is_file():
                raise Failure(f"{name}: listed in the evidence of {target} but missing from dist/")
            if path.stat().st_size != item["size"] or sha256(path) != item["sha256"]:
                raise Failure(f"{name}: bytes differ from the evidence written on the {target} leg")
            owner[name] = target
            by_name[name] = item
            flat.append(
                {
                    "kind": item["kind"],
                    "name": name,
                    "sha256": item["sha256"],
                    "size": item["size"],
                    "target": target,
                }
            )
        kinds = {item["kind"] for item in files}
        for kind in spec["bundles"]:
            if kind not in kinds and not (kind in UPDATER_ONLY and not enabled):
                raise Failure(f"{target}: no {kind} bundle in the evidence")
        updater = leg["updater"]
        if enabled:
            if not isinstance(updater, dict) or updater.get("kind") != spec["updater_bundle"]:
                raise Failure(f"{target}: updater enabled but the {spec['updater_bundle']} entry is missing")
            bundle = by_name.get(updater.get("name"))
            signature = by_name.get(updater.get("signature"))
            if (
                bundle is None
                or signature is None
                or not bundle["signature"]
                or signature["kind"] != "signature"
                or updater["signature"] != f"{updater['name']}.sig"
            ):
                raise Failure(f"{target}: the updater bundle or its signature is not in the evidence")
        elif updater is not None:
            raise Failure(f"{target}: an updater entry without the updater")
        legs.append(leg)
    if len(updater_states) != 1:
        raise Failure("the legs the updater serves disagree on whether it is enabled")
    disk = {path.name for path in dist.iterdir()}
    if disk != set(owner):
        raise Failure(
            f"dist/ differs from the evidence: unlisted {sorted(disk - set(owner))}, "
            f"missing {sorted(set(owner) - disk)}"
        )
    return legs, sorted(flat, key=lambda item: item["name"]), updater_states.pop()


def promotable_files(manifest: dict) -> list[str]:
    """Paths relative to the candidate root, in upload order: dist/ files, then the manifest."""
    return [f"dist/{item['name']}" for item in manifest["files"]] + [MANIFEST_NAME]


# ---------------------------------------------------------------------------------- seal


def cmd_seal(args: argparse.Namespace) -> None:
    if not REPOSITORY.fullmatch(args.repository):
        raise Failure("--repository must be owner/name")
    for label, value in (
        ("--workflow-sha", args.workflow_sha),
        ("--head-sha", args.head_sha),
        ("--release-pr-head-sha", args.release_pr_head_sha),
        ("--tree-sha", args.tree_sha),
        ("--source-gate-sha", args.source_gate_sha),
    ):
        require_sha(label, value)
    for label, value in (
        ("--run-id", args.run_id),
        ("--run-attempt", args.run_attempt),
        ("--release-pr-number", args.release_pr_number),
    ):
        require_positive(label, value)
    if not SEMVER.fullmatch(args.version):
        raise Failure(f"--version is not SemVer: {args.version!r}")
    if args.source_gate_conclusion != "success":
        raise Failure(f"the source gate concluded {args.source_gate_conclusion!r}, not success")
    if args.mode == "automatic" and args.head_sha != args.release_pr_head_sha:
        raise Failure("an automatic candidate must build the release PR head")

    root = args.candidate_root.resolve()
    check_layout(root, set())
    legs, files, updater_enabled = check_evidence(root, target_specs(args.targets_file))
    manifest = {
        "files": files,
        "head_sha": args.head_sha,
        "mode": args.mode,
        "release_pr_head_sha": args.release_pr_head_sha,
        "release_pr_number": args.release_pr_number,
        "repository": args.repository,
        "run_attempt": args.run_attempt,
        "run_id": args.run_id,
        "schema_version": 1,
        "source_gate": {
            "check": SOURCE_GATE_CHECK,
            "conclusion": args.source_gate_conclusion,
            "sha": args.source_gate_sha,
        },
        "tag": f"v{args.version}",
        "targets": legs,
        "tree_sha": args.tree_sha,
        "updater_enabled": updater_enabled,
        "version": args.version,
        "workflow_ref": args.workflow_ref,
        "workflow_sha": args.workflow_sha,
    }
    (root / MANIFEST_NAME).write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    if args.subjects_out is not None:
        # sha256sum format, the input of actions/attest `subject-checksums`: the attestation covers
        # exactly the sealed file set, never a glob.
        args.subjects_out.write_text(
            "".join(f"{item['sha256']}  {item['name']}\n" for item in files), encoding="utf-8"
        )
    print(
        json.dumps(
            {"files": len(files), "targets": [leg["target"] for leg in legs], "tag": manifest["tag"]}
        )
    )


# -------------------------------------------------------------------------------- verify


def cmd_verify(args: argparse.Namespace) -> None:
    root = args.candidate_root.resolve()
    check_layout(root, {MANIFEST_NAME})
    manifest = read_json(root / MANIFEST_NAME)
    if not isinstance(manifest, dict):
        raise Failure("the candidate manifest must be an object")

    expected = {
        "head_sha": require_sha("--expected-head-sha", args.expected_head_sha),
        "release_pr_head_sha": require_sha("--expected-pr-head-sha", args.expected_pr_head_sha),
        "release_pr_number": args.expected_pr_number,
        "repository": args.expected_repository,
        "run_attempt": args.expected_run_attempt,
        "run_id": args.expected_run_id,
        "schema_version": 1,
        "tag": args.expected_tag,
        "tree_sha": require_sha("--expected-tree-sha", args.expected_tree_sha),
        "updater_enabled": args.expected_updater == "true",
        "version": args.expected_version,
        "workflow_ref": (
            f"{args.expected_repository}/{args.expected_workflow_path}@{args.expected_workflow_ref}"
        ),
        "workflow_sha": require_sha("--expected-workflow-sha", args.expected_workflow_sha),
    }
    for key, value in expected.items():
        if manifest.get(key) != value:
            raise Failure(f"candidate {key} is {manifest.get(key)!r}, expected {value!r}")
    if args.expected_tag != f"v{args.expected_version}":
        raise Failure(f"--expected-tag {args.expected_tag} is not v{args.expected_version}")
    mode = manifest.get("mode")
    if mode not in PROMOTABLE_MODES:
        raise Failure(f"a {mode!r} candidate is not promotable (automatic or backfill only)")
    if mode == "automatic" and manifest["head_sha"] != manifest["release_pr_head_sha"]:
        raise Failure("an automatic candidate differs from the release PR head")
    gate = manifest.get("source_gate")
    if (
        not isinstance(gate, dict)
        or gate.get("check") != SOURCE_GATE_CHECK
        or gate.get("conclusion") != "success"
        or not isinstance(gate.get("sha"), str)
        or not SHA.fullmatch(gate["sha"])
    ):
        raise Failure(f"the candidate did not record a successful {SOURCE_GATE_CHECK} gate")

    legs, files, updater_enabled = check_evidence(root, target_specs(args.targets_file))
    if manifest.get("targets") != legs:
        raise Failure("the evidence differs from the sealed manifest")
    if manifest.get("files") != files:
        raise Failure("the file list differs from the sealed manifest")
    if updater_enabled != expected["updater_enabled"]:
        raise Failure("the evidence disagrees with the manifest on the updater")

    if args.print_files:
        sys.stdout.write("".join(f"{path}\n" for path in promotable_files(manifest)))
    else:
        print(f"verified {args.expected_tag}: run {args.expected_run_id}, tree {args.expected_tree_sha}")


# ----------------------------------------------------------------------------------- CLI


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    seal = sub.add_parser("seal", help="check every leg and write candidate-manifest.json")
    seal.add_argument("--candidate-root", required=True, type=Path)
    seal.add_argument("--targets-file", required=True, type=Path)
    seal.add_argument("--repository", required=True)
    seal.add_argument("--workflow-ref", required=True)
    seal.add_argument("--workflow-sha", required=True)
    seal.add_argument("--run-id", required=True, type=int)
    seal.add_argument("--run-attempt", required=True, type=int)
    seal.add_argument("--release-pr-number", required=True, type=int)
    seal.add_argument("--mode", required=True, choices=MODES)
    seal.add_argument("--head-sha", required=True)
    seal.add_argument("--release-pr-head-sha", required=True)
    seal.add_argument("--tree-sha", required=True)
    seal.add_argument("--version", required=True)
    seal.add_argument("--source-gate-sha", required=True)
    seal.add_argument("--source-gate-conclusion", required=True)
    seal.add_argument("--subjects-out", type=Path, help="write the dist/ digests in sha256sum format")
    seal.set_defaults(func=cmd_seal)

    verify = sub.add_parser("verify", help="re-check a downloaded candidate before promotion")
    verify.add_argument("--candidate-root", required=True, type=Path)
    verify.add_argument("--targets-file", required=True, type=Path)
    verify.add_argument("--expected-repository", required=True)
    verify.add_argument("--expected-workflow-path", required=True)
    verify.add_argument("--expected-workflow-ref", required=True)
    verify.add_argument("--expected-workflow-sha", required=True)
    verify.add_argument("--expected-run-id", required=True, type=int)
    verify.add_argument("--expected-run-attempt", required=True, type=int)
    verify.add_argument("--expected-pr-number", required=True, type=int)
    verify.add_argument("--expected-head-sha", required=True)
    verify.add_argument("--expected-pr-head-sha", required=True)
    verify.add_argument("--expected-tree-sha", required=True)
    verify.add_argument("--expected-version", required=True)
    verify.add_argument("--expected-tag", required=True)
    verify.add_argument("--expected-updater", required=True, choices=("true", "false"))
    verify.add_argument(
        "--print-files",
        action="store_true",
        help="after verification, print the promotable files (relative to the candidate root)",
    )
    verify.set_defaults(func=cmd_verify)
    return parser


def main(argv: list[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    args.func(args)


if __name__ == "__main__":
    main()
