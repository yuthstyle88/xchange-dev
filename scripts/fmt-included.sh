#!/usr/bin/env bash
# Gate that checks formatting on changed .rs files pulled in via include!(concat!(env!("OUT_DIR"), ...)) (#1474).
set -euo pipefail

BASE=${1:-}
if [ -z "$BASE" ] || ! git rev-parse --verify --quiet "$BASE^{commit}" >/dev/null 2>&1; then
  echo "fmt-included: base ref '${BASE:-<missing>}' does not resolve" >&2
  exit 1
fi

KNOWN=(
  api/src/presentation/dto/v3
  api/src/presentation/http/v3/views
  api/src/presentation/http/v3/actions
  api/src/presentation/http/v3/commands
  api/src/bootstrap/outbox_handlers
  api/src/bootstrap/exchange_effects
  api/src/bootstrap/approval_subjects
  api/crates/xc-auth/src/domain/policy/areas
  api/crates/xc-cash/src/application/reconcile/checks
  api/crates/xc-notify/src/templates
)

is_known() {
  local want="$1" k
  for k in "${KNOWN[@]}"; do
    [ "$k" = "$want" ] && return 0
  done
  return 1
}

status=0
for b in api/build.rs api/crates/*/build.rs; do
  [ -f "$b" ] || continue
  crate_dir=$(dirname "$b")
  while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    folder="$crate_dir/$rel"
    if ! is_known "$folder"; then
      echo "fmt-included: unknown include! folder in $b: $folder" >&2
      status=1
    fi
  done < <(sed -nE 's/.*Path::new\(.*\)\.join\("(src\/[^"]+)"\).*/\1/p' "$b")
done
[ "$status" -eq 0 ] || exit 1

while IFS= read -r f; do
  case "$f" in
    *.rs) ;;
    *) continue ;;
  esac
  [ -f "$f" ] || continue
  in_scope=0
  for k in "${KNOWN[@]}"; do
    case "$f" in
      "$k"/*) in_scope=1; break ;;
    esac
  done
  [ "$in_scope" -eq 1 ] || continue
  if ! rustfmt --edition 2024 --check "$f"; then
    echo "fmt-included: unformatted file: $f" >&2
    status=1
  fi
done < <(git diff --name-only --diff-filter=d "$BASE...HEAD")

exit "$status"
