#!/usr/bin/env bash
# Additional tests for deploy/server scripts (#1084).
# Specifically verifies:
#   M1: install.sh preserves rotated multi-line pii.key byte for byte.
#   M2: update.sh missing-key refusal prints a generation hint whose command
#       yields a key format accepted by PiiKeys (k1: + base64 of 32 bytes).
set -euo pipefail
# shellcheck shell=bash

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ROOT="$(mktemp -d /var/tmp/xc-scripts-more-test.XXXXXX)"
trap 'rm -rf "$ROOT"' EXIT
LOGS="$ROOT/logs"
STUBS="$ROOT/stubs"
mkdir -p "$LOGS" "$STUBS"

PASS=0
FAILURES=0
CASE_FAIL=""

ck() {
  local what="$1"
  shift
  "$@" || CASE_FAIL="${CASE_FAIL:+$CASE_FAIL; }$what"
}

ckeq() {
  [ "$2" = "$3" ] || CASE_FAIL="${CASE_FAIL:+$CASE_FAIL; }$1 (expected '$2', got '$3')"
}

absent() {
  local pattern="$1" file="$2"
  [ -f "$file" ] || return 0
  ! grep -Eq "$pattern" "$file"
}

report() {
  if [ -z "$CASE_FAIL" ]; then
    echo "ok   - $1: $2"
    PASS=$((PASS + 1))
  else
    echo "FAIL - $1: $2 — $CASE_FAIL"
    FAILURES=$((FAILURES + 1))
    log="$LOGS/$(echo "$1" | tr '[:upper:]' '[:lower:]').out"
    [ -f "$log" ] && cat "$log" >&2 || true
  fi
  CASE_FAIL=""
}

write_stub() {
  cat >"$STUBS/$1"
  chmod 755 "$STUBS/$1"
}

# sudo stub: maps /etc/systemd/system/ into scratch, tracks chown/install
write_stub sudo <<'STUB'
#!/usr/bin/env bash
printf 'sudo %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
rec="${XC_TEST_OWNERS:?}"
map() {
  case "$1" in
    /etc/systemd/system/*) printf '%s' "${XC_TEST_SYSTEMD:?}/${1#/etc/systemd/system/}" ;;
    /etc/systemd/system) printf '%s' "${XC_TEST_SYSTEMD:?}" ;;
    *) printf '%s' "$1" ;;
  esac
}
args=()
for a in "$@"; do args+=("$(map "$a")"); done
set -- "${args[@]}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -u) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
cmd="$1"
shift
case "$cmd" in
  chown | chgrp)
    who=""
    for a in "$@"; do
      case "$a" in
        -*) ;;
        *)
          if [ -z "$who" ]; then
            who="$a"
          else
            if [ "$cmd" = chown ]; then
              printf '%s\t%s %s\n' "$a" "${who%%:*}" "${who#*:}" >>"$rec"
            else
              printf '%s\troot %s\n' "$a" "$who" >>"$rec"
            fi
          fi
          ;;
      esac
    done
    exit 0
    ;;
  install)
    o=root g=root skip="" argv=()
    for a in "$@"; do
      if [ -n "$skip" ]; then
        case "$skip" in o) o="$a" ;; g) g="$a" ;; esac
        skip=""
        continue
      fi
      case "$a" in
        -o) skip=o ;;
        -g) skip=g ;;
        *) argv+=("$a") ;;
      esac
    done
    install "${argv[@]}" || exit
    printf '%s\t%s %s\n' "${argv[-1]}" "$o" "$g" >>"$rec"
    exit 0
    ;;
  mkdir)
    m=755 skip="" argv=()
    for a in "$@"; do
      if [ -n "$skip" ]; then
        m="$a"
        skip=""
        continue
      fi
      case "$a" in
        -m) skip=m ;;
        *) argv+=("$a") ;;
      esac
    done
    mkdir -m "$m" "${argv[@]}" || exit
    for d in "${argv[@]}"; do
      printf '%s\troot root\n' "$d" >>"$rec"
    done
    exit 0
    ;;
esac
exec "$cmd" "$@"
STUB

write_stub stat <<'STUB'
#!/usr/bin/env bash
real=/usr/bin/stat
[ -x "$real" ] || real=/bin/stat
if [ "$#" = 3 ] && [ "$1" = -c ] && [ "$2" = '%U %G %a' ]; then
  mode="$("$real" -c '%a' "$3")" || exit
  og="$(awk -F'\t' -v p="$3" '$1 == p { v = $2 } END { print v }' "${XC_TEST_OWNERS:?}")"
  printf '%s %s\n' "${og:-root root}" "$mode"
  exit 0
fi
exec "$real" "$@"
STUB

write_stub curl <<'STUB'
#!/usr/bin/env bash
out="" take=0 url=""
for a in "$@"; do
  if [ "$take" = 1 ]; then
    out="$a"
    take=0
    continue
  fi
  case "$a" in
    -o) take=1 ;;
    http://* | https://*) url="$a" ;;
  esac
done
body=""
case "$url" in
  */health/ready) body='{"version":"1.0.0","build":"0123456789ab","builtAt":"2026-10-07T00:00:00Z","channel":"staging","database":true,"ui":"built"}' ;;
esac
[ -n "$out" ] && printf '%s' "$body" >"$out"
printf '200'
STUB

for s in useradd systemctl journalctl ss createdb dropdb pg_dump; do
  write_stub "$s" <<STUB
#!/usr/bin/env bash
printf '$s %s\n' "\$*" >>"\${XC_TEST_LOG:?}/calls.log"
out="" take=0
for a in "\$@"; do
  if [ "\$take" = 1 ]; then
    out="\$a"
    take=0
    continue
  fi
  case "\$a" in
    -f) take=1 ;;
    --file=*) out="\${a#--file=}" ;;
    -f*) out="\${a#-f}" ;;
  esac
done
if [ -n "\$out" ] && [ "\$out" != "-" ]; then
  printf -- '-- stub %s\n' '$s' >"\$out"
elif [ '$s' = "pg_dump" ]; then
  printf -- '-- stub pg_dump\n'
fi
exit 0
STUB
done

write_stub psql <<'STUB'
#!/usr/bin/env bash
printf 'psql %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
cat >/dev/null
exit 0
STUB

write_stub node <<'STUB'
#!/usr/bin/env bash
mkdir -p "$3"
printf '<!doctype html>\n' >"$3/index.html"
STUB

make_stage() {
  local d="$1"
  mkdir -p "$d/configuration" "$d/migrations"
  printf '#!/bin/sh\necho not-a-real-binary\n' >"$d/xchange"
  chmod 755 "$d/xchange"
  printf '# base\n' >"$d/configuration/base.yaml"
  printf '// pages extractor\n' >"$d/extract-v2-pages.mjs"
  printf '0123456789ab\n' >"$d/BUILD"
  (
    cd "$d"
    sha256sum xchange configuration/base.yaml extract-v2-pages.mjs BUILD >SHA256SUMS
  )
  chmod 700 "$d"
}

# ---- M1: rotated multi-line pii.key preserved by install.sh -----------------
m1="$ROOT/m1"
mkdir -p "$m1/log" "$m1/systemd" "$m1/v2bundle" "$m1/etc/xc-v3"
chmod 750 "$m1/etc/xc-v3"
: >"$m1/owners"
printf '%s\troot %s\n' "$m1/etc/xc-v3" "xc-v3" >>"$m1/owners"
make_stage "$m1/stage"
# Pre-create rotated key with k1, k2 and comments
printf '%s\n' \
  "# pii key rotation test" \
  "k1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=" \
  "k2:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=" >"$m1/etc/xc-v3/pii.key"
chmod 640 "$m1/etc/xc-v3/pii.key"
printf '%s\troot %s\n' "$m1/etc/xc-v3/pii.key" "xc-v3" >>"$m1/owners"
cp -p "$m1/etc/xc-v3/pii.key" "$m1/key.expected"

set +e
OUT="$(PATH="$STUBS:$PATH" \
  XC_TEST_LOG="$m1/log" XC_TEST_SYSTEMD="$m1/systemd" XC_TEST_OWNERS="$m1/owners" \
  SERVICE_NAME="xc-v3" SERVICE_USER="xc-v3" \
  INSTALL_DIR="$m1/opt/xc-v3" ETC_DIR="$m1/etc/xc-v3" \
  UPLOADS_DIR="$m1/var/lib/xc-v3/uploads" \
  V2_BUNDLE_DIR="$m1/v2bundle" STAGE="$m1/stage" \
  SMOKE_WAIT_SECS=0 \
  bash "$HERE/install.sh" </dev/null 2>&1)"
RC=$?
set -e
printf '%s\n' "$OUT" >"$LOGS/m1.out"

ck "install.sh exited 0" test "$RC" -eq 0
ck "rotated key was kept byte for byte" cmp -s "$m1/key.expected" "$m1/etc/xc-v3/pii.key"
ck "output names pii.key kept" grep -qsF "$m1/etc/xc-v3/pii.key" "$LOGS/m1.out"
report M1 "install.sh keeps existing rotated multi-line pii.key byte for byte"

# ---- M2: update.sh printed hint produces a key format PiiKeys accepts --------
# When the PII key is missing, update.sh refuses and prints an openssl hint.
# That hint must produce a single k1: line followed by 32 base64-encoded bytes
# (api/crates/xc-kernel/src/pii/key_file.rs).
m2="$ROOT/m2"
mkdir -p "$m2/log" "$m2/systemd" "$m2/opt/xc-v3/configuration" "$m2/var/lib/xc-v3/uploads" "$m2/etc/xc-v3"
chmod 750 "$m2/etc/xc-v3"
: >"$m2/owners"
printf '%s\troot %s\n' "$m2/etc/xc-v3" "xc-v3" >>"$m2/owners"
make_stage "$m2/stage"
printf 'old' >"$m2/opt/xc-v3/xchange"
chmod 755 "$m2/opt/xc-v3/xchange"
printf '# base\n' >"$m2/opt/xc-v3/configuration/base.yaml"
printf 'APP_FILES__DIR=%s\n' "$m2/var/lib/xc-v3/uploads" >"$m2/etc/xc-v3/env"
printf '%s\troot %s\n' "$m2/etc/xc-v3/env" "xc-v3" >>"$m2/owners"
touch "$m2/systemd/xc-v3.service"

set +e
OUT="$(PATH="$STUBS:$PATH" \
  XC_TEST_LOG="$m2/log" XC_TEST_SYSTEMD="$m2/systemd" XC_TEST_OWNERS="$m2/owners" \
  SERVICE_NAME="xc-v3" SERVICE_USER="xc-v3" \
  INSTALL_DIR="$m2/opt/xc-v3" ETC_DIR="$m2/etc/xc-v3" \
  UPLOADS_DIR="$m2/var/lib/xc-v3/uploads" \
  STAGE="$m2/stage" SMOKE_WAIT_SECS=0 \
  bash "$HERE/update.sh" </dev/null 2>&1)"
RC=$?
set -e
printf '%s\n' "$OUT" >"$LOGS/m2.out"

ck "update.sh exited non-zero" test "$RC" -ne 0
ck "refusal names key file" grep -qsF "$m2/etc/xc-v3/pii.key" "$LOGS/m2.out"
ck "refusal hint contains openssl generator command" grep -qsF "{ printf 'k1:'; openssl rand -base64 32; }" "$LOGS/m2.out"
{ printf 'k1:'; openssl rand -base64 32; } >"$m2/hint_key"
HINT_LINES="$(wc -l <"$m2/hint_key" | tr -d ' ')"
ckeq "hint generates exactly 1 line" 1 "$HINT_LINES"
ck "hint matches key_id:base64(32 bytes) format" grep -Eq '^[a-zA-Z0-9_-]+:[A-Za-z0-9+/]{43}=$' "$m2/hint_key"
HINT_BYTE_LEN="$(sed 's/^k1://' "$m2/hint_key" | openssl base64 -d 2>/dev/null | wc -c | tr -d ' ')"
ckeq "hint base64 decodes to 32 bytes" 32 "$HINT_BYTE_LEN"
report M2 "update.sh printed refusal hint command yields a key PiiKeys accepts"

echo "# $PASS passed, $FAILURES failed"
[ "$FAILURES" -eq 0 ]
