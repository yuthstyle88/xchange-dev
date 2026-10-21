# XCHANGE v3 — the road to the production release order

> written-by: claude/c5ca141a · owner's order 2026-10-17 ~05:0xZ, in that session: *"วางแผน ปิดงาน และจ่ายงาน เพื่อให้เอเย่นเป็นแนวทางในการออก production"* (plan the close-out and the dispatch, so that the agents have a guideline for going to production).
> Corrected 2026-10-17 ~05:5xZ after the merger's review (PR 2834 PASS+follow-up): #1413 is not in tq. Measured 2026-10-17T05:0xZ–05:1xZ: `origin/main` `abc46caf`, tq `GET /v1/snapshot` at 05:06:34Z, `gh issue list` / `gh pr list` on `108-Plaza/xchange`, `curl https://xc-v3.spacing.cloud/health/ready`, `ssh Dell-server uptime` at 05:13Z.
> §1–§5 rewritten 2026-10-21 by claude/studio-xc-ready on the BE lane controller claude/553782's order (#564 issuecomment-6323756336), to the state measured 2026-10-21T18:43Z–18:50Z at `main` `1fd766ec`. The owner's goal for that round, verbatim: *"ทำให้พร้อม แล้วคุณสั่งเอง"* (make it ready, then you give the order).

**Who this is for.** Every agent working on `108-Plaza/xchange`: the BE controller, the FE controller, the merge lane, the reviewers, and the Gemini implementers. **Lanes are named by role here, never by session id**: the owner's account moves and sessions change, so the current holder of each lane is whoever last announced it on #564 (the lane's notice board), and a new holder announces itself there before acting. It does not add rules. It says, in one place, what has to be true before the owner can give the order to push `release/prod`, which open ticket belongs to which of those conditions, and in what order the lanes release them.

**Who decides.** The owner decides business rules, secrets, deploys and every item in §5. The lane controllers (📨) decide what is pinned to whom inside the order of §4. This document records those decisions with their source; it makes none of its own.

**Production is a state the owner declares.** The fleet's `STATE.md` says no app is in production and that `release/prod`, a `prod` channel or real-looking data prove nothing. This plan therefore ends at *"the owner can give the release order safely"*, not at *"xchange is in production"*. When the owner declares it, `STATE.md` changes; nothing here does.

---

## 1. Where v3 stands (measured)

Measured 2026-10-21T18:43Z–18:50Z by claude/studio-xc-ready. Re-measure a row before acting on it.

| Item | Value | How measured |
|---|---|---|
| `main` head | `1fd766ec`; its `api`, `web`, `app` and `branch-hygiene` push runs are all `success` (created 18:29Z) | `gh api 'repos/108-Plaza/xchange/actions/runs?branch=main'` |
| Build on the host `xc-v3.spacing.cloud` | `7e9de9ca`, builtAt 2026-10-21T15:17:48Z, `version 1.0.0`, `channel staging`, `database true`, `ui built`: deploy round 10 (#564 issuecomment-6320509987) | `curl -s https://xc-v3.spacing.cloud/health/ready` |
| How far the host is behind `main` | 143 commits, 55 files. **No migration** differs (the host has 69 applied, max `20261219210000`, which is also the newest file in `api/migrations/` on `main`) | `gh api repos/108-Plaza/xchange/compare/7e9de9ca...1fd766ec`; `git diff --name-only 7e9de9ca 1fd766ec`; `select count(*), max(version) from _sqlx_migrations where success` on the host |
| Release path | by hand on the host, on the owner's order each time (G2) | #422 closed `not planned` 2026-10-17 |
| Identity `xc-id.spacing.cloud` | `/health`: `version 1.0.0`, `build unknown`, `built_at unknown`, `channel unknown` (G6) | `curl -s https://xc-id.spacing.cloud/health` |
| Backups | timer active, last run 2026-10-21T17:00:01Z `success`; 7 encrypted sets on the host; no off-host copy (G2) | read-only `ssh` checks, #564 issuecomment-6323812080 |
| Open issues | 35: the umbrella #564, 9 `bug` (including #4324 and #4395), 14 kiosk-web-queue tickets, 11 XC-PRICING tickets; each is listed in §3 | `gh issue list --state open` |
| Open PRs | 10: 7 into `main` (FE bug fixes, an FE batch, a pricing design doc), 3 into `feat/kwq-a` | `gh pr list --state open` |
| Queue (xchange rows) | ready 1 · leased 4 · pr_open 7 · in_review 0 · rework 0 · blocked 0 | `GET /v1/snapshot` 18:47:10Z |
| e2e on the host | the round-10 deploy run: 96 passed, 11 failed, 4 skipped (89.7 %); every failure has a ticket (#4300–#4305) | #564 issuecomment-6320509987, issuecomment-6322752218 |

**Reading.** The code side is close: the drop migrations of the old schema (former G3) are written, merged and running on the host, the tracking tickets of former G4 are closed, and `main` is green. What still stands between the owner and a safe release order is mostly **not code**:
- a backup nobody has ever restored;
- three host secrets or settings only the owner can provide (the loyalty token, the OTP sender, the backup key's whereabouts);
- a person's desk day on the host;
- one host recipe (G6).

The code work still open is the pricing Approve path (#4324, XC-PRICING) and the round-9/10 e2e bugs.

---

## 2. The gates: what must be true before the release order

Each gate is **open** or **closed**, with the evidence that says so. A ticket number is the *source* of a statement, not its status: read the ticket before acting on a line.

| Gate | State | One-line reason |
|---|---|---|
| G1 `main` is green | **closed** | every push run at `1fd766ec` is `success` |
| G2 release path and a rehearsed restore | **open** | no restore has ever been rehearsed; backups have no off-host copy; the loyalty token is missing on the host |
| G3 one model in the schema | **closed** | batch A merged and deployed (round 10) |
| G4 rulings that change code | **open** | pricing Approve cannot be reached (#4324) |
| G5 a person has used it on the host | **open** | nobody has run `docs/uat/desk-day-v1.md`; round-9/10 e2e bugs are open |
| G6 every running thing says what it is | **open** | xc-id reports `build unknown` |
| G7 data and secrets the host needs | **open** | OTP sender is `noop`; retention periods unset (owner) |

### G1 — `main` is green and stays green · **closed**

- Evidence: the `api`, `web`, `app` and `branch-hygiene` push runs on `1fd766ec` all concluded `success` (created 2026-10-21T18:29Z).
- Keeping it closed is the merge lane's job, and the rule is unchanged: tests run on the push to `main`, and a red `main` is reverted (§6).
- The former G1 rows (#2569 CI timeout, #2797 tmp dirs, #2563 phone test flake, #2832 two reapers, #1474 fmt gate) were all closed by 2026-10-20.

### G2 — the release path is a hand deploy on the host · **open**

The owner decided on 2026-10-17 (*"เรารันที่ host เลย"*, "deploy on the host"; #422 closed `not planned` with the record) that **releases are done by hand on the host**. The procedure has now run ten times. Round 10 is the reference (#564 issuecomment-6320509987):
1. Build on Dell from a clean checkout.
2. Check `sha256sum -c` on every hop.
3. Run `deploy/server/update.sh`: migration rehearsal on a scratch copy, encrypted backup, keep `.prev`, migrate, smoke.
4. Confirm by the build that `/health/ready` reports.
5. Record the round on #564.

**Two notes from round 10:**
- Two sessions deployed the same sha 18 minutes apart (15:21Z and 15:39Z), and one of them did not announce itself. One deploy session per order, announced on #564 before it touches the host, is the rule that would have prevented it.
- Rollback after a migration needs the pre-migration set (`20261021T152126Z`). The host keeps 7 sets and prunes the oldest at each backup, so **that set leaves the host after about five more backups** (daily timer plus one per `update.sh`).

| Condition | State | Evidence |
|---|---|---|
| Daily encrypted backups run | **holds** | `xc-v3-backup.timer` active; last run 2026-10-21T17:00:01Z, `Result=success`; newest set `20261021T170001Z` (dump 663,404 B, uploads 29,694,215 B, both `age-encryption.org/v1`) |
| `restore.sh` can target a scratch DB | **holds** | `/opt/xc-v3/restore.sh` is byte-identical to `deploy/server/restore.sh` at `1fd766ec`; it refuses the live DB and the live uploads dir |
| **A restore has been rehearsed and recorded** | **open** | never done. The private key (`age` identity) is kept **off the host by the owner** (`deploy/README.md` §5), so only the owner can run it. Runbook with commands and success checks: #564 issuecomment-6323812080 (§5 D1) |
| Backups survive the loss of the droplet | **open** | `/etc/xc-v3/env` has no `XC_BACKUP_OFFHOST` key, so `backup.sh` skips the off-host copy; all 7 sets sit on the droplet's own disk (§5 D2) |
| The loyalty service is reachable from the host | **open** | `/etc/xc-v3/env` has no `XC_LOYALTY_*` key (key names read, values never). The server answers `GET /api/v3/views/loyalty_rule_versions` with `503 loyaltyUnavailable` and logs `loyalty service token is not configured` (round 10). This is the designed behaviour when `XC_LOYALTY_SERVICE_TOKEN` is unset (`docs/design/blueprint/membership-points.md` R3); provisioning it is the owner's (§5 D3) |

### G3 — one model in the schema, no duplicates · **closed**

- Batch A was approved by the owner on 2026-10-17 (#564 issuecomment-6229594646). Every ticket in it is closed `completed`: #626 CA-10, #677 TX-9, #1023/#1026/#1027 OR-7, #1043/#1044/#1046/#1047 LG-6, #1413 BK-11.
- The last drop migration (`20261219210000_drop_password_login`) runs on the host since round 10. The four old identity tables and `users.legacy_employee_id` are gone, and they held 0 rows before the drop (#564 issuecomment-6320509987).

### G4 — rulings that change code are settled · **open**

| Question | State | Evidence |
|---|---|---|
| Rates: blueprint v3 vs `rates.md` | settled | #986 closed `completed` 2026-10-17 |
| RF-7, campaignSimulate, BK-12, EX-10 | settled: not in the first release | #1036, #1996, #1414, #1191 closed `not planned` on the owner's answers of 2026-10-17 |
| Money tracking (bounded inputs, unchecked arithmetic, fallback to zero) | settled | #2054, #2063, #2108 closed `completed` |
| Desk passport OCR | shipped | #542 closed `completed` 2026-10-20 |
| **Pricing: Approve is unreachable** | **open** | `pricing_board.rs:342` always sends `suggestion: None`, so `TierEditor.tsx` disables Approve (#4324). The work is cut as XC-PRICING PX-1…PX-11 (#4385–#4393, #4402, #4403), under its own sub-controller. **Closes when** #4324's path is merged into `main` and `pricingApprove` can be pressed on the screen |

### G5 — a person has used it on the host · **open**

| Check | State | Evidence |
|---|---|---|
| Booking end to end (#1683) | done | closed `completed` 2026-10-20 |
| Chinese texts read by a person (#1605) | waived | closed `not planned` 2026-10-20 |
| **A desk day by a person** (`docs/uat/desk-day-v1.md`) | **open** | the script exists (#2835). It has no recorded run: searched `desk-day-v1` across the repo's issues and PRs, and only the script's own ticket mentions it. The script still says `https://<staging-host>`; the host is `https://xc-v3.spacing.cloud`, channel `staging`, test data only |
| **The e2e bugs of rounds 9–10** | **open** | #4305, #4337, #4347, #4359, #4360, #4362, #4397 open (FE lane); FE fix PRs 4365, 4377, 4378, 4394, 4399, 4404 open into `main` |
| A day of use with the drop migrations applied | holds | they run since round 10 (2026-10-21T15:21Z); the e2e run that followed passed 96 of 107 that ran, and each failure has a ticket |

### G6 — every running thing can say what it is · **open**

| Item | State | Evidence |
|---|---|---|
| v3 `/health/ready` | holds | reports `version`, `build`, `builtAt`, `channel`; `smoke.sh` refuses a build that does not match |
| `version` | holds | `1.0.0`. The first release commit sets `1.1.0` and nothing else bumps it; it is made **only on the release order** |
| **Identity `xc-id.spacing.cloud` `/health`** | **open** | `build unknown`. Root cause: the `xc-identity` unit's env never sets `APP_BUILD`/`APP_BUILT_AT`/`APP_CHANNEL`; the host recipe writes the sha only to `/opt/xc-identity/BUILD` (`681bea2`). Fix is a script plus test in this repo (#4395); running it on the host is a deploy step on the owner's order |

### G7 — data and secrets the host needs · **open**

| Item | State | Owner's part |
|---|---|---|
| `/etc/xc-v3/pii.key` | present (created 2026-10-10, #422) | keep it backed up off-host; losing it makes encrypted identities unreadable |
| **Customer OTP** | **open**: `AUTH_OTP_DELIVERY_MODE` is set in `/etc/xc-identity/env` and documented as `noop` (`deploy/HOST-do-sgp1.md` §4, I-3) | choose the sender (§5 D4) |
| **Identity-document and login-history retention** | **open**: no period is set, so nothing is purged (`docs/design/blueprint/verification.md` V13, U2; `user-management-rbac.md` §4 item 5) | set the periods as data through `retentionSet` (§5 D5) |

---

## 3. Close-out: every open ticket, one line

Verb per ticket: **before** (release blocker) · **after** (not needed for the release order) · **finish** (in flight; the owner ordered it).

| # | Ticket | Verb | Why |
|---|---|---|---|
| 564 | umbrella | **finish** | the lane's notice board; closes after the release order |
| 4395 | G6 xc-id build stamp | **before** | G6; `class:mechanical` |
| 4324 | pricing Approve unreachable | **before** | G4 |
| 4385–4393, 4402, 4403 | XC-PRICING PX-1…PX-11 | **before** | the work that closes #4324 |
| 4305, 4337, 4347, 4359, 4360, 4362 | round-9/10 e2e bugs | **before** | G5; FE lane |
| 4397 | customer display name cap (100 chars) | **before** | G5; bug, a server/screen mismatch a user meets |
| 4366, 4368, 4370–4373, 4375 | QM/QW kiosk queue machine and web | **finish** | kiosk web queue, owner ruling in PR 4249; see §5 D6 on whether the release waits for it |
| 4379–4384, 4396 | KWQ-A1…A6, KWQ-B1 | **finish** | same feature, BE slices a and b |

Every ticket on the 2026-10-17 version of this table is closed (measured: `gh issue view` on each, 18:4xZ).

---

## 4. Order of work to the release order

| Step | Who | What | Gate |
|---|---|---|---|
| now | XC-READY sub-controller | #4395 to a Gemini worker, review, merge | G6 |
| now | XC-PRICING sub-controller | PX tickets → #4324's path on `main` | G4 |
| now | FE lane | the round-9/10 bug tickets and their open PRs | G5 |
| now | XC-KS sub-controller | kiosk web queue slices | §5 D6 |
| owner's order | owner, with the runbook | the restore rehearsal, recorded on #564 | G2 |
| owner | owner | D2–D5 below (secrets and settings on the host) | G2, G7 |
| after the code steps | owner's deploy order | deploy round 11 of a green `main` sha: carries #4324's path and the FE fixes; runs #4395's `update-identity.sh` | G4, G6 |
| after round 11 | a person the owner names | `docs/uat/desk-day-v1.md` on `https://xc-v3.spacing.cloud`, result recorded on #564 | G5 |
| release order | owner | release commit `1.0.0 → 1.1.0`, hand deploy of that sha, public smoke, then `STATE.md` by the owner | — |

---

## 5. Owner decisions, as one batch

Each line says what is needed, why it blocks, and our recommendation.

| # | Decision | Why it blocks | Recommendation |
|---|---|---|---|
| D1 | **Run the restore rehearsal** (G2) | Only the owner holds the `age` identity that decrypts a backup; a backup nobody has restored is not yet a rollback path | **Run it now** with the runbook in #564 issuecomment-6323812080: about 5 minutes, scratch DB only, live service untouched. Its step 0 also proves the identity file still exists |
| D2 | **Off-host backup copy** (G2) | All 7 sets sit on the droplet's own disk: a lost droplet takes the data and its backups together | **Set `XC_BACKUP_OFFHOST`** per `deploy/README.md` §5 step 4, to a target the droplet can reach over ssh (that route is not measured). It is a host write, so it needs the owner's order |
| D3 | **Loyalty service token** (G2) | Without `XC_LOYALTY_*` on the host, EXECUTIVE rule versions answer 503 and a customer's membership summary is unavailable | **Name the Loyalty instance xchange uses and provision `XC_LOYALTY_BASE_URL`/`XC_LOYALTY_SERVICE_TOKEN`** (token with `loyalty:read` + `loyalty:write`, R3). If none exists yet, say "first release without loyalty": the 503 is the designed fallback, and no code changes |
| D4 | **OTP sender** (G7) | Customer sign-in by phone sends no code while `AUTH_OTP_DELIVERY_MODE` is `noop` | **Pick one SMS provider** for the identity service. The sender is configured in `108-platform-services` and on the host, not in xchange |
| D5 | **Retention periods** (G7) | `VERIFICATION_PASSPORT`, `VERIFICATION_NATIONAL_ID` and `login_history` have no period; nothing is ever purged | **Not a release blocker**: the code keeps everything until a period is set. Set the values through `retentionSet` (EXECUTIVE screen, data, no deploy) before real customers' documents are taken |
| D6 | **First-release scope: does the release wait for the kiosk web queue (KWQ/QM/QW)?** | 14 tickets are in flight; waiting for them moves the release order | **Release without waiting** unless the owner wants it in v1: none of the gates above depends on it, and it lands on `main` slice by slice with its own e2e |

The release commit (`1.0.0 → 1.1.0`) is made only on the release order, so it is not a decision here.

---

## 6. Rules of the road until the release order (restated, with source)

- **Bugs only.** No new tickets unless an important bug; finish the old work first (owner 2026-10-16, recorded on #564 18:38Z). Batch A counts as allowed (owner 2026-10-17 04:4xZ).
- **Tests run on the way into `main`.** PR CI proves it compiles; the push run decides; a red main is reverted with `git revert -m 1` via a PR and the batch is bisected (PR 2786, owner's batch rule 2026-10-16).
- **Merges go through the merge lane** (its current holder is announced on #564), batch PRs merged with a merge commit; no single-PR merges between batches.
- **Destructive migrations:** write + merge approved for batch A; **deploy is a separate order** every time.
- **Deploy and lane branches only on the owner's order** (`deploy/README.md`, `release.yml` header).
- **Poll tq, not GitHub** (owner "ทำ A" 2026-10-17); GitHub at most every 5 min for CI results.
- **Report on the ticket, never in chat**; proposals from a worker go as a comment on the ticket it holds (🎛).
- **Workers take orders from their lane controller or the owner only** (📨).

---

## 7. Keeping this document true

- The BE controller re-measures §1 once a day while the lane is active and updates this file through a docs-only PR (no build or test CI runs on it).
- A line in §3 that names a ticket is true only as long as the ticket is; before acting on it, read the ticket.
- When the owner gives the release order, §1's "build on the host" line gets the released sha, `STATE.md` in `108-Plaza/claude-global-rules` is updated by the owner's order, and #564 closes.
