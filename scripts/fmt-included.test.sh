#!/usr/bin/env bash
# #1474 up-front test of scripts/fmt-included.sh, by the ticket writer claude/dell-claude-2.
# 🔴 Do not edit, delete or weaken.
#
# Each case builds a throwaway git repo holding a copy of the script under test
# and copies of this repo's real api/build.rs and api/crates/*/build.rs, so the
# script sees today's include! folder list. It then runs
# `bash scripts/fmt-included.sh base` from that repo's root.
#
# Cases (one per row of #1474's behaviour table):
#   F1  an unformatted included file changed since base -> exit 1, output names it
#   F2  an unformatted included file NOT changed since base -> exit 0
#   F3  a formatted included file changed since base -> exit 0
#   F4  build.rs reads a folder the script does not know -> exit 1, output names it
#   F5  the base ref does not exist -> non-zero exit, message on stderr
#   F6  an unformatted included file deleted since base -> exit 0
#   F7  an unformatted changed file outside the include! folders -> exit 0
#       (that file is `cargo fmt --all --check`'s job)
#
# Needs git and rustfmt (edition 2024). Temp dirs go under
# ${XC_TEST_TMP:-${RUNNER_TEMP:-$PWD}}, because /tmp is noexec on some machines (#3830).
set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SUT="$ROOT/scripts/fmt-included.sh"
[ -f "$SUT" ] || { echo "Error: $SUT not found" >&2; exit 1; }
command -v rustfmt >/dev/null || { echo "Error: rustfmt not on PATH" >&2; exit 1; }

TMP_BASE=${XC_TEST_TMP:-${RUNNER_TEMP:-$PWD}}
TMP_ROOT=$(mktemp -d "$TMP_BASE/fmt-included.XXXXXX")
trap 'rm -rf "$TMP_ROOT"' EXIT

export GIT_CONFIG_GLOBAL="$TMP_ROOT/gitconfig"
export GIT_CONFIG_NOSYSTEM=1
printf '[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n' >"$GIT_CONFIG_GLOBAL"

CMD=api/src/presentation/http/v3/commands
UNFMT='pub fn f( )->u8{1}'
FMT=$'pub fn f() -> u8 {\n    1\n}'

pass=0
fail=0
ok() { echo "ok   $1"; pass=$((pass + 1)); }
bad() { echo "FAIL $1: $2"; fail=$((fail + 1)); }

# new_repo: a repo whose base commit (tag `base`) holds the real build.rs files,
# a formatted and an unformatted included file, and an unformatted file outside the folders.
new_repo() {
  T=$(mktemp -d "$TMP_ROOT/case.XXXXXX")
  mkdir -p "$T/scripts" "$T/$CMD" "$T/api/src/other"
  cp "$SUT" "$T/scripts/fmt-included.sh"
  cp "$ROOT/api/build.rs" "$T/api/build.rs"
  for b in "$ROOT"/api/crates/*/build.rs; do
    c=${b#"$ROOT/"}
    mkdir -p "$T/$(dirname "$c")"
    cp "$b" "$T/$c"
  done
  printf '%s\n' "$FMT" >"$T/$CMD/formatted.rs"
  printf '%s\n' "$UNFMT" >"$T/$CMD/legacy.rs"
  printf '%s\n' "$UNFMT" >"$T/api/src/other/plain.rs"
  git -C "$T" init -q
  git -C "$T" add -A
  git -C "$T" commit -qm base
  git -C "$T" tag base
}

commit() { git -C "$T" add -A && git -C "$T" commit -qm "$1"; }

# run <ref>: sets RC, OUT (stdout+stderr) and ERR (stderr only)
run() {
  local errf="$T/.err" outf="$T/.out"
  (cd "$T" && bash scripts/fmt-included.sh "$1") >"$outf" 2>"$errf"
  RC=$?
  ERR=$(cat "$errf")
  OUT="$(cat "$outf")$ERR"
}

# F1
new_repo
printf '%s\n' "$UNFMT" >"$T/$CMD/new_cmd.rs"
commit "add unformatted included file"
run base
if [ "$RC" -eq 1 ] && grep -q 'new_cmd.rs' <<<"$OUT"; then ok F1; else bad F1 "want exit 1 naming new_cmd.rs, got exit $RC: $OUT"; fi

# F2
new_repo
echo readme >"$T/README.md"
commit "touch nothing included"
run base
if [ "$RC" -eq 0 ]; then ok F2; else bad F2 "unchanged legacy.rs must not fail the gate, got exit $RC: $OUT"; fi

# F3
new_repo
printf '%s\n' "$FMT" '' 'pub fn g() {}' >"$T/$CMD/formatted.rs"
commit "edit formatted included file"
run base
if [ "$RC" -eq 0 ]; then ok F3; else bad F3 "formatted change must pass, got exit $RC: $OUT"; fi

# F4
new_repo
cat >>"$T/api/build.rs" <<'RS'

fn new_registry() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/new_registry");
    for entry in std::fs::read_dir(&dir).expect("read bootstrap/new_registry") {
        let _ = entry;
    }
}
RS
commit "build.rs scans a new folder"
run base
if [ "$RC" -eq 1 ] && grep -q 'new_registry' <<<"$OUT"; then ok F4; else bad F4 "want exit 1 naming new_registry, got exit $RC: $OUT"; fi

# F5
new_repo
echo readme >"$T/README.md"
commit "anything"
run no-such-ref
if [ "$RC" -ne 0 ] && [ -n "$ERR" ]; then ok F5; else bad F5 "want non-zero exit and a stderr message, got exit $RC, stderr '$ERR'"; fi

# F6
new_repo
git -C "$T" rm -q "$CMD/legacy.rs"
commit "delete unformatted included file"
run base
if [ "$RC" -eq 0 ]; then ok F6; else bad F6 "a deleted file must not be checked, got exit $RC: $OUT"; fi

# F7
new_repo
printf '%s\n' "$UNFMT" 'pub fn h( ){}' >"$T/api/src/other/plain.rs"
commit "edit file outside the include folders"
run base
if [ "$RC" -eq 0 ]; then ok F7; else bad F7 "a file outside the folders is cargo fmt's job, got exit $RC: $OUT"; fi

echo "fmt-included: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
