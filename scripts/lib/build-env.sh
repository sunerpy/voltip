# shellcheck shell=bash
# Source from build scripts: exports the compile-time defaults the shells bake in via option_env!
# (VOLTIP_ASR_URL / VOLTIP_ASR_TOKEN / VOLTIP_ASR_MODEL / VOLTIP_REFINE_URL / VOLTIP_REFINE_MODEL /
# VOLTIP_REFINE_API_KEY / VOLTIP_RELAY_URL) from the git-ignored .env.build at the repo root.
# CI sets the same variables from GitHub Environment variables/secrets instead of this file.
# Values already present in the environment win, so a caller can override a single one.
voltip_load_build_env() {
  local file=${VOLTIP_BUILD_ENV:-.env.build}
  [ -f "$file" ] || { echo "build-env: no $file — building without built-in engine defaults (users must configure engines)"; return 0; }
  local line key value
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    key=${line%%=*}; value=${line#*=}
    case "$key" in VOLTIP_*) ;; *) continue ;; esac
    if [ -z "${!key:-}" ]; then export "$key=$value"; fi
  done < "$file"
  echo "build-env: loaded $(grep -cE '^VOLTIP_[A-Z_]+=' "$file") defaults from $file (values not shown)"
}

# One setting from the environment, else from the build env file (empty when neither has it), for
# a script that needs a single value rather than every compile-time default:
#   ssh_spec=$(voltip_build_env_value VOLTIP_WINDOWS_SSH)
voltip_build_env_value() {
  local key=$1 file=${VOLTIP_BUILD_ENV:-.env.build}
  if [ -n "${!key:-}" ]; then printf '%s' "${!key}"; return 0; fi
  [ -f "$file" ] || return 0
  sed -n "s/^$key=//p" "$file" | tail -n 1
}
