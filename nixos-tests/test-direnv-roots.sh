#!/usr/bin/env bash
# Exercise the complete packaged function with deterministic Nix responses.
set -euo pipefail
source "$1"
calls=0
_nix() {
  case "$1" in
  print-dev-env)
    touch "$tmp_profile"
    printf 'renewed\n'
    ;;
  flake)
    [[ $2 == archive ]]
    printf '%s\n' "$input_json"
    return "${archive_failure:-0}"
    ;;
  build)
    calls=$((calls + 1))
    args=("$@")
    return "${root_failure:-0}"
    ;;
  *) return 99 ;;
  esac
}
_nix_direnv_info() { :; }
_nix_direnv_preflight() { :; }
watch_file() { :; }
direnv_layout_dir() { printf '%s\n' "$layout"; }
_nix_argsum_suffix() { :; }
_nix_direnv_watches() {
  local -n watched=$1
  watched=()
}
_nix_clean_old_gcroots() { :; }
_nix_add_gcroot() { touch "$2"; }
_nix_import_env() { cat "$1" >/dev/null; }

layout="$PWD/layout"
mkdir -p "$layout"
profile_rc="$layout/flake-profile.rc"
renew_cache() {
  rm -f "$layout/flake-profile"
  use_flake .
}
input_json='["/nix/store/aaaaaaaa-source", "/nix/store/bbbbbbbb-source", "/nix/store/aaaaaaaa-source"]'
renew_cache
[[ $calls == 1 ]]
[[ $(cat "$profile_rc") == renewed ]]
[[ ${args[*]} == "build --max-jobs 0 --option builders  --out-link $layout/flake-inputs//input -- /nix/store/aaaaaaaa-source /nix/store/bbbbbbbb-source" ]]

input_json='[]'
renew_cache
[[ $calls == 1 ]]
[[ $(cat "$profile_rc") == renewed ]]

# Failures must propagate even when use_flake is called conditionally, and
# neither archive nor root failure may publish the fresh environment cache.
input_json='["/nix/store/aaaaaaaa-source"]'
root_failure=1
printf 'old\n' >"$profile_rc"
if renew_cache; then
  exit 1
else
  [[ $? == 1 ]]
fi
[[ $(cat "$profile_rc") == old ]]
[[ $calls == 2 ]]

root_failure=0
archive_failure=1
if renew_cache; then
  exit 1
else
  [[ $? == 1 ]]
fi
[[ $(cat "$profile_rc") == old ]]
[[ $calls == 2 ]]
