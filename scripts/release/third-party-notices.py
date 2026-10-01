#!/usr/bin/env python3
"""Write THIRD-PARTY-NOTICES.txt for a desktop package, or with --app mobile for the Android app.

Three sources, each with the licence texts themselves, since most of these licences ask for the text
to travel with the binary:

- the Rust crates the shell links, from cargo-about (`about.toml`: the deny.toml allow-list, build
  and dev dependencies excluded; the three desktop targets, or aarch64-linux-android for the app);
- the npm packages in the web frontend's production dependency tree, from `pnpm licenses list`,
  with the licence file each package ships (the three fonts are OFL-1.1);
- the native libraries that ship inside or beside the desktop binary: transcribe.cpp and ggml (from
  the transcribe-cpp-sys sources), sherpa-onnx, ONNX Runtime and the Khronos Vulkan loader (texts in
  scripts/release/licenses/). The Android app links none of them.

Usage: third-party-notices.py --out FILE [--app desktop|mobile] [--version V] [--about-json FILE]
                              [--pnpm-json FILE]
The two JSON options replace running cargo-about / pnpm (tests, or a CI step that ran them already).
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LICENSES = Path(__file__).resolve().parent / "licenses"
DESKTOP_MANIFEST = ROOT / "apps" / "desktop" / "src-tauri" / "Cargo.toml"
MOBILE_MANIFEST = ROOT / "apps" / "mobile" / "src-tauri" / "Cargo.toml"
ANDROID_TARGET = "aarch64-linux-android"
LICENSE_FILE_PREFIXES = ("license", "licence", "copying", "notice", "ofl", "unlicense")
RULE = "=" * 78
# Where the Corresponding Source of each release is (AGPL-3.0 s. 6): the tag of that version.
SOURCE_URL = "https://github.com/sunerpy/voltip"


class Failure(Exception):
    pass


def run_json(argv: list[str], cwd: Path) -> object:
    try:
        out = subprocess.run(argv, cwd=cwd, check=True, capture_output=True, text=True).stdout
    except FileNotFoundError as e:
        raise Failure(f"{argv[0]} not installed: {e}") from e
    except subprocess.CalledProcessError as e:
        raise Failure(f"{' '.join(argv)} failed: {e.stderr.strip()[-2000:]}") from e
    return json.loads(out)


def cargo_about(app: str) -> dict:
    if app == "mobile":
        scope = ["-m", str(MOBILE_MANIFEST), "--target", ANDROID_TARGET]
    else:
        scope = ["-m", str(DESKTOP_MANIFEST), "--features", "gpu-vulkan"]
    return run_json(["cargo", "about", "generate", "--format", "json", *scope, "--locked", "--fail"], ROOT)


def pnpm_licenses(app: str) -> dict:
    return run_json(["pnpm", "licenses", "list", "--prod", "--json"], ROOT / "apps" / app)


def crate_dir(name: str) -> Path:
    """The source directory of the locked `name` crate (for the licences it carries inside)."""
    meta = run_json(["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", str(DESKTOP_MANIFEST)], ROOT)
    for pkg in meta["packages"]:
        if pkg["name"] == name:
            return Path(pkg["manifest_path"]).parent
    raise Failure(f"{name} is not in the desktop shell's dependency graph")


def package_licence_text(paths: list[str]) -> str | None:
    """The licence file an npm package ships, if it ships one."""
    for path in paths:
        folder = Path(path)
        if not folder.is_dir():
            continue
        for entry in sorted(folder.iterdir()):
            if entry.is_file() and entry.name.lower().startswith(LICENSE_FILE_PREFIXES):
                return entry.read_text(encoding="utf-8", errors="replace").strip()
    return None


def native_components(transcribe_dir: Path | None) -> list[tuple[str, str, str, str]]:
    """(name, licence, where it comes from, text) for the libraries no package manager lists."""
    out = []
    if transcribe_dir is not None:
        out.append(("transcribe.cpp", "MIT", "https://github.com/handy-computer/transcribe.cpp (the transcribe-cpp-sys crate)", (transcribe_dir / "LICENSE").read_text(encoding="utf-8").strip()))
        out.append(("ggml", "MIT", "https://github.com/ggml-org/ggml (vendored by transcribe.cpp)", (transcribe_dir / "ggml" / "LICENSE").read_text(encoding="utf-8").strip()))
    out.append(("sherpa-onnx (prebuilt shared libraries)", "Apache-2.0", "https://github.com/k2-fsa/sherpa-onnx", "The licence text is the sherpa-onnx-sys crate's, under the Apache License 2.0 in the Rust section below."))
    ort = (LICENSES / "onnxruntime-LICENSE.txt").read_text(encoding="utf-8").strip()
    ort_notices = (LICENSES / "onnxruntime-ThirdPartyNotices.txt").read_text(encoding="utf-8").strip()
    out.append(("ONNX Runtime 1.28.2 (shared library shipped with sherpa-onnx)", "MIT", "https://github.com/microsoft/onnxruntime", f"{ort}\n\nONNX Runtime's own third-party notices:\n\n{ort_notices}"))
    out.append(("Khronos Vulkan loader (vulkan-1.dll on Windows, libvulkan.so.1 in the AppImage)", "Apache-2.0 AND MIT", "https://github.com/KhronosGroup/Vulkan-Loader", (LICENSES / "vulkan-loader-LICENSE.txt").read_text(encoding="utf-8").strip()))
    out.append(("Microsoft Edge WebView2 bootstrapper (Windows installer only)", "Microsoft Software License Terms", "https://developer.microsoft.com/microsoft-edge/webview2/", "Installed by the Windows installer when WebView2 is missing; distributed under Microsoft's WebView2 distribution terms."))
    return out


def render(version: str, about: dict, pnpm: dict, natives: list[tuple[str, str, str, str]]) -> str:
    lines = [
        f"Voltip {version} — third-party notices",
        "",
        "Voltip is free software: you can redistribute it and/or modify it under the terms of the GNU",
        "Affero General Public License as published by the Free Software Foundation, either version 3",
        "of the License, or (at your option) any later version. Its full text follows. Voltip 0.0.20 and",
        "earlier were released under the Apache License 2.0.",
        "",
        f"The complete source code of this version: {SOURCE_URL}/tree/v{version}",
        "",
        "Voltip includes the third-party software listed after the licence, each under its own licence,",
        "whose text follows its entry.",
        "",
        RULE,
        "Voltip's licence",
        RULE,
        "",
        (ROOT / "LICENSE").read_text(encoding="utf-8").strip(),
        "",
    ]

    if natives:
        lines += [RULE, "Native libraries", RULE, ""]
    for name, licence, origin, text in natives:
        lines += [f"{name}", f"Licence: {licence}", f"Source: {origin}", "", text, "", "-" * 78, ""]

    crates = about.get("crates", [])
    lines += [RULE, f"Rust crates ({len(crates)})", RULE, ""]
    for crate in sorted(crates, key=lambda c: (c["package"]["name"], c["package"]["version"])):
        pkg = crate["package"]
        lines.append(f"{pkg['name']} {pkg['version']} — {crate.get('license', pkg.get('license') or '?')}")
    lines.append("")
    seen: set[str] = set()
    for licence in about.get("licenses", []):
        text = licence["text"].strip()
        key = f"{licence['id']}\0{text}"
        if key in seen:
            continue
        seen.add(key)
        users = sorted({f"{u['crate']['name']} {u['crate']['version']}" for u in licence.get("used_by", [])})
        lines += ["-" * 78, f"{licence['name']} ({licence['id']})", "Used by: " + ", ".join(users), "", text, ""]

    packages = [(lic, p) for lic, group in pnpm.items() for p in group]
    lines += [RULE, f"Web frontend packages ({len(packages)})", RULE, ""]
    for licence, p in sorted(packages, key=lambda item: item[1]["name"]):
        versions = ", ".join(p.get("versions") or [])
        lines += ["-" * 78, f"{p['name']} {versions} — {licence}"]
        if p.get("homepage"):
            lines.append(f"Homepage: {p['homepage']}")
        text = package_licence_text(p.get("paths") or [])
        lines += ["", text if text is not None else f"(no licence file in the package; the licence is {licence})", ""]
    return "\n".join(lines).rstrip() + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--app", choices=("desktop", "mobile"), default="desktop", help="the package: the desktop shell or the Android app")
    parser.add_argument("--version", default=None, help="the app version (default: package.json)")
    parser.add_argument("--about-json", type=Path, help="cargo-about JSON instead of running it")
    parser.add_argument("--pnpm-json", type=Path, help="`pnpm licenses list --prod --json` output instead of running it")
    parser.add_argument("--no-crate-sources", action="store_true", help="skip transcribe.cpp / ggml (tests without a Cargo checkout)")
    args = parser.parse_args(argv)
    try:
        version = args.version or json.loads((ROOT / "package.json").read_text(encoding="utf-8"))["version"]
        about = json.loads(args.about_json.read_text(encoding="utf-8")) if args.about_json else cargo_about(args.app)
        pnpm = json.loads(args.pnpm_json.read_text(encoding="utf-8")) if args.pnpm_json else pnpm_licenses(args.app)
        if args.app == "mobile":
            natives = []
        else:
            natives = native_components(None if args.no_crate_sources else crate_dir("transcribe-cpp-sys"))
        text = render(version, about, pnpm, natives)
    except Failure as e:
        print(f"third-party-notices: {e}", file=sys.stderr)
        return 1
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(text, encoding="utf-8")
    print(f"third-party-notices: {len(about.get('crates', []))} crates, {sum(len(g) for g in pnpm.values())} npm packages → {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
