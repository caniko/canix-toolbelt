#!/usr/bin/env bash
# Required gates have their own setup; they do not inherit ci.extra_setup.
set -euo pipefail
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
test "${GITHUB_RUN_ATTEMPT:-}" = 1
if [ "${GITHUB_EVENT_NAME:-}" = pull_request ]; then
  revision="$(jq -er '.pull_request.head.sha' "$GITHUB_EVENT_PATH")"
else
  revision="${GITHUB_SHA:?missing hosted source revision}"
fi
[[ "$revision" =~ ^[0-9a-f]{40}$ ]]
git fetch --no-tags origin "$revision"
git checkout --detach "$revision"
test "$(git rev-parse HEAD)" = "$revision"
git show --no-patch --format='checkout=%H%ntree=%T%nparents=%P' HEAD
