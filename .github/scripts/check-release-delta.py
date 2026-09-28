#!/usr/bin/env python3
"""Prove that a release-PR head is a bot-shaped delta on top of a known base.

The candidate profile builds distributables from the release PR head and later
promotes those exact bytes to the tag that the squash merge creates. That is
only sound when the head is what release-please produced: a commit whose only
changes are the files the release strategy rewrites, sitting directly on the
default-branch commit it will be merged into. This check turns that assumption
into a verified invariant before a 15-24 minute build starts.

Accepted commit shapes (BASE is the default-branch head):

  single-parent    HEAD^ == BASE                      release-please rewrote the PR
  update-branch    HEAD^2 == BASE, HEAD^1 = old head  GitHub "Update branch" merged BASE in

In both shapes BASE must be an ancestor of HEAD (``git merge-base
--is-ancestor``), so the squash-merged tree equals HEAD's tree when the base has
not moved.

File rules, evaluated on ``git diff BASE HEAD``:

  --allow-globs          paths whose content may change freely (CHANGELOG.md,
                         .release-please-manifest.json)
  --version-only-globs   manifests where every changed line must differ only by
                         a version string (Cargo.toml, Cargo.lock, package.json).
                         A [patch] section, a git dependency, or a changed lock
                         checksum is therefore rejected.

``git diff --check`` must also pass. Globs use ``*`` (within one path segment),
``?`` and ``**/`` (any number of directories). The glob lists are the adapter
input; the check itself knows nothing about a language.

Output: one JSON object on stdout ({"ok", "shape", "head", "base", "parents",
"files", "violations"}). Exit status: 1 when ok is false, in every mode.

Voltip (release-candidate.yml) runs this before any build leg, and those legs hold the production
and signing secrets, so the scaffold template's ``--allow-unproven`` escape for dry runs is gone:
a dry run proves the delta exactly like an automatic run. Tests:
``scripts/release/test_check_release_delta.py``.

Sources: release-please rust/node strategies rewrite exactly these files
(https://raw.githubusercontent.com/googleapis/release-please/main/src/strategies/rust.ts);
GitHub compare/update-branch semantics
(https://docs.github.com/en/rest/commits/commits#compare-two-commits).
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import re
import subprocess
import sys
from pathlib import Path

SHA = re.compile(r"^[0-9a-f]{40}$")
VERSION_ANYWHERE = re.compile(
    r"(?<![0-9A-Za-z.])(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(?![0-9A-Za-z])"
)


def git(repo: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        check=check,
        capture_output=True,
        text=True,
    )


def glob_to_regex(pattern: str) -> re.Pattern:
    """Translate a path glob into a full-match regex with ``**/`` support."""
    out = []
    i = 0
    while i < len(pattern):
        char = pattern[i]
        if pattern.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
            continue
        if pattern.startswith("**", i):
            out.append(".*")
            i += 2
            continue
        if char == "*":
            out.append("[^/]*")
        elif char == "?":
            out.append("[^/]")
        else:
            out.append(re.escape(char))
        i += 1
    return re.compile("^" + "".join(out) + "$")


def matches_any(path: str, patterns: list[re.Pattern]) -> bool:
    return any(pattern.fullmatch(path) for pattern in patterns)


def split_globs(value: str | None) -> list[str]:
    if not value:
        return []
    globs = value.split()
    for glob in globs:
        if glob.startswith("/") or ".." in glob.split("/"):
            raise SystemExit(f"glob must be repository-relative without '..': {glob}")
        # fnmatch is not used for matching, but its translate() validates syntax.
        fnmatch.translate(glob)
    return globs


def version_only_ok(diff_text: str) -> str | None:
    """Return None when every changed line differs only by a version string."""
    if "Binary files" in diff_text:
        return "binary content changed"
    removed = []
    added = []
    for line in diff_text.splitlines():
        if line.startswith("---") or line.startswith("+++"):
            continue
        if line.startswith("-"):
            removed.append(VERSION_ANYWHERE.sub("<VERSION>", line[1:]))
        elif line.startswith("+"):
            added.append(VERSION_ANYWHERE.sub("<VERSION>", line[1:]))
    if not removed and not added:
        return "no textual change recorded"
    if sorted(removed) != sorted(added):
        return "changed lines are not version-only substitutions"
    return None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--head", required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--allow-globs", default="")
    parser.add_argument("--version-only-globs", default="")
    parser.add_argument("--mode", choices=("automatic", "dry-run"), default="automatic")
    args = parser.parse_args()

    for name, value in (("head", args.head), ("base", args.base)):
        if not SHA.fullmatch(value):
            raise SystemExit(f"--{name} must be a full lowercase Git SHA")
    if args.head == args.base:
        raise SystemExit("head and base are the same commit")
    repo = args.repo.resolve()
    if not (repo / ".git").exists():
        raise SystemExit(f"not a git repository: {repo}")
    allow = [glob_to_regex(glob) for glob in split_globs(args.allow_globs)]
    version_only_globs = split_globs(args.version_only_globs)
    version_only = [glob_to_regex(glob) for glob in version_only_globs]
    if not allow and not version_only:
        raise SystemExit("at least one of --allow-globs or --version-only-globs is required")

    violations: list[str] = []
    for name, sha in (("head", args.head), ("base", args.base)):
        if git(repo, "cat-file", "-e", f"{sha}^{{commit}}", check=False).returncode != 0:
            raise SystemExit(f"{name} commit {sha} is not present in {repo}")
    actual_head = git(repo, "rev-parse", "HEAD").stdout.strip()
    if actual_head != args.head:
        raise SystemExit(f"checkout HEAD is {actual_head}, expected {args.head}")

    parents = git(repo, "rev-list", "--parents", "-n1", args.head).stdout.split()[1:]
    shape = "unknown"
    if len(parents) == 1 and parents[0] == args.base:
        shape = "single-parent"
    elif len(parents) == 2 and parents[1] == args.base:
        shape = "update-branch"
    else:
        violations.append(
            f"head parents {parents} are neither [BASE] nor [old head, BASE] (BASE={args.base})"
        )
    if git(repo, "merge-base", "--is-ancestor", args.base, args.head, check=False).returncode != 0:
        violations.append("base is not an ancestor of head")

    files = git(repo, "diff", "--name-only", args.base, args.head).stdout.split("\n")
    files = [path for path in files if path]
    if not files:
        violations.append("head changes no files relative to base")
    for path in files:
        if matches_any(path, version_only):
            diff_text = git(repo, "diff", "--unified=0", args.base, args.head, "--", path).stdout
            problem = version_only_ok(diff_text)
            if problem:
                violations.append(f"{path}: {problem}")
        elif not matches_any(path, allow):
            violations.append(f"{path}: not a release-please owned file")

    whitespace = git(repo, "diff", "--check", args.base, args.head, check=False)
    if whitespace.returncode != 0:
        violations.append("git diff --check reported whitespace errors")

    result = {
        "base": args.base,
        "files": files,
        "head": args.head,
        "mode": args.mode,
        "ok": not violations,
        "parents": parents,
        "shape": shape,
        "violations": violations,
    }
    json.dump(result, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    for violation in violations:
        print(f"release-delta violation: {violation}", file=sys.stderr)
    if violations:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
