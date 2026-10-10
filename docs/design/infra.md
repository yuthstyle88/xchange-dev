# Infra tickets: the one list

> designed-by: claude/5edf7bb5 (xchange lane controller), on the owner's order 2026-10-11:
> *"ทาง ก: ทำเอกสาร docs/design/infra.md เป็นรายการใบ infra ใบเดียว ใบ infra ต้องอยู่ในรายการนี้ก่อนจึงเปิดได้ คุมจำนวนได้แบบเดียวกับฟีเจอร์"*
> Measured on `origin/main` `c326b31b`.

Feature work is bounded by the Blueprint (`docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`,
`docs/design/xchange-system-blueprint-v3.html`) and each design's ticket list (owner's order 2026-10-11, #564).
Infra work has no Blueprint feature behind it, so this file is its ticket list. It covers deploy and host
scripts, backups, CI workflows and runners, and build or registration architecture.

## 1. Rules

1. **No infra ticket without a row here.** A new row comes in by PR (review → merge) *before* the ticket is opened.
   The row must carry measured evidence: `file:line`, a run id, or a host log line. "Might break" is not a row.
2. **One row, one ticket, one outcome.** Splitting a row needs its own rows here first. That is the same rule as feature designs.
3. **Not infra:** anything that changes what a user, a screen or the data model does. That belongs to a Blueprint design.
4. Every infra ticket cites its row code (`I-n`) on its first lines.

No exception: an emergency path was proposed and withdrawn on 2026-10-11 (no production exists; xc-v3 holds test data only).

## 2. Tickets

| Code | Issue | What is wrong (measured) | Files | State 2026-10-11 |
|---|---|---|---|---|
| I-1 | #422 | The v3 server reaches a machine only by hand: no `deploy/staging` (mirror) or `release/prod` (org) lane | `.github/workflows/`, `deploy/server/` | open (deploy lanes are the owner's call) |
| I-2 | #1146 | Registration files (`xc-cash/src/application/reconcile/mod.rs`, `http/v3/commands/mod.rs`, `xc-auth` permission tables) collide on every parallel ticket | the registration files; build-time generation | open (design note before code) |
| I-3 | #1151 | api CI pool selection counts only queued jobs, so api jobs pile onto Dell | `.github/workflows/api.yml` | closed not planned 2026-10-10: measured, the mac pool is slower. One isolated run of PR 1152's api job on studio-org-3: Test step 22m26s and the job hit its 30-min timeout, against 11–16 min for the whole job on Dell (#1151 issuecomment-6101843237). PR 1152 closed. |
| I-4 | #1162 | `deploy/server/backup.sh:105` pipes into `grep -q` under `pipefail`; a large dump list breaks the pipe and every backup fails | `deploy/server/backup.sh`, `deploy/server/scripts*.test.sh` | open; PR 1164 PASS, merges last in the queue (already done, no further work). **Backups are final-phase work** (owner 2026-10-11) |
| I-5 | — | `deploy/server/install.sh:77`: `ss -ltnH … \| grep -q .` has the same latent SIGPIPE pattern under `pipefail` (PR 1164 verdict issuecomment-6101180580) | `deploy/server/install.sh` | listed, not opened. It is the installer's port check, not backup work; it stays unopened because nothing runs past development yet (owner: *"งานพัฒนา ยังไม่ถึงระยะ staging เลย"*) |
| I-6 | #1184 | main's own api run never finishes while main moves faster than one run: `api-main` `cancel-in-progress` cancelled the runs at `50b352a`, `f0a75ea` and `d200a3b`, so a deploy has no green main run to point at (claude/08f7f46f, #422 issuecomment-6101203798) | `.github/workflows/api.yml` | closed by PR 1185 (`7ea71059`, 2026-10-10): `cancel-in-progress` only for pull requests |
| I-7 | — | CI's per-run postgres keeps `max_locks_per_transaction` at the default 64 while test binaries run concurrently, each dropping its schema with `DROP SCHEMA … CASCADE` over hundreds of tables. Under load the shared lock table runs out: run 38081952961 attempt 1 (PR 1168, `8c6e5a25`) failed 16 tests at teardown with `53200 out of shared memory … increase max_locks_per_transaction`; the rerun on the same head was green (claude/89bd61d9). In 6 other failed api jobs, sampled with `gh run view --log` on 6 branches, it did not appear, so it is load-dependent | `.github/workflows/api.yml` (`Start postgres (per run)`, `:234`) | listed, not opened |
| I-8 | — | `person_columns_reference_users_never_employees` (`api/crates/xc-shift/tests/cash_schema_test.rs:503`) hangs at 0% CPU in a concurrent workspace run. Its `information_schema.key_column_usage` / `referential_constraints` join (`:518`) scans every schema's catalog, and test schemas pile up in a shared test database (154 schemas / 72,830 relations measured by claude/709ec749 on #623; gemini/warut-10 killed it after it hung, #1094 issuecomment-6102039962). It passes alone on a fresh database | `api/crates/xc-shift/tests/cash_schema_test.rs` | listed, not opened |
| I-9 | — | A fresh install or an `update.sh` run gives the xchange server no Identity settings. `deploy/server/xc-v3.service:19` loads only `EnvironmentFile=/etc/xc-v3/env`, and `render_unit` (`deploy/server/common.sh:213`) rewrites only that line. `git grep identity-client origin/main -- deploy/server` has 0 hits, so nothing creates or loads `/etc/xc-v3/identity-client.env`, although `deploy/HOST-do-sgp1.md:61`/`:89` say the server reads it. Without it the sign-in routes answer 503 (`api/tests/identity_signin_test.rs`, `without_identity_settings_the_routes_that_need_identity_are_503`). On do-sgp1 a hand-made drop-in `/etc/systemd/system/xc-v3.service.d/identity.conf` fills the gap since 2026-10-10T19:35Z; it exists on that host only (claude/08f7f46f; PR 1202 records it) | `deploy/server/xc-v3.service`, `deploy/server/common.sh`, `deploy/server/install.sh`, `deploy/server/scripts.test.sh` | listed, not opened |

**Backups are final-phase work.** Owner, 2026-10-11: *"backup เราจะทำ ระยะ สุดท้าย"* and *"งานพัฒนา ยังไม่ถึงระยะ staging เลย จะมากลัวอะไรหาย"*. No new backup or restore ticket opens before the last phase. Recorded on #1162: issuecomment-6101322958.

## 3. Counts

Listed 9 · opened 5 (open 3, closed 2) · not yet opened 4. Update this line with every row change.
