# shellcheck shell=bash
# Source after voltip_load_build_env. A distributable package must carry the built-in default
# engines (the product promise: dictation works out of the box). Refuse to package when any of the
# compile-time defaults is missing, unless the caller explicitly opts into an engine-less build.
#
#   VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1   build anyway (users configure engines themselves)
voltip_require_builtin_engines() {
  if [ "${VOLTIP_ALLOW_NO_BUILTIN_ENGINES:-0}" = "1" ]; then
    echo "require-builtin-engines: skipped (VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1) — this package has no built-in engines"
    return 0
  fi
  local missing=0 name
  for name in VOLTIP_ASR_URL VOLTIP_ASR_TOKEN VOLTIP_ASR_MODEL VOLTIP_REFINE_URL VOLTIP_REFINE_API_KEY VOLTIP_REFINE_MODEL VOLTIP_RELAY_URL; do
    if [ -z "${!name:-}" ]; then
      echo "require-builtin-engines: $name is empty" >&2
      missing=1
    fi
  done
  if [ "$missing" = 1 ]; then
    echo "require-builtin-engines: refusing to package without the built-in engine defaults (fill .env.build or the GitHub secrets; set VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1 to build an engine-less package on purpose)" >&2
    return 1
  fi
  echo "require-builtin-engines: ASR, refine and relay defaults present (values not shown)"
}
