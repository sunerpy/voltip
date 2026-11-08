#!/bin/sh
# The verdict of scripts/repro-gnome-remote-desktop.sh (docs/dictation.md §14.9, issue #235): each
# step of each side against what the issue and the fix lead to expect. A step is the windows GNOME
# showed during it and the pastes whose text reached the field; a side, where its windows came from
# and, on the fix side, how many portal sessions its pastes opened and closed.
#
#   verdict.sh OUT SIDE...
#
# Reads OUT/<side>/steps.log and OUT/<side>/summary.txt (session.sh). Prints one line per check,
# then PASS or FAIL; exits 1 on FAIL.
set -eu
out=$1
shift

fail=0
check() {
  if [ "$1" = ok ]; then
    printf 'ok    %s\n' "$2"
  else
    printf 'FAIL  %s\n' "$2"
    fail=1
  fi
}

# A field of the step LABEL in steps.log: `<label>\tpastes=N\twindows=N\tarrived=N\t…`.
field() {
  awk -F '\t' -v label="$2" -v key="$3" '$1 == label {
    for (i = 2; i <= NF; i++) if (index($i, key "=") == 1) { print substr($i, length(key) + 2); exit }
  }' "$out/$1/steps.log"
}

# What the harness printed for the step LABEL, one `<label> <n>: <via> <code>` per paste.
results() {
  awk -F '\t' -v label="$2" '$1 == label { print $5; exit }' "$out/$1/steps.log"
}

summary() {
  sed -n "s/^$2=//p" "$out/$1/summary.txt"
}

# How long the side waited between its third and fourth step (VOLTIP_REPRO_REUSE_WAIT).
wait_seconds() {
  awk -F '\t' '$1 == "wait" { split($2, w, " "); print w[1]; exit }' "$out/$1/steps.log"
}

# One line per step: label, windows, pastes whose text arrived, why.
expected() {
  case $1 in
    base)
      # Xwayland keeps a session ten minutes for the next client with the same command name.
      if [ "$(wait_seconds base)" -ge 600 ]; then
        echo "base-d 1 2 ten minutes and a half later: Xwayland's session has run out" > "$tmp.d"
      else
        echo "base-d 0 2 only $(wait_seconds base) s later (VOLTIP_REPRO_REUSE_WAIT): Xwayland still has the session" > "$tmp.d"
      fi
      echo "base-a 1 3 the first paste asks"
      echo "base-b 0 2 Voltip restarted: Xwayland reuses the session of a client with the same command name"
      echo "base-c 1 2 started from another path, another command name: a new session"
      cat "$tmp.d"
      echo "base-e 1 2 GNOME Shell restarted: a new Xwayland knows no session of the last one"
      ;;
    fix)
      echo "fix-a 1 3 the first paste asks"
      echo "fix-b 0 2 Voltip restarted: the saved grant"
      echo "fix-c 0 2 started from another path: the saved grant"
      echo "fix-d 0 2 $(wait_seconds fix) s later: the saved grant"
      echo "fix-e 1 2 the grant revoked: GNOME asks once more"
      echo "fix-f 1 0 the window cancelled: nothing pressed, the text left on the clipboard"
      echo "fix-g 1 2 the paste after the refusal asks again"
      echo "fix-h 0 2 GNOME Shell restarted: the saved grant"
      ;;
  esac
}

tmp=$(mktemp)
trap 'rm -f "$tmp" "$tmp.d"' EXIT
base_windows="" base_pastes="" kept_windows=0 kept_pastes=0
for side in "$@"; do
  if [ ! -f "$out/$side/steps.log" ] || [ ! -f "$out/$side/summary.txt" ]; then
    check no "$side: the session did not finish (see $side/session.log)"
    continue
  fi
  expected "$side" > "$tmp"
  while read -r label want_windows want_arrived why; do
    windows=$(field "$side" "$label" windows)
    arrived=$(field "$side" "$label" arrived)
    pastes=$(field "$side" "$label" pastes)
    if [ "$windows" = "$want_windows" ] && [ "$arrived" = "$want_arrived" ]; then r=ok; else r=no; fi
    check "$r" "$label: ${windows:-?} window(s), the text of ${arrived:-?} of ${pastes:-?} pastes arrived (expected $want_windows and $want_arrived): $why"
    case $label in
      fix-[abcdh])
        kept_windows=$((kept_windows + ${windows:-0}))
        kept_pastes=$((kept_pastes + ${pastes:-0}))
        ;;
    esac
  done < "$tmp"

  pastes=$(summary "$side" pastes)
  case $side in
    base)
      base_windows=$(summary base windows)
      base_pastes=$pastes
      # Xwayland's request (liboeffis) has no persist_mode: GNOME shows no "Remember This Selection".
      if [ "$(summary base windows_with_remember)" = 0 ]; then r=ok; else r=no; fi
      check "$r" "base: every window was Xwayland's request, without \"Remember This Selection\" ($(summary base windows_without_remember) of $base_windows)"
      ;;
    fix)
      # Voltip's own request asks to be remembered (persist_mode 2): the box is there, ticked.
      if [ "$(summary fix windows_without_remember)" = 0 ]; then r=ok; else r=no; fi
      check "$r" "fix: every window was Voltip's request, with \"Remember This Selection\" ($(summary fix windows_with_remember) of $(summary fix windows))"
      created=$(summary fix portal_create_session)
      closed=$(summary fix portal_session_close)
      if [ "$created" = "$pastes" ] && [ "$closed" = "$pastes" ]; then r=ok; else r=no; fi
      check "$r" "fix: one portal session per paste, closed after it ($created CreateSession and ${closed:-?} Session.Close for $pastes pastes)"
      case $(results fix fix-f) in
        *RemoteDenied*) r=ok ;;
        *) r=no ;;
      esac
      check "$r" "fix-f: the cancelled paste reported remote_denied ($(results fix fix-f))"
      ;;
  esac
done

if [ "$fail" = 0 ]; then
  for side in "$@"; do
    case $side in
      base) echo "repro-gnome-remote-desktop: PASS: base asked $base_windows times for $base_pastes pastes" ;;
      fix) echo "repro-gnome-remote-desktop: PASS: fix asked $kept_windows time(s) for its $kept_pastes pastes outside the revocation steps" ;;
    esac
  done
else
  echo "repro-gnome-remote-desktop: FAIL: a check above differs from what docs/dictation.md §14.9 describes"
fi
exit "$fail"
