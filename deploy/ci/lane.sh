#!/usr/bin/env bash
# Which deploy lane a release-workflow run belongs to (#422).
#
#   GITHUB_EVENT_NAME=<event> lane.sh <repository> <ref>
#
# Prints one word and exits 0:
#   prod        push to release/prod on the org repo           -> do-sgp1
#   staging     push to deploy/staging on the personal mirror  -> staging on Dell
#   build-only  a manual run (workflow_dispatch) on either repo -> build, deploy nowhere
# Anything else prints nothing and exits 1 with a GitHub error annotation.
#
# The lane is decided by the pair (repository, branch), never by the branch
# alone: the same workflow file runs on both sides (the mirror gets a copy), so
# an org push to deploy/staging or a mirror push to release/prod must not deploy
# (fleet rule "Deploy lanes"; reference jh-api release-image.yml).
# The logic lives here rather than in a `run:` block so it can be tested without
# merging: deploy/ci/lane.test.sh.
set -u

ORG_REPO="108-Plaza/xchange"
DEV_REPO="yuthstyle88/xchange-dev"

refuse() {
  echo "::error::$1 — deploy lanes are ${ORG_REPO} release/prod (prod) and ${DEV_REPO} deploy/staging (staging); a manual run builds only" >&2
  exit 1
}

[ "$#" -eq 2 ] || refuse "usage: lane.sh <repository> <ref>"
repo="$1"
ref="$2"
event="${GITHUB_EVENT_NAME:-}"

case "$repo" in
  "$ORG_REPO" | "$DEV_REPO") ;;
  *) refuse "${repo} is not a repository of this workflow" ;;
esac

case "$event" in
  workflow_dispatch)
    echo build-only
    ;;
  push)
    case "${repo}:${ref}" in
      "${ORG_REPO}:refs/heads/release/prod") echo prod ;;
      "${DEV_REPO}:refs/heads/deploy/staging") echo staging ;;
      *) refuse "${ref} is not a deploy lane of ${repo}" ;;
    esac
    ;;
  *)
    refuse "event '${event}' does not deploy"
    ;;
esac
