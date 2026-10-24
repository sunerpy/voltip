#!/bin/sh
# Run by the nginx image before nginx starts: turns /etc/voltip/tokens (one token a line, '#' starts
# a comment) into the map the gateway checks every request against. A request gets in with
# "Authorization: Bearer <token>" for one of them. The log names a token by its line ("t3"), never
# by its value. Edit the file and restart the gateway to add or revoke a token.
set -eu

src=/etc/voltip/tokens
out=/etc/nginx/voltip/tokens.map
mkdir -p "${out%/*}"
umask 077
: >"$out"

if [ ! -r "$src" ]; then
	echo "voltip: no tokens file at $src; every request will be refused" >&2
	exit 0
fi

line=0
kept=0
while IFS= read -r raw || [ -n "$raw" ]; do
	line=$((line + 1))
	token=$(printf '%s' "$raw" | sed 's/#.*//; s/^[[:space:]]*//; s/[[:space:]]*$//')
	[ -n "$token" ] || continue
	case "$token" in
	*[!A-Za-z0-9._~+/=-]*)
		echo "voltip: line $line of the tokens file has a character a token cannot have; skipped" >&2
		continue
		;;
	esac
	if [ "${#token}" -lt 16 ]; then
		echo "voltip: line $line of the tokens file is shorter than 16 characters; skipped" >&2
		continue
	fi
	printf '"Bearer %s" t%d;\n' "$token" "$line" >>"$out"
	kept=$((kept + 1))
done <"$src"

if [ "$kept" -eq 0 ]; then
	echo "voltip: no usable token in the tokens file; every request will be refused" >&2
else
	echo "voltip: $kept access token(s) loaded" >&2
fi
