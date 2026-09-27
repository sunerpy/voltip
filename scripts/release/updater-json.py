#!/usr/bin/env python3
"""Voltip release helpers for the Tauri updater: config preflight, per-target bundle
collection, and the single-writer ``latest.json``.

Why this exists next to ``.github/scripts/tauri-release.py`` (the scaffold skill's asset): the
release has no runner matrix (Windows is cross-built from Linux, macOS is not released yet), the
updater is optional (on only when the pubkey secret is set) and ``tauri.conf.json`` carries no
updater block, which that script's ``check-config`` expects. The release passes
``--base-url https://github.com/<repo>/releases/download``, so every URL in ``latest.json`` is an
asset of the release it describes. Signature verification still uses the skill's
``tauri-release.py verify-signatures``; the evidence written here follows the same schema.

Stdlib only. Every value crosses the shell boundary as an argument, never inside a shell
string. Subcommands:

  check-config   validate tauri.conf.json / package.json against the compile-time updater
                 secrets and emit GITHUB_OUTPUT values (updater_enabled, version, product_name)
  collect        copy one target's bundles (+ .sig) into dist/ and write evidence JSON
  write          build latest.json from the evidence of every leg

Tested by ``python3 -m unittest discover -s scripts/release -p 'test_*.py'``.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import re
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import quote, urlsplit

SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"
)
SAFE_ATOM = re.compile(r"^[A-Za-z0-9_.+-]+$")
SAFE_ASSET_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._+-]*$")

# Bundle kind -> directory under <bundle-dir> -> file suffixes (tauri-bundler layout).
KIND_DIRS = {"deb": "deb", "rpm": "rpm", "appimage": "appimage", "nsis": "nsis", "msi": "msi"}
KIND_SUFFIXES = {
    "deb": (".deb",),
    "rpm": (".rpm",),
    "appimage": (".AppImage",),
    "nsis": ("-setup.exe",),
    "msi": (".msi",),
}
UPDATER_BUNDLES = {"appimage", "nsis", "msi"}
ARCHES = {"x86_64", "aarch64", "i686", "armv7"}


class Failure(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(f"updater-json: {message}")


# ------------------------------------------------------------------------------ helpers


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
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_semver(version: object, label: str) -> str:
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        raise Failure(f"{label} is not a SemVer version: {version!r}")
    return version


def updater_platform_for(target: str) -> str:
    """OS-ARCH key as tauri-plugin-updater computes it (linux|darwin|windows, x86_64|aarch64|…)."""
    arch = target.split("-", 1)[0]
    if arch not in ARCHES:
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
    return {"linux": "linux", "darwin": "macos", "windows": "windows"}[
        updater_platform_for(target).split("-", 1)[0]
    ]


def decode_minisign_pubkey(pubkey: str) -> str:
    """Base64 -> two-line minisign public key text, as the updater plugin decodes it."""
    try:
        text = base64.b64decode(pubkey, validate=True).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError, ValueError) as error:
        raise Failure(f"updater pubkey is not base64 minisign text ({error})") from error
    lines = [line for line in text.splitlines() if line.strip()]
    if len(lines) != 2 or not lines[0].startswith("untrusted comment:"):
        raise Failure("updater pubkey must decode to a two-line minisign public key")
    try:
        raw = base64.b64decode(lines[1], validate=True)
    except (binascii.Error, ValueError) as error:
        raise Failure("updater pubkey key line is not base64") from error
    if len(raw) != 42 or raw[:2] != b"Ed":
        raise Failure("updater pubkey is not an Ed25519 minisign public key")
    return text


def decode_signature(path: Path) -> str:
    raw = path.read_text(encoding="utf-8").strip()
    try:
        text = base64.b64decode(raw, validate=True).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError, ValueError) as error:
        raise Failure(f"{path}: not a base64 minisign signature ({error})") from error
    if "trusted comment:" not in text:
        raise Failure(f"{path}: decoded signature lacks a trusted comment line")
    return text


def safe_asset_name(name: str) -> str:
    candidate = re.sub(r"\s+", ".", name.strip())
    if not SAFE_ASSET_NAME.fullmatch(candidate):
        raise Failure(f"not a safe release asset name: {name!r}")
    return candidate


def write_github_output(path: Path | None, values: dict[str, object]) -> None:
    if path is None:
        return
    with path.open("a", encoding="utf-8") as stream:
        for key, value in values.items():
            if isinstance(value, bool):
                value = "true" if value else "false"
            value = str(value)
            if "\n" in value:
                raise Failure(f"{key} contains a newline")
            stream.write(f"{key}={value}\n")


# -------------------------------------------------------------------------- check-config


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


def read_package_version(path: Path) -> str:
    data = read_json(path)
    if not isinstance(data, dict):
        raise Failure(f"{path}: must be an object")
    return require_semver(data.get("version"), f"{path} version")


def resolve_version(project: Path, config: dict, package_json: Path) -> tuple[str, str]:
    """Return (version, source); source is 'package.json' or 'literal'.

    Either tauri.conf.json points at ../package.json (single source) or it carries a literal that
    release-please bumps through extra-files; in both cases it must equal package.json."""
    declared = config.get("version")
    if declared is None:
        raise Failure('tauri.conf.json has no "version"')
    if not isinstance(declared, str):
        raise Failure('tauri.conf.json "version" must be a string')
    package_version = read_package_version(package_json)
    if declared.endswith("package.json"):
        target = (project / declared).resolve()
        if target != package_json.resolve():
            raise Failure(
                f'tauri.conf.json "version" points at {target}, not {package_json.resolve()}'
            )
        return package_version, "package.json"
    version = require_semver(declared, 'tauri.conf.json "version"')
    if version != package_version:
        raise Failure(
            f"tauri.conf.json version {version} differs from {package_json} version "
            f"{package_version}; release-please must bump both (extra-files)"
        )
    return version, "literal"


def conf_updater_flag(config: dict) -> object:
    bundle = config.get("bundle") or {}
    if not isinstance(bundle, dict):
        raise Failure('tauri.conf.json "bundle" must be an object')
    return bundle.get("createUpdaterArtifacts", False)


def check_conf_updater_block(config: dict, pubkey: str) -> None:
    """tauri.conf.json never holds the updater key: apps/desktop/src-tauri/src/update.rs injects
    plugins.updater (pubkey + endpoint) at runtime from the compile-time VOLTIP_UPDATE_URL /
    VOLTIP_UPDATE_PUBKEY, and the release workflow enables signing with `--config` on the bundle
    step only. A createUpdaterArtifacts flag in the file would make `make windows-x64` demand a
    key; a diverging pubkey would make the shipped app reject every update."""
    flag = conf_updater_flag(config)
    if flag is not False:
        raise Failure(
            "keep bundle.createUpdaterArtifacts out of tauri.conf.json (found "
            f"{flag!r}); the release workflow enables it with --config on the signing step"
        )
    plugins = config.get("plugins") or {}
    updater = plugins.get("updater") if isinstance(plugins, dict) else None
    if updater is None:
        return
    if not isinstance(updater, dict):
        raise Failure("plugins.updater must be an object when present")
    conf_pubkey = updater.get("pubkey")
    if conf_pubkey is not None:
        if not isinstance(conf_pubkey, str) or not conf_pubkey.strip():
            raise Failure("plugins.updater.pubkey must be a non-empty string when present")
        decode_minisign_pubkey(conf_pubkey.strip())
        if conf_pubkey.strip() != pubkey:
            raise Failure(
                "plugins.updater.pubkey differs from VOLTIP_UPDATE_PUBKEY; the shipped app would "
                "reject every update"
            )
    endpoints = updater.get("endpoints")
    if endpoints is not None:
        if not isinstance(endpoints, list) or not endpoints:
            raise Failure("plugins.updater.endpoints, when present, must be a non-empty array")
        for endpoint in endpoints:
            if not isinstance(endpoint, str) or not endpoint.startswith("https://"):
                raise Failure(f"updater endpoint must be https: {endpoint!r}")


def cmd_check_config(args: argparse.Namespace) -> None:
    config = load_tauri_config(args.project)
    version, source = resolve_version(args.project, config, args.package_json)
    if args.expect_version is not None:
        expected = require_semver(args.expect_version, "--expect-version")
        if version != expected:
            raise Failure(f"configured version {version} != release version {expected}")
    bundle = config.get("bundle") or {}
    if not isinstance(bundle, dict) or bundle.get("active") is False:
        raise Failure("bundle.active must not be false; the release needs bundles")
    product_name = config.get("productName")
    if not isinstance(product_name, str) or not product_name.strip():
        raise Failure('tauri.conf.json "productName" must be a non-empty string')

    # The updater exists exactly when both compile-time values are baked in
    # (UpdaterConfig::from_values in update.rs requires both).
    url_present = args.update_url_present == "true"
    pubkey = (args.update_pubkey or "").strip()
    if url_present != bool(pubkey):
        raise Failure(
            "VOLTIP_UPDATE_URL and VOLTIP_UPDATE_PUBKEY must be set together (the app ignores a "
            "lone value and ships without an updater)"
        )
    enabled = url_present and bool(pubkey)
    if enabled:
        decode_minisign_pubkey(pubkey)
    check_conf_updater_block(config, pubkey)

    summary = {
        "product_name": product_name,
        "updater_enabled": enabled,
        "version": version,
        "version_source": source,
    }
    write_github_output(
        args.github_output,
        {key: summary[key] for key in ("version", "updater_enabled", "product_name")},
    )
    print(json.dumps(summary, indent=2, sort_keys=True))


# ------------------------------------------------------------------------------- collect


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


def parse_extra(value: str) -> tuple[str, Path]:
    if "=" not in value:
        raise Failure(f"--extra expects NAME=PATH, got {value!r}")
    name, _, path = value.partition("=")
    return safe_asset_name(name), Path(path)


def cmd_collect(args: argparse.Namespace) -> None:
    if not SAFE_ATOM.fullmatch(args.target):
        raise Failure(f"--target contains unsafe characters: {args.target!r}")
    platform = platform_for(args.target)
    kinds = [kind for kind in args.bundles.split(",") if kind]
    if not kinds or len(set(kinds)) != len(kinds):
        raise Failure("--bundles must be a unique, non-empty comma-separated list")
    for kind in kinds:
        if kind not in KIND_DIRS:
            raise Failure(f"unsupported bundle kind {kind!r}; choose from {sorted(KIND_DIRS)}")
    if args.updater_bundle not in UPDATER_BUNDLES or args.updater_bundle not in kinds:
        raise Failure(
            f"--updater-bundle must be one of {sorted(UPDATER_BUNDLES)} and listed in --bundles"
        )
    enabled = args.updater == "true"
    root = args.bundle_dir
    if not root.is_dir():
        raise Failure(f"bundle directory not found: {root}")
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    args.evidence.parent.mkdir(parents=True, exist_ok=True)

    files: list[dict] = []
    names: set[str] = set()
    updater: dict | None = None

    def record(path: Path, kind: str, signature: bool) -> dict:
        return {
            "kind": kind,
            "name": path.name,
            "sha256": sha256(path),
            "signature": signature,
            "size": path.stat().st_size,
        }

    def place(source: Path, name: str) -> Path:
        if name in names:
            raise Failure(f"{args.target}: duplicate asset name {name}")
        names.add(name)
        shutil.copy2(source, out / name)
        return out / name

    for kind in kinds:
        found = find_bundle_files(root, kind)
        if not found:
            raise Failure(f"{args.target}: no {kind} bundle under {root / KIND_DIRS[kind]}")
        signed: list[dict] = []
        for source in found:
            name = safe_asset_name(source.name)
            copied = place(source, name)
            entry = record(copied, kind, signature=False)
            signature = source.with_name(source.name + ".sig")
            if signature.is_file():
                decode_signature(signature)
                sig_copied = place(signature, f"{name}.sig")
                entry["signature"] = True
                files.append(entry)
                files.append(record(sig_copied, "signature", signature=False))
                signed.append(entry)
            else:
                files.append(entry)
        if kind == args.updater_bundle and enabled:
            if len(signed) != 1:
                raise Failure(
                    f"{args.target}: expected exactly one signed {kind} bundle, found "
                    f"{len(signed)}; was TAURI_SIGNING_PRIVATE_KEY set on the bundle step?"
                )
            updater = {
                "kind": kind,
                "name": signed[0]["name"],
                "signature": f"{signed[0]['name']}.sig",
            }

    for value in args.extra:
        name, source = parse_extra(value)
        if not source.is_file():
            raise Failure(f"--extra {name}: {source} is not a file")
        files.append(record(place(source, name), "extra", signature=False))

    evidence = {
        "files": sorted(files, key=lambda item: item["name"]),
        "platform": platform,
        "schema_version": 1,
        "target": args.target,
        "updater": updater,
        "updater_enabled": enabled,
        "updater_platform": updater_platform_for(args.target),
    }
    with args.evidence.open("w", encoding="utf-8") as stream:
        json.dump(evidence, stream, indent=2, sort_keys=True)
        stream.write("\n")
    print(json.dumps({"collected": len(files), "target": args.target, "updater": updater}))


# --------------------------------------------------------------------------------- write


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
    for entry in evidence:
        for item in entry["files"]:
            name = item["name"]
            if name in seen:
                raise Failure(f"{name}: produced by both {seen[name]} and {entry['target']}")
            seen[name] = entry["target"]
            path = dist / name
            if not path.is_file():
                raise Failure(f"{name}: listed in evidence for {entry['target']} but missing")
            if path.stat().st_size != item["size"] or sha256(path) != item["sha256"]:
                raise Failure(f"{name}: bytes differ from the evidence written on the build leg")


def normalise_base_url(base_url: str) -> str:
    parts = urlsplit(base_url)
    if parts.scheme != "https" or not parts.netloc:
        raise Failure(f"--base-url must be an https URL, got {base_url!r}")
    if parts.query or parts.fragment:
        raise Failure("--base-url must not carry a query string or fragment")
    return base_url.rstrip("/")


def cmd_write(args: argparse.Namespace) -> None:
    version = require_semver(args.version, "--version")
    if args.tag != f"v{version}":
        raise Failure(f"tag {args.tag!r} must equal v{version}")
    base_url = normalise_base_url(args.base_url)
    evidence = load_evidence(args.evidence_dir)
    check_dist_against_evidence(args.dist, evidence)

    platforms: dict[str, dict] = {}
    for entry in evidence:
        updater = entry["updater"]
        if updater is None:
            if entry["updater_enabled"]:
                raise Failure(f"{entry['target']}: updater enabled but no updater bundle recorded")
            continue
        key = entry["updater_platform"]
        if key in platforms:
            raise Failure(f"duplicate updater platform {key}")
        if key != updater_platform_for(entry["target"]):
            raise Failure(f"{entry['target']}: evidence updater_platform {key} is wrong")
        signature_path = args.dist / updater["signature"]
        if not signature_path.is_file():
            raise Failure(f"{key}: signature file {updater['signature']} is missing")
        decode_signature(signature_path)
        platforms[key] = {
            "signature": signature_path.read_text(encoding="utf-8").strip(),
            "url": f"{base_url}/{quote(args.tag, safe='')}/{quote(updater['name'], safe='')}",
        }

    if not platforms:
        raise Failure("no updater platforms collected; refusing to write an empty latest.json")
    missing = sorted(set(args.require_platform) - set(platforms))
    if missing:
        raise Failure(f"latest.json lacks required platforms: {missing}")

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
    print(json.dumps({"out": str(args.out), "platforms": sorted(platforms)}))


# ----------------------------------------------------------------------------------- CLI


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    check = sub.add_parser("check-config", help="validate tauri.conf.json against package.json")
    check.add_argument("--project", required=True, type=Path, help="directory of tauri.conf.json")
    check.add_argument("--package-json", required=True, type=Path)
    check.add_argument("--expect-version", default=None)
    check.add_argument(
        "--update-url-present", choices=("true", "false"), required=True,
        help="whether the VOLTIP_UPDATE_URL secret is non-empty",
    )
    check.add_argument("--update-pubkey", default="", help="VOLTIP_UPDATE_PUBKEY (base64 minisign)")
    check.add_argument("--github-output", default=None, type=Path)
    check.set_defaults(func=cmd_check_config)

    collect = sub.add_parser("collect", help="copy one target's bundles into dist/")
    collect.add_argument("--target", required=True, help="Rust target triple")
    collect.add_argument("--bundle-dir", required=True, type=Path, help="…/release/bundle")
    collect.add_argument("--bundles", required=True, help="comma-separated kinds, e.g. nsis")
    collect.add_argument("--updater-bundle", required=True)
    collect.add_argument("--updater", choices=("true", "false"), required=True)
    collect.add_argument("--out", required=True, type=Path)
    collect.add_argument("--evidence", required=True, type=Path)
    collect.add_argument(
        "--extra", action="append", default=[], help="NAME=PATH of a non-bundle file to ship"
    )
    collect.set_defaults(func=cmd_collect)

    write = sub.add_parser("write", help="write latest.json from the collected evidence")
    write.add_argument("--dist", required=True, type=Path)
    write.add_argument("--evidence-dir", required=True, type=Path)
    write.add_argument("--base-url", required=True, help="asset base, e.g. https://github.com/<owner>/<repo>/releases/download")
    write.add_argument("--tag", required=True)
    write.add_argument("--version", required=True)
    write.add_argument("--notes-file", default=None, type=Path)
    write.add_argument("--require-platform", action="append", default=[])
    write.add_argument("--out", required=True, type=Path)
    write.set_defaults(func=cmd_write)
    return parser


def main(argv: list[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    for name in ("target", "tag"):
        value = getattr(args, name, None)
        if value is not None and not SAFE_ATOM.fullmatch(value):
            raise Failure(f"--{name} contains unsafe characters: {value!r}")
    args.func(args)


if __name__ == "__main__":
    main()
