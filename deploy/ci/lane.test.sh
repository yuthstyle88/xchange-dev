#!/usr/bin/env bash
# Tests for deploy/ci/lane.sh: which deploy lane a run belongs to (#422).
# Run: bash deploy/ci/lane.test.sh   (exit 0 = every case passed)
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LANE="$HERE/lane.sh"
fails=0

# expect <event> <repository> <ref> <exit> <stdout>
expect() {
  local event="$1" repo="$2" ref="$3" want_rc="$4" want_out="$5" out rc
  out="$(GITHUB_EVENT_NAME="$event" bash "$LANE" "$repo" "$ref" 2>/dev/null)"
  rc=$?
  if [ "$rc" != "$want_rc" ] || [ "$out" != "$want_out" ]; then
    echo "FAIL: $event $repo $ref -> rc=$rc out='$out' (want rc=$want_rc out='$want_out')"
    fails=$((fails + 1))
  else
    echo "ok:   $event $repo $ref -> ${out:-<refused>}"
  fi
}

ORG=108-Plaza/xchange
DEV=yuthstyle88/xchange-dev

# the two lanes
expect push "$ORG" refs/heads/release/prod   0 prod
expect push "$DEV" refs/heads/deploy/staging 0 staging

# crossed lanes and other branches are refused before anything is built
expect push "$ORG" refs/heads/deploy/staging 1 ""
expect push "$DEV" refs/heads/release/prod   1 ""
expect push "$ORG" refs/heads/main           1 ""
expect push "$DEV" refs/heads/main           1 ""
expect push "$ORG" refs/heads/release/prod-x 1 ""
expect push "$ORG" refs/tags/release/prod    1 ""

# another repository (a fork, a renamed copy) never deploys, whatever the branch
expect push someone/xchange refs/heads/release/prod   1 ""
expect push 108-Plaza/xchange-dev refs/heads/deploy/staging 1 ""

# a manual run builds on either side and deploys nowhere
expect workflow_dispatch "$ORG" refs/heads/main           0 build-only
expect workflow_dispatch "$DEV" refs/heads/deploy/staging 0 build-only
expect workflow_dispatch "$ORG" refs/heads/release/prod   0 build-only
expect workflow_dispatch someone/xchange refs/heads/main  1 ""

# any other event is refused
expect pull_request "$ORG" refs/heads/release/prod 1 ""
expect schedule     "$ORG" refs/heads/release/prod 1 ""

# a refusal names the allowed pairs on stderr, as a GitHub error annotation
err="$(GITHUB_EVENT_NAME=push bash "$LANE" "$ORG" refs/heads/main 2>&1 >/dev/null)"
case "$err" in
  *"::error::"*"108-Plaza/xchange"*"release/prod"*"yuthstyle88/xchange-dev"*"deploy/staging"*)
    echo "ok:   refusal names both allowed pairs" ;;
  *) echo "FAIL: refusal message: $err"; fails=$((fails + 1)) ;;
esac

# missing arguments are refused, not read as a lane
out="$(GITHUB_EVENT_NAME=push bash "$LANE" 2>/dev/null)"; rc=$?
if [ "$rc" = 0 ] || [ -n "$out" ]; then echo "FAIL: no arguments -> rc=$rc out='$out'"; fails=$((fails + 1)); else echo "ok:   no arguments refused"; fi

# an unset event is refused (the workflow always sets it)
out="$(env -u GITHUB_EVENT_NAME bash "$LANE" "$ORG" refs/heads/release/prod 2>/dev/null)"; rc=$?
if [ "$rc" = 0 ] || [ -n "$out" ]; then echo "FAIL: unset event -> rc=$rc out='$out'"; fails=$((fails + 1)); else echo "ok:   unset event refused"; fi

if [ "$fails" -ne 0 ]; then
  echo "$fails case(s) failed"
  exit 1
fi
echo "all cases passed"
