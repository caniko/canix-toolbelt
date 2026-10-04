#!/usr/bin/env bash
# Exercise the packaged function, including duplicate/empty inputs and failure.
set -euo pipefail
source "$1"
calls=0
_nix() {
  calls=$((calls + 1))
  args=("$@")
  return "${failure:-0}"
}
_nix_direnv_info() { :; }

# Extract only the patched archival block from the actual installed function.
# This keeps the regression coupled to the package rather than a copied loop.
body=$(declare -f use_flake)
block=${body#*local -A seen_inputs}
block="local -A seen_inputs${block%%_nix_direnv_info*}"
archive_inputs() { eval "$block"; }
flake_inputs="$PWD/inputs"
profile_rc="$PWD/profile.rc"
tmp_profile_rc=renewed
flake_input_paths='["/nix/store/aaaaaaaa-source", "/nix/store/bbbbbbbb-source", "/nix/store/aaaaaaaa-source"]'
archive_inputs
[[ $calls == 1 ]]
[[ $(cat "$profile_rc") == renewed ]]
[[ ${args[*]} == "build --max-jobs 0 --option builders  --out-link $flake_inputs/input -- /nix/store/aaaaaaaa-source /nix/store/bbbbbbbb-source" ]]

flake_input_paths='[]'
archive_inputs
[[ $calls == 1 ]]

# Failure must not be turned into a successful cache renewal.
flake_input_paths='["/nix/store/aaaaaaaa-source"]'
failure=1
printf 'old\n' >"$profile_rc"
if archive_inputs; then
  exit 1
else
  [[ $? == 1 ]]
fi
[[ $(cat "$profile_rc") == old ]]
