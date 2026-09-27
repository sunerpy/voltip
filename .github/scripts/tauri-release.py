#!/usr/bin/env python3
"""Tauri v2 release helpers: target matrix, config checks, bundle collection,
updater signature verification, and the single-writer latest.json.

Stdlib only. The only external program invoked is `minisign` (verify-signatures).
Copy to `.github/scripts/tauri-release.py`; the release workflow calls one
subcommand per step so every input crosses the shell boundary as data.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import quote

SAFE_ATOM = re.compile(r"^[A-Za-z0-9_.+-]+$")
SAFE_ASSET_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._+-]*$")
SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"
)
REPO_SLUG = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")

MANIFEST_FIELDS = {
    "bundles",
    "cross",
    "platform",
    "runner",
    "target",
    "updater_bundle",
    "updater_platform",
}
PLATFORMS = {"linux", "macos", "windows"}
BUNDLES_BY_PLATFORM = {
    "linux": {"deb", "rpm", "appimage"},
    "macos": {"app", "dmg"},
    "windows": {"nsis", "msi"},
}
UPDATER_BUNDLES = {"appimage", "app", "nsis", "msi"}
RUNNER_PREFIX = {"linux": "ubuntu-", "macos": "macos-", "windows": "windows-"}
# Bundle kind -> directory under target/<triple>/release/bundle/ -> file suffixes.
# https://v2.tauri.app/plugin/updater/ lists the updater artifact per platform.
KIND_DIRS = {
    "deb": "deb",
    "rpm": "rpm",
    "appimage": "appimage",
    "app": "macos",
    "dmg": "dmg",
    "nsis": "nsis",
    "msi": "msi",
}
KIND_SUFFIXES = {
    "deb": (".deb",),
    "rpm": (".rpm",),
    "appimage": (".AppImage", ".AppImage.tar.gz"),
    "app": (".app.tar.gz",),
    "dmg": (".dmg",),
    "nsis": ("-setup.exe",),
    "msi": (".msi",),
}
# Arch token Tauri itself uses in dmg/nsis/msi file names, reused when a bundle
# name would otherwise collide across targets (macOS `.app.tar.gz`).
ARCH_FILE_TOKEN = {"x86_64": "x64", "aarch64": "aarch64", "i686": "x86", "armv7": "armv7"}


class Failure(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(f"tauri-release: {message}")


def read_json(path: Path) -> object:
    try:
        with path.open(encoding="utf-8") as stream:
            return json.load(stream)
    except FileNotFoundError as error:
        raise Failure(f"{path}: not found") from error
    except json.JSONDecodeError as error:
        raise Failure(f"{path}: invalid JSON ({error})") from error


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_semver(version: str, label: str) -> str:
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        raise Failure(f"{label} is not a SemVer version: {version!r}")
    return version


def updater_platform_for(target: str) -> str:
    """OS-ARCH key exactly as tauri-plugin-updater computes it (updater.rs
    updater_os/updater_arch: linux|darwin|windows and x86_64|aarch64|i686|armv7)."""
    arch = target.split("-", 1)[0]
    if arch not in ARCH_FILE_TOKEN:
        raise Failure(f"{target}: unsupported architecture {arch!r}")
    if "-linux-" in target:
        os_name = "linux"
    elif target.endswith("-apple-darwin"):
        os_name = "darwin"
    elif "-windows-" in target:
        os_name = "windows"
    else:
        raise Failure(f"{target}: unsupported operating system")
    return f"{os_name}-{arch}"


def platform_for(target: str) -> str:
    os_name = updater_platform_for(target).split("-", 1)[0]
    return {"linux": "linux", "darwin": "macos", "windows": "windows"}[os_name]


# --------------------------------------------------------------------------- matrix


def load_targets(path: Path) -> list[dict]:
    data = read_json(path)
    if not isinstance(data, dict):
        raise Failure(f"{path}: manifest must be an object")
    allowed = {"schema_version", "profile", "targets"}
    if set(data) - allowed or "schema_version" not in data or "targets" not in data:
        raise Failure(f"{path}: manifest fields are incomplete or unexpected")
    if data["schema_version"] != 1:
        raise Failure(f"{path}: unsupported schema_version {data['schema_version']!r}")
    if data.get("profile", "tauri") != "tauri":
        raise Failure(f"{path}: profile must be 'tauri'")
    targets = data["targets"]
    if not isinstance(targets, list) or not targets:
        raise Failure(f"{path}: targets must be a non-empty array")

    seen_targets: set[str] = set()
    seen_updater_platforms: set[str] = set()
    for index, entry in enumerate(targets):
        if not isinstance(entry, dict) or set(entry) != MANIFEST_FIELDS:
            raise Failure(f"target {index}: fields are incomplete or unexpected")
        target = entry["target"]
        runner = entry["runner"]
        platform = entry["platform"]
        bundles = entry["bundles"]
        updater_bundle = entry["updater_bundle"]
        updater_platform = entry["updater_platform"]
        if not all(
            isinstance(value, str) and SAFE_ATOM.fullmatch(value)
            for value in (target, runner, updater_platform)
        ):
            raise Failure(f"target {index}: target, runner, or updater_platform is unsafe")
        if target in seen_targets:
            raise Failure(f"duplicate target: {target}")
        seen_targets.add(target)
        if platform not in PLATFORMS or platform != platform_for(target):
            raise Failure(f"{target}: platform {platform!r} does not match the triple")
        if not runner.startswith(RUNNER_PREFIX[platform]):
            raise Failure(f"{target}: {platform} target requires a {RUNNER_PREFIX[platform]}* runner")
        if updater_platform != updater_platform_for(target):
            raise Failure(
                f"{target}: updater_platform must be {updater_platform_for(target)!r}"
            )
        if updater_platform in seen_updater_platforms:
            raise Failure(f"{target}: more than one target serves {updater_platform}")
        seen_updater_platforms.add(updater_platform)
        if (
            not isinstance(bundles, list)
            or not bundles
            or len(set(bundles)) != len(bundles)
            or not set(bundles) <= BUNDLES_BY_PLATFORM[platform]
        ):
            raise Failure(
                f"{target}: bundles must be a unique subset of "
                f"{sorted(BUNDLES_BY_PLATFORM[platform])}"
            )
        if updater_bundle not in UPDATER_BUNDLES or updater_bundle not in bundles:
            raise Failure(
                f"{target}: updater_bundle must be one of {sorted(UPDATER_BUNDLES)} "
                "and listed in bundles"
            )
        if not isinstance(entry["cross"], bool):
            raise Failure(f"{target}: cross must be boolean")
    return targets


def target_entry(targets: list[dict], target: str) -> dict:
    for entry in targets:
        if entry["target"] == target:
            return entry
    raise Failure(f"{target}: not present in the target manifest")


def cmd_matrix(args: argparse.Namespace) -> None:
    targets = load_targets(args.file)
    include = []
    for entry in targets:
        row = dict(entry)
        row["bundles"] = ",".join(entry["bundles"])
        include.append(row)
    print(json.dumps({"include": include}, separators=(",", ":"), sort_keys=True))


# --------------------------------------------------------------------------- config


def decode_minisign_pubkey(pubkey: str) -> str:
    """Decode the base64 pubkey exactly as tauri-plugin-updater does
    (base64 -> UTF-8 minisign public key text) and sanity-check its shape."""
    try:
        text = base64.b64decode(pubkey, validate=True).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError, ValueError) as error:
        raise Failure(f"plugins.updater.pubkey is not base64 minisign text ({error})") from error
    lines = [line for line in text.splitlines() if line.strip()]
    if len(lines) != 2 or not lines[0].startswith("untrusted comment:"):
        raise Failure("plugins.updater.pubkey must decode to a two-line minisign public key")
    try:
        raw = base64.b64decode(lines[1], validate=True)
    except (binascii.Error, ValueError) as error:
        raise Failure("plugins.updater.pubkey key line is not base64") from error
    if len(raw) != 42 or raw[:2] != b"Ed":
        raise Failure("plugins.updater.pubkey is not an Ed25519 minisign public key")
    return text


def load_tauri_config(project: Path) -> dict:
    conf = project / "tauri.conf.json"
    if not conf.exists():
        for alt in ("tauri.conf.json5", "Tauri.toml"):
            if (project / alt).exists():
                raise Failure(f"{project / alt}: only tauri.conf.json (JSON) is supported")
    data = read_json(conf)
    if not isinstance(data, dict):
        raise Failure(f"{conf}: must be an object")
    return data


def resolve_version(project: Path, config: dict, package_json: Path | None) -> tuple[str, str]:
    """Return (version, source) where source is 'package.json' or 'literal'."""
    declared = config.get("version")
    if declared is None:
        raise Failure(
            'tauri.conf.json has no "version"; set "version": "../package.json" so '
            "package.json is the single version source"
        )
    if not isinstance(declared, str):
        raise Failure('tauri.conf.json "version" must be a string')
    if declared.endswith("package.json"):
        target = (project / declared).resolve()
        if package_json is not None and package_json.resolve() != target:
            raise Failure(
                f'tauri.conf.json "version" points at {target}, not {package_json.resolve()}'
            )
        version = read_package_version(target)
        return version, "package.json"
    version = require_semver(declared, 'tauri.conf.json "version"')
    if package_json is not None:
        package_version = read_package_version(package_json)
        if package_version != version:
            raise Failure(
                f"tauri.conf.json version {version} differs from package.json version "
                f"{package_version}; point tauri.conf.json at ../package.json or align them"
            )
    return version, "literal"


def read_package_version(path: Path) -> str:
    data = read_json(path)
    if not isinstance(data, dict):
        raise Failure(f"{path}: must be an object")
    return require_semver(data.get("version"), f"{path} version")


def updater_enabled(config: dict) -> bool:
    bundle = config.get("bundle") or {}
    if not isinstance(bundle, dict):
        raise Failure('tauri.conf.json "bundle" must be an object')
    flag = bundle.get("createUpdaterArtifacts", False)
    if flag is True or flag == "v1Compatible":
        return True
    if flag is False:
        return False
    raise Failure(f"bundle.createUpdaterArtifacts has unsupported value {flag!r}")


def updater_pubkey(config: dict) -> str:
    plugins = config.get("plugins") or {}
    updater = plugins.get("updater") if isinstance(plugins, dict) else None
    if not isinstance(updater, dict):
        raise Failure("bundle.createUpdaterArtifacts is set but plugins.updater is missing")
    pubkey = updater.get("pubkey")
    if not isinstance(pubkey, str) or not pubkey.strip():
        raise Failure("plugins.updater.pubkey must be a non-empty base64 string")
    decode_minisign_pubkey(pubkey.strip())
    endpoints = updater.get("endpoints")
    if not isinstance(endpoints, list) or not endpoints:
        raise Failure("plugins.updater.endpoints must be a non-empty array")
    for endpoint in endpoints:
        if not isinstance(endpoint, str) or not endpoint.startswith("https://"):
            raise Failure(f"updater endpoint must be https: {endpoint!r}")
    return pubkey.strip()


def check_bundle_targets(config: dict, targets: list[dict]) -> None:
    bundle = config.get("bundle") or {}
    if bundle.get("active") is False:
        raise Failure("bundle.active is false; the release profile needs bundles")
    declared = bundle.get("targets", "all")
    if declared == "all":
        return
    if isinstance(declared, str):
        declared = [declared]
    if not isinstance(declared, list):
        raise Failure("bundle.targets must be 'all', a string, or an array")
    declared_set = {str(item).lower() for item in declared}
    for entry in targets:
        missing = sorted(set(entry["bundles"]) - declared_set)
        if missing:
            raise Failure(
                f"{entry['target']}: bundle.targets lacks {missing} required by the manifest"
            )


def cmd_check_config(args: argparse.Namespace) -> None:
    project = args.project
    config = load_tauri_config(project)
    version, source = resolve_version(project, config, args.package_json)
    if args.expect_version is not None:
        expected = require_semver(args.expect_version, "--expect-version")
        if version != expected:
            raise Failure(f"configured version {version} != release version {expected}")
    enabled = updater_enabled(config)
    pubkey = updater_pubkey(config) if enabled else ""
    targets = load_targets(args.targets_file) if args.targets_file else []
    if targets:
        check_bundle_targets(config, targets)
    product_name = config.get("productName")
    if not isinstance(product_name, str) or not product_name.strip():
        raise Failure('tauri.conf.json "productName" must be a non-empty string')
    summary = {
        "product_name": product_name,
        "pubkey": pubkey,
        "updater_enabled": enabled,
        "version": version,
        "version_source": source,
    }
    if args.github_output is not None:
        with args.github_output.open("a", encoding="utf-8") as stream:
            for key in ("version", "updater_enabled", "pubkey", "product_name"):
                value = summary[key]
                if isinstance(value, bool):
                    value = "true" if value else "false"
                if "\n" in value:
                    raise Failure(f"{key} contains a newline")
                stream.write(f"{key}={value}\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


# --------------------------------------------------------------------------- collect


def safe_asset_name(name: str) -> str:
    """GitHub replaces whitespace in asset names; do it deterministically here so
    evidence, latest.json and the remote asset list agree."""
    candidate = re.sub(r"\s+", ".", name.strip())
    if not SAFE_ASSET_NAME.fullmatch(candidate):
        raise Failure(f"bundle file name is not a safe release asset name: {name!r}")
    return candidate


def bundle_root(project: Path, target: str, target_dir: Path | None) -> Path:
    candidates = []
    if target_dir is not None:
        candidates.append(target_dir / target / "release" / "bundle")
    else:
        candidates.append(project / "target" / target / "release" / "bundle")
        candidates.append(Path("target") / target / "release" / "bundle")
    for candidate in candidates:
        if candidate.is_dir():
            return candidate
    raise Failure(
        "bundle directory not found; looked in "
        + ", ".join(str(path) for path in candidates)
        + " (pass --target-dir when CARGO_TARGET_DIR is customised)"
    )


def find_bundle_files(root: Path, kind: str) -> list[Path]:
    directory = root / KIND_DIRS[kind]
    if not directory.is_dir():
        return []
    files = []
    for path in sorted(directory.iterdir()):
        if not path.is_file() or path.name.endswith(".sig"):
            continue
        if any(path.name.endswith(suffix) for suffix in KIND_SUFFIXES[kind]):
            files.append(path)
    return files


def collected_name(source: Path, kind: str, target: str) -> str:
    name = source.name
    if kind == "app" and name.endswith(".app.tar.gz"):
        arch = ARCH_FILE_TOKEN[target.split("-", 1)[0]]
        stem = name[: -len(".app.tar.gz")]
        if not stem.endswith(f"_{arch}"):
            name = f"{stem}_{arch}.app.tar.gz"
    return safe_asset_name(name)


def cmd_collect(args: argparse.Namespace) -> None:
    targets = load_targets(args.targets_file)
    entry = target_entry(targets, args.target)
    config = load_tauri_config(args.project)
    if args.updater == "auto":
        enabled = updater_enabled(config)
    else:
        enabled = args.updater == "true"
    root = bundle_root(args.project, args.target, args.target_dir)
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    args.evidence.parent.mkdir(parents=True, exist_ok=True)

    files: list[dict] = []
    updater: dict | None = None
    names: set[str] = set()
    for kind in entry["bundles"]:
        found = find_bundle_files(root, kind)
        if not found:
            raise Failure(f"{args.target}: no {kind} bundle under {root / KIND_DIRS[kind]}")
        signed: list[dict] = []
        for source in found:
            name = collected_name(source, kind, args.target)
            if name in names:
                raise Failure(f"{args.target}: duplicate asset name {name}")
            names.add(name)
            shutil.copy2(source, out / name)
            record = {
                "kind": kind,
                "name": name,
                "sha256": sha256(out / name),
                "signature": False,
                "size": (out / name).stat().st_size,
            }
            signature = source.with_name(source.name + ".sig")
            if signature.is_file():
                sig_name = f"{name}.sig"
                if sig_name in names:
                    raise Failure(f"{args.target}: duplicate asset name {sig_name}")
                names.add(sig_name)
                shutil.copy2(signature, out / sig_name)
                record["signature"] = True
                files.append(record)
                files.append(
                    {
                        "kind": "signature",
                        "name": sig_name,
                        "sha256": sha256(out / sig_name),
                        "signature": False,
                        "size": (out / sig_name).stat().st_size,
                    }
                )
                signed.append(record)
            else:
                files.append(record)
        if kind == entry["updater_bundle"] and enabled:
            if not signed:
                raise Failure(
                    f"{args.target}: updater bundle {kind} has no .sig; was "
                    "TAURI_SIGNING_PRIVATE_KEY set on the bundle step?"
                )
            if len(signed) != 1:
                raise Failure(
                    f"{args.target}: {len(signed)} signed {kind} files; expected exactly one"
                )
            updater = {
                "kind": kind,
                "name": signed[0]["name"],
                "signature": f"{signed[0]['name']}.sig",
            }

    evidence = {
        "files": sorted(files, key=lambda item: item["name"]),
        "platform": entry["platform"],
        "schema_version": 1,
        "target": args.target,
        "updater": updater,
        "updater_enabled": enabled,
        "updater_platform": entry["updater_platform"],
    }
    with args.evidence.open("w", encoding="utf-8") as stream:
        json.dump(evidence, stream, indent=2, sort_keys=True)
        stream.write("\n")
    print(json.dumps({"collected": len(files), "target": args.target, "updater": updater}))


# --------------------------------------------------------------------- verify-signatures


def decode_signature(path: Path) -> str:
    raw = path.read_text(encoding="utf-8").strip()
    try:
        text = base64.b64decode(raw, validate=True).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError, ValueError) as error:
        raise Failure(f"{path}: not a base64 minisign signature ({error})") from error
    if "trusted comment:" not in text:
        raise Failure(f"{path}: decoded signature lacks a trusted comment line")
    return text


def cmd_verify_signatures(args: argparse.Namespace) -> None:
    if args.pubkey is not None:
        pubkey = args.pubkey.strip()
    else:
        pubkey = updater_pubkey(load_tauri_config(args.pubkey_from))
    pubkey_text = decode_minisign_pubkey(pubkey)
    minisign = shutil.which(args.minisign)
    if minisign is None:
        raise Failure(f"{args.minisign} is not installed; apt-get install -y minisign")

    signatures = sorted(path for path in args.dist.iterdir() if path.name.endswith(".sig"))
    if args.expect_updater and not signatures:
        raise Failure(f"{args.dist}: no .sig files although the updater is enabled")
    with tempfile.TemporaryDirectory() as scratch:
        pub_path = Path(scratch) / "updater.pub"
        pub_path.write_text(pubkey_text if pubkey_text.endswith("\n") else pubkey_text + "\n")
        for signature in signatures:
            payload = signature.with_name(signature.name[: -len(".sig")])
            if not payload.is_file():
                raise Failure(f"{signature.name}: signed file {payload.name} is missing")
            sig_path = Path(scratch) / (payload.name + ".minisig")
            text = decode_signature(signature)
            sig_path.write_text(text if text.endswith("\n") else text + "\n")
            result = subprocess.run(
                [minisign, "-V", "-q", "-p", str(pub_path), "-x", str(sig_path), "-m", str(payload)],
                check=False,
                capture_output=True,
                text=True,
            )
            if result.returncode != 0:
                raise Failure(
                    f"{payload.name}: signature does not verify against the configured pubkey"
                    f" ({result.stderr.strip() or result.stdout.strip()})"
                )
            print(f"verified {payload.name}")
    print(f"{len(signatures)} signature(s) verified")


# ----------------------------------------------------------------------- updater-json


def load_evidence(directory: Path) -> list[dict]:
    files = sorted(directory.glob("*.json"))
    if not files:
        raise Failure(f"{directory}: no evidence files")
    evidence = []
    for path in files:
        data = read_json(path)
        if not isinstance(data, dict) or data.get("schema_version") != 1:
            raise Failure(f"{path}: unsupported evidence schema")
        for key in ("target", "updater_platform", "files", "updater", "updater_enabled"):
            if key not in data:
                raise Failure(f"{path}: evidence lacks {key}")
        evidence.append(data)
    return evidence


def check_dist_against_evidence(dist: Path, evidence: list[dict]) -> None:
    seen: dict[str, str] = {}
    for record in evidence:
        for item in record["files"]:
            name = item["name"]
            if name in seen:
                raise Failure(f"{name}: produced by both {seen[name]} and {record['target']}")
            seen[name] = record["target"]
            path = dist / name
            if not path.is_file():
                raise Failure(f"{name}: listed in evidence for {record['target']} but missing")
            if path.stat().st_size != item["size"] or sha256(path) != item["sha256"]:
                raise Failure(f"{name}: bytes differ from the evidence written on the build leg")


def cmd_updater_json(args: argparse.Namespace) -> None:
    version = require_semver(args.version, "--version")
    if args.tag != f"v{version}":
        raise Failure(f"tag {args.tag!r} must equal v{version}")
    if not REPO_SLUG.fullmatch(args.repo):
        raise Failure(f"--repo must look like owner/name: {args.repo!r}")
    evidence = load_evidence(args.evidence_dir)
    check_dist_against_evidence(args.dist, evidence)

    expected_platforms: set[str] = set()
    if args.targets_file is not None:
        expected_platforms = {entry["updater_platform"] for entry in load_targets(args.targets_file)}

    platforms: dict[str, dict] = {}
    for record in evidence:
        updater = record["updater"]
        if updater is None:
            if record["updater_enabled"]:
                raise Failure(f"{record['target']}: updater enabled but no updater bundle recorded")
            continue
        key = record["updater_platform"]
        if key in platforms:
            raise Failure(f"duplicate updater platform {key}")
        if key != updater_platform_for(record["target"]):
            raise Failure(f"{record['target']}: evidence updater_platform {key} is wrong")
        signature_path = args.dist / updater["signature"]
        if not signature_path.is_file():
            raise Failure(f"{key}: signature file {updater['signature']} is missing")
        decode_signature(signature_path)
        platforms[key] = {
            "signature": signature_path.read_text(encoding="utf-8").strip(),
            "url": f"https://github.com/{args.repo}/releases/download/{args.tag}/"
            + quote(updater["name"], safe=""),
        }

    if not platforms:
        raise Failure("no updater platforms collected; refusing to write an empty latest.json")
    missing = sorted(expected_platforms - set(platforms))
    if missing:
        raise Failure(f"latest.json lacks platforms required by the manifest: {missing}")

    notes = args.notes_file.read_text(encoding="utf-8").strip() if args.notes_file else ""
    manifest = {
        "notes": notes,
        "platforms": dict(sorted(platforms.items())),
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "version": version,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True)
        stream.write("\n")
    print(json.dumps({"platforms": sorted(platforms), "out": str(args.out)}))


# ------------------------------------------------------------------------------- CLI


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    matrix = sub.add_parser("matrix", help="validate the target manifest and print a matrix")
    matrix.add_argument("--file", required=True, type=Path)
    matrix.set_defaults(func=cmd_matrix)

    check = sub.add_parser("check-config", help="validate tauri.conf.json and package.json")
    check.add_argument("--project", default=Path("src-tauri"), type=Path)
    check.add_argument("--package-json", default=None, type=Path)
    check.add_argument("--expect-version", default=None)
    check.add_argument("--targets-file", default=None, type=Path)
    check.add_argument("--github-output", default=None, type=Path)
    check.set_defaults(func=cmd_check_config)

    collect = sub.add_parser("collect", help="copy one target's bundles into dist/")
    collect.add_argument("--project", default=Path("src-tauri"), type=Path)
    collect.add_argument("--target", required=True)
    collect.add_argument("--targets-file", required=True, type=Path)
    collect.add_argument("--out", required=True, type=Path)
    collect.add_argument("--evidence", required=True, type=Path)
    collect.add_argument("--target-dir", default=None, type=Path)
    collect.add_argument("--updater", choices=("auto", "true", "false"), default="auto")
    collect.set_defaults(func=cmd_collect)

    verify = sub.add_parser("verify-signatures", help="minisign-verify every .sig in dist/")
    verify.add_argument("--dist", required=True, type=Path)
    key = verify.add_mutually_exclusive_group(required=True)
    key.add_argument("--pubkey", help="base64 pubkey as stored in tauri.conf.json")
    key.add_argument("--pubkey-from", type=Path, help="src-tauri directory holding tauri.conf.json")
    verify.add_argument("--minisign", default="minisign")
    verify.add_argument("--expect-updater", action="store_true")
    verify.set_defaults(func=cmd_verify_signatures)

    updater = sub.add_parser("updater-json", help="write the single-writer latest.json")
    updater.add_argument("--dist", required=True, type=Path)
    updater.add_argument("--evidence-dir", required=True, type=Path)
    updater.add_argument("--repo", required=True)
    updater.add_argument("--tag", required=True)
    updater.add_argument("--version", required=True)
    updater.add_argument("--notes-file", default=None, type=Path)
    updater.add_argument("--targets-file", default=None, type=Path)
    updater.add_argument("--out", required=True, type=Path)
    updater.set_defaults(func=cmd_updater_json)
    return parser


def main(argv: list[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    for name in ("target", "tag", "repo"):
        value = getattr(args, name, None)
        if value is not None and not (SAFE_ATOM.fullmatch(value) or name == "repo"):
            raise Failure(f"--{name} contains unsafe characters: {value!r}")
    args.func(args)


if __name__ == "__main__":
    main()
