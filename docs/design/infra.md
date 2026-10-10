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
2. **The one exception: the server is down or data is at risk now** (a failing backup, a failed deploy, a crash
   loop). The owner may order the ticket opened first. The row is then added within the same day, citing that order.
3. **One row, one ticket, one outcome.** Splitting a row needs its own rows here first. That is the same rule as feature designs.
4. **Not infra:** anything that changes what a user, a screen or the data model does. That belongs to a Blueprint design.
5. Every infra ticket cites its row code (`I-n`) on its first lines.

## 2. Tickets

| Code | Issue | What is wrong (measured) | Files | State 2026-10-11 |
|---|---|---|---|---|
| I-1 | #422 | The v3 server reaches a machine only by hand: no `deploy/staging` (mirror) or `release/prod` (org) lane | `.github/workflows/`, `deploy/server/` | open (deploy lanes are the owner's call) |
| I-2 | #1146 | Registration files (`xc-cash/src/application/reconcile/mod.rs`, `http/v3/commands/mod.rs`, `xc-auth` permission tables) collide on every parallel ticket | the registration files; build-time generation | open (design note before code) |
| I-3 | #1151 | api CI pool selection counts only queued jobs, so api jobs pile onto Dell | `.github/workflows/api.yml` | open; PR 1152 held: Studio timed out under worker load (PR 1152 issuecomment-6101014075) |
| I-4 | #1162 | `deploy/server/backup.sh:105` pipes into `grep -q` under `pipefail`; a large dump list breaks the pipe and every backup fails | `deploy/server/backup.sh`, `deploy/server/scripts*.test.sh` | open; PR 1164 PASS, in the merge queue; host runs a stopgap (`backup.sh.pre-1162` kept) |
| I-5 | — | `deploy/server/install.sh:77`: `ss -ltnH … \| grep -q .` has the same latent SIGPIPE pattern under `pipefail` (PR 1164 verdict issuecomment-6101180580) | `deploy/server/install.sh` | listed, not opened |
| I-6 | — | main's own api run never finishes while main moves faster than one run: `api-main` `cancel-in-progress` cancelled the runs at `50b352a`, `f0a75ea` and `d200a3b`, so a deploy has no green main run to point at (claude/08f7f46f, #422 issuecomment-6101203798) | `.github/workflows/api.yml` | listed, not opened |

## 3. Counts

Listed 6 · opened 4 (open 4) · not yet opened 2. Update this line with every row change.
