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
    [[ $(cat "$profile_rc") == old ]]
    if [[ ${REAL_ROOTS:-0} == 1 ]]; then
      "$NIX_ROOT_BINARY" "$@"
      return
    fi
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
if [[ ${REAL_ROOTS:-0} == 1 ]]; then
  first=$("$NIX_STORE_BINARY" --add "$1")
  second=$("$NIX_STORE_BINARY" --add "$0")
  input_json="[\"$first\", \"$second\", \"$first\"]"
fi
printf 'old\n' >"$profile_rc"
renew_cache
[[ $calls == 1 ]]
[[ $(cat "$profile_rc") == renewed ]]
[[ ${args[0]} == build && ${args[1]} == --max-jobs && ${args[2]} == 0 ]]
[[ ${args[3]} == --option && ${args[4]} == builders && ${args[5]} == "" ]]
[[ ${args[6]} == --out-link && ${args[7]} == "$layout/flake-inputs//input-"* && ${args[8]} == -- ]]
[[ ${#args[@]} == 11 ]]
if [[ ${REAL_ROOTS:-0} == 1 ]]; then
  [[ ${args[9]} == "$first" && ${args[10]} == "$second" ]]
  [[ $(readlink "${args[7]}") == "$first" ]]
  [[ $(readlink "${args[7]}-1") == "$second" ]]
  "$NIX_STORE_BINARY" --query --roots "$first" | grep -F "${args[7]}"
  "$NIX_STORE_BINARY" --query --roots "$second" | grep -F "${args[7]}-1"
else
  [[ ${args[9]} == /nix/store/aaaaaaaa-source && ${args[10]} == /nix/store/bbbbbbbb-source ]]
fi

input_json='[]'
printf 'old\n' >"$profile_rc"
renew_cache
[[ $calls == 1 ]]
[[ $(cat "$profile_rc") == renewed ]]
[[ -z $(find "$layout/flake-inputs" -type l -print) ]]

# Failures must propagate even when use_flake is called conditionally, and
# neither archive nor root failure may publish the fresh environment cache.
input_json='["/nix/store/aaaaaaaa-source"]'
root_failure=1
REAL_ROOTS=0
printf 'old\n' >"$profile_rc"
if renew_cache; then
  exit 1
else
  [[ $? == 1 ]]
fi
[[ $(cat "$profile_rc") == old ]]
[[ $calls == 2 ]]
printf 'Packaged renewal: duplicates, empty input, conditional archive/root failures, previous-cache retention passed\n'

root_failure=0
archive_failure=1
if renew_cache; then
  exit 1
else
  [[ $? == 1 ]]
fi
[[ $(cat "$profile_rc") == old ]]
[[ $calls == 2 ]]
