#!/usr/bin/env bash
# Namespace admission for this disposable hosted qualification job only.
set -euo pipefail
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
test "${RUNNER_OS:-}" = Linux
# Ubuntu's path-scoped AppArmor allowance does not cover Nix-store Bubblewrap.
sudo -n sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
exec nix develop --max-jobs 1 --cores 2 .#roborev-worker -c bash scripts/roborev-worker-regressions.sh
