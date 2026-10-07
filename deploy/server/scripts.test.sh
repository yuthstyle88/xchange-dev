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
#
# Cases B1-B9 are the backups of #121 (docs/design/xchange-v3.md §7b, row
# BK), run against backup.sh / restore.sh with pg_dump, pg_restore, tar,
# rsync, createdb and date stubbed:
#   B1/B1b/B1c  one run makes one set: db-<stamp>.dump and uploads-<stamp>.tar.gz
#        in <stamp>/ (0700, in a 0700 backup directory it creates), pg_dump -Fc,
#        no password on its command line, the set's path as the last line of
#        stdout; a stamp that already exists is refused and the set is left
#        alone; a backup directory of another mode is refused and not changed
#   B2/B2b  the 8th run removes the oldest set, leaving 7, and nothing else;
#        an XC_BACKUP_KEEP that is not an integer >= 1 is refused
#   B3/B3b  a failing pg_dump or tar leaves no partial set, prunes nothing,
#        exits non-zero
#   B4   an empty XC_BACKUP_OFFHOST: one "skipped" line, no rsync, exit 0
#   B5   a failing rsync exits non-zero and keeps the local set
#   B6   rsync copies the directory's contents with an include of the set
#        directories and --exclude='*' (never the working directory or the
#        lock), and passes --delete only once the local directory holds 7 sets
#   B7/B7b/B7c/B7d  restore.sh refuses the live database and the live uploads
#        directory, restores into a scratch database (createdb and pg_restore
#        as postgres through runuser, the dump on pg_restore's stdin) and
#        directory, and drops what it created
#        when pg_restore fails
#   B8   the timer is daily and Persistent=true; the service is a oneshot
#   B9   env.example names every XC_BACKUP_* setting
#
# Cases U1-U5c are update.sh taking a backup first (#290, design §7b, the
# #290 paragraph), run as full update.sh runs on an installed host laid out
# under the scratch root, with their own stubs first on PATH (sudo mapping
# /etc/systemd/system/ into the case, systemctl answering for
# xc-v3-backup.service, pg_restore, psql counting migrations, curl answering
# the smoke check):
#   U1   with a migration applied: the backup unit is started, the new set's
#        dump checked with pg_restore --list (as postgres, dump on stdin), both
#        before the stop; the NOTE and the rollback name that set
#   U2   with no migration applied: the backup still comes before the stop,
#        and there is no NOTE
#   U3   a failing backup: no stop, nothing replaced, the message names BK
#   U4   xc-v3-backup.service not installed: refused the same way
#   U5/U5b/U5c  the new set's dump fails pg_restore --list, or lists no
#        _sqlx_migrations, or no new set appeared: no stop, nothing replaced
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
# B7: a refused restore must not have sent a CREATE through psql either
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/psql.log"
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

# ==== backups (#121, design §7b, row BK) ====================================
# ---- stubs of the backup tools ---------------------------------------------
# Each logs its command line to $XC_TEST_LOG/<name>.log. pg_dump and tar
# (creating) write a small file where they were told to (-f/--file, or a
# short-option cluster ending in f, e.g. -czf), or to standard output; with
# XC_STUB_PG_DUMP_FAIL=1 / XC_STUB_TAR_FAIL=1 they write that partial file
# and then fail. rsync exits with $XC_STUB_RSYNC_RC (default 0) and logs
# RSYNC_RSH too, so an ssh command given either way is seen; it also writes
# the arguments of its last call one per line to rsync.args. pg_restore
# copies what it got on stdin to pg_restore.stdin (restore.sh runs with
# stdin from /dev/null, so a dump named as a path leaves it empty) and
# fails with XC_STUB_PG_RESTORE_FAIL=1. runuser logs its command line and
# runs what follows "--" as the caller (no user switch).
write_stub pg_dump <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/pg_dump.log"
out="" take=0
for a in "$@"; do
  if [ "$take" = 1 ]; then
    out="$a"
    take=0
    continue
  fi
  case "$a" in
    --file=*) out="${a#--file=}" ;;
    -f | --file) take=1 ;;
  esac
done
if [ -n "$out" ] && [ "$out" != - ]; then
  printf 'PGDMP stub dump\n' >"$out"
else
  printf 'PGDMP stub dump\n'
fi
[ "${XC_STUB_PG_DUMP_FAIL:-0}" = 1 ] && exit 1
exit 0
STUB

write_stub tar <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/tar.log"
out="" take=0 create=0
for a in "$@"; do
  if [ "$take" = 1 ]; then
    out="$a"
    take=0
    continue
  fi
  case "$a" in
    --create) create=1 ;;
    --file=*) out="${a#--file=}" ;;
    -f | --file) take=1 ;;
    --*) ;;
    -*)
      case "$a" in -*c*) create=1 ;; esac
      case "$a" in -*f) take=1 ;; esac
      ;;
  esac
done
if [ "$create" = 1 ]; then
  if [ -n "$out" ] && [ "$out" != - ]; then
    printf 'tar stub archive\n' >"$out"
  else
    printf 'tar stub archive\n'
  fi
  [ "${XC_STUB_TAR_FAIL:-0}" = 1 ] && exit 2
fi
exit 0
STUB

write_stub rsync <<'STUB'
#!/usr/bin/env bash
printf '%s RSYNC_RSH=%s\n' "$*" "${RSYNC_RSH:-}" >>"${XC_TEST_LOG:?}/rsync.log"
printf '%s\n' "$@" >"${XC_TEST_LOG:?}/rsync.args"
exit "${XC_STUB_RSYNC_RC:-0}"
STUB

write_stub pg_restore <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/pg_restore.log"
cat >"${XC_TEST_LOG:?}/pg_restore.stdin"
[ "${XC_STUB_PG_RESTORE_FAIL:-0}" = 1 ] && exit 1
exit 0
STUB

write_stub runuser <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/runuser.log"
while [ "$#" -gt 0 ] && [ "$1" != -- ]; do shift; done
[ "$#" -gt 0 ] && shift
exec "$@"
STUB

for s in createdb dropdb; do
  write_stub "$s" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/$(basename "$0").log"
exit 0
STUB
done

# date: "date -u +%Y%m%dT%H%M%SZ" (design §7b) answers the next line of
# $XC_TEST_STAMPS; any other use goes to the real date.
write_stub date <<'STUB'
#!/usr/bin/env bash
for a in "$@"; do
  case "$a" in
    +*%Y%m%dT%H%M%S*)
      f="${XC_TEST_STAMPS:?}"
      s="$(head -n 1 "$f")"
      tail -n +2 "$f" >"$f.next"
      mv "$f.next" "$f"
      [ -n "$s" ] || {
        echo "date stub: no stamp left" >&2
        exit 1
      }
      echo "$s"
      exit 0
      ;;
  esac
done
exec /bin/date "$@"
STUB

# ---- helpers ----------------------------------------------------------------
BK_DB=xc_bk_db
BK_ROLE=xc_bk_role
BK_SECRET=pBackupStubSecret42

# bk_case <dir> <stamp...> — a case directory: log/, an uploads directory
# var/up-files with one file in it, and the stamps the date stub hands out.
bk_case() {
  local c="$1"
  shift
  mkdir -p "$c/log" "$c/var/up-files"
  printf 'an uploaded file\n' >"$c/var/up-files/f1"
  printf '%s\n' "$@" >"$c/stamps"
}

# run_backup <case dir> [NAME=value...] — backup.sh with the stubs and the
# server's settings in its environment (as EnvironmentFile= gives them);
# OUT and RC hold its output (both streams) and exit status; <case>/stdout
# holds its standard output alone.
run_backup() {
  local c="$1"
  shift
  set +e
  OUT="$({ env PATH="$BIN_STUBS:$PATH" XC_TEST_LOG="$c/log" XC_TEST_STAMPS="$c/stamps" \
    XC_BACKUP_DIR="$c/backups" XC_BACKUP_KEEP=7 XC_BACKUP_OFFHOST= \
    XC_BACKUP_SSH_KEY="$c/offhost.key" \
    APP_DATABASE__HOST=127.0.0.1 APP_DATABASE__PORT=5432 \
    APP_DATABASE__USERNAME="$BK_ROLE" APP_DATABASE__PASSWORD="$BK_SECRET" \
    APP_DATABASE__DATABASE_NAME="$BK_DB" APP_FILES__DIR="$c/var/up-files" \
    "$@" bash "$HERE/backup.sh" 2>&3 | tee "$c/stdout"; } 3>&1)"
  RC=$?
  set -e
}

# run_restore <case dir> <args...> — restore.sh with the stubs and an env
# file naming the live database and uploads directory.
run_restore() {
  local c="$1"
  shift
  printf '%s\n' \
    "APP_DATABASE__USERNAME=$BK_ROLE" \
    "APP_DATABASE__PASSWORD=$BK_SECRET" \
    "APP_DATABASE__DATABASE_NAME=$BK_DB" \
    "APP_FILES__DIR=$c/var/up-files" >"$c/env"
  set +e
  OUT="$(env PATH="$BIN_STUBS:$PATH" XC_TEST_LOG="$c/log" XC_ENV_FILE="$c/env" \
    bash "$HERE/restore.sh" "$@" </dev/null 2>&1)"
  RC=$?
  set -e
}

# sets_in <dir> — the set directories (<stamp>/) in name order, space-separated
sets_in() {
  local d n
  for d in "$1"/*; do
    n="${d##*/}"
    [ -d "$d" ] && [[ "$n" =~ ^[0-9]{8}T[0-9]{6}Z$ ]] && printf '%s ' "$n"
  done
  return 0
}

# entries_of <dir> — every entry but the lock file, space-separated
entries_of() { (ls -A1 "$1" 2>/dev/null || true) | (grep -vx '.lock' || true) | tr '\n' ' '; }

# make_set <backups dir> <stamp> — a finished set, as backup.sh leaves one
make_set() {
  mkdir -p "$1/$2"
  printf 'old dump\n' >"$1/$2/db-$2.dump"
  printf 'old archive\n' >"$1/$2/uploads-$2.tar.gz"
}

# last_line <file> — its last line, or "missing"
last_line() { tail -n 1 "$1" 2>/dev/null || echo missing; }

# mode_of <path> — octal mode, or "missing"
mode_of() { stat -c '%a' "$1" 2>/dev/null || echo missing; }

# absent <pattern> <file> — the file is missing or has no line matching
absent() { ! grep -Eqs -- "$1" "$2" 2>/dev/null; }

# ---- B1: one run makes one set ---------------------------------------------
b1="$ROOT/b1"
bk_case "$b1" 20261007T010203Z
run_backup "$b1"
printf '%s\n' "$OUT" >"$LOGS/b1.out"
s=20261007T010203Z
ck "backup.sh exited 0" test "$RC" -eq 0
ckeq "exactly one set" "$s " "$(sets_in "$b1/backups")"
ckeq "the set holds the dump and the archive, same stamp" \
  "db-$s.dump uploads-$s.tar.gz " "$(entries_of "$b1/backups/$s")"
ck "the dump is not empty" test -s "$b1/backups/$s/db-$s.dump"
ck "the archive is not empty" test -s "$b1/backups/$s/uploads-$s.tar.gz"
ck "pg_dump asked for the custom format" \
  grep -Eqs -- '(^| )(-Fc|-F c|--format=c|--format=custom)( |$)' "$b1/log/pg_dump.log"
ck "pg_dump named the database" grep -qs -- "$BK_DB" "$b1/log/pg_dump.log"
ck "the password is not on pg_dump's command line" absent "$BK_SECRET" "$b1/log/pg_dump.log"
ck "tar archived the uploads directory" grep -qs -- "up-files" "$b1/log/tar.log"
ckeq "nothing but the set is left in the backup directory" "$s " "$(entries_of "$b1/backups")"
ckeq "the last line of stdout is the set's path" "$b1/backups/$s" "$(last_line "$b1/stdout")"
ckeq "the backup directory it created is 0700" "700" "$(mode_of "$b1/backups")"
ckeq "the set directory is 0700" "700" "$(mode_of "$b1/backups/$s")"
report B1 "one run makes one set: dump and uploads archive under one UTC stamp"

# ---- B1b: a stamp that already exists is refused ---------------------------
b1b="$ROOT/b1b"
bk_case "$b1b" 20261007T020000Z 20261007T020000Z
run_backup "$b1b"
B1B_FIRST_RC=$RC
s=20261007T020000Z
cp "$b1b/backups/$s/db-$s.dump" "$b1b/first.dump" 2>/dev/null || true
run_backup "$b1b"
printf '%s\n' "$OUT" >"$LOGS/b1b.out"
ck "the first run exited 0" test "$B1B_FIRST_RC" -eq 0
ck "the second run exited non-zero" test "$RC" -ne 0
ck "the refusal names the stamp" grep -qs -- "$s" "$LOGS/b1b.out"
ckeq "still one set" "$s " "$(sets_in "$b1b/backups")"
ck "the first set's dump is unchanged" cmp -s "$b1b/first.dump" "$b1b/backups/$s/db-$s.dump"
report B1b "a set whose stamp already exists is refused, not overwritten"

# ---- B1c: a backup directory of another mode is refused, not changed --------
b1c="$ROOT/b1c"
bk_case "$b1c" 20261007T021500Z
mkdir -m 755 "$b1c/backups"
chmod 755 "$b1c/backups"
run_backup "$b1c"
printf '%s\n' "$OUT" >"$LOGS/b1c.out"
ck "backup.sh exited non-zero" test "$RC" -ne 0
ck "the refusal names the directory" grep -qs -- "$b1c/backups" "$LOGS/b1c.out"
ckeq "the directory's mode is unchanged" "755" "$(mode_of "$b1c/backups")"
ckeq "nothing was written in it" "" "$(entries_of "$b1c/backups")"
ck "pg_dump never ran" test ! -s "$b1c/log/pg_dump.log"
report B1c "an XC_BACKUP_DIR that is not 0700 is refused and left as it is"

# ---- B2: the 8th run removes the oldest set, leaving 7 ---------------------
b2="$ROOT/b2"
bk_case "$b2" 20261001T000001Z 20261001T000002Z 20261001T000003Z 20261001T000004Z \
  20261001T000005Z 20261001T000006Z 20261001T000007Z 20261001T000008Z
mkdir -m 700 "$b2/backups"
printf 'kept by hand\n' >"$b2/backups/notes.txt"
mkdir "$b2/backups/manual-copy"
B2_RCS=""
for i in 1 2 3 4 5 6 7; do
  run_backup "$b2"
  B2_RCS="$B2_RCS$RC "
done
B2_AFTER7="$(sets_in "$b2/backups")"
run_backup "$b2"
B2_RCS="$B2_RCS$RC "
printf '%s\n' "$OUT" >"$LOGS/b2.out"
ckeq "all eight runs exited 0" "0 0 0 0 0 0 0 0 " "$B2_RCS"
ckeq "after 7 runs the 7 sets are all there" \
  "20261001T000001Z 20261001T000002Z 20261001T000003Z 20261001T000004Z 20261001T000005Z 20261001T000006Z 20261001T000007Z " \
  "$B2_AFTER7"
ckeq "after the 8th the oldest is gone, 7 remain" \
  "20261001T000002Z 20261001T000003Z 20261001T000004Z 20261001T000005Z 20261001T000006Z 20261001T000007Z 20261001T000008Z " \
  "$(sets_in "$b2/backups")"
ck "a file that is not a set is kept" test -f "$b2/backups/notes.txt"
ck "a directory that is not a set is kept" test -d "$b2/backups/manual-copy"
report B2 "the 8th run deletes the oldest set and nothing else"

# ---- B2b: XC_BACKUP_KEEP must be an integer of at least 1 ------------------
b2b="$ROOT/b2b"
bk_case "$b2b" 20261007T030000Z 20261007T030001Z
mkdir -m 700 "$b2b/backups"
make_set "$b2b/backups" 20260901T000001Z
run_backup "$b2b" XC_BACKUP_KEEP=0
B2B_RC0=$RC
printf '%s\n' "$OUT" >"$LOGS/b2b.out"
run_backup "$b2b" XC_BACKUP_KEEP=seven
B2B_RCX=$RC
printf '%s\n' "$OUT" >>"$LOGS/b2b.out"
ck "KEEP=0 exited non-zero" test "$B2B_RC0" -ne 0
ck "KEEP=seven exited non-zero" test "$B2B_RCX" -ne 0
ck "the refusal names XC_BACKUP_KEEP" grep -qs "XC_BACKUP_KEEP" "$LOGS/b2b.out"
ckeq "no set made, the old one kept" "20260901T000001Z " "$(sets_in "$b2b/backups")"
ck "pg_dump never ran" test ! -s "$b2b/log/pg_dump.log"
report B2b "an XC_BACKUP_KEEP that is not an integer >= 1 is refused before anything"

# ---- B3: a failing pg_dump leaves no partial set ---------------------------
b3="$ROOT/b3"
bk_case "$b3" 20261007T040000Z
mkdir -m 700 "$b3/backups"
for i in 1 2 3 4 5 6 7; do make_set "$b3/backups" "20260901T00000${i}Z"; done
B3_BEFORE="$(entries_of "$b3/backups")"
run_backup "$b3" XC_STUB_PG_DUMP_FAIL=1
printf '%s\n' "$OUT" >"$LOGS/b3.out"
ck "backup.sh exited non-zero" test "$RC" -ne 0
ck "pg_dump was run" test -s "$b3/log/pg_dump.log"
ckeq "the directory is as before: no partial set, nothing pruned" "$B3_BEFORE" "$(entries_of "$b3/backups")"
report B3 "a failing pg_dump leaves no partial set, prunes nothing, exits non-zero"

# ---- B3b: a failing tar leaves no partial set either ------------------------
b3b="$ROOT/b3b"
bk_case "$b3b" 20261007T050000Z
mkdir -m 700 "$b3b/backups"
for i in 1 2 3 4 5 6 7; do make_set "$b3b/backups" "20260901T00000${i}Z"; done
B3B_BEFORE="$(entries_of "$b3b/backups")"
run_backup "$b3b" XC_STUB_TAR_FAIL=1
printf '%s\n' "$OUT" >"$LOGS/b3b.out"
ck "backup.sh exited non-zero" test "$RC" -ne 0
ck "tar was run" test -s "$b3b/log/tar.log"
ckeq "the directory is as before: the dump went with the failed archive" "$B3B_BEFORE" "$(entries_of "$b3b/backups")"
report B3b "a failing uploads archive removes the dump too and exits non-zero"

# ---- B4: an empty off-host setting skips the copy --------------------------
b4="$ROOT/b4"
bk_case "$b4" 20261007T060000Z
run_backup "$b4" XC_BACKUP_OFFHOST=
printf '%s\n' "$OUT" >"$LOGS/b4.out"
ck "backup.sh exited 0" test "$RC" -eq 0
ckeq "one 'off-host copy skipped' line" "1" "$(grep -c "off-host copy skipped" "$LOGS/b4.out" || true)"
ck "rsync never ran" test ! -s "$b4/log/rsync.log"
ckeq "the set was made" "20261007T060000Z " "$(sets_in "$b4/backups")"
ckeq "the set's path is still the last line of stdout" "$b4/backups/20261007T060000Z" "$(last_line "$b4/stdout")"
report B4 "an empty XC_BACKUP_OFFHOST skips the copy with one line and exits 0"

# ---- B5: a failing rsync exits non-zero and keeps the local set ------------
b5="$ROOT/b5"
bk_case "$b5" 20261007T070000Z
s=20261007T070000Z
run_backup "$b5" XC_BACKUP_OFFHOST=xcbackup@offhost.invalid:/data/xc-v3-backups XC_STUB_RSYNC_RC=23
printf '%s\n' "$OUT" >"$LOGS/b5.out"
ck "backup.sh exited non-zero" test "$RC" -ne 0
ck "rsync was run toward the target" grep -qs "xcbackup@offhost.invalid:/data/xc-v3-backups" "$b5/log/rsync.log"
ckeq "the local set is kept, both pieces" "db-$s.dump uploads-$s.tar.gz " "$(entries_of "$b5/backups/$s")"
report B5 "a failing off-host copy exits non-zero and keeps the local set"

# ---- B6: --delete only once the local directory holds 7 sets ---------------
b6="$ROOT/b6"
bk_case "$b6" 20261007T080000Z
run_backup "$b6" XC_BACKUP_OFFHOST=xcbackup@offhost.invalid:/data/xc-v3-backups
B6_RC1=$RC
printf '%s\n' "$OUT" >"$LOGS/b6.out"
b6f="$ROOT/b6full"
bk_case "$b6f" 20261007T080000Z
mkdir -m 700 "$b6f/backups"
for i in 1 2 3 4 5 6; do make_set "$b6f/backups" "20260901T00000${i}Z"; done
run_backup "$b6f" XC_BACKUP_OFFHOST=xcbackup@offhost.invalid:/data/xc-v3-backups
B6_RC7=$RC
printf '%s\n' "$OUT" >>"$LOGS/b6.out"
# include_globs <args file> — the patterns given to --include, one per line
include_globs() {
  local take=0 a
  while IFS= read -r a; do
    if [ "$take" = 1 ]; then
      printf '%s\n' "$a"
      take=0
      continue
    fi
    case "$a" in
      --include=*) printf '%s\n' "${a#--include=}" ;;
      --include) take=1 ;;
    esac
  done <"$1"
}
# include_takes <args file> <name> — whether a top-level directory <name> is
# sent: some include pattern "/<glob>/***" matches it (bash globbing of the
# [0-9]/*/? forms is the same as rsync's for these patterns)
include_takes() {
  local g
  while IFS= read -r g; do
    case "$g" in
      /*/\*\*\*)
        g="${g#/}"
        g="${g%/\*\*\*}"
        # shellcheck disable=SC2053  # $g is a pattern on purpose
        [[ "$2" == $g ]] && return 0
        ;;
    esac
  done < <(include_globs "$1" 2>/dev/null || true)
  return 1
}
# excludes_all <args file> — --exclude='*' given as one argument or as two
excludes_all() {
  local prev="" a
  [ -f "$1" ] || return 1
  while IFS= read -r a; do
    [ "$a" = "--exclude=*" ] && return 0
    [ "$prev" = "--exclude" ] && [ "$a" = "*" ] && return 0
    prev="$a"
  done <"$1"
  return 1
}
# not <command...> — succeeds when the command fails
not() { ! "$@"; }
B6_ARGS="$b6/log/rsync.args"
B6_OFF=xcbackup@offhost.invalid:/data/xc-v3-backups
ck "the run with one set exited 0" test "$B6_RC1" -eq 0
ckeq "one rsync with one local set" "1" "$( (cat "$b6/log/rsync.log" 2>/dev/null || true) | wc -l | tr -d ' ')"
ck "no --delete with one local set" absent '^--delete$' "$B6_ARGS"
ck "the ssh key is used" grep -qs -- "$b6/offhost.key" "$b6/log/rsync.log"
ck "ssh never prompts (BatchMode=yes)" grep -qs "BatchMode=yes" "$b6/log/rsync.log"
ckeq "the source is the backup directory's contents" "$b6/backups/" "$(tail -n 2 "$B6_ARGS" 2>/dev/null | head -n 1)"
ckeq "the target is the off-host directory" "$B6_OFF/" "$(last_line "$B6_ARGS")"
ck "everything not included is excluded (--exclude='*')" excludes_all "$B6_ARGS"
ck "excluded names on the target are never deleted" absent '^--delete-excluded$' "$B6_ARGS"
ck "a set directory is included" include_takes "$B6_ARGS" 20261007T080000Z
ck "the working directory is not" not include_takes "$B6_ARGS" .work-20261007T080000Z
ck "the lock is not" not include_takes "$B6_ARGS" .lock
ck "a stamp with a suffix is not" not include_takes "$B6_ARGS" 20261007T080000Z.old
ck "another name is not" not include_takes "$B6_ARGS" notes.txt
ck "the run reaching 7 sets exited 0" test "$B6_RC7" -eq 0
ck "--delete once 7 sets are local" grep -qsx -- '--delete' "$b6f/log/rsync.args"
ckeq "with 7 sets the source is still the directory's contents" "$b6f/backups/" "$(tail -n 2 "$b6f/log/rsync.args" 2>/dev/null | head -n 1)"
ck "with 7 sets a set directory is included" include_takes "$b6f/log/rsync.args" 20260901T000001Z
report B6 "the off-host copy sends the set directories only, with --delete once 7 are local"

# ---- B7: restore.sh refuses the live database ------------------------------
b7="$ROOT/b7"
bk_case "$b7"
make_set "$b7/backups" 20261007T090000Z
run_restore "$b7" "$b7/backups/20261007T090000Z" "$BK_DB" "$b7/scratch-files"
B7_RC_LIVE=$RC
printf '%s\n' "$OUT" >"$LOGS/b7.out"
run_restore "$b7" "$b7/backups/20261007T090000Z" "XC_BK_DB" "$b7/scratch-files"
B7_RC_UPPER=$RC
printf '%s\n' "$OUT" >>"$LOGS/b7.out"
ck "the live name exited non-zero" test "$B7_RC_LIVE" -ne 0
ck "its spelling in capitals exited non-zero" test "$B7_RC_UPPER" -ne 0
ck "the refusal names the live database" grep -qs -- "$BK_DB" "$LOGS/b7.out"
ck "no database was created" test ! -s "$b7/log/createdb.log"
ck "no CREATE went through psql" absent '[Cc][Rr][Ee][Aa][Tt][Ee]' "$b7/log/psql.log"
ck "pg_restore never ran" test ! -s "$b7/log/pg_restore.log"
ck "nothing was extracted" test ! -s "$b7/log/tar.log"
ck "the scratch directory was not created" test ! -e "$b7/scratch-files"
report B7 "restore.sh refuses the live database name"

# ---- B7b: restore.sh never writes the live uploads directory ---------------
b7b="$ROOT/b7b"
bk_case "$b7b"
make_set "$b7b/backups" 20261007T100000Z
run_restore "$b7b" "$b7b/backups/20261007T100000Z" xc_bk_rehearsal "$b7b/var/up-files"
B7B_RC_SAME=$RC
printf '%s\n' "$OUT" >"$LOGS/b7b.out"
run_restore "$b7b" "$b7b/backups/20261007T100000Z" xc_bk_rehearsal "$b7b/var/up-files/rehearsal"
B7B_RC_INSIDE=$RC
printf '%s\n' "$OUT" >>"$LOGS/b7b.out"
ck "the live uploads directory exited non-zero" test "$B7B_RC_SAME" -ne 0
ck "a directory inside it exited non-zero" test "$B7B_RC_INSIDE" -ne 0
ck "the refusal names the live uploads directory" grep -qs -- "$b7b/var/up-files" "$LOGS/b7b.out"
ckeq "the live uploads directory is unchanged" "f1 " "$(entries_of "$b7b/var/up-files")"
ck "pg_restore never ran" test ! -s "$b7b/log/pg_restore.log"
ck "nothing was extracted" test ! -s "$b7b/log/tar.log"
report B7b "restore.sh refuses the live uploads directory and anything inside it"

# ---- B7c: a rehearsal restores into the scratch database and directory -----
b7c="$ROOT/b7c"
bk_case "$b7c"
s=20261007T110000Z
make_set "$b7c/backups" "$s"
run_restore "$b7c" "$b7c/backups/$s" xc_bk_rehearsal "$b7c/scratch-files"
printf '%s\n' "$OUT" >"$LOGS/b7c.out"
ck "restore.sh exited 0" test "$RC" -eq 0
ck "pg_restore wrote into the scratch database" grep -qs "xc_bk_rehearsal" "$b7c/log/pg_restore.log"
ckeq "pg_restore read the set's dump on stdin" "old dump" "$(cat "$b7c/log/pg_restore.stdin" 2>/dev/null || echo missing)"
ck "the dump is not named to pg_restore as a path (postgres cannot open it)" absent "db-$s.dump" "$b7c/log/pg_restore.log"
ck "pg_restore never named the live database" absent "(^|[ =])$BK_DB( |$)" "$b7c/log/pg_restore.log"
ck "tar extracted the set's archive" grep -qs -- "uploads-$s.tar.gz" "$b7c/log/tar.log"
ck "tar extracted into the scratch directory" grep -qs -- "$b7c/scratch-files" "$b7c/log/tar.log"
ckeq "the live uploads directory is unchanged" "f1 " "$(entries_of "$b7c/var/up-files")"
ck "createdb ran as postgres through runuser" grep -Eqs -- '^-u postgres -- createdb( .*)? xc_bk_rehearsal( |$)' "$b7c/log/runuser.log"
ck "pg_restore ran as postgres through runuser" grep -Eqs -- '^-u postgres -- pg_restore( |$).*xc_bk_rehearsal' "$b7c/log/runuser.log"
ck "nothing was dropped" test ! -s "$b7c/log/dropdb.log"
report B7c "a rehearsal restores into the scratch database and directory"

# ---- B7d: a failed pg_restore drops what the rehearsal created ------------
b7d="$ROOT/b7d"
bk_case "$b7d"
s=20261007T120000Z
make_set "$b7d/backups" "$s"
XC_STUB_PG_RESTORE_FAIL=1
export XC_STUB_PG_RESTORE_FAIL
run_restore "$b7d" "$b7d/backups/$s" xc_bk_rehearsal "$b7d/scratch-files"
unset XC_STUB_PG_RESTORE_FAIL
printf '%s\n' "$OUT" >"$LOGS/b7d.out"
ck "restore.sh exited non-zero" test "$RC" -ne 0
ck "pg_restore was run" test -s "$b7d/log/pg_restore.log"
ck "the scratch database was dropped as postgres" grep -Eqs -- '^-u postgres -- dropdb( .*)? xc_bk_rehearsal( |$)' "$b7d/log/runuser.log"
ck "the live database was never dropped" absent "(^| )$BK_DB( |$)" "$b7d/log/dropdb.log"
ck "the scratch directory was removed" test ! -e "$b7d/scratch-files"
ckeq "the live uploads directory is unchanged" "f1 " "$(entries_of "$b7d/var/up-files")"
report B7d "a failed pg_restore drops the scratch database and directory it made"

# ---- B8: the units -----------------------------------------------------------
timer="$HERE/xc-v3-backup.timer"
service="$HERE/xc-v3-backup.service"
ck "the timer runs daily" grep -qsx 'OnCalendar=daily' "$timer"
ck "the timer is Persistent=true" grep -qsx 'Persistent=true' "$timer"
ck "the timer is wanted by timers.target" grep -qsx 'WantedBy=timers.target' "$timer"
ck "the service is a oneshot" grep -qsx 'Type=oneshot' "$service"
ck "the service runs backup.sh" grep -Eqs '^ExecStart=.*/backup\.sh$' "$service"
ck "the service reads the server's env file" grep -qsx 'EnvironmentFile=/etc/xc-v3/env' "$service"
report B8 "the backup timer is daily with Persistent=true; the service is a oneshot"

# ---- B9: env.example names every backup setting ------------------------------
for v in XC_BACKUP_DIR XC_BACKUP_KEEP XC_BACKUP_OFFHOST XC_BACKUP_SSH_KEY; do
  ck "env.example names $v" grep -Eqs "^#?$v=" "$HERE/env.example"
done
report B9 "env.example names every XC_BACKUP_* setting"

# ---- B10: restore.sh refuses the live database name written in capitals ------
b10="$ROOT/b10"
bk_case "$b10"
make_set "$b10/backups" 20261007T130000Z
run_restore "$b10" "$b10/backups/20261007T130000Z" "XC_BK_DB" "$b10/scratch-files"
B10_RC_UPPER=$RC
printf '%s\n' "$OUT" >"$LOGS/b10.out"
printf '%s\n' \
  "APP_DATABASE__USERNAME=$BK_ROLE" \
  "APP_DATABASE__PASSWORD=$BK_SECRET" \
  "APP_DATABASE__DATABASE_NAME=XC_BK_DB" \
  "APP_FILES__DIR=$b10/var/up-files" >"$b10/env-upper"
set +e
OUT_ENV_UPPER="$(env PATH="$BIN_STUBS:$PATH" XC_TEST_LOG="$b10/log" XC_ENV_FILE="$b10/env-upper" \
  bash "$HERE/restore.sh" "$b10/backups/20261007T130000Z" "$BK_DB" "$b10/scratch-files" </dev/null 2>&1)"
B10_RC_ENV_UPPER=$?
set -e
printf '%s\n' "$OUT_ENV_UPPER" >>"$LOGS/b10.out"
ck "scratch db in capitals exited non-zero" test "$B10_RC_UPPER" -ne 0
ck "live db in env in capitals exited non-zero" test "$B10_RC_ENV_UPPER" -ne 0
ck "the refusal names the live database" grep -qs -- "$BK_DB" "$LOGS/b10.out"
ck "the refusal matches live database" grep -qs -- "matches live database" "$LOGS/b10.out"
ck "no database was created" test ! -s "$b10/log/createdb.log"
ck "no CREATE went through psql" absent '[Cc][Rr][Ee][Aa][Tt][Ee]' "$b10/log/psql.log"
ck "pg_restore never ran" test ! -s "$b10/log/pg_restore.log"
ck "nothing was extracted" test ! -s "$b10/log/tar.log"
ck "the scratch directory was not created" test ! -e "$b10/scratch-files"
report B10 "restore.sh refuses the live database name written in capitals"

# ==== update.sh backs up first (#290, design §7b) ===========================
# ---- stubs of the update cases (first on PATH for these runs only) ---------
# Every stub appends its command line to $XC_TEST_LOG/calls.log, so the order
# of the calls can be read back.
U_STUBS="$ROOT/ustubs"
mkdir -m 700 "$U_STUBS"
write_ustub() {
  cat >"$U_STUBS/$1"
  chmod 755 "$U_STUBS/$1"
}

# sudo: as the shared stub (no privilege; -u/-o/-g and chown dropped), and
# every argument under /etc/systemd/system/ is mapped into $XC_TEST_SYSTEMD,
# so the installed units of the case are where update.sh looks for them.
write_ustub sudo <<'STUB'
#!/usr/bin/env bash
printf 'sudo %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
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
    /etc/systemd/system/*) argv+=("${XC_TEST_SYSTEMD:?}/${a#/etc/systemd/system/}") ;;
    *) argv+=("$a") ;;
  esac
done
exec "${argv[@]}"
STUB

# systemctl: logs. For xc-v3-backup(.service) it answers from whether
# $XC_TEST_SYSTEMD/xc-v3-backup.service exists: cat, list-unit-files,
# status, is-enabled and show -p LoadState as systemd does for a present or
# a missing unit; start of a missing unit exits 5. A start of the present
# unit exits $XC_STUB_BACKUP_RC (default 0) and, when that is 0 and
# $XC_STUB_BACKUP_SET is set, leaves that set directory as backup.sh would
# (dump "fresh dump").
write_ustub systemctl <<'STUB'
#!/usr/bin/env bash
printf 'systemctl %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
printf 'systemctl %s\n' "$*" >>"${XC_TEST_LOG:?}/systemctl.log"
unit_file="${XC_TEST_SYSTEMD:?}/xc-v3-backup.service"
backup=0
sub=""
for a in "$@"; do
  case "$a" in
    xc-v3-backup | xc-v3-backup.service) backup=1 ;;
    -*) ;;
    *) [ -z "$sub" ] && sub="$a" ;;
  esac
done
[ "$backup" = 1 ] || exit 0
present=0
[ -f "$unit_file" ] && present=1
case "$sub" in
  start)
    if [ "$present" = 0 ]; then
      echo "Failed to start xc-v3-backup.service: Unit xc-v3-backup.service not found." >&2
      exit 5
    fi
    rc="${XC_STUB_BACKUP_RC:-0}"
    if [ "$rc" = 0 ] && [ -n "${XC_STUB_BACKUP_SET:-}" ]; then
      s="${XC_STUB_BACKUP_SET##*/}"
      mkdir -m 700 "$XC_STUB_BACKUP_SET"
      printf 'fresh dump\n' >"$XC_STUB_BACKUP_SET/db-$s.dump"
      printf 'fresh archive\n' >"$XC_STUB_BACKUP_SET/uploads-$s.tar.gz"
    fi
    exit "$rc"
    ;;
  cat)
    [ "$present" = 1 ] && cat "$unit_file" && exit 0
    echo "No files found for xc-v3-backup.service." >&2
    exit 1
    ;;
  list-unit-files)
    [ "$present" = 1 ] && echo "xc-v3-backup.service static -" && exit 0
    echo "0 unit files listed."
    exit 1
    ;;
  status) [ "$present" = 1 ] && exit 3 || exit 4 ;;
  is-enabled)
    [ "$present" = 1 ] && echo static && exit 0
    echo "Failed to get unit file state for xc-v3-backup.service: No such file or directory" >&2
    exit 1
    ;;
  show)
    [ "$present" = 1 ] && echo "LoadState=loaded" || echo "LoadState=not-found"
    exit 0
    ;;
esac
exit 0
STUB

# pg_restore: logs, copies its stdin to pg_restore.stdin, and for --list/-l
# prints a table of contents: with _sqlx_migrations (default), without it
# (XC_STUB_LIST=nomig), or fails (XC_STUB_LIST=fail).
write_ustub pg_restore <<'STUB'
#!/usr/bin/env bash
printf 'pg_restore %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/pg_restore.log"
cat >"${XC_TEST_LOG:?}/pg_restore.stdin"
case "${XC_STUB_LIST:-ok}" in
  fail)
    echo "pg_restore: error: input file does not appear to be a valid archive" >&2
    exit 1
    ;;
  nomig)
    printf ';\n; Archive created by stub\n;\n215; 1259 16390 TABLE public files xc_v3\n'
    ;;
  *)
    printf ';\n; Archive created by stub\n;\n214; 1259 16385 TABLE public _sqlx_migrations xc_v3\n215; 1259 16390 TABLE public files xc_v3\n'
    ;;
esac
exit 0
STUB

# psql: the migration count answers the lines of $XC_TEST_MIGRATIONS in
# turn, the last one repeating; anything else prints nothing.
write_ustub psql <<'STUB'
#!/usr/bin/env bash
printf 'psql %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
case "$*" in
  *_sqlx_migrations*)
    f="${XC_TEST_MIGRATIONS:?}"
    head -n 1 "$f"
    if [ "$(wc -l <"$f")" -gt 1 ]; then
      tail -n +2 "$f" >"$f.next"
      mv "$f.next" "$f"
    fi
    ;;
esac
exit 0
STUB

# curl: the smoke check's four requests answer 200, /health/ready with a body
# that has every field smoke.sh reads.
write_ustub curl <<'STUB'
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
  */health/ready) body='{"version":"1.0.0","build":"0123456789ab","builtAt":"2026-10-07T00:00:00Z","channel":"staging","database":true}' ;;
esac
[ -n "$out" ] && printf '%s' "$body" >"$out"
printf '200'
STUB

# ---- helpers ----------------------------------------------------------------
U_OLD_SET=20261006T000000Z
U_NEW_SET=20261007T140000Z

# u_case <dir> — an installed xc-v3 under <dir>: the binary and
# configuration, the env file (naming the uploads and the backup directory),
# xc-v3.service and xc-v3-backup.service in <dir>/systemd, a backup directory
# holding one older set, a stage with another binary, and the migration
# counts 9 then 10.
u_case() {
  local c="$1"
  mkdir -p "$c/log" "$c/opt/xc-v3/configuration" "$c/etc/xc-v3" "$c/var/lib/xc-v3/uploads" "$c/systemd"
  mkdir -m 700 "$c/backups"
  printf 'old-binary-A' >"$c/opt/xc-v3/xchange"
  chmod 755 "$c/opt/xc-v3/xchange"
  printf '# base, installed\n' >"$c/opt/xc-v3/configuration/base.yaml"
  printf '%s\n' \
    "APP_ENVIRONMENT=staging" \
    "APP_APPLICATION__PORT=8094" \
    "APP_DATABASE__DATABASE_NAME=xc_v3" \
    "APP_DATABASE__PASSWORD=pUpdateStubSecret42" \
    "APP_FILES__DIR=$c/var/lib/xc-v3/uploads" \
    "XC_BACKUP_DIR=$c/backups" >"$c/etc/xc-v3/env"
  cp "$HERE/xc-v3.service" "$c/systemd/xc-v3.service"
  printf '[Service]\nType=oneshot\nExecStart=/opt/xc-v3/backup.sh\n' >"$c/systemd/xc-v3-backup.service"
  make_set "$c/backups" "$U_OLD_SET"
  make_stage "$c/stage"
  printf '9\n10\n' >"$c/migrations"
}

# run_update <case dir> [NAME=value...] — the repository's update.sh against
# the case; the backup unit's start leaves the set $U_NEW_SET. OUT/RC.
run_update() {
  local c="$1"
  shift
  set +e
  OUT="$(env PATH="$U_STUBS:$BIN_STUBS:$PATH" XC_TEST_LOG="$c/log" XC_TEST_SYSTEMD="$c/systemd" \
    XC_TEST_MIGRATIONS="$c/migrations" XC_STUB_BACKUP_SET="$c/backups/$U_NEW_SET" \
    STAGE="$c/stage" INSTALL_DIR="$c/opt/xc-v3" ETC_DIR="$c/etc/xc-v3" \
    UPLOADS_DIR="$c/var/lib/xc-v3/uploads" SMOKE_WAIT_SECS=0 \
    "$@" bash "$HERE/update.sh" </dev/null 2>&1)"
  RC=$?
  set -e
}

# first_line <file> <ERE> — number of the first line matching, or 999999
first_line() {
  local n
  n="$(grep -En -- "$2" "$1" 2>/dev/null | head -n 1 | cut -d: -f1)"
  echo "${n:-999999}"
}

U_BACKUP_START='^systemctl (.* )?start (.* )?xc-v3-backup(\.service)?( |$)'
U_LIST='^pg_restore (.* )?(--list|-l)( |$)'
U_STOP='^systemctl (.* )?stop (.* )?xc-v3(\.service)?( |$)'

# u_before <case dir> <ERE a> <ERE b> — a call matching a happened, and before
# the first call matching b (or b never happened)
u_before() {
  local a b
  a="$(first_line "$1/log/calls.log" "$2")"
  b="$(first_line "$1/log/calls.log" "$3")"
  [ "$a" != 999999 ] && [ "$a" -lt "$b" ]
}

# u_no_stop <case dir> — xc-v3 was never stopped
u_no_stop() { absent "$U_STOP" "$1/log/calls.log"; }

# u_untouched <case dir> — the installed binary is the old one, no .prev
u_untouched() {
  [ "$(cat "$1/opt/xc-v3/xchange")" = old-binary-A ] && [ ! -e "$1/opt/xc-v3/xchange.prev" ]
}

# u_as_postgres <case dir> — pg_restore was run as postgres: sudo -u postgres
# or runuser -u postgres, directly or inside a root shell
u_as_postgres() {
  grep -Eqs -- '^sudo (.* )?-u postgres (-- )?pg_restore( |$)' "$1/log/calls.log" ||
    grep -Eqs -- '^-u postgres -- pg_restore( |$)' "$1/log/runuser.log"
}

# names_bk <file> — the word BK appears
names_bk() { grep -Eqs -- '(^|[^A-Za-z0-9_])BK([^A-Za-z0-9_]|$)' "$1"; }

# note_names <out file> <text> — <text> is on the NOTE: line or the 3 after it
note_names() {
  awk -v want="$2" '/^NOTE:/ { n = 4 } n > 0 { if (index($0, want)) found = 1; n-- } END { exit(found ? 0 : 1) }' "$1"
}

# ---- U1: a migration applied — backup, check, then stop; NOTE names the set --
u1="$ROOT/u1"
u_case "$u1"
run_update "$u1"
printf '%s\n' "$OUT" >"$LOGS/u1.out"
U1_DUMP="$u1/backups/$U_NEW_SET/db-$U_NEW_SET.dump"
ck "update.sh exited 0" test "$RC" -eq 0
ck "the backup unit was started before xc-v3 was stopped" u_before "$u1" "$U_BACKUP_START" "$U_STOP"
ck "the dump was checked with pg_restore --list before the stop" u_before "$u1" "$U_LIST" "$U_STOP"
ck "the check came after the backup" u_before "$u1" "$U_BACKUP_START" "$U_LIST"
ck "pg_restore ran as postgres" u_as_postgres "$u1"
ckeq "pg_restore read the new set's dump on stdin" "fresh dump" "$(cat "$u1/log/pg_restore.stdin" 2>/dev/null || echo missing)"
ck "the dump is not named to pg_restore as a path" absent "db-" "$u1/log/pg_restore.log"
ck "xc-v3 was stopped and started" grep -Eqs -- '^systemctl (.* )?start (.* )?xc-v3(\.service)?( |$)' "$u1/log/calls.log"
ckeq "the new binary is installed" "$(cat "$u1/stage/xchange")" "$(cat "$u1/opt/xc-v3/xchange" 2>/dev/null || echo missing)"
ck "a NOTE says migrations were applied" grep -qs '^NOTE:' "$LOGS/u1.out"
ck "the NOTE names the new set's dump" note_names "$LOGS/u1.out" "$U1_DUMP"
ck "no 'backup: not written yet' any more" absent 'not written yet' "$LOGS/u1.out"
ck "the rollback names restore.sh" grep -qs 'restore\.sh' "$LOGS/u1.out"
ck "the rollback names the new set" grep -qsF -- "$u1/backups/$U_NEW_SET" "$LOGS/u1.out"
ck "the older set is not the one named" absent "$U_OLD_SET" "$LOGS/u1.out"
report U1 "a migrating update backs up and checks the dump before the stop, and names the set"

# ---- U2: no migration applied — still backed up first, no NOTE ---------------
u2="$ROOT/u2"
u_case "$u2"
printf '9\n' >"$u2/migrations"
run_update "$u2"
printf '%s\n' "$OUT" >"$LOGS/u2.out"
ck "update.sh exited 0" test "$RC" -eq 0
ck "the backup unit was started before xc-v3 was stopped" u_before "$u2" "$U_BACKUP_START" "$U_STOP"
ck "the dump was checked before the stop" u_before "$u2" "$U_LIST" "$U_STOP"
ck "no NOTE without a migration" absent '^NOTE:' "$LOGS/u2.out"
report U2 "an update with no migration is still backed up first, and prints no NOTE"

# ---- U3: a failing backup stops the update before anything -------------------
u3="$ROOT/u3"
u_case "$u3"
run_update "$u3" XC_STUB_BACKUP_RC=1
printf '%s\n' "$OUT" >"$LOGS/u3.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "the backup unit was started" grep -Eqs -- "$U_BACKUP_START" "$u3/log/calls.log"
ck "xc-v3 was never stopped" u_no_stop "$u3"
ck "nothing was replaced" u_untouched "$u3"
ck "pg_restore never ran" test ! -s "$u3/log/pg_restore.log"
ck "the message names BK" names_bk "$LOGS/u3.out"
report U3 "a failing backup means no stop and no binary swap"

# ---- U4: no backup unit installed — refused -----------------------------------
u4="$ROOT/u4"
u_case "$u4"
rm -f "$u4/systemd/xc-v3-backup.service"
run_update "$u4"
printf '%s\n' "$OUT" >"$LOGS/u4.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "xc-v3 was never stopped" u_no_stop "$u4"
ck "nothing was replaced" u_untouched "$u4"
ck "the message names BK" names_bk "$LOGS/u4.out"
ck "the message names xc-v3-backup.service" grep -qs 'xc-v3-backup\.service' "$LOGS/u4.out"
ck "the message says it is not installed" grep -qsi 'not installed' "$LOGS/u4.out"
report U4 "a host without xc-v3-backup.service refuses to update"

# ---- U5: the new dump fails pg_restore --list ---------------------------------
u5="$ROOT/u5"
u_case "$u5"
run_update "$u5" XC_STUB_LIST=fail
printf '%s\n' "$OUT" >"$LOGS/u5.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "pg_restore --list was run" grep -Eqs -- "$U_LIST" "$u5/log/calls.log"
ck "xc-v3 was never stopped" u_no_stop "$u5"
ck "nothing was replaced" u_untouched "$u5"
ck "the message names the dump" grep -qsF -- "$u5/backups/$U_NEW_SET/db-$U_NEW_SET.dump" "$LOGS/u5.out"
report U5 "a dump pg_restore cannot list means no update"

# ---- U5b: the new dump lists no _sqlx_migrations ------------------------------
u5b="$ROOT/u5b"
u_case "$u5b"
run_update "$u5b" XC_STUB_LIST=nomig
printf '%s\n' "$OUT" >"$LOGS/u5b.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "pg_restore --list was run" grep -Eqs -- "$U_LIST" "$u5b/log/calls.log"
ck "xc-v3 was never stopped" u_no_stop "$u5b"
ck "nothing was replaced" u_untouched "$u5b"
ck "the message names _sqlx_migrations" grep -qs '_sqlx_migrations' "$LOGS/u5b.out"
report U5b "a dump that lists no _sqlx_migrations means no update"

# ---- U5c: the backup left no new set — the older one does not count ----------
u5c="$ROOT/u5c"
u_case "$u5c"
run_update "$u5c" XC_STUB_BACKUP_SET=
printf '%s\n' "$OUT" >"$LOGS/u5c.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "the backup unit was started" grep -Eqs -- "$U_BACKUP_START" "$u5c/log/calls.log"
ck "xc-v3 was never stopped" u_no_stop "$u5c"
ck "nothing was replaced" u_untouched "$u5c"
ck "the older set's dump was not taken for the new one" absent '^old dump$' "$u5c/log/pg_restore.stdin"
ck "the message names the backup directory" grep -qsF -- "$u5c/backups" "$LOGS/u5c.out"
report U5c "a backup that left no new set means no update"

echo "# $PASS passed, $FAILURES failed"
[ "$FAILURES" -eq 0 ]

