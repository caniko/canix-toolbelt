#!/usr/bin/env bash
# Required gates have their own setup; they do not inherit ci.extra_setup.
set -euo pipefail
test "${RUNNER_ENVIRONMENT:-}" = github-hosted
# Trusted verification runs after Simit's signed-source/archive admission. The
# workflow_run SHA names its default-branch publisher, not the admitted source.
# These gates must keep the source selected by that original producer, including
# signed-tag recovery, while branch/PR/manual qualification stays attempt 1.
case "${GITHUB_EVENT_NAME:-}" in
pull_request)
  test "${GITHUB_RUN_ATTEMPT:-}" = 1
  revision="$(jq -er '.pull_request.head.sha' "$GITHUB_EVENT_PATH")"
  ;;
workflow_run)
  revision="$(jq -er '.workflow_run.head_sha' "$GITHUB_EVENT_PATH")"
  ;;
push)
  if [ "${GITHUB_REF_TYPE:-}" != tag ]; then
    test "${GITHUB_RUN_ATTEMPT:-}" = 1
  fi
  revision="${GITHUB_SHA:?missing hosted source revision}"
  ;;
workflow_dispatch)
  test "${GITHUB_RUN_ATTEMPT:-}" = 1
  revision="${GITHUB_SHA:?missing hosted source revision}"
  ;;
*)
  echo 'Unsupported hosted source event' >&2
  exit 1
  ;;
esac
[[ $revision =~ ^[0-9a-f]{40}$ ]]
git fetch --no-tags origin "$revision"
git checkout --detach "$revision"
test "$(git rev-parse HEAD)" = "$revision"
git show --no-patch --format='checkout=%H%ntree=%T%nparents=%P' HEAD
