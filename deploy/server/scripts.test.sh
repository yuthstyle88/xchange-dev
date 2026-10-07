#!/usr/bin/env bash
# Self-test of the deploy/server scripts (#232). Runs on Linux only (GNU
# stat -c, sha256sum) as a non-root user: every root-only command goes
# through a stub, so it needs no root, no systemd, no PostgreSQL.
#
# Cases are the rows T1-T6 of the ticket's behaviour table:
#   T1   a first install that refuses a directory must not have created the
#        service user (full install.sh run with stubs; the useradd stub's
#        log must stay empty)
#   T2/T2b/T2c/T2d  check_stage must require every named file and every
#        staged configuration/*.yaml to be listed in SHA256SUMS by exact
#        name (old/xchange, xchange.bak and "old xchange" do not count;
#        only a binary-mode "hash *name" line does besides the exact one)
#   T3/T3b/T3c  replace_binary: differs -> .prev kept + BIN_SAVED=1; same
#        again -> nothing touched; none installed -> plain install
#   T4/T4b  replace_configuration: two-way — a yaml the stage no longer
#        carries is removed (kept in configuration.prev); identical ->
#        nothing touched, old .prev kept
#   T5   warn_if_unreachable: one WARNING naming the first ancestor the
#        service user cannot enter, and return 0 either way
#   T6   ensure_dir on an existing directory of another mode still refuses
#        and changes nothing (today's behaviour, kept)
# Nothing here touches a host: all paths live under a /var/tmp scratch root
# (not /tmp or /home: check_own_path refuses those).
set -euo pipefail
# shellcheck shell=bash

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ROOT="$(mktemp -d /var/tmp/xc-scripts-test.XXXXXX)"
trap 'rm -rf "$ROOT"' EXIT
LOGS="$ROOT/logs"
BIN_STUBS="$ROOT/stubs"
mkdir -m 700 "$LOGS" "$BIN_STUBS"

PASS=0
FAILURES=0
CASE_FAIL=""

# ck <what> <command...> — records <what> when the command fails.
ck() {
  local what="$1"
  shift
  "$@" || CASE_FAIL="${CASE_FAIL:+$CASE_FAIL; }$what"
}

# ckeq <what> <expected> <actual>
ckeq() {
  [ "$2" = "$3" ] || CASE_FAIL="${CASE_FAIL:+$CASE_FAIL; }$1 (expected '$2', got '$3')"
}

# report <id> <description>
report() {
  if [ -z "$CASE_FAIL" ]; then
    echo "ok   - $1: $2"
    PASS=$((PASS + 1))
  else
    echo "FAIL - $1: $2 — $CASE_FAIL"
    FAILURES=$((FAILURES + 1))
  fi
  CASE_FAIL=""
}

write_stub() {
  cat >"$BIN_STUBS/$1"
  chmod 755 "$BIN_STUBS/$1"
}

# ---- stubs ------------------------------------------------------------------
# sudo: runs the command as the caller (no privilege), dropping the parts
# that would need root: sudo's -u <user>, install's -o/-g <owner>, and
# chown entirely.
write_stub sudo <<'STUB'
#!/usr/bin/env bash
skip=0
argv=()
for a in "$@"; do
  if [ "$skip" = 1 ]; then
    skip=0
    continue
  fi
  case "$a" in
    -u | -o | -g) skip=1 ;;
    chown) exit 0 ;;
    *) argv+=("$a") ;;
  esac
done
exec "${argv[@]}"
STUB

write_stub useradd <<'STUB'
#!/usr/bin/env bash
echo "useradd $*" >>"${XC_TEST_LOG:?}/useradd.log"
STUB

write_stub systemctl <<'STUB'
#!/usr/bin/env bash
echo "systemctl $*" >>"${XC_TEST_LOG:?}/systemctl.log"
exit 0
STUB

write_stub journalctl <<'STUB'
#!/usr/bin/env bash
exit 0
STUB

write_stub psql <<'STUB'
#!/usr/bin/env bash
exit 0
STUB

write_stub ss <<'STUB'
#!/usr/bin/env bash
exit 0
STUB

write_stub curl <<'STUB'
#!/usr/bin/env bash
exit 0
STUB

export XC_TEST_LOG="$LOGS"

# ---- helpers ----------------------------------------------------------------
# run_fn <STAGE> <INSTALL_DIR> <XC_PRE> <function> [args...]
# Runs <function> from the common.sh under test through a runner that
# sources it (and $XC_PRE after it, for test overrides); OUT/RC hold the
# output and exit status. The runner prints a trailing RCFN/BIN_SAVED/
# CONFIG_SAVED block — absent when the function died.
RUNNER="$ROOT/run_fn.sh"
cat >"$RUNNER" <<'RUNNER'
#!/usr/bin/env bash
source "$XC_COMMON"
[ -n "${XC_PRE:-}" ] && source "$XC_PRE"
fn="$1"
shift
set +e
"$fn" "$@"
rc=$?
set -e
printf 'RCFN=%s\nBIN_SAVED=%s\nCONFIG_SAVED=%s\n' "$rc" "${BIN_SAVED:-unset}" "${CONFIG_SAVED:-unset}"
RUNNER
chmod 755 "$RUNNER"

run_fn() {
  RC_STAGE="$1"
  RC_INSTALL="$2"
  RC_PRE="$3"
  RC_FN="$4"
  shift 4
  set +e
  OUT="$(PATH="$BIN_STUBS:$PATH" XC_COMMON="$HERE/common.sh" \
    STAGE="$RC_STAGE" INSTALL_DIR="$RC_INSTALL" XC_PRE="$RC_PRE" \
    bash "$RUNNER" "$RC_FN" "$@" 2>&1)"
  RC=$?
  set -e
}

# A one-binary-one-yaml stage whose SHA256SUMS lists everything in it.
make_stage() { # <dir> [extra files made and listed...]
  local d="$1"
  shift
  mkdir -p "$d/configuration"
  printf '#!/bin/sh\necho not-a-real-binary\n' >"$d/xchange"
  chmod 755 "$d/xchange"
  printf '# base\n' >"$d/configuration/base.yaml"
  local f
  for f in "$@"; do
    : >>"$d/$f"
  done
  (
    cd "$d"
    sha256sum xchange configuration/base.yaml "$@" >SHA256SUMS
  )
  chmod 700 "$d"
}

# ---- T1: a refused first install leaves no user behind ----------------------
t1="$ROOT/t1"
T1_SERVICE=xc232n
T1_USER=xc232u
mkdir -p "$t1/stage/configuration" "$t1/var/lib/$T1_SERVICE/uploads" "$t1/v2bundle"
cp "$HERE/install.sh" "$HERE/common.sh" "$HERE/xc-v3.service" "$HERE/smoke.sh" "$t1/stage/"
printf '#!/bin/sh\necho not-a-real-binary\n' >"$t1/stage/xchange"
chmod 755 "$t1/stage/xchange"
printf '// pages extractor stub\n' >"$t1/stage/extract-v2-pages.mjs"
printf '# base\n' >"$t1/stage/configuration/base.yaml"
(
  cd "$t1/stage"
  sha256sum xchange extract-v2-pages.mjs configuration/base.yaml >SHA256SUMS
)
chmod 700 "$t1/stage"
# the uploads directory already exists, with another owner and mode
chmod 755 "$t1/var/lib/$T1_SERVICE/uploads"
set +e
OUT="$(PATH="$BIN_STUBS:$PATH" \
  SERVICE_NAME="$T1_SERVICE" SERVICE_USER="$T1_USER" \
  INSTALL_DIR="$t1/opt/$T1_SERVICE" ETC_DIR="$t1/etc/$T1_SERVICE" \
  UPLOADS_DIR="$t1/var/lib/$T1_SERVICE/uploads" V2_BUNDLE_DIR="$t1/v2bundle" \
  bash "$t1/stage/install.sh" 2>&1)"
T1_RC=$?
set -e
printf '%s\n' "$OUT" >"$LOGS/t1.out"
ck "install exited non-zero" test "$T1_RC" -ne 0
ck "refusal names the directory" grep -q "already exists" "$LOGS/t1.out"
ck "useradd never ran (log empty or absent)" test ! -s "$LOGS/useradd.log"
report T1 "a refused first install creates no service user"

# ---- T2: a file named to check_stage but absent from SHA256SUMS -------------
t2="$ROOT/t2stage"
make_stage "$t2" extract-v2-pages.mjs
# rebuild SHA256SUMS without extract-v2-pages.mjs
(
  cd "$t2"
  sha256sum xchange configuration/base.yaml >SHA256SUMS
)
run_fn "$t2" "$ROOT/t2inst" "" check_stage xchange configuration/base.yaml extract-v2-pages.mjs
printf '%s\n' "$OUT" >"$LOGS/t2.out"
ck "check_stage refused" test "$RC" -ne 0
ck "refusal names the file" grep -q "SHA256SUMS does not list extract-v2-pages.mjs" "$LOGS/t2.out"
report T2 "a file named to check_stage must be listed in SHA256SUMS"

# ---- T2b: a staged yaml absent from SHA256SUMS ------------------------------
t2b="$ROOT/t2bstage"
make_stage "$t2b" configuration/extra.yaml
(
  cd "$t2b"
  sha256sum xchange configuration/base.yaml >SHA256SUMS
)
run_fn "$t2b" "$ROOT/t2binst" "" check_stage xchange configuration/base.yaml
printf '%s\n' "$OUT" >"$LOGS/t2b.out"
ck "check_stage refused" test "$RC" -ne 0
ck "refusal names the yaml" grep -q "SHA256SUMS does not list configuration/extra.yaml" "$LOGS/t2b.out"
report T2b "every staged configuration/*.yaml must be listed in SHA256SUMS"

# ---- T2c: similar names are not the exact name ------------------------------
t2c="$ROOT/t2cstage"
mkdir -p "$t2c/configuration" "$t2c/old"
printf '#!/bin/sh\necho not-a-real-binary\n' >"$t2c/xchange"
chmod 755 "$t2c/xchange"
printf '# base\n' >"$t2c/configuration/base.yaml"
: >"$t2c/old/xchange"
: >"$t2c/xchange.bak"
: >"$t2c/old xchange"
(
  cd "$t2c"
  sha256sum old/xchange xchange.bak 'old xchange' configuration/base.yaml >SHA256SUMS
)
chmod 700 "$t2c"
run_fn "$t2c" "$ROOT/t2cinst" "" check_stage xchange configuration/base.yaml
printf '%s\n' "$OUT" >"$LOGS/t2c.out"
ck "check_stage refused" test "$RC" -ne 0
ck "refusal names the binary" grep -q "SHA256SUMS does not list xchange" "$LOGS/t2c.out"
report T2c "old/xchange, xchange.bak and 'old xchange' do not count as listing xchange"

# ---- T2d: a binary-mode line is accepted, as today --------------------------
t2d="$ROOT/t2dstage"
make_stage "$t2d"
(
  cd "$t2d"
  printf '%s *xchange\n' "$(sha256sum xchange | cut -d' ' -f1)" >SHA256SUMS
  sha256sum configuration/base.yaml >>SHA256SUMS
)
run_fn "$t2d" "$ROOT/t2dinst" "" check_stage xchange configuration/base.yaml
printf '%s\n' "$OUT" >"$LOGS/t2d.out"
ck "check_stage accepted" grep -q "RCFN=0" "$LOGS/t2d.out"
report T2d "a 'hash *xchange' binary-mode line still counts"

# ---- T3: replace_binary keeps the old binary as .prev -----------------------
t3="$ROOT/t3"
mkdir -p "$t3/opt/xc232n" "$t3/stage/configuration"
printf 'old-binary-A' >"$t3/opt/xc232n/xchange"
chmod 755 "$t3/opt/xc232n/xchange"
printf 'new-binary-B' >"$t3/stage/xchange"
printf '# new\n' >"$t3/stage/configuration/base.yaml"
chmod 755 "$t3/stage"
run_fn "$t3/stage" "$t3/opt/xc232n" "" replace_binary
printf '%s\n' "$OUT" >"$LOGS/t3.out"
ck "replace_binary succeeded" grep -q "RCFN=0" "$LOGS/t3.out"
ck "BIN_SAVED=1" grep -q "BIN_SAVED=1" "$LOGS/t3.out"
ckeq "new binary installed" "new-binary-B" "$(cat "$t3/opt/xc232n/xchange" 2>/dev/null || echo missing)"
ckeq "old binary kept as .prev" "old-binary-A" "$(cat "$t3/opt/xc232n/xchange.prev" 2>/dev/null || echo missing)"
report T3 "a differing binary is replaced and kept as xchange.prev"

# ---- T3b: the same binary again touches nothing -----------------------------
t3b="$ROOT/t3b"
mkdir -p "$t3b/opt/xc232n" "$t3b/stage/configuration"
printf 'same-binary-B' >"$t3b/opt/xc232n/xchange"
chmod 755 "$t3b/opt/xc232n/xchange"
printf 'the-real-previous-binary' >"$t3b/opt/xc232n/xchange.prev"
printf 'same-binary-B' >"$t3b/stage/xchange"
chmod 755 "$t3b/stage"
run_fn "$t3b/stage" "$t3b/opt/xc232n" "" replace_binary
printf '%s\n' "$OUT" >"$LOGS/t3b.out"
ck "replace_binary succeeded" grep -q "RCFN=0" "$LOGS/t3b.out"
ck "BIN_SAVED=0" grep -q "BIN_SAVED=0" "$LOGS/t3b.out"
ckeq "binary untouched" "same-binary-B" "$(cat "$t3b/opt/xc232n/xchange")"
ckeq ".prev untouched" "the-real-previous-binary" "$(cat "$t3b/opt/xc232n/xchange.prev")"
report T3b "the same binary again replaces nothing and keeps .prev"

# ---- T3c: first install, no previous binary ---------------------------------
t3c="$ROOT/t3c"
mkdir -p "$t3c/opt/xc232n" "$t3c/stage/configuration"
printf 'new-binary-B' >"$t3c/stage/xchange"
chmod 755 "$t3c/stage"
run_fn "$t3c/stage" "$t3c/opt/xc232n" "" replace_binary
printf '%s\n' "$OUT" >"$LOGS/t3c.out"
ck "replace_binary succeeded" grep -q "RCFN=0" "$LOGS/t3c.out"
ck "BIN_SAVED=0" grep -q "BIN_SAVED=0" "$LOGS/t3c.out"
ckeq "binary installed" "new-binary-B" "$(cat "$t3c/opt/xc232n/xchange" 2>/dev/null || echo missing)"
ck "no .prev left behind" test ! -e "$t3c/opt/xc232n/xchange.prev"
report T3c "with no installed binary it just installs, no .prev"

# ---- T4: a yaml the stage no longer carries is removed ----------------------
t4="$ROOT/t4"
mkdir -p "$t4/opt/xc232n/configuration" "$t4/stage/configuration"
printf 'old-base' >"$t4/opt/xc232n/configuration/base.yaml"
printf 'stale-settings' >"$t4/opt/xc232n/configuration/stale.yaml"
printf 'new-base' >"$t4/stage/configuration/base.yaml"
chmod 755 "$t4/stage"
run_fn "$t4/stage" "$t4/opt/xc232n" "" replace_configuration
printf '%s\n' "$OUT" >"$LOGS/t4.out"
ck "replace_configuration succeeded" grep -q "RCFN=0" "$LOGS/t4.out"
ck "CONFIG_SAVED=1" grep -q "CONFIG_SAVED=1" "$LOGS/t4.out"
ckeq "new yaml installed" "new-base" "$(cat "$t4/opt/xc232n/configuration/base.yaml" 2>/dev/null || echo missing)"
ck "stale yaml removed" test ! -e "$t4/opt/xc232n/configuration/stale.yaml"
ckeq "old base kept in .prev" "old-base" "$(cat "$t4/opt/xc232n/configuration.prev/base.yaml" 2>/dev/null || echo missing)"
ckeq "stale yaml kept in .prev" "stale-settings" "$(cat "$t4/opt/xc232n/configuration.prev/stale.yaml" 2>/dev/null || echo missing)"
report T4 "an installed yaml absent from the stage is removed and kept in configuration.prev"

# ---- T4b: identical configuration touches nothing ---------------------------
t4b="$ROOT/t4b"
mkdir -p "$t4b/opt/xc232n/configuration" "$t4b/opt/xc232n/configuration.prev" "$t4b/stage/configuration"
printf 'same-base' >"$t4b/opt/xc232n/configuration/base.yaml"
printf 'sentinel-prev' >"$t4b/opt/xc232n/configuration.prev/base.yaml"
printf 'same-base' >"$t4b/stage/configuration/base.yaml"
chmod 755 "$t4b/stage"
run_fn "$t4b/stage" "$t4b/opt/xc232n" "" replace_configuration
printf '%s\n' "$OUT" >"$LOGS/t4b.out"
ck "replace_configuration succeeded" grep -q "RCFN=0" "$LOGS/t4b.out"
ck "CONFIG_SAVED=0" grep -q "CONFIG_SAVED=0" "$LOGS/t4b.out"
ckeq "yaml untouched" "same-base" "$(cat "$t4b/opt/xc232n/configuration/base.yaml")"
ckeq "old .prev kept" "sentinel-prev" "$(cat "$t4b/opt/xc232n/configuration.prev/base.yaml")"
report T4b "the same configuration again replaces nothing and keeps .prev"

# ---- T5: one WARNING at the first ancestor the user cannot enter ------------
t5="$ROOT/t5"
mkdir -p "$t5/blocked/a"
T5_PRE="$ROOT/t5_pre.sh"
cat >"$T5_PRE" <<PRE
can_enter_as() {
  [ "\$2" = "$t5/blocked" ] && return 1
  return 0
}
PRE
run_fn "$ROOT/empty-stage" "$ROOT/t5inst" "$T5_PRE" warn_if_unreachable xc232u "$t5/blocked/a/b"
printf '%s\n' "$OUT" >"$LOGS/t5.out"
ck "returned 0" grep -q "RCFN=0" "$LOGS/t5.out"
ck "exactly one WARNING line" test "$(grep -c '^WARNING:' "$LOGS/t5.out")" -eq 1
ck "WARNING names the blocked dir" grep -q "^WARNING: xc232u cannot enter $t5/blocked, so the service cannot reach $t5/blocked/a/b; this script does not change $t5/blocked$" "$LOGS/t5.out"
run_fn "$ROOT/empty-stage" "$ROOT/t5inst" "$T5_PRE" warn_if_unreachable xc232u "$t5/open/a/b"
printf '%s\n' "$OUT" >"$LOGS/t5b.out"
ck "returned 0 when reachable" grep -q "RCFN=0" "$LOGS/t5b.out"
ck "no WARNING when reachable" test "$(grep -c '^WARNING:' "$LOGS/t5b.out")" -eq 0
report T5 "warn_if_unreachable warns once and never fails"

# ---- T6: ensure_dir on an existing dir of another mode ----------------------
t6="$ROOT/t6/dir"
mkdir -p "$t6"
chmod 755 "$t6"
run_fn "$ROOT/empty-stage" "$ROOT/t6inst" "" ensure_dir "$t6" root root 700
printf '%s\n' "$OUT" >"$LOGS/t6.out"
ck "ensure_dir refused" test "$RC" -ne 0
ckeq "directory left unchanged" "755" "$(stat -c '%a' "$t6")"
report T6 "ensure_dir still refuses an existing directory of another mode"

echo "# $PASS passed, $FAILURES failed"
[ "$FAILURES" -eq 0 ]
