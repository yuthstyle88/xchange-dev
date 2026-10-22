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
#   T7/T7b  render_unit (#1204): the unit loads the server's env file and the
#        optional Identity file; a non-default ENV_FILE rewrites only the
#        server's env line, never the Identity line
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
# Cases marked BK-ENC are the encrypted backups (design §7b "Encryption of
# every set"), with age stubbed (base64 under the age header line, so the
# plain text never appears in what it writes):
#   B1   (BK-ENC) the set holds db-<stamp>.dump.age and uploads-<stamp>.tar.gz.age
#        only, each starting "age-encryption.org/v1", encrypted to the
#        recipients file; no plain dump or archive is left anywhere in the
#        backup directory or TMPDIR; the plain dump was checked with
#        pg_restore --list
#   B6   (BK-ENC) the off-host filter sends the .age files of the set
#        directories only — never a plain file inside a set directory (a set
#        made before BK-ENC), the working directory, the lock or another name
#   B7c  (BK-ENC) restore decrypts with XC_BACKUP_IDENTITY and hands the plain
#        dump to pg_restore on stdin
#   B7e  (BK-ENC) restore refuses an empty, missing, or group/other-readable
#        XC_BACKUP_IDENTITY before creating anything
#   B7f  (BK-ENC) a decryption that fails drops the scratch database and
#        directory the rehearsal made
#   B9b  (BK-ENC) env.example names XC_BACKUP_RECIPIENTS, holds no identity
#   B11  (BK-ENC) a missing, empty or comment-only recipients file fails the
#        backup before pg_dump, with nothing written
#   B11b (BK-ENC) a failing age leaves no set, no plain file, prunes nothing
#   B12  (BK-ENC) a dump pg_restore cannot list, or that lists no
#        _sqlx_migrations, makes no set
#   B12b (BK-ENC) a dump list larger than 1 MiB containing _sqlx_migrations
#        makes a set under pipefail (#1162)
#   B13  (BK-ENC) pg_dump toward a non-local host runs with PGSSLMODE=require
#        (or verify-full); toward a local host it is not forced
#   B13b (BK-ENC) locality rule matches DB-LINK: loopback IPs and sockets local;
#        names that only look local are non-local and require TLS
#   B13c (BK-ENC) non-local host keeps verify-ca or verify-full and upgrades
#        weaker modes to require
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
#   U5c  no new set appeared: no stop, nothing replaced
# After BK-ENC the server cannot read a set, so update.sh checks that the new
# set is encrypted instead of running pg_restore (backup.sh runs that check):
#   U1/U2 (BK-ENC) the new set's .age dump is read (through sudo) before the
#        stop; pg_restore never runs and age never decrypts; the NOTE names
#        db-<stamp>.dump.age; the rollback names XC_BACKUP_IDENTITY
#   U5   (BK-ENC) a .age dump without the age header line: no stop, nothing
#        replaced, the message names the file
#   U5b  (BK-ENC) a plain (pre-BK-ENC) set: no stop, nothing replaced
#   U5d  (BK-ENC) a set without the .age uploads archive: no stop
#   U5e  (BK-ENC) a set whose dump lacks the age header is refused even if the
#        uploads archive has it (mutant E10)
# Since #1084 every update case is an installed host whose /etc/xc-v3 holds a
# pii.key (a placeholder of 32 zero bytes, never a real key) and whose stage
# carries migrations/ (one already applied, one pending); psql, pg_dump,
# createdb and dropdb share a fake cluster (one file per database) and log
# every operation as a "pgop" line, so what touched which database is known.
# Cases K1-K3 are the PII key file (#1084), R1-R3 the migration rehearsal:
#   K1   install.sh on a clean host writes $ETC_DIR/pii.key: one k1: line of
#        base64 of 32 bytes, root:<service group> 640, and the key never shows
#        on stdout, stderr or any command line the stubs saw
#   K2   install.sh run again keeps an existing key byte for byte, and says so
#   K3   update.sh without the key refuses before anything: no systemctl at
#        all (so no backup and no stop), no pg_dump, nothing replaced; the
#        message names the file and how to create it
#   R1   a pending migration that refuses on a copy of the live database stops
#        update.sh before the backup: the failing file and its error printed,
#        later migrations not tried, the scratch database dropped, the live
#        database never written
#   R2   a clean set is rehearsed on a scratch copy loaded from a dump of the
#        live database (only the pending migration, before the backup), the
#        scratch is dropped and the update goes on
#   R3   a stage without migrations/ cannot be rehearsed: refused before the
#        backup
# Cases SM1/SM2 are smoke.sh reading the `ui` field /health/ready gained in
# V0-A (docs/design/xchange-v3-web.md §3.2), with curl stubbed:
#   SM1  ui "missing", or no ui field at all: exit 1, a FAIL line naming ui
#   SM2  ui "built": exit 0, an ok line naming ui
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
  UPLOADS_DIR="$t1/var/lib/$T1_SERVICE/uploads" \
  VERIFICATION_STORE_DIR="$t1/var/lib/$T1_SERVICE/verification-documents" \
  V2_BUNDLE_DIR="$t1/v2bundle" \
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

# ---- T7: the rendered unit loads both env files (#1204) ---------------------
run_fn "$ROOT/empty-stage" "$ROOT/t7inst" "" render_unit
printf '%s\n' "$OUT" >"$LOGS/t7.out"
ck "render_unit succeeded" test "$RC" -eq 0
ck "the unit reads the server's env file" grep -qsx 'EnvironmentFile=/etc/xc-v3/env' "$LOGS/t7.out"
ck "the unit reads the optional Identity file" grep -qsx 'EnvironmentFile=-/etc/xc-v3/identity-client.env' "$LOGS/t7.out"
ckeq "exactly two EnvironmentFile lines" "2" "$(grep -c '^EnvironmentFile=' "$LOGS/t7.out")"
ck "the unit's ReadWritePaths holds both the uploads and verification directories" grep -qsx 'ReadWritePaths=/var/lib/xc-v3/uploads /var/lib/xc-v3/verification-documents' "$LOGS/t7.out"
report T7 "the rendered unit loads the server's env file and the optional Identity file"

# ---- T7b: a non-default ENV_FILE leaves the Identity line alone (#1204) -----
t7b="$ROOT/t7b"
mkdir -p "$t7b"
printf 'ENV_FILE=/tmp/x.env\nUPLOADS_DIR=/var/lib/xc-v3/custom-uploads\nVERIFICATION_STORE_DIR=/var/lib/xc-v3/custom-verification\n' >"$t7b/pre.sh"
run_fn "$ROOT/empty-stage" "$ROOT/t7binst" "$t7b/pre.sh" render_unit
printf '%s\n' "$OUT" >"$LOGS/t7b.out"
ck "render_unit succeeded" test "$RC" -eq 0
ck "the server's env file is the one given" grep -qsx 'EnvironmentFile=/tmp/x.env' "$LOGS/t7b.out"
ck "the Identity line is unchanged" grep -qsx 'EnvironmentFile=-/etc/xc-v3/identity-client.env' "$LOGS/t7b.out"
ckeq "no line still names the default env file" "0" "$(grep -cx 'EnvironmentFile=/etc/xc-v3/env' "$LOGS/t7b.out")"
ck "ReadWritePaths follows UPLOADS_DIR and VERIFICATION_STORE_DIR" grep -qsx 'ReadWritePaths=/var/lib/xc-v3/custom-uploads /var/lib/xc-v3/custom-verification' "$LOGS/t7b.out"
report T7b "a non-default ENV_FILE rewrites only the server's env line"

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
printf 'PGSSLMODE=%s\n' "${PGSSLMODE:-}" >>"${XC_TEST_LOG:?}/pg_dump.env"
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
elif [ -z "$out" ] || [ "$out" = - ]; then
  # extracting from standard input (e.g. "age -d ... | tar -xz"): read it all
  cat >/dev/null
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
list=0
for a in "$@"; do
  case "$a" in --list | -l) list=1 ;; esac
done
if [ "$list" = 1 ]; then
  # BK-ENC: backup.sh lists the plain dump before encrypting it. A table of
  # contents with _sqlx_migrations (default), without it (XC_STUB_LIST=nomig),
  # or a failure (XC_STUB_LIST=fail).
  cat >/dev/null
  case "${XC_STUB_LIST:-ok}" in
    fail)
      echo "pg_restore: error: input file does not appear to be a valid archive" >&2
      exit 1
      ;;
    nomig) printf ';\n215; 1259 16390 TABLE public files xc_v3\n' ;;
    large)
      printf ';\n214; 1259 16385 TABLE public _sqlx_migrations xc_v3\n'
      head -c 1200000 </dev/zero | tr '\0' 'x'
      printf '\n'
      ;;
    *) printf ';\n214; 1259 16385 TABLE public _sqlx_migrations xc_v3\n215; 1259 16390 TABLE public files xc_v3\n' ;;
  esac
  exit 0
fi
cat >"${XC_TEST_LOG:?}/pg_restore.stdin"
[ "${XC_STUB_PG_RESTORE_FAIL:-0}" = 1 ] && exit 1
exit 0
STUB

# age (BK-ENC): logs its command line to age.log. Encrypting needs -r age1…
# or -R <file> holding a line starting "age1" (a missing, empty or
# comment-only file is refused before any output is written, as age 1.1.1
# does); it writes the three header lines of this stub's format and then its
# input in base64 — so the plain text never appears in its output — to -o
# <file> or stdout, reading the last operand or stdin. XC_STUB_AGE_FAIL=1:
# writes the first header line and exits 1 (a partial file). Decrypting (-d)
# needs -i <file> holding a line starting "AGE-SECRET-KEY-1" and an input
# starting "age-encryption.org/v1"; it writes the decoded body.
write_stub age <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${XC_TEST_LOG:?}/age.log"
mode=enc out="" in="" rcpt=0 idf="" take=""
for a in "$@"; do
  if [ -n "$take" ]; then
    case "$take" in
      o) out="$a" ;;
      R)
        if [ -r "$a" ] && grep -q '^age1' "$a"; then
          rcpt=1
        else
          echo "age: error: failed to parse recipient file \"$a\"" >&2
          exit 1
        fi
        ;;
      r)
        case "$a" in
          age1*) rcpt=1 ;;
          *)
            echo "age: error: malformed recipient \"$a\"" >&2
            exit 1
            ;;
        esac
        ;;
      i) idf="$a" ;;
    esac
    take=""
    continue
  fi
  case "$a" in
    -d | --decrypt) mode=dec ;;
    -o | --output) take=o ;;
    --output=*) out="${a#--output=}" ;;
    -R | --recipients-file) take=R ;;
    -r | --recipient) take=r ;;
    -i | --identity) take=i ;;
    -e | --encrypt) ;;
    -) in="" ;;
    -*)
      echo "age stub: unknown flag $a" >&2
      exit 2
      ;;
    *) in="$a" ;;
  esac
done
src() { if [ -n "$in" ]; then cat -- "$in"; else cat; fi; }
emit() { if [ -n "$out" ] && [ "$out" != - ]; then cat >"$out"; else cat; fi; }
if [ "$mode" = enc ]; then
  if [ "$rcpt" != 1 ]; then
    echo "age: error: missing recipients" >&2
    exit 1
  fi
  if [ "${XC_STUB_AGE_FAIL:-0}" = 1 ]; then
    src >/dev/null
    printf 'age-encryption.org/v1\n' | emit
    exit 1
  fi
  { printf 'age-encryption.org/v1\n-> X25519 stub\n--- stub\n'; src | base64; } | emit
  exit 0
fi
if [ -z "$idf" ] || [ ! -r "$idf" ] || ! grep -q '^AGE-SECRET-KEY-1' "$idf"; then
  src >/dev/null
  echo "age: error: no identity matched any of the recipients" >&2
  exit 1
fi
tmp="$(mktemp)"
src >"$tmp"
if [ "$(head -n 1 "$tmp")" != age-encryption.org/v1 ]; then
  rm -f "$tmp"
  echo "age: error: failed to read header" >&2
  exit 1
fi
tail -n +4 "$tmp" | base64 -d | emit
rc=$?
rm -f "$tmp"
exit "$rc"
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
# var/up-files with one file in it, the stamps the date stub hands out, a
# TMPDIR tmp/, the recipients file (BK-ENC) and the owner's identity (0600).
bk_case() {
  local c="$1"
  shift
  mkdir -p "$c/log" "$c/var/up-files" "$c/tmp"
  printf 'an uploaded file\n' >"$c/var/up-files/f1"
  printf '%s\n' "$@" >"$c/stamps"
  printf '# xc-v3 backups\nage1stubrecipientxcv3backups\n' >"$c/recipients"
  printf '# created: stub\nAGE-SECRET-KEY-1STUBIDENTITYXCV3\n' >"$c/identity"
  chmod 600 "$c/identity"
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
    XC_BACKUP_SSH_KEY="$c/offhost.key" XC_BACKUP_RECIPIENTS="$c/recipients" \
    TMPDIR="$c/tmp" PGSSLMODE= \
    APP_DATABASE__HOST=127.0.0.1 APP_DATABASE__PORT=5432 \
    APP_DATABASE__USERNAME="$BK_ROLE" APP_DATABASE__PASSWORD="$BK_SECRET" \
    APP_DATABASE__DATABASE_NAME="$BK_DB" APP_FILES__DIR="$c/var/up-files" \
    "$@" bash "$HERE/backup.sh" </dev/null 2>&3 | tee "$c/stdout"; } 3>&1)"
  RC=$?
  set -e
}

# run_restore <case dir> <args...> — restore.sh with the stubs, an env
# file naming the live database and uploads directory, and
# XC_BACKUP_IDENTITY=<case>/identity (BK-ENC) — or $RESTORE_IDENTITY when that
# is set, even to empty.
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
    XC_BACKUP_IDENTITY="${RESTORE_IDENTITY-$c/identity}" \
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

# age_stub_file <text> <file> — <text> in the age stub's format (BK-ENC)
age_stub_file() {
  { printf 'age-encryption.org/v1\n-> X25519 stub\n--- stub\n'; printf '%s\n' "$1" | base64; } >"$2"
}

# make_set <backups dir> <stamp> — a finished set, as backup.sh leaves one
# after BK-ENC: the two pieces encrypted
make_set() {
  mkdir -p "$1/$2"
  age_stub_file 'old dump' "$1/$2/db-$2.dump.age"
  age_stub_file 'old archive' "$1/$2/uploads-$2.tar.gz.age"
}

# make_plain_set <backups dir> <stamp> — a set as backup.sh made it before
# BK-ENC: the two pieces in plain text
make_plain_set() {
  mkdir -p "$1/$2"
  printf 'PGDMP stub dump\n' >"$1/$2/db-$2.dump"
  printf 'tar stub archive\n' >"$1/$2/uploads-$2.tar.gz"
}

# no_plaintext <dir...> — no file under the directories holds the plain text
# the pg_dump and tar stubs write
no_plaintext() { ! grep -rqsE 'PGDMP stub dump|tar stub archive' "$@"; }

# all_age <dir> — the directory holds at least one file, and every entry is a
# file whose first line is the age header
all_age() {
  local f n=0
  for f in "$1"/* "$1"/.[!.]*; do
    [ -e "$f" ] || continue
    [ -f "$f" ] || return 1
    [ "$(head -n 1 "$f")" = age-encryption.org/v1 ] || return 1
    n=$((n + 1))
  done
  [ "$n" -gt 0 ]
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
ckeq "the set holds the encrypted dump and archive only, same stamp (BK-ENC)" \
  "db-$s.dump.age uploads-$s.tar.gz.age " "$(entries_of "$b1/backups/$s")"
ck "every file of the set starts with the age header (BK-ENC)" all_age "$b1/backups/$s"
ck "no plain dump or archive anywhere in the backup directory or TMPDIR (BK-ENC)" \
  no_plaintext "$b1/backups" "$b1/tmp"
ck "age encrypted to the recipients file (BK-ENC)" grep -qsF -- "$b1/recipients" "$b1/log/age.log"
ck "the plain dump was checked with pg_restore --list (BK-ENC)" \
  grep -Eqs -- '(^| )(--list|-l)( |$)' "$b1/log/pg_restore.log"
ck "the server never decrypts (BK-ENC)" absent '(^| )(-d|--decrypt)( |$)' "$b1/log/age.log"
ck "pg_dump asked for the custom format" \
  grep -Eqs -- '(^| )(-Fc|-F c|--format=c|--format=custom)( |$)' "$b1/log/pg_dump.log"
ck "pg_dump named the database" grep -qs -- "$BK_DB" "$b1/log/pg_dump.log"
ck "the password is not on pg_dump's command line" absent "$BK_SECRET" "$b1/log/pg_dump.log"
ck "tar archived the uploads directory" grep -qs -- "up-files" "$b1/log/tar.log"
ckeq "nothing but the set is left in the backup directory" "$s " "$(entries_of "$b1/backups")"
ckeq "the last line of stdout is the set's path" "$b1/backups/$s" "$(last_line "$b1/stdout")"
ckeq "the backup directory it created is 0700" "700" "$(mode_of "$b1/backups")"
ckeq "the set directory is 0700" "700" "$(mode_of "$b1/backups/$s")"
report B1 "one run makes one set: encrypted dump and uploads archive under one UTC stamp"

# ---- B1b: a stamp that already exists is refused ---------------------------
b1b="$ROOT/b1b"
bk_case "$b1b" 20261007T020000Z 20261007T020000Z
run_backup "$b1b"
B1B_FIRST_RC=$RC
s=20261007T020000Z
cp "$b1b/backups/$s/db-$s.dump.age" "$b1b/first.dump" 2>/dev/null || true
run_backup "$b1b"
printf '%s\n' "$OUT" >"$LOGS/b1b.out"
ck "the first run exited 0" test "$B1B_FIRST_RC" -eq 0
ck "the second run exited non-zero" test "$RC" -ne 0
ck "the refusal names the stamp" grep -qs -- "$s" "$LOGS/b1b.out"
ckeq "still one set" "$s " "$(sets_in "$b1b/backups")"
ck "the first set's dump is unchanged" cmp -s "$b1b/first.dump" "$b1b/backups/$s/db-$s.dump.age"
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
ckeq "the local set is kept, both pieces" "db-$s.dump.age uploads-$s.tar.gz.age " "$(entries_of "$b5/backups/$s")"
report B5 "a failing off-host copy exits non-zero and keeps the local set"

# ---- B6: --delete only once the local directory holds 7 sets ---------------
b6="$ROOT/b6"
bk_case "$b6" 20261007T080000Z
mkdir -m 700 "$b6/backups"
# a set made before BK-ENC, plain, still in the local directory
make_plain_set "$b6/backups" 20260901T000009Z
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
# ---- an rsync filter, evaluated as rsync 3.2 does (BK-ENC) -------------------
# not_sends <args file> <path> [is_dir] — rsync_sends fails
not_sends() { ! rsync_sends "$@"; }
# rsync_rules <args file> — "+ <pattern>" / "- <pattern>" for every
# --include/--exclude (one argument or two), in order. The implementation must
# filter with --include/--exclude; --filter/-f forms are not read here.
rsync_rules() {
  local take="" a
  [ -f "$1" ] || return 0
  while IFS= read -r a; do
    if [ -n "$take" ]; then
      printf '%s %s\n' "$take" "$a"
      take=""
      continue
    fi
    case "$a" in
      --include=*) printf '+ %s\n' "${a#--include=}" ;;
      --exclude=*) printf -- '- %s\n' "${a#--exclude=}" ;;
      --include) take=+ ;;
      --exclude) take=- ;;
    esac
  done <"$1"
}
# glob_path <pattern> <path> — the same number of /-separated components, each
# matched by bash globbing ("*", "?" and "[..]" never cross a "/", as in rsync)
glob_path() {
  local -a pc sc
  local i
  IFS=/ read -ra pc <<<"$1"
  IFS=/ read -ra sc <<<"$2"
  [ "${#pc[@]}" -eq "${#sc[@]}" ] || return 1
  for i in "${!pc[@]}"; do
    # shellcheck disable=SC2053  # the component is a pattern on purpose
    [[ "${sc[i]}" == ${pc[i]} ]] || return 1
  done
}
# rule_matches <pattern> <path> <is_dir 0|1> — rsync's matching of one rule:
# a trailing "/***" matches the directory and everything below it; a
# trailing "/" matches directories only; a leading "/" anchors at the
# transfer root; a pattern with no "/" matches the last component; one with
# an inner "/" matches the trailing components.
rule_matches() {
  local p="$1" path="$2" isdir="$3" k n pre
  local -a cs
  if [[ "$p" == */\*\*\* ]]; then
    p="${p%/\*\*\*}"
    IFS=/ read -ra cs <<<"$path"
    n="${#cs[@]}"
    pre=""
    for ((k = 0; k < n; k++)); do
      pre="${pre:+$pre/}${cs[k]}"
      if [ "$k" -lt $((n - 1)) ] || [ "$isdir" = 1 ]; then
        rule_matches "$p/" "$pre" 1 && return 0
      fi
      rule_matches "$p" "$pre" 0 && return 0
    done
    return 1
  fi
  if [[ "$p" == */ ]]; then
    [ "$isdir" = 1 ] || return 1
    p="${p%/}"
  fi
  if [[ "$p" == /* ]]; then
    glob_path "${p#/}" "$path"
    return
  fi
  if [[ "$p" == */* ]]; then
    local -a pc
    IFS=/ read -ra pc <<<"$p"
    IFS=/ read -ra cs <<<"$path"
    [ "${#cs[@]}" -ge "${#pc[@]}" ] || return 1
    local IFS=/
    glob_path "$p" "${cs[*]: -${#pc[@]}}"
    return
  fi
  # shellcheck disable=SC2053  # $p is a pattern on purpose
  [[ "${path##*/}" == $p ]]
}
# rsync_sends <args file> <relative path> [is_dir 0|1] — whether rsync, given
# those arguments, sends that path: the first matching rule decides for each
# component from the top (no match = included), and an excluded directory is
# never entered.
rsync_sends() {
  local args="$1" path="$2" isdir="${3:-0}" rules pre="" k n d t pat verdict
  local -a cs
  rules="$(rsync_rules "$args")"
  IFS=/ read -ra cs <<<"$path"
  n="${#cs[@]}"
  for ((k = 0; k < n; k++)); do
    pre="${pre:+$pre/}${cs[k]}"
    d=1
    [ "$k" -eq $((n - 1)) ] && d="$isdir"
    verdict=+
    while read -r t pat; do
      [ -n "$t" ] || continue
      if rule_matches "$pat" "$pre" "$d"; then
        verdict="$t"
        break
      fi
    done <<<"$rules"
    [ "$verdict" = - ] && return 1
  done
  return 0
}
# The simulator against what rsync 3.2.7 did on scratch directories
# (design §7b, Off-host copy): it must agree before the cases use it.
SIM="$ROOT/sim.args"
G='[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z'
printf '%s\n' -a "--include=/$G/" "--include=/$G/*.age" '--exclude=*' src/ dst/ >"$SIM"
ck "sim: a set directory is entered" rsync_sends "$SIM" 20261007T080000Z 1
ck "sim: its .age file is sent" rsync_sends "$SIM" 20261007T080000Z/db-20261007T080000Z.dump.age
ck "sim: a plain file in it is not" not_sends "$SIM" 20261007T080000Z/stray.dump
ck "sim: the working directory is not" not_sends "$SIM" .work-20261007T090000Z/db.dump
ck "sim: the lock is not" not_sends "$SIM" .lock
ck "sim: another directory is not" not_sends "$SIM" manual/k
printf '%s\n' -a "--include=/$G/***" '--exclude=*' src/ dst/ >"$SIM"
ck "sim: /<stamp>/*** sends a plain file of a set" rsync_sends "$SIM" 20261007T080000Z/stray.dump
ck "sim: /<stamp>/*** does not send the lock" not_sends "$SIM" .lock
report B0 "the rsync filter simulator agrees with rsync 3.2.7 on the measured cases"

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
ckeq "one rsync with two local sets" "1" "$( (cat "$b6/log/rsync.log" 2>/dev/null || true) | wc -l | tr -d ' ')"
ck "no --delete with two local sets" absent '^--delete$' "$B6_ARGS"
ck "the ssh key is used" grep -qs -- "$b6/offhost.key" "$b6/log/rsync.log"
ck "ssh never prompts (BatchMode=yes)" grep -qs "BatchMode=yes" "$b6/log/rsync.log"
ckeq "the source is the backup directory's contents" "$b6/backups/" "$(tail -n 2 "$B6_ARGS" 2>/dev/null | head -n 1)"
ckeq "the target is the off-host directory" "$B6_OFF/" "$(last_line "$B6_ARGS")"
ck "everything not included is excluded (--exclude='*')" excludes_all "$B6_ARGS"
ck "excluded names on the target are never deleted" absent '^--delete-excluded$' "$B6_ARGS"
ck "a set directory is entered" rsync_sends "$B6_ARGS" 20261007T080000Z 1
ck "its encrypted dump is sent (BK-ENC)" rsync_sends "$B6_ARGS" 20261007T080000Z/db-20261007T080000Z.dump.age
ck "its encrypted archive is sent (BK-ENC)" rsync_sends "$B6_ARGS" 20261007T080000Z/uploads-20261007T080000Z.tar.gz.age
ck "a plain dump in a set directory is not sent (BK-ENC)" \
  not_sends "$B6_ARGS" 20260901T000009Z/db-20260901T000009Z.dump
ck "a plain archive in a set directory is not sent (BK-ENC)" \
  not_sends "$B6_ARGS" 20260901T000009Z/uploads-20260901T000009Z.tar.gz
ck "the working directory is not" not_sends "$B6_ARGS" .work-20261007T080000Z/db-20261007T080000Z.dump
ck "the lock is not" not_sends "$B6_ARGS" .lock
ck "a stamp with a suffix is not" not_sends "$B6_ARGS" 20261007T080000Z.old/db-20261007T080000Z.dump.age
ck "another name is not" not_sends "$B6_ARGS" notes.txt
ck "the run reaching 7 sets exited 0" test "$B6_RC7" -eq 0
ck "--delete once 7 sets are local" grep -qsx -- '--delete' "$b6f/log/rsync.args"
ckeq "with 7 sets the source is still the directory's contents" "$b6f/backups/" "$(tail -n 2 "$b6f/log/rsync.args" 2>/dev/null | head -n 1)"
ck "with 7 sets a set's encrypted dump is sent" rsync_sends "$b6f/log/rsync.args" 20260901T000001Z/db-20260901T000001Z.dump.age
report B6 "the off-host copy sends the .age files of the set directories only, with --delete once 7 are local"

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
ck "age decrypted the set's dump with the identity (BK-ENC)" \
  grep -Eqs -- "(^| )(-d|--decrypt)( |$).*$b7c/identity|$b7c/identity.*(^| )(-d|--decrypt)( |$)" "$b7c/log/age.log"
ck "age decrypted the set's dump (BK-ENC)" grep -qs -- "db-$s.dump.age" "$b7c/log/age.log"
ck "age decrypted the set's archive (BK-ENC)" grep -qs -- "uploads-$s.tar.gz.age" "$b7c/log/age.log"
ck "the identity's key is never printed (BK-ENC)" absent 'AGE-SECRET-KEY' "$LOGS/b7c.out"
ck "tar extracted into the scratch directory" grep -qs -- "$b7c/scratch-files" "$b7c/log/tar.log"
ckeq "the live uploads directory is unchanged" "f1 " "$(entries_of "$b7c/var/up-files")"
ck "createdb ran as postgres through runuser" grep -Eqs -- '^-u postgres -- createdb( .*)? xc_bk_rehearsal( |$)' "$b7c/log/runuser.log"
ck "pg_restore ran as postgres through runuser" grep -Eqs -- '^-u postgres -- pg_restore( |$).*xc_bk_rehearsal' "$b7c/log/runuser.log"
ck "nothing was dropped" test ! -s "$b7c/log/dropdb.log"
report B7c "a rehearsal decrypts and restores into the scratch database and directory"

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

# ---- B7e: restore.sh needs the identity, closed to others (BK-ENC) ----------
b7e="$ROOT/b7e"
bk_case "$b7e"
s=20261007T121500Z
make_set "$b7e/backups" "$s"
RESTORE_IDENTITY=""
run_restore "$b7e" "$b7e/backups/$s" xc_bk_rehearsal "$b7e/scratch-files"
B7E_RC_EMPTY=$RC
printf '%s\n' "$OUT" >"$LOGS/b7e-1.out"
RESTORE_IDENTITY="$b7e/no-such-identity"
run_restore "$b7e" "$b7e/backups/$s" xc_bk_rehearsal "$b7e/scratch-files"
B7E_RC_MISSING=$RC
printf '%s\n' "$OUT" >"$LOGS/b7e-2.out"
cp "$b7e/identity" "$b7e/identity-open"
chmod 644 "$b7e/identity-open"
RESTORE_IDENTITY="$b7e/identity-open"
run_restore "$b7e" "$b7e/backups/$s" xc_bk_rehearsal "$b7e/scratch-files"
B7E_RC_OPEN=$RC
printf '%s\n' "$OUT" >"$LOGS/b7e-3.out"
unset RESTORE_IDENTITY
ck "an empty XC_BACKUP_IDENTITY exited non-zero" test "$B7E_RC_EMPTY" -ne 0
ck "a missing identity file exited non-zero" test "$B7E_RC_MISSING" -ne 0
ck "an identity readable by others exited non-zero" test "$B7E_RC_OPEN" -ne 0
for i in 1 2 3; do
  ck "refusal $i names XC_BACKUP_IDENTITY" grep -qs XC_BACKUP_IDENTITY "$LOGS/b7e-$i.out"
done
ck "no database was created" test ! -s "$b7e/log/createdb.log"
ck "pg_restore never ran" test ! -s "$b7e/log/pg_restore.log"
ck "nothing was decrypted" absent '(^| )(-d|--decrypt)( |$)' "$b7e/log/age.log"
ck "the scratch directory was not created" test ! -e "$b7e/scratch-files"
ck "the identity's key is never printed" not grep -qs 'AGE-SECRET-KEY' "$LOGS/b7e-1.out" "$LOGS/b7e-2.out" "$LOGS/b7e-3.out"
report B7e "restore.sh refuses an empty, missing or group/other-readable identity before anything"

# ---- B7f: a failed decryption drops what the rehearsal made (BK-ENC) --------
b7f="$ROOT/b7f"
bk_case "$b7f"
s=20261007T123000Z
make_set "$b7f/backups" "$s"
printf '# another key\nNOT-THE-SECRET\n' >"$b7f/wrong-identity"
chmod 600 "$b7f/wrong-identity"
RESTORE_IDENTITY="$b7f/wrong-identity"
run_restore "$b7f" "$b7f/backups/$s" xc_bk_rehearsal "$b7f/scratch-files"
unset RESTORE_IDENTITY
printf '%s\n' "$OUT" >"$LOGS/b7f.out"
ck "restore.sh exited non-zero" test "$RC" -ne 0
ck "age was asked to decrypt" grep -Eqs -- '(^| )(-d|--decrypt)( |$)' "$b7f/log/age.log"
ck "the scratch database was dropped as postgres" grep -Eqs -- '^-u postgres -- dropdb( .*)? xc_bk_rehearsal( |$)' "$b7f/log/runuser.log"
ck "the live database was never dropped" absent "(^| )$BK_DB( |$)" "$b7f/log/dropdb.log"
ck "the scratch directory was removed" test ! -e "$b7f/scratch-files"
ckeq "the live uploads directory is unchanged" "f1 " "$(entries_of "$b7f/var/up-files")"
report B7f "a failed decryption drops the scratch database and directory it made"

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
for v in XC_BACKUP_DIR XC_BACKUP_KEEP XC_BACKUP_OFFHOST XC_BACKUP_SSH_KEY APP_VERIFICATION__STORE_DIR; do
  ck "env.example names $v" grep -Eqs "^#?$v=" "$HERE/env.example"
done
report B9 "env.example names every XC_BACKUP_* setting"

# ---- B9b: the recipients setting, and no identity in the env file (BK-ENC) --
ck "env.example names XC_BACKUP_RECIPIENTS" grep -Eqs "^#?XC_BACKUP_RECIPIENTS=" "$HERE/env.example"
ck "env.example sets no XC_BACKUP_IDENTITY" absent '^#?XC_BACKUP_IDENTITY=' "$HERE/env.example"
ck "env.example holds no secret key" absent 'AGE-SECRET-KEY' "$HERE/env.example"
report B9b "env.example names XC_BACKUP_RECIPIENTS and holds no identity"

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
  XC_BACKUP_IDENTITY="$b10/identity" \
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

# ---- B11: no usable recipient = no backup, before pg_dump (BK-ENC) ----------
b11="$ROOT/b11"
bk_case "$b11" 20261007T131500Z 20261007T131501Z 20261007T131502Z
mkdir -m 700 "$b11/backups"
: >"$b11/recipients-empty"
printf '# no key here\n\n' >"$b11/recipients-comment"
B11_RCS=""
i=0
for r in "$b11/no-such-recipients" "$b11/recipients-empty" "$b11/recipients-comment"; do
  i=$((i + 1))
  run_backup "$b11" XC_BACKUP_RECIPIENTS="$r"
  B11_RCS="$B11_RCS$RC "
  printf '%s\n' "$OUT" >"$LOGS/b11-$i.out"
done
ck "a missing file exited non-zero" test "$(echo "$B11_RCS" | cut -d' ' -f1)" -ne 0
ck "an empty file exited non-zero" test "$(echo "$B11_RCS" | cut -d' ' -f2)" -ne 0
ck "a comment-only file exited non-zero" test "$(echo "$B11_RCS" | cut -d' ' -f3)" -ne 0
for i in 1 2 3; do
  ck "refusal $i names XC_BACKUP_RECIPIENTS" grep -qs XC_BACKUP_RECIPIENTS "$LOGS/b11-$i.out"
done
ck "pg_dump never ran" test ! -s "$b11/log/pg_dump.log"
ck "tar never ran" test ! -s "$b11/log/tar.log"
ckeq "nothing was written in the backup directory" "" "$(entries_of "$b11/backups")"
ck "no plain file anywhere" no_plaintext "$b11/backups" "$b11/tmp"
report B11 "a missing, empty or comment-only recipients file fails before pg_dump"

# ---- B11b: a failing age leaves no set and no plain file (BK-ENC) ------------
b11b="$ROOT/b11b"
bk_case "$b11b" 20261007T132000Z
mkdir -m 700 "$b11b/backups"
for i in 1 2 3 4 5 6 7; do make_set "$b11b/backups" "20260901T00000${i}Z"; done
B11B_BEFORE="$(entries_of "$b11b/backups")"
run_backup "$b11b" XC_STUB_AGE_FAIL=1
printf '%s\n' "$OUT" >"$LOGS/b11b.out"
ck "backup.sh exited non-zero" test "$RC" -ne 0
ck "age was run" test -s "$b11b/log/age.log"
ckeq "the directory is as before: no partial set, nothing pruned" "$B11B_BEFORE" "$(entries_of "$b11b/backups")"
ck "no plain file anywhere" no_plaintext "$b11b/backups" "$b11b/tmp"
report B11b "a failing encryption leaves no set and no plain file, prunes nothing"

# ---- B12: the plain dump is checked before it is encrypted (BK-ENC) ---------
b12="$ROOT/b12"
bk_case "$b12" 20261007T133000Z 20261007T133001Z
mkdir -m 700 "$b12/backups"
run_backup "$b12" XC_STUB_LIST=nomig
B12_RC_NOMIG=$RC
printf '%s\n' "$OUT" >"$LOGS/b12.out"
run_backup "$b12" XC_STUB_LIST=fail
B12_RC_FAIL=$RC
printf '%s\n' "$OUT" >>"$LOGS/b12.out"
ck "a dump listing no _sqlx_migrations exited non-zero" test "$B12_RC_NOMIG" -ne 0
ck "a dump pg_restore cannot list exited non-zero" test "$B12_RC_FAIL" -ne 0
ck "pg_restore --list was run" grep -Eqs -- '(^| )(--list|-l)( |$)' "$b12/log/pg_restore.log"
ck "the message names _sqlx_migrations" grep -qs '_sqlx_migrations' "$LOGS/b12.out"
ckeq "no set was made" "" "$(sets_in "$b12/backups")"
ck "no plain file anywhere" no_plaintext "$b12/backups" "$b12/tmp"
report B12 "a dump that pg_restore cannot list, or that lists no _sqlx_migrations, makes no set"

# ---- B12b: a dump list larger than 1 MiB passes under pipefail (#1162) ------
b12b="$ROOT/b12b"
bk_case "$b12b" 20261007T133500Z
mkdir -m 700 "$b12b/backups"
run_backup "$b12b" XC_STUB_LIST=large
printf '%s\n' "$OUT" >"$LOGS/b12b.out"
ck "backup.sh exited 0" test "$RC" -eq 0
ck "the set was made" test -n "$(sets_in "$b12b/backups")"
ck "pg_restore --list was run" grep -Eqs -- '(^| )(--list|-l)( |$)' "$b12b/log/pg_restore.log"
ck "no plain file anywhere" no_plaintext "$b12b/backups" "$b12b/tmp"
report B12b "a dump list larger than 1 MiB containing _sqlx_migrations makes a set"

# ---- B13: pg_dump toward a non-local host requires TLS (BK-ENC, §7c) --------
b13="$ROOT/b13"
bk_case "$b13" 20261007T134000Z 20261007T134001Z 20261007T134002Z
run_backup "$b13" APP_DATABASE__HOST=db.example.invalid
B13_RC_REMOTE=$RC
B13_ENV_REMOTE="$(tail -n 1 "$b13/log/pg_dump.env" 2>/dev/null || echo missing)"
printf '%s\n' "$OUT" >"$LOGS/b13.out"
run_backup "$b13" APP_DATABASE__HOST=localhost
B13_ENV_LOCAL="$(tail -n 1 "$b13/log/pg_dump.env" 2>/dev/null || echo missing)"
run_backup "$b13" APP_DATABASE__HOST=/var/run/postgresql
B13_ENV_SOCKET="$(tail -n 1 "$b13/log/pg_dump.env" 2>/dev/null || echo missing)"
ck "the remote run exited 0" test "$B13_RC_REMOTE" -eq 0
ck "a non-local host gets PGSSLMODE=require or verify-full" \
  grep -Eqx 'PGSSLMODE=(require|verify-full)' <<<"$B13_ENV_REMOTE"
ckeq "localhost is not forced" "PGSSLMODE=" "$B13_ENV_LOCAL"
ckeq "a socket directory is not forced" "PGSSLMODE=" "$B13_ENV_SOCKET"
report B13 "pg_dump toward a non-local database host runs with PGSSLMODE=require"

# ---- B13b: locality rule matches DB-LINK (#359, #371) ------------------------
b13b="$ROOT/b13b"
bk_case "$b13b" \
  20261007T134101Z 20261007T134102Z 20261007T134103Z \
  20261007T134104Z 20261007T134105Z 20261007T134106Z 20261007T134107Z
# Local loopback addresses are not forced
run_backup "$b13b" APP_DATABASE__HOST=127.0.0.1
ckeq "127.0.0.1 is not forced" "PGSSLMODE=" "$(tail -n 1 "$b13b/log/pg_dump.env")"
run_backup "$b13b" APP_DATABASE__HOST=127.0.0.53
ckeq "127.0.0.53 is not forced" "PGSSLMODE=" "$(tail -n 1 "$b13b/log/pg_dump.env")"
run_backup "$b13b" APP_DATABASE__HOST=::1
ckeq "::1 is not forced" "PGSSLMODE=" "$(tail -n 1 "$b13b/log/pg_dump.env")"

# Hosts that only look local are non-local and require TLS
run_backup "$b13b" APP_DATABASE__HOST=127.0.0.1.example.com
ckeq "127.0.0.1.example.com requires TLS" "PGSSLMODE=require" "$(tail -n 1 "$b13b/log/pg_dump.env")"
run_backup "$b13b" APP_DATABASE__HOST=localhost.example.com
ckeq "localhost.example.com requires TLS" "PGSSLMODE=require" "$(tail -n 1 "$b13b/log/pg_dump.env")"
run_backup "$b13b" APP_DATABASE__HOST=0.0.0.0
ckeq "0.0.0.0 requires TLS" "PGSSLMODE=require" "$(tail -n 1 "$b13b/log/pg_dump.env")"
run_backup "$b13b" APP_DATABASE__HOST=relative/socket/dir
ckeq "relative socket dir requires TLS" "PGSSLMODE=require" "$(tail -n 1 "$b13b/log/pg_dump.env")"
report B13b "locality rule treats loopback IPs as local and fake local names as non-local"

# ---- B13c: non-local host sslmode cannot be downgraded (#371) ----------------
b13c="$ROOT/b13c"
bk_case "$b13c" 20261007T134201Z 20261007T134202Z 20261007T134203Z 20261007T134204Z
run_backup "$b13c" APP_DATABASE__HOST=db.example.invalid PGSSLMODE=verify-ca
ckeq "verify-ca is preserved" "PGSSLMODE=verify-ca" "$(tail -n 1 "$b13c/log/pg_dump.env")"
run_backup "$b13c" APP_DATABASE__HOST=db.example.invalid PGSSLMODE=verify-full
ckeq "verify-full is preserved" "PGSSLMODE=verify-full" "$(tail -n 1 "$b13c/log/pg_dump.env")"
run_backup "$b13c" APP_DATABASE__HOST=db.example.invalid PGSSLMODE=prefer
ckeq "prefer is upgraded to require" "PGSSLMODE=require" "$(tail -n 1 "$b13c/log/pg_dump.env")"
run_backup "$b13c" APP_DATABASE__HOST=db.example.invalid PGSSLMODE=disable
ckeq "disable is upgraded to require" "PGSSLMODE=require" "$(tail -n 1 "$b13c/log/pg_dump.env")"
report B13c "a non-local host keeps verify-ca/verify-full and upgrades weaker ssl modes"

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
    chown)
      if [ -n "${XC_TEST_OWNERS:-}" ] && [ "$#" -ge 3 ]; then
        printf '%s\t%s %s\n' "$3" "${2%%:*}" "${2#*:}" >>"$XC_TEST_OWNERS"
      fi
      exit 0
      ;;
    /etc/systemd/system/*) argv+=("${XC_TEST_SYSTEMD:?}/${a#/etc/systemd/system/}") ;;
    *) argv+=("$a") ;;
  esac
done
exec "${argv[@]}"
STUB

write_ustub stat <<'STUB'
#!/usr/bin/env bash
real=/usr/bin/stat
[ -x "$real" ] || real=/bin/stat
if [ "$#" = 3 ] && [ "$1" = -c ] && [ "$2" = '%U %G %a' ] && [ -n "${XC_TEST_OWNERS:-}" ] && [ -f "$XC_TEST_OWNERS" ]; then
  mode="$("$real" -c '%a' "$3")" || exit
  og="$(awk -F'\t' -v p="$3" '$1 == p { v = $2 } END { print v }' "$XC_TEST_OWNERS")"
  printf '%s %s\n' "${og:-root root}" "$mode"
  exit 0
fi
exec "$real" "$@"
STUB

# systemctl: logs. For xc-v3-backup(.service) it answers from whether
# $XC_TEST_SYSTEMD/xc-v3-backup.service exists: cat, list-unit-files,
# status, is-enabled and show -p LoadState as systemd does for a present or
# a missing unit; start of a missing unit exits 5. A start of the present
# unit exits $XC_STUB_BACKUP_RC (default 0) and, when that is 0 and
# $XC_STUB_BACKUP_SET is set, leaves that set directory as backup.sh would
# after BK-ENC — both pieces in the age stub's format (dump "fresh dump") —
# or, with XC_STUB_BACKUP_FORM: plain (the two pieces as before BK-ENC),
# noheader (.age files that do not start with the age header), nouploads
# (the encrypted dump only).
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
      d="$XC_STUB_BACKUP_SET"
      mkdir -m 700 "$d"
      hdr='age-encryption.org/v1\n-> X25519 stub\n--- stub\n'
      case "${XC_STUB_BACKUP_FORM:-enc}" in
        plain)
          printf 'fresh dump\n' >"$d/db-$s.dump"
          printf 'fresh archive\n' >"$d/uploads-$s.tar.gz"
          ;;
        noheader)
          printf 'fresh dump\n' | base64 >"$d/db-$s.dump.age"
          printf 'fresh archive\n' | base64 >"$d/uploads-$s.tar.gz.age"
          ;;
        nodumpheader)
          printf 'fresh dump\n' | base64 >"$d/db-$s.dump.age"
          { printf "$hdr"; printf 'fresh archive\n' | base64; } >"$d/uploads-$s.tar.gz.age"
          ;;
        nouploads)
          { printf "$hdr"; printf 'fresh dump\n' | base64; } >"$d/db-$s.dump.age"
          ;;
        *)
          { printf "$hdr"; printf 'fresh dump\n' | base64; } >"$d/db-$s.dump.age"
          { printf "$hdr"; printf 'fresh archive\n' | base64; } >"$d/uploads-$s.tar.gz.age"
          ;;
      esac
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

# psql: a fake cluster (#1084). Databases are files in $XC_TEST_PG/db; every
# operation appends one "pgop <what> <db> ..." line to calls.log and pg.log.
#   -c on _sqlx_migrations: count( answers the lines of $XC_TEST_MIGRATIONS
#      in turn, the last one repeating (as before #1084); max( the highest
#      line of $XC_TEST_APPLIED; anything else lists $XC_TEST_APPLIED
#   -c CREATE DATABASE / DROP DATABASE [IF EXISTS]: as createdb / dropdb
#   -c naming pg_database and a quoted name: 1 when that database exists
#   -c anything else: prints nothing
#   no -c: the SQL from -f (- = stdin) or stdin goes into the database:
#      "pgop load <db> dump:<src>" for a stub dump, else "pgop apply <db>
#      <v...>" with every "xc-test-migration: <v>" it holds; SQL holding
#      XC_TEST_REFUSE fails as psql does on a RAISE EXCEPTION (exit 3)
# A database that does not exist fails as psql does (exit 2). Nothing it
# reads is ever written to a log.
write_ustub psql <<'STUB'
#!/usr/bin/env bash
printf 'psql %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
cl="${XC_TEST_PG:?}/db"
op() { printf 'pgop %s\n' "$*" | tee -a "$XC_TEST_LOG/pg.log" >>"$XC_TEST_LOG/calls.log"; }
db="" sql="" file="" take=""
for a in "$@"; do
  if [ -n "$take" ]; then
    case "$take" in d) db="$a" ;; c) sql="$a" ;; f) file="$a" ;; esac
    take=""
    continue
  fi
  case "$a" in
    --dbname=*) db="${a#--dbname=}" ;;
    --command=*) sql="${a#--command=}" ;;
    --file=*) file="${a#--file=}" ;;
    --dbname) take=d ;;
    --command) take=c ;;
    --file) take=f ;;
    --set | --variable | --username | --host | --port | --output | --log-file) take=x ;;
    --*) ;;
    -?*)
      s="${a#-}"
      while [ -n "$s" ]; do
        ch="${s:0:1}"
        s="${s:1}"
        case "$ch" in
          d | c | f | v | U | h | p | P | o | L | T | F | R)
            if [ -n "$s" ]; then
              case "$ch" in d) db="$s" ;; c) sql="$s" ;; f) file="$s" ;; esac
            else
              take="$ch"
            fi
            s=""
            ;;
        esac
      done
      ;;
    *) [ -z "$db" ] && db="$a" ;;
  esac
done
db="${db:-postgres}"
missing() {
  echo "psql: error: connection to server failed: FATAL:  database \"$1\" does not exist" >&2
  exit 2
}
if [ -n "$sql" ]; then
  [ -e "$cl/$db" ] || missing "$db"
  lc="$(printf '%s' "$sql" | tr '[:upper:]' '[:lower:]')"
  case "$lc" in
    *_sqlx_migrations*)
      op "query $db _sqlx_migrations"
      case "$lc" in
        *"count("*)
          f="${XC_TEST_MIGRATIONS:?}"
          head -n 1 "$f"
          if [ "$(wc -l <"$f")" -gt 1 ]; then
            tail -n +2 "$f" >"$f.next"
            mv "$f.next" "$f"
          fi
          ;;
        *"max("*) sort -n "${XC_TEST_APPLIED:?}" | tail -n 1 ;;
        *) sort -n "${XC_TEST_APPLIED:?}" ;;
      esac
      ;;
    *"create database"*)
      name="$(printf '%s' "$lc" | sed -E 's/.*create database +"?([a-z0-9_]+)"?.*/\1/')"
      if [ -e "$cl/$name" ]; then
        echo "ERROR:  database \"$name\" already exists" >&2
        exit 1
      fi
      : >"$cl/$name"
      op "create $name"
      ;;
    *"drop database"*)
      name="$(printf '%s' "$lc" | sed -E 's/.*drop database +(if exists +)?"?([a-z0-9_]+)"?.*/\2/')"
      if [ ! -e "$cl/$name" ]; then
        case "$lc" in *"if exists"*) exit 0 ;; esac
        echo "ERROR:  database \"$name\" does not exist" >&2
        exit 1
      fi
      rm -f "$cl/$name"
      op "drop $name"
      ;;
    *pg_database*)
      name="$(printf '%s' "$sql" | sed -nE "s/.*'([A-Za-z0-9_]+)'.*/\1/p")"
      [ -n "$name" ] && [ -e "$cl/$name" ] && echo 1
      ;;
  esac
  exit 0
fi
if [ -n "$file" ] && [ "$file" != - ]; then
  content="$(cat "$file")"
else
  content="$(cat)"
fi
[ -e "$cl/$db" ] || missing "$db"
case "$content" in
  *"xc-test stub plain dump of "*)
    src="$(printf '%s\n' "$content" | sed -nE 's/.*xc-test stub plain dump of ([^ ]+).*/\1/p' | head -n 1)"
    op "load $db dump:$src"
    ;;
  *)
    marks="$(printf '%s\n' "$content" | sed -nE 's/.*xc-test-migration: ([0-9]+).*/\1/p' | tr '\n' ' ')"
    op "apply $db ${marks:--}"
    ;;
esac
case "$content" in
  *XC_TEST_REFUSE*)
    echo "psql:${file:-<stdin>}:4: ERROR:  recreate the test database first (U4)" >&2
    echo "CONTEXT:  PL/pgSQL function inline_code_block line 3 at RAISE" >&2
    exit 3
    ;;
esac
exit 0
STUB

# pg_dump: a plain dump of the named database ("-- xc-test stub plain dump
# of <db>") to -f/--file or stdout; "pgop dump <db>". A database that does not
# exist fails.
write_ustub pg_dump <<'STUB'
#!/usr/bin/env bash
printf 'pg_dump %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
cl="${XC_TEST_PG:?}/db"
db="" out="" take=""
for a in "$@"; do
  if [ -n "$take" ]; then
    case "$take" in d) db="$a" ;; f) out="$a" ;; esac
    take=""
    continue
  fi
  case "$a" in
    --dbname=*) db="${a#--dbname=}" ;;
    --file=*) out="${a#--file=}" ;;
    -d | --dbname) take=d ;;
    -f | --file) take=f ;;
    -F | -h | -p | -U | -n | -N | -t | -T | -E | -j | -Z | --format | --host | --port | --username | --schema | --exclude-schema | --table | --exclude-table | --encoding | --jobs | --compress) take=x ;;
    -d?*) db="${a#-d}" ;;
    -f?*) out="${a#-f}" ;;
    -*) ;;
    *) [ -z "$db" ] && db="$a" ;;
  esac
done
if [ -z "$db" ] || [ ! -e "$cl/$db" ]; then
  echo "pg_dump: error: connection to server failed: FATAL:  database \"$db\" does not exist" >&2
  exit 1
fi
printf 'pgop dump %s\n' "$db" | tee -a "$XC_TEST_LOG/pg.log" >>"$XC_TEST_LOG/calls.log"
if [ -n "$out" ] && [ "$out" != - ]; then
  printf -- '-- xc-test stub plain dump of %s\n' "$db" >"$out"
else
  printf -- '-- xc-test stub plain dump of %s\n' "$db"
fi
exit 0
STUB

# createdb / dropdb: on the same fake cluster; "pgop create|drop <db>".
write_ustub createdb <<'STUB'
#!/usr/bin/env bash
printf 'createdb %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
cl="${XC_TEST_PG:?}/db"
name="" take=""
for a in "$@"; do
  if [ -n "$take" ]; then
    take=""
    continue
  fi
  case "$a" in
    -O | -T | -E | -l | -h | -p | -U | -D | -S | --owner | --template | --encoding | --locale | --host | --port | --username | --tablespace | --strategy | --lc-collate | --lc-ctype) take=x ;;
    -*) ;;
    *) [ -z "$name" ] && name="$a" ;;
  esac
done
if [ -z "$name" ] || [ -e "$cl/$name" ]; then
  echo "createdb: error: database creation failed: ERROR:  database \"$name\" already exists" >&2
  exit 1
fi
: >"$cl/$name"
printf 'pgop create %s\n' "$name" | tee -a "$XC_TEST_LOG/pg.log" >>"$XC_TEST_LOG/calls.log"
STUB

write_ustub dropdb <<'STUB'
#!/usr/bin/env bash
printf 'dropdb %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
cl="${XC_TEST_PG:?}/db"
name="" take="" ife=0
for a in "$@"; do
  if [ -n "$take" ]; then
    take=""
    continue
  fi
  case "$a" in
    --if-exists) ife=1 ;;
    -h | -p | -U | --host | --port | --username | --maintenance-db) take=x ;;
    -*) ;;
    *) [ -z "$name" ] && name="$a" ;;
  esac
done
if [ -z "$name" ] || [ ! -e "$cl/$name" ]; then
  [ "$ife" = 1 ] && exit 0
  echo "dropdb: error: database removal failed: ERROR:  database \"$name\" does not exist" >&2
  exit 1
fi
rm -f "$cl/$name"
printf 'pgop drop %s\n' "$name" | tee -a "$XC_TEST_LOG/pg.log" >>"$XC_TEST_LOG/calls.log"
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
  */health/ready) body='{"version":"1.0.0","build":"0123456789ab","builtAt":"2026-10-07T00:00:00Z","channel":"staging","database":true,"ui":"built"}' ;;
esac
[ -n "$out" ] && printf '%s' "$body" >"$out"
printf '200'
STUB

# ---- helpers ----------------------------------------------------------------
U_OLD_SET=20261006T000000Z
U_NEW_SET=20261007T140000Z
# base64 of 32 zero bytes: the shape of a key line, not a key
U_KEY_PLACEHOLDER=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
U_MIG_APPLIED=20261001000000
U_MIG_PENDING=20261002000000

# u_case <dir> — an installed xc-v3 under <dir>: the binary and
# configuration, the env file (naming the uploads and the backup directory),
# xc-v3.service and xc-v3-backup.service in <dir>/systemd, a backup directory
# holding one older set, a stage with another binary, and the migration
# counts 9 then 10.
u_case() {
  local c="$1"
  mkdir -p "$c/log" "$c/opt/xc-v3/configuration" "$c/etc/xc-v3" "$c/var/lib/xc-v3/uploads" "$c/systemd"
  mkdir -m 700 "$c/backups"
  : >"$c/owners"
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
  # #1084: the PII key (a placeholder, never a real key), a stage carrying
  # one applied and one pending migration, and the fake cluster
  printf 'k1:%s\n' "$U_KEY_PLACEHOLDER" >"$c/etc/xc-v3/pii.key"
  chmod 640 "$c/etc/xc-v3/pii.key"
  mkdir -p "$c/stage/migrations" "$c/pg/db"
  stage_migration "$c" "$U_MIG_APPLIED" first
  stage_migration "$c" "$U_MIG_PENDING" second
  relist_stage "$c"
  printf '%s\n' "$U_MIG_APPLIED" >"$c/applied"
  : >"$c/pg/db/postgres"
  : >"$c/pg/db/xc_v3"
}

# stage_migration <case dir> <version> <name> [refuse] — a staged migration
# carrying its version as a marker; with "refuse" it fails when applied.
stage_migration() {
  {
    printf -- '-- xc-test-migration: %s\n' "$2"
    [ "${4:-}" = refuse ] && printf -- '-- XC_TEST_REFUSE\n'
    printf 'SELECT 1;\n'
  } >"$1/stage/migrations/$2_$3.sql"
}

# relist_stage <case dir> — SHA256SUMS lists every staged file again
relist_stage() {
  (
    cd "$1/stage"
    shopt -s nullglob
    sha256sum xchange configuration/*.yaml migrations/*.sql >SHA256SUMS
  )
}

# run_update <case dir> [NAME=value...] — the repository's update.sh against
# the case; the backup unit's start leaves the set $U_NEW_SET. OUT/RC.
run_update() {
  local c="$1"
  shift
  set +e
  OUT="$(env PATH="$U_STUBS:$BIN_STUBS:$PATH" XC_TEST_LOG="$c/log" XC_TEST_SYSTEMD="$c/systemd" \
    XC_TEST_OWNERS="$c/owners" \
    XC_TEST_MIGRATIONS="$c/migrations" XC_STUB_BACKUP_SET="$c/backups/$U_NEW_SET" \
    XC_TEST_PG="$c/pg" XC_TEST_APPLIED="$c/applied" \
    STAGE="$c/stage" INSTALL_DIR="$c/opt/xc-v3" ETC_DIR="$c/etc/xc-v3" \
    UPLOADS_DIR="$c/var/lib/xc-v3/uploads" VERIFICATION_STORE_DIR="$c/var/lib/xc-v3/verification-documents" SMOKE_WAIT_SECS=0 \
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
# BK-ENC: the new set's encrypted dump, read through sudo (the set is root 0700)
U_READ_DUMP="^sudo (.* )?[^ ]*/$U_NEW_SET/db-$U_NEW_SET\\.dump\\.age( |\$)"
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
U1_DUMP="$u1/backups/$U_NEW_SET/db-$U_NEW_SET.dump.age"
ck "update.sh exited 0" test "$RC" -eq 0
ck "the backup unit was started before xc-v3 was stopped" u_before "$u1" "$U_BACKUP_START" "$U_STOP"
ck "the new set's encrypted dump was read before the stop (BK-ENC)" u_before "$u1" "$U_READ_DUMP" "$U_STOP"
ck "the check came after the backup" u_before "$u1" "$U_BACKUP_START" "$U_READ_DUMP"
ck "pg_restore never ran: the server cannot read a set (BK-ENC)" test ! -s "$u1/log/pg_restore.log"
ck "age never decrypted on the server (BK-ENC)" absent '(^| )(-d|--decrypt)( |$)' "$u1/log/age.log"
ck "xc-v3 was stopped and started" grep -Eqs -- '^systemctl (.* )?start (.* )?xc-v3(\.service)?( |$)' "$u1/log/calls.log"
ckeq "the new binary is installed" "$(cat "$u1/stage/xchange")" "$(cat "$u1/opt/xc-v3/xchange" 2>/dev/null || echo missing)"
ck "a NOTE says migrations were applied" grep -qs '^NOTE:' "$LOGS/u1.out"
ck "the NOTE names the new set's dump" note_names "$LOGS/u1.out" "$U1_DUMP"
ck "no 'backup: not written yet' any more" absent 'not written yet' "$LOGS/u1.out"
ck "the rollback names restore.sh" grep -qs 'restore\.sh' "$LOGS/u1.out"
ck "the rollback names the new set" grep -qsF -- "$u1/backups/$U_NEW_SET" "$LOGS/u1.out"
ck "the rollback names XC_BACKUP_IDENTITY (BK-ENC)" grep -qs 'XC_BACKUP_IDENTITY' "$LOGS/u1.out"
ck "the older set is not the one named" absent "$U_OLD_SET" "$LOGS/u1.out"
report U1 "a migrating update backs up and checks the set is encrypted before the stop, and names it"

# ---- U2: no migration applied — still backed up first, no NOTE ---------------
u2="$ROOT/u2"
u_case "$u2"
printf '9\n' >"$u2/migrations"
run_update "$u2"
printf '%s\n' "$OUT" >"$LOGS/u2.out"
ck "update.sh exited 0" test "$RC" -eq 0
ck "the backup unit was started before xc-v3 was stopped" u_before "$u2" "$U_BACKUP_START" "$U_STOP"
ck "the encrypted dump was checked before the stop (BK-ENC)" u_before "$u2" "$U_READ_DUMP" "$U_STOP"
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

# ---- U5: the new dump does not start with the age header (BK-ENC) ------------
u5="$ROOT/u5"
u_case "$u5"
run_update "$u5" XC_STUB_BACKUP_FORM=noheader
printf '%s\n' "$OUT" >"$LOGS/u5.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "xc-v3 was never stopped" u_no_stop "$u5"
ck "nothing was replaced" u_untouched "$u5"
ck "the message names the dump" grep -qsF -- "$u5/backups/$U_NEW_SET/db-$U_NEW_SET.dump.age" "$LOGS/u5.out"
report U5 "a set whose dump is not age-encrypted means no update"

# ---- U5b: the new set is plain, as before BK-ENC -----------------------------
u5b="$ROOT/u5b"
u_case "$u5b"
run_update "$u5b" XC_STUB_BACKUP_FORM=plain
printf '%s\n' "$OUT" >"$LOGS/u5b.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "xc-v3 was never stopped" u_no_stop "$u5b"
ck "nothing was replaced" u_untouched "$u5b"
ck "the message names the new set" grep -qsF -- "$u5b/backups/$U_NEW_SET" "$LOGS/u5b.out"
ck "pg_restore never ran" test ! -s "$u5b/log/pg_restore.log"
report U5b "a plain (unencrypted) set means no update"

# ---- U5d: the new set has no encrypted uploads archive (BK-ENC) --------------
u5d="$ROOT/u5d"
u_case "$u5d"
run_update "$u5d" XC_STUB_BACKUP_FORM=nouploads
printf '%s\n' "$OUT" >"$LOGS/u5d.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "xc-v3 was never stopped" u_no_stop "$u5d"
ck "nothing was replaced" u_untouched "$u5d"
ck "the message names the uploads archive" grep -qs -- "uploads-$U_NEW_SET.tar.gz.age" "$LOGS/u5d.out"
report U5d "a set without its encrypted uploads archive means no update"

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

# ---- U5e: dump lacks age header while uploads archive is encrypted (mutant E10)
u5e="$ROOT/u5e"
u_case "$u5e"
run_update "$u5e" XC_STUB_BACKUP_FORM=nodumpheader
printf '%s\n' "$OUT" >"$LOGS/u5e.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "xc-v3 was never stopped" u_no_stop "$u5e"
ck "nothing was replaced" u_untouched "$u5e"
ck "the message says the dump is not age-encrypted" grep -qs 'backup dump is not age-encrypted' "$LOGS/u5e.out"
ck "the message names the dump" grep -qsF -- "$u5e/backups/$U_NEW_SET/db-$U_NEW_SET.dump.age" "$LOGS/u5e.out"
report U5e "a set whose dump lacks the age header is refused even if the archive is encrypted"

# ---- U6: update.sh on a host without verification store dir or env key (#4788)
u6="$ROOT/u6"
u_case "$u6"
U6_VDIR="$u6/var/lib/xc-v3/verification-documents"
ck "precondition: verification directory absent before update" test ! -e "$U6_VDIR"
ck "precondition: APP_VERIFICATION__STORE_DIR absent from env before update" absent '^APP_VERIFICATION__STORE_DIR=' "$u6/etc/xc-v3/env"
run_update "$u6"
printf '%s\n' "$OUT" >"$LOGS/u6.out"
ck "update.sh exited 0" test "$RC" -eq 0
ck "verification store directory exists after update" test -d "$U6_VDIR"
ckeq "verification store directory is mode 700" 700 "$(stat -c '%a' "$U6_VDIR" 2>/dev/null || echo missing)"
ckeq "verification store directory is owned by service user" "xc-v3 xc-v3" "$(awk -F'\t' -v p="$U6_VDIR" '$1 == p { v = $2 } END { print v }' "$u6/owners")"
ck "env file sets APP_VERIFICATION__STORE_DIR" grep -qsx "APP_VERIFICATION__STORE_DIR=$U6_VDIR" "$u6/etc/xc-v3/env"
ck "installed unit ReadWritePaths holds both uploads and verification store dirs" \
  grep -qsx "ReadWritePaths=$u6/var/lib/xc-v3/uploads $U6_VDIR" "$u6/systemd/xc-v3.service"
report U6 "update.sh creates missing verification directory (700, service user), adds APP_VERIFICATION__STORE_DIR to env, and renders both in ReadWritePaths"

# ---- stubs of the install cases (#1084; first on PATH, before the update stubs)
# A full install.sh run needs the owners it set to be read back, so that a
# second run judges its own directories as the first run left them:
#   sudo  as the update stub (no privilege, -u dropped, /etc/systemd/system/
#         mapped into $XC_TEST_SYSTEMD), and every owner it is asked to set —
#         chown <o>:<g>, chgrp <g>, install -o <o> -g <g> — recorded per path
#         in $XC_TEST_OWNERS instead of applied
#   stat  -c '%U %G %a' answers the recorded owner and group (root root when
#         none was recorded) and the real mode; anything else is the real stat
#   node  the pages extractor: writes one page into the output directory
I_STUBS="$ROOT/istubs"
mkdir -m 700 "$I_STUBS"
write_istub() {
  cat >"$I_STUBS/$1"
  chmod 755 "$I_STUBS/$1"
}

write_istub sudo <<'STUB'
#!/usr/bin/env bash
printf 'sudo %s\n' "$*" >>"${XC_TEST_LOG:?}/calls.log"
rec="${XC_TEST_OWNERS:?}"
map() {
  case "$1" in
    /etc/systemd/system/*) printf '%s' "${XC_TEST_SYSTEMD:?}/${1#/etc/systemd/system/}" ;;
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
esac
exec "$cmd" "$@"
STUB

write_istub stat <<'STUB'
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

write_istub node <<'STUB'
#!/usr/bin/env bash
mkdir -p "$3"
printf '<!doctype html>\n' >"$3/index.html"
STUB

K_SERVICE=xc1084n
K_USER=xc1084u

# k_case <dir> — a clean host: nothing installed, a stage with the extractor,
# a v2 bundle directory, a cluster holding only postgres
k_case() {
  local c="$1"
  mkdir -p "$c/log" "$c/systemd" "$c/v2bundle" "$c/pg/db"
  : >"$c/owners"
  : >"$c/applied"
  : >"$c/pg/db/postgres"
  printf '0\n' >"$c/migrations"
  make_stage "$c/stage" extract-v2-pages.mjs
}

# run_install <case dir> [NAME=value...] — the repository's install.sh
# against the case. OUT/RC.
run_install() {
  local c="$1"
  shift
  set +e
  OUT="$(env PATH="$I_STUBS:$U_STUBS:$BIN_STUBS:$PATH" XC_TEST_LOG="$c/log" \
    XC_TEST_SYSTEMD="$c/systemd" XC_TEST_OWNERS="$c/owners" XC_TEST_PG="$c/pg" \
    XC_TEST_MIGRATIONS="$c/migrations" XC_TEST_APPLIED="$c/applied" \
    SERVICE_NAME="$K_SERVICE" SERVICE_USER="$K_USER" STAGE="$c/stage" \
    INSTALL_DIR="$c/opt/$K_SERVICE" ETC_DIR="$c/etc/$K_SERVICE" \
    UPLOADS_DIR="$c/var/lib/$K_SERVICE/uploads" \
    VERIFICATION_STORE_DIR="$c/var/lib/$K_SERVICE/verification-documents" \
    V2_BUNDLE_DIR="$c/v2bundle" \
    SMOKE_WAIT_SECS=0 "$@" bash "$HERE/install.sh" </dev/null 2>&1)"
  RC=$?
  set -e
}

# owner_of <case dir> <path> — the owner and group last recorded for <path>
owner_of() { awk -F'\t' -v p="$2" '$1 == p { v = $2 } END { print v }' "$1/owners"; }

# key_lines <file> — the lines that are neither blank nor a # comment
key_lines() { grep -Ev '^[[:space:]]*(#|$)' "$1" 2>/dev/null || true; }

# is_key_line <line> — k1: and the base64 of exactly 32 bytes
is_key_line() {
  printf '%s\n' "$1" | grep -Eqx 'k1:[A-Za-z0-9+/]{43}=' &&
    [ "$(printf '%s' "${1#k1:}" | base64 -d 2>/dev/null | wc -c | tr -d ' ')" = 32 ]
}

# not_shown <secret> <file...> — the secret is in none of the files
not_shown() {
  local s="$1"
  shift
  [ -n "$s" ] && ! grep -rqsF -- "$s" "$@"
}

# ---- K1: a first install writes the PII key ---------------------------------
k1="$ROOT/k1"
k_case "$k1"
run_install "$k1"
printf '%s\n' "$OUT" >"$LOGS/k1.out"
K1_KEY="$k1/etc/$K_SERVICE/pii.key"
K1_LINE="$(key_lines "$K1_KEY")"
ck "install.sh exited 0" test "$RC" -eq 0
ck "the key file exists" test -f "$K1_KEY"
ckeq "one key line" 1 "$(printf '%s' "$K1_LINE" | grep -c . || true)"
ck "the line is k1: and base64 of 32 bytes" is_key_line "$K1_LINE"
ckeq "mode 640" 640 "$(stat -c '%a' "$K1_KEY" 2>/dev/null || echo missing)"
ckeq "owned root:<service group>" "root $K_USER" "$(owner_of "$k1" "$K1_KEY")"
ck "the key is not on stdout or stderr" not_shown "${K1_LINE#k1:}" "$LOGS/k1.out"
ck "the key is on no command line or log the stubs saw" not_shown "${K1_LINE#k1:}" "$k1/log"
K1_VDIR="$k1/var/lib/$K_SERVICE/verification-documents"
ck "the verification store directory exists" test -d "$K1_VDIR"
ckeq "verification store directory is mode 700" 700 "$(stat -c '%a' "$K1_VDIR" 2>/dev/null || echo missing)"
ckeq "verification store directory is owned by service user" "$K_USER $K_USER" "$(owner_of "$k1" "$K1_VDIR")"
ck "the env file sets APP_VERIFICATION__STORE_DIR" grep -qsx "APP_VERIFICATION__STORE_DIR=$K1_VDIR" "$k1/etc/$K_SERVICE/env"
ck "the installed unit ReadWritePaths holds both uploads and verification store dirs" \
  grep -qsx "ReadWritePaths=$k1/var/lib/$K_SERVICE/uploads $K1_VDIR" "$k1/systemd/$K_SERVICE.service"
report K1 "a first install writes pii.key: one k1: line of 32 random bytes, root:<group> 640, never shown"

# ---- K2: an existing key is never overwritten -------------------------------
k2="$ROOT/k2"
k_case "$k2"
run_install "$k2"
printf '%s\n' "$OUT" >"$LOGS/k2-first.out"
K2_KEY="$k2/etc/$K_SERVICE/pii.key"
# the owner's key, rotated once (placeholders: 32 bytes of 0x01 and of 0x02)
printf '%s\n' "# rotated" "k1:AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=" \
  "k2:AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=" >"$K2_KEY"
chmod 640 "$K2_KEY"
printf '%s\troot %s\n' "$K2_KEY" "$K_USER" >>"$k2/owners"
cp -p "$K2_KEY" "$k2/key.before"
run_install "$k2"
printf '%s\n' "$OUT" >"$LOGS/k2.out"
ck "the second install.sh exited 0" test "$RC" -eq 0
ck "the key file is byte-identical" cmp -s "$k2/key.before" "$K2_KEY"
ckeq "its mode is unchanged" 640 "$(stat -c '%a' "$K2_KEY" 2>/dev/null || echo missing)"
ck "the output names the key file it kept" grep -qsF -- "$K2_KEY" "$LOGS/k2.out"
# If an older host env file lacked APP_VERIFICATION__STORE_DIR, re-running install.sh adds it
sed -i '/^APP_VERIFICATION__STORE_DIR=/d' "$k2/etc/$K_SERVICE/env"
# VERIFICATION_STORE_DIR under UPLOADS_DIR is refused by check_names
run_install "$k2" VERIFICATION_STORE_DIR="$k2/var/lib/$K_SERVICE/uploads/verification"
ck "VERIFICATION_STORE_DIR under UPLOADS_DIR is refused" test "$RC" -ne 0
run_install "$k2"
ck "re-run on host missing APP_VERIFICATION__STORE_DIR exited 0" test "$RC" -eq 0
ck "re-run added APP_VERIFICATION__STORE_DIR to existing env file" \
  grep -qsx "APP_VERIFICATION__STORE_DIR=$k2/var/lib/$K_SERVICE/verification-documents" "$k2/etc/$K_SERVICE/env"
report K2 "install.sh run again keeps an existing pii.key byte for byte and says so"

# ---- K3: update.sh without the key refuses before anything ------------------
k3="$ROOT/k3"
u_case "$k3"
rm -f "$k3/etc/xc-v3/pii.key"
run_update "$k3"
printf '%s\n' "$OUT" >"$LOGS/k3.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "no systemctl call at all: no backup, no stop, no restart" test ! -s "$k3/log/systemctl.log"
ck "no pg_dump" absent '^pg_dump' "$k3/log/calls.log"
ck "no database created" absent '^pgop create' "$k3/log/calls.log"
ck "nothing was replaced" u_untouched "$k3"
ck "the message names the key file" grep -qsF -- "$k3/etc/xc-v3/pii.key" "$LOGS/k3.out"
ck "the message says how to create it" grep -Eqs 'install\.sh|openssl' "$LOGS/k3.out"
report K3 "update.sh without pii.key refuses before the backup and before any swap"

# ---- R1..R3: the migration rehearsal (#1084) --------------------------------
U_LIVE=xc_v3

# scratch_of <case dir> — the first database the run created, other than live
scratch_of() {
  sed -nE 's/^pgop create ([^ ]+)$/\1/p' "$1/log/pg.log" 2>/dev/null | grep -vx "$U_LIVE" | head -n 1 || true
}

# dbs_left <case dir> — the databases the cluster holds, sorted, one line
dbs_left() { find "$1/pg/db" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort | tr '\n' ' '; }

# live_untouched <case dir> — nothing was loaded into, applied to, created
# as or dropped from the live database
live_untouched() { absent "^pgop (apply|load|create|drop) $U_LIVE( |\$)" "$1/log/pg.log"; }

# applied_to <case dir> <db> <version> — ERE of that migration going into <db>
applied_to() { printf '^pgop apply %s (.* )?%s( |$)' "$2" "$3"; }

# ---- R1: a refusing migration stops the update before the backup ------------
r1="$ROOT/r1"
u_case "$r1"
R1_REFUSE=20261019100000
R1_LATER=20261020000000
stage_migration "$r1" "$R1_REFUSE" customers_org refuse
stage_migration "$r1" "$R1_LATER" after_it
relist_stage "$r1"
run_update "$r1"
printf '%s\n' "$OUT" >"$LOGS/r1.out"
R1_S="$(scratch_of "$r1")"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "the backup unit was never started" absent "$U_BACKUP_START" "$r1/log/calls.log"
ck "xc-v3 was never stopped" u_no_stop "$r1"
ck "nothing was replaced" u_untouched "$r1"
ck "a scratch database was created" test -n "$R1_S"
ck "the scratch was loaded from a dump of the live database" grep -Eqs "^pgop load ${R1_S:-none} dump:$U_LIVE\$" "$r1/log/pg.log"
ck "the refusing migration was tried on the scratch" grep -Eqs "$(applied_to "$r1" "${R1_S:-none}" "$R1_REFUSE")" "$r1/log/pg.log"
ck "the pending migrations ran in version order" u_before "$r1" "$(applied_to "$r1" "${R1_S:-none}" "$U_MIG_PENDING")" "$(applied_to "$r1" "${R1_S:-none}" "$R1_REFUSE")"
ck "nothing after the refusing one was tried" absent "^pgop apply .* $R1_LATER( |\$)" "$r1/log/pg.log"
ck "the output names the refusing file" grep -qsF -- "${R1_REFUSE}_customers_org.sql" "$LOGS/r1.out"
ck "the output carries the error" grep -qsF -- 'recreate the test database first (U4)' "$LOGS/r1.out"
ck "the scratch was dropped" grep -Eqs "^pgop drop ${R1_S:-none}\$" "$r1/log/pg.log"
ckeq "the cluster holds only what it held" "postgres $U_LIVE " "$(dbs_left "$r1")"
ck "the live database was never written" live_untouched "$r1"
report R1 "a migration that refuses on a copy of the live database stops update.sh before the backup"

# ---- R2: a clean set is rehearsed, then the update goes on -------------------
r2="$ROOT/r2"
u_case "$r2"
run_update "$r2"
printf '%s\n' "$OUT" >"$LOGS/r2.out"
R2_S="$(scratch_of "$r2")"
ck "update.sh exited 0" test "$RC" -eq 0
ck "a scratch database was created" test -n "$R2_S"
ck "the scratch was loaded from a dump of the live database before the migration" \
  u_before "$r2" "^pgop load ${R2_S:-none} dump:$U_LIVE\$" "$(applied_to "$r2" "${R2_S:-none}" "$U_MIG_PENDING")"
ck "the pending migration was rehearsed before the backup" \
  u_before "$r2" "$(applied_to "$r2" "${R2_S:-none}" "$U_MIG_PENDING")" "$U_BACKUP_START"
ck "the applied migration was not run again" absent "^pgop apply .* $U_MIG_APPLIED( |\$)" "$r2/log/pg.log"
ck "the scratch was dropped before the backup" u_before "$r2" "^pgop drop ${R2_S:-none}\$" "$U_BACKUP_START"
ckeq "the cluster holds only what it held" "postgres $U_LIVE " "$(dbs_left "$r2")"
ck "the live database was never written" live_untouched "$r2"
ckeq "the new binary is installed" "$(cat "$r2/stage/xchange")" "$(cat "$r2/opt/xc-v3/xchange" 2>/dev/null || echo missing)"
report R2 "a clean set is rehearsed on a scratch copy before the backup, the scratch dropped, the update done"

# ---- R3: a stage without migrations/ is refused --------------------------------
r3="$ROOT/r3"
u_case "$r3"
rm -rf "$r3/stage/migrations"
relist_stage "$r3"
run_update "$r3"
printf '%s\n' "$OUT" >"$LOGS/r3.out"
ck "update.sh exited non-zero" test "$RC" -ne 0
ck "the backup unit was never started" absent "$U_BACKUP_START" "$r3/log/calls.log"
ck "xc-v3 was never stopped" u_no_stop "$r3"
ck "nothing was replaced" u_untouched "$r3"
ck "the message names the missing migrations/" grep -qs 'migrations/' "$LOGS/r3.out"
report R3 "a stage that carries no migrations/ cannot be rehearsed and is refused"

# ---- SM1/SM2: smoke.sh fails a server whose screens were not built (V0-A) ---
# docs/design/xchange-v3-web.md §3.2: a server built without `web/dist` answers
# every v3 screen 503 and /health/ready reports ui "missing"; smoke.sh refuses it.
SM_STUBS="$ROOT/sm-stubs"
mkdir -p "$SM_STUBS"
cat >"$SM_STUBS/curl" <<'STUB'
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
  */health/ready) body='{"status":"ok","database":true,"version":"1.0.0","build":"0123456789ab","builtAt":"2026-10-07T00:00:00Z","channel":"staging"'"${XC_STUB_UI_FIELD}"'}' ;;
esac
[ -n "$out" ] && printf '%s' "$body" >"$out"
printf '200'
STUB
chmod 755 "$SM_STUBS/curl"

# sm_run <ui field text> — smoke.sh against the stub; OUT/RC.
sm_run() {
  set +e
  OUT="$(PATH="$SM_STUBS:$PATH" XC_STUB_UI_FIELD="$1" bash "$HERE/smoke.sh" http://127.0.0.1:8094 2>&1)"
  RC=$?
  set -e
}

for sm in missing absent; do
  if [ "$sm" = missing ]; then sm_run ',"ui":"missing"'; else sm_run ''; fi
  printf '%s\n' "$OUT" >"$LOGS/sm1-$sm.out"
  ck "smoke.sh exited 1" test "$RC" -eq 1
  ck "a FAIL line names ui" grep -Eq '^FAIL .*ui' "$LOGS/sm1-$sm.out"
  report "SM1-$sm" "smoke.sh fails a server whose /health/ready ui is $sm"
done

sm_run ',"ui":"built"'
printf '%s\n' "$OUT" >"$LOGS/sm2.out"
ck "smoke.sh exited 0" test "$RC" -eq 0
ck "an ok line names ui" grep -Eq '^ok +ui' "$LOGS/sm2.out"
report SM2 "smoke.sh passes a server whose /health/ready ui is built"

echo "# $PASS passed, $FAILURES failed"
[ "$FAILURES" -eq 0 ]

