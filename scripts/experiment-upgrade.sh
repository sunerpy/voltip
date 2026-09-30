#!/usr/bin/env bash
# Experiment (never merged): manual checklist item 15 on a GitHub Mac with the released packages.
# 0.0.10 runs and stores its identity in the keychain; 0.0.11 replaces it at the same path, as the
# in-app updater does, and must read that identity without a prompt (nobody answers a prompt on a
# runner, so the start would stop at the keychain read). Only the checks' results are printed,
# never the apps' own log lines.
set -euo pipefail
old=${1:-0.0.10}
new=${2:-0.0.11}
case $(uname -m) in arm64) arch=aarch64 ;; x86_64) arch=x64 ;; *) exit 2 ;; esac
work=$(mktemp -d)
cd "$work"
for v in "$old" "$new"; do
	mkdir -p "$v"
	curl -fsSL --retry 3 -o "$v/app.tar.gz" "https://github.com/sunerpy/voltip/releases/download/v$v/Voltip_${v}_${arch}.app.tar.gz"
	tar -xzf "$v/app.tar.gz" -C "$v"
	codesign --verify --strict "$v/Voltip.app"
	echo "$v: $(codesign -d -r- "$v/Voltip.app" 2>&1 | sed -n 's/^designated => //p')"
done
requirement=$(codesign -d -r- "$old/Voltip.app" 2>&1 | sed -n 's/^designated => //p')
if codesign --verify -R="$requirement" "$new/Voltip.app"; then
	echo "ok: $new satisfies $old's designated requirement (what the keychain entries and the TCC grants of $old name)"
else
	echo "::error::$new does not satisfy $old's designated requirement"
	exit 1
fi

# The user's default keychain is a temporary one for this run.
keychain="$work/upgrade.keychain-db"
password=$(openssl rand -hex 16)
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
default=$(security default-keychain -d user | sed 's/[" ]//g')
existing=()
while IFS= read -r line; do
	line=${line//\"/}
	line=${line## }
	line=${line%% }
	[ -n "$line" ] && existing+=("$line")
done < <(security list-keychains -d user)
restore() {
	pkill -f "$work/app/Voltip.app/Contents/MacOS/voltip-desktop" 2>/dev/null || true
	security default-keychain -d user -s "$default" || true
	security list-keychains -d user -s "${existing[@]}" || true
	security delete-keychain "$keychain" 2>/dev/null || true
}
trap restore EXIT
security list-keychains -d user -s "$keychain" "${existing[@]}"
security default-keychain -d user -s "$keychain"

entry="dev.voltip.desktop/voltip.identity.x25519"
# run <version> <log>: start the app at the one install path; true once it has passed its keychain
# read at start-up ("compute devices" comes after the core started with the identity).
run() {
	"$work/app/Voltip.app/Contents/MacOS/voltip-desktop" >"$2" 2>&1 &
	local pid=$! i
	for i in $(seq 90); do
		if grep -q "compute devices" "$2"; then
			echo "ok: $1 started past its keychain read in ${i}s"
			# Let its windows run their permission checks: up to 20 s, until tccd has decided for it.
			for _ in $(seq 10); do
				log show --last 30s --style compact --predicate 'subsystem == "com.apple.TCC"' 2>/dev/null | grep -q "dev\.voltip\.desktop" && break
				sleep 2
			done
			kill "$pid" 2>/dev/null || true
			wait "$pid" 2>/dev/null || true
			return 0
		fi
		kill -0 "$pid" 2>/dev/null || break
		sleep 1
	done
	echo "::error::$1 did not get past its keychain read (still running: $(kill -0 "$pid" 2>/dev/null && echo yes || echo no); SecurityAgent: $(pgrep -x SecurityAgent >/dev/null && echo running || echo not running); ERROR lines: $(grep -c ERROR "$2" || true))"
	kill "$pid" 2>/dev/null || true
	return 1
}

# TCC grants as a user's would be stored for the old version: by bundle id, with its requirement.
csreq -r- -b "$work/req.bin" <<<"$requirement"
hex=$(xxd -p "$work/req.bin" | tr -d '\n')
grant() { # grant <db> <service> [sudo]
	${3:-} sqlite3 "$1" "INSERT OR REPLACE INTO access (service, client, client_type, auth_value, auth_reason, auth_version, csreq, indirect_object_identifier, flags, last_modified) VALUES ('$2', 'dev.voltip.desktop', 0, 2, 4, 1, X'$hex', 'UNUSED', 0, $(date +%s));" 2>&1
}
if out=$(grant "/Library/Application Support/com.apple.TCC/TCC.db" kTCCServiceAccessibility sudo); then echo "ok: Accessibility granted to $old (csreq = its requirement)"; else echo "note: system TCC.db: $out"; fi
if out=$(grant "$HOME/Library/Application Support/com.apple.TCC/TCC.db" kTCCServiceMicrophone); then echo "ok: Microphone granted to $old (csreq = its requirement)"; else echo "note: user TCC.db not writable here: $out"; fi
# tcc <since>: what tccd decided for Voltip since then (no app output): each request's service and
# result, joined by msgID (authValue 2 = allowed, 0 = denied).
tcc() {
	log show --start "$1" --style compact --predicate 'subsystem == "com.apple.TCC"' >"$work/tcc.log" 2>/dev/null || true
	grep -E "dev\.voltip\.desktop" "$work/tcc.log" | grep -oE "msgID=[0-9]+\.[0-9]+" | sort -u | while read -r id; do
		grep -F "$id," "$work/tcc.log" | grep -oE "service=kTCCService[A-Za-z]+|authValue=[0-9]+" | sort -u | tr '\n' ' '
		echo
	done | grep -v "^$" | sort | uniq -c || true
}

mkdir -p app
cp -R "$old/Voltip.app" app/
since_old=$(date "+%Y-%m-%d %H:%M:%S")
run "$old" old.log
echo "tccd for $old:"; tcc "$since_old"
security find-generic-password -s "$entry" "$keychain" >/dev/null && echo "ok: $old stored $entry"
security dump-keychain -a "$keychain" 2>/dev/null | awk -v s="\"svce\"<blob>=\"$entry\"" 'index($0, s) {f=1} f && /requirement:/ {print "  trusted: " $0; exit}'
# The update: the same path, the new bundle.
rm -rf app/Voltip.app
cp -R "$new/Voltip.app" app/
since_new=$(date "+%Y-%m-%d %H:%M:%S")
run "$new" new.log
echo "tccd for $new:"; tcc "$since_new"
echo "ok: updating $old to $new kept the keychain entry readable without a prompt"
