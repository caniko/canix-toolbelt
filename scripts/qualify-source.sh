#!/usr/bin/env bash
set -euo pipefail
printf 'workflow=%s run=%s job=%s head=%s base=%s\n' \
  "${GITHUB_WORKFLOW_REF:-}" "${GITHUB_RUN_ID:-}" "${GITHUB_JOB:-}" \
  "${GITHUB_SHA:-}" "${GITHUB_BASE_REF:-}"
git show --no-patch --format='checkout=%H%ntree=%T%nparents=%P' HEAD
nix --version
for system in x86_64-linux aarch64-linux; do
  # Force every declared check derivation on each supported system. Realization
  # remains in the existing full flake gate and exact-installable matrix.
  nix eval --json --no-write-lock-file ".#checks.$system" \
    --apply 'checks: builtins.mapAttrs (_: check: check.drvPath) checks'
done
