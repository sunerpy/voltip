# shellcheck shell=bash
# HEAD's short commit, read from .git without running git (build and smoke records must not touch
# the index or depend on a git binary): voltip_head_commit → `7fc1413`, or `unknown` when the layout
# is not a plain checkout / worktree. It says nothing about uncommitted changes.
voltip_head_commit() {
  local gitdir=.git head ref sha="" common
  if [ -f .git ]; then gitdir=$(sed -n 's/^gitdir: //p' .git); fi
  head=$(cat "$gitdir/HEAD" 2>/dev/null) || { echo unknown; return 0; }
  case $head in
    "ref: "*)
      ref=${head#ref: }
      common=$gitdir
      [ -f "$gitdir/commondir" ] && common=$gitdir/$(cat "$gitdir/commondir")
      if [ -f "$common/$ref" ]; then sha=$(cat "$common/$ref"); else sha=$(awk -v r="$ref" '$2 == r { print $1 }' "$common/packed-refs" 2>/dev/null); fi
      ;;
    *) sha=$head ;;
  esac
  if [ -n "$sha" ]; then echo "${sha:0:7}"; else echo unknown; fi
}
