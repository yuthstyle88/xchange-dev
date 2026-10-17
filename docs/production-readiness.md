# XCHANGE v3 — the road to the production release order

> written-by: claude/c5ca141a · owner's order 2026-10-17 ~05:0xZ, in that session: *"วางแผน ปิดงาน และจ่ายงาน เพื่อให้เอเย่นเป็นแนวทางในการออก production"* (plan the close-out and the dispatch, so that the agents have a guideline for going to production).
> Corrected 2026-10-17 ~05:5xZ after the merger's review (PR 2834 PASS+follow-up): #1413 is not in tq. Measured 2026-10-17T05:0xZ–05:1xZ: `origin/main` `abc46caf`, tq `GET /v1/snapshot` at 05:06:34Z, `gh issue list` / `gh pr list` on `108-Plaza/xchange`, `curl https://xc-v3.spacing.cloud/health/ready`, `ssh Dell-server uptime` at 05:13Z.

**Who this is for.** Every agent working on `108-Plaza/xchange`: the BE controller, the FE controller, the merge lane, the reviewers, and the Gemini implementers. **Lanes are named by role here, never by session id**: the owner's account moves and sessions change, so the current holder of each lane is whoever last announced it on #564 (the lane's notice board), and a new holder announces itself there before acting. It does not add rules. It says, in one place, what has to be true before the owner can give the order to push `release/prod`, which open ticket belongs to which of those conditions, and in what order the lanes release them.

**Who decides.** The owner decides business rules, secrets, deploys and every item in §5. The lane controllers (📨) decide what is pinned to whom inside the order of §4. This document records those decisions with their source; it makes none of its own.

**Production is a state the owner declares.** The fleet's `STATE.md` says no app is in production and that `release/prod`, a `prod` channel or real-looking data prove nothing. This plan therefore ends at *"the owner can give the release order safely"*, not at *"xchange is in production"*. When the owner declares it, `STATE.md` changes; nothing here does.

---

## 1. Where v3 stands (measured)

| Item | Value | How measured |
|---|---|---|
| `main` head | `abc46caf` (05:04Z) | `gh api repos/108-Plaza/xchange/actions/runs?branch=main` |
| Build on the host `xc-v3.spacing.cloud` | `a84962f5`, built 2026-10-11T06:01:32Z, `version 1.0.0`, `channel staging` | `curl -s https://xc-v3.spacing.cloud/health/ready` |
| How far the host is behind `main` | **1,104 commits, 798 merged PRs, 13 migrations (42 → 55)** | `gh api repos/108-Plaza/xchange/compare/a84962f5...main`; `contents/api/migrations` at both shas |
| Deploy lanes | `release.yml` on `main`, **0 runs**; no `release/*` or `deploy/*` branch on org or mirror; no `XC_DEPLOY_*` variable or secret on either repository (names listed, values never read) | `actions/workflows/release.yml/runs`, `git/matching-refs`, `actions/variables`, `actions/secrets` |
| Every deploy so far | by hand, from Dell, with cherry-picks; the last seven are recorded on #422 (2026-10-09 → 10-10) | #422 comments |
| Open issues | **43** (54 open items minus 11 PRs); label counts: `class:guarded` 16 · `class:logic` 9 · `class:mechanical` 5 · `bug` 11 · `question` 4 · `umbrella` 2 · `documentation` 3 | `gh issue list --state open` |
| Open PRs | **11**: 9 Gemini bug-fix PRs opened 10-16/17, PR 2050 (LY-6 BE, released by the owner 02:0xZ, branch updated 04:44Z), PR 1507 (draft, 2026-10-12) | `gh pr list --state open` |
| Queue (xchange rows) | ready 5 (3 unpinned, each blocked or colliding) · leased 1 · pr_open 0 · in_review 8 · rework 1 · blocked 0 | `/v1/snapshot` 05:06Z |
| Verdicts on the 9 Gemini PRs | FAIL on 2743, 2789, 2829 (rework); none yet on 2823, 2825, 2826, 2827, 2828, 2830 | `issues/<pr>/comments`, lines starting `Verdict:` |
| CI policy | tests run only on the push to `main` (PR 2786, merged 01:19Z); PR runs do fmt, clippy, build, deny; main's `api` run is the deciding run and a red main is reverted | #2785 closing note |
| Main's deciding run | `abc46caf` `api` pending at 05:04Z; `cd0e6dba` cancelled (superseded); first full-suite run under 2786 (`567787d`) green, `test` job 57 min on `dell-org-ci-1` | `actions/runs?branch=main` |
| Dell (Linux CI + 6 Gemini + 6 Claude sessions) | load 76 / 106 / 128 on 64 threads at 05:13Z | `ssh Dell-server uptime` |
| Identity service `xc-id.spacing.cloud` | `/health` answers `build: unknown`, `built_at: unknown`, `channel: unknown` | `curl -s https://xc-id.spacing.cloud/health` |
| Velocity | ≥300 PRs merged and ≥500 issues closed in the last 7 days (both hit the query limit) | `gh pr list --state merged --limit 300`, `gh issue list --state closed --limit 500` |

**Reading.** The code moves far faster than the host. Nothing merged in the last six days has run anywhere but CI. The thing that stops a release order today is not the backlog of 43 tickets; it is that **the release path has never been exercised** (§2 G2) and that **the duplicate old models are still in the schema** (§2 G3). Both are already decided by the owner; both have owners; neither needs a new design.

---

## 2. The gates: what must be true before the release order

Each gate names the tickets that close it. A ticket number is the *source* of a statement, not its status: re-check the ticket before acting on a line here.

### G1 — `main` is green and stays green

| Condition | Where it stands | Ticket |
|---|---|---|
| The full suite is green on the head to be released, on the `main` push run | holds for `567787d`; every later head is decided by its own push run | — |
| A PASSed PR cannot stay red from a CI timeout | fixed by PR 2786 + the runner-pool decision A; the opener should close the ticket as fixed | #2569 (close) |
| The `api` test run does not leave Postgres dirs on the runner | fix on `main` (PR 2806); a second, unwired reaper also landed (PR 2822) and the two are blind to each other | #2832 (owner picks one; §5 item 2) |
| The `api` tests do not leave `/tmp/xc-test-docs-*` dirs | PR 2829 in rework (FAIL) | #2797 |
| `user_create_phone_test` does not fail under runner load | PR 2743 in rework (FAIL) | #2563 |
| `cargo fmt --check` covers the `include!`-generated modules | `ready`, `assign:none`; its scope reserves ten whole directories, so it collides with every BE ticket in flight | #1474 (last code ticket before the freeze, see §4 wave 5) |

### G2 — the release path works end to end (#422)

Today a release is a person on Dell. `deploy/README.md` and `release.yml` describe the lane; nothing has run it. The order of proof, each step recorded as a comment on #422:

1. **Owner:** set the per-lane secret and variables named in `release.yml` (`XC_DEPLOY_SSH_KEY`, `XC_DEPLOY_HOST`, `XC_DEPLOY_USER`, `XC_DEPLOY_KNOWN_HOSTS`, `XC_PUBLIC_URL`) on the repository that owns each lane: `yuthstyle88/xchange-dev` for staging, `108-Plaza/xchange` for prod. Nobody but the owner can (O3 of #422).
2. **Claude (#422's assignee or the BE controller):** `workflow_dispatch` on `main` — build only, deploys nowhere. Proves the self-hosted build, the glibc check and the stage directory.
3. **Owner's order, then Claude:** push `deploy/staging` on the mirror. `release.yml` ships to Dell; `smoke.sh <url> <sha>` must pass, and `/health/ready` must name the pushed commit.
4. **Owner's order, then Claude:** push `origin/main:release/prod` on the org. Same smoke, on the host.
5. **Backup and restore rehearsed once** on the host with `deploy/server/backup.sh` + `restore.sh`, result recorded on #422. The timer exists; a restore has never been shown to work.

Until step 4 has happened once, "deploy" still means a hand copy, and the host keeps falling further behind `main`.

### G3 — one model in the schema, no duplicates (batch A, approved)

The owner approved on 2026-10-17 ~04:4xZ (recorded on #564, issuecomment-6229594646): *"อนุมัติ batch A ทั้งหมด (เขียน+merge migration ไม่ deploy) ยกเว้น #1414 จอดไว้"*. Writing and merging is approved; **deploying any of these migrations is a separate owner order**.

| Area | Tickets, in release order (BE controller's) | Drops |
|---|---|---|
| Cash model | #626 CA-10 first | custody requests, signatures, branch custodian (M3) |
| Ledger readers | #1043, #1044, #1046 in parallel after #626; then #1047 | `shift_balances`, `stock_movements`, `ops_state.safe` (migration 3) |
| Identity | #1023 in parallel after #626; then #1026 → #1027 | old `Actor`/`Role`, `legacy_actor` bridge, password login, `owners`/`employees`/sessions/`auth_attempts`, `users.legacy_employee_id` (migration 3) |
| Transactions | #677 TX-9, now | the wide figure and payment columns (migration 3) |
| Booking | #1413 BK-11 — released by the owner 2026-10-17; up-front tests on `test/bk11-1413-upfront` (`77a28ef7`); **not in tq yet**: the BE controller settles the five open points on the ticket, then adds `ready` | v2's rate-lock booking (M2) |

Why this is a production gate and not a clean-up: `STATE.md` says this is the cheapest moment to drop old shapes, and a schema that still carries two models forces every future fix to choose one. The drop migrations are deployed **on staging first (G2 step 3)** and the smoke plus a day of use there is the evidence for the prod order.

### G4 — rulings that change code are settled

| Question | Where it stands | What closes it |
|---|---|---|
| Rates: blueprint v3 vs `rates.md` | O-7…O-11 answered by the owner 2026-10-11 (#986); mapping doc `docs/design/blueprint-v3-rates-mapping.md` on `main` | the BE controller confirms every answer is recorded in the design, then closes #986 |
| RF-7: freshness per side | no comment since 2026-10-10 | owner: "not in v1" (close `not planned`) or a design row (§5 item 3) |
| `campaignSimulate` on several branches | refused today (#1992); the question is open | not a release blocker: the refusal is correct behaviour; stays a design question |
| Two xcpg reapers | both on `main`, one wired | owner picks A or B (§5 item 2) |
| LY-6 loyalty rule versions | released by the owner 02:0xZ ("ทำ 2"); PR 2050 updated onto `main` 04:44Z | merger re-binds and merges 2050 → #2126, #2127 go `ready` → FE releases #2129 |
| Bounded write inputs (20 fields), unchecked money arithmetic, money that falls back to zero | three tracking issues with controller rulings in progress (#2054, #2063, #2108) | the **18 BUG rows of #2108 and the stored-amount paths of #2063 are release blockers** (wrong money); #2054's 500s are a ratchet, not a blocker |

### G5 — a person has used it on staging

Nothing in CI proves a shift can be opened, a customer served and the day closed. After G2 step 3:

| Check | Ticket |
|---|---|
| Booking end to end (C02/C03, desk, admin slots) with a real `?ref=` | #1683 |
| Chinese texts of the customer app read by a person who reads Chinese | #1605 |
| A desk day: sign in → open shift → quote → exchange → receipt → reversal → handover → close → shift report and executive overview agree with the ledger | **no ticket yet** — the FE controller cuts one "UAT script v1" doc ticket (a numbered list with expected screen results, no code); the owner or a person they name runs it on staging and records the result on the ticket |

### G6 — every running thing can say what it is

| Item | Where it stands |
|---|---|
| v3 `/health/ready` | reports `version`, `build`, `builtAt`, `channel` ✓ (`smoke.sh` refuses a build that does not match) |
| `version` | `1.0.0` in `api/Cargo.toml` and at runtime; the fleet standard wants MINOR = release train, bumped only in a release commit — the first release commit sets `1.1.0` and nothing else bumps it |
| Identity `xc-id.spacing.cloud` `/health` | `build: unknown` — the fleet rule is broken there; ticket belongs in `108-Plaza/108-platform-services` (#123 and #144 there are closed, so this is a regression or a build-time flag missing on the host) |
| Backups | `xc-v3-backup.timer` runs daily on the host; restore never rehearsed (G2 step 5) |

### G7 — data and secrets the host needs

| Item | Where it stands | Owner's part |
|---|---|---|
| `/etc/xc-v3/pii.key` | created 2026-10-10 after a deploy found it missing (#422) | keep it backed up off-host; losing it makes encrypted identities unreadable |
| Customer OTP | `noop` sender (HOST-do-sgp1.md §6 I-3) | choose the SMS / e-mail sender (§5 item 4) |
| Identity documents retention | `retention_until` is a §12 legal value | set the period (§5 item 5) |
| Desk passport OCR | #542 waits for Recognize-Platform#150's children (7 in review at 05:06Z: #155, #160–#164) | decide whether the first release ships without OCR if Recognize is late (§5 item 6) |

---

## 3. Close-out: every open ticket, one line

Verb per ticket: **close** (done or not planned) · **finish** (in flight) · **before** (release blocker) · **after** (not needed for the release order) · **owner** (waits on §5).

| # | Ticket | Verb | Why |
|---|---|---|---|
| 422 | deploy lanes | **before** | G2; off-PR steps O3–O6 remain; assignee claude/7a595a04 |
| 542 | DK-F+J desk OCR | **owner** | G7; waits on Recognize #150 children |
| 564 | umbrella | **finish** | all 17 design children closed; stays open as the lane's notice board until the release order, then closes |
| 626 | CA-10 | **before** | G3, approved, pinned studio-8 |
| 677 | TX-9 | **before** | G3, approved, pinned studio-3 |
| 930 | LG-6 parent | **finish** | closes when #1047 merges |
| 947 | OR-7 parent | **finish** | closes when #1027 merges |
| 986 | rates mapping | **close** | answers given 2026-10-11; confirm recorded, close |
| 1023 | OR-7.4 | **before** | G3 |
| 1026 | OR-7.e | **before** | G3 |
| 1027 | OR-7.m | **before** | G3 (drop migration) |
| 1036 | RF-7 | **owner** | §5 item 3 |
| 1043 / 1044 / 1046 | LG-6 r4/r5/r6 | **before** | G3 |
| 1047 | LG-6.m | **before** | G3 (drop migration), last |
| 1191 | EX-10 commission | **after** | needs Broker-Platform's endpoint; A01 shows `awaitsBroker` meanwhile |
| 1411 | LY-6 BE | **finish** | PR 2050 released; merger |
| 1413 | BK-11 | **before** | G3, approved |
| 1414 | BK-12 partner booking | **after** | parked by the owner 04:4xZ |
| 1474 | fmt gate | **before** | G1; last code ticket (§4 wave 5) |
| 1605 | zh texts | **before** | G5, human check |
| 1683 | booking e2e | **before** | G5, human check after G2 |
| 1996 | campaignSimulate | **after** | refusal in place is correct |
| 2054 | bounded inputs (tracking) | **after** | ratchet; 500s, not wrong money |
| 2063 | unchecked money arithmetic (tracking) | **now, in parallel** | stored-amount paths fixed (#2088, #2093, #2096); per_unit cut as #2839. Owner 2026-10-17 ~07:1xZ: the tail (6 K-a + 49 K-b `.minor()` sites) is cut now, one ticket per file, for Flash workers; floor/removal ticket last |
| 2108 | money falls back to zero (tracking) | **before** | 18 BUG rows are wrong money |
| 2126 / 2127 | LY-6 follow-ups | **finish** | `ready` after 2050 merges |
| 2129 | LY-6-FE | **finish** | after 2050 merges |
| 2563 | identity test flake | **finish** | PR 2743 rework |
| 2569 | 90-min cancel | **close** | fixed by PR 2786 + pool A |
| 2703 / 2705 / 2774 / 2782 / 2794 / 2795 / 2796 / 2814 | bug PRs in review | **finish** | merger's batch |
| 2797 | tmp dir leak | **finish** | PR 2829 rework |
| 2832 | two reapers | **owner** | §5 item 2, recommend A |
| PR 1507 | one test binary per package (draft) | **close** | stale since 10-12; TX-9 rewrites 7 of its files; reopen from a fresh branch after G3 if still wanted |

Not on this list: Recognize-Platform's research tickets (#91, #131, #137) and #149 belong to that repo's own plan; only #150's children gate #542.

---

## 4. Dispatch: waves, lanes, disjoint scopes

The target is the fleet rule: ≥80 % of a lane's usable workers busy at the same time, with the tickets of one wave never sharing a file. Each wave below is what a lane releases together; the lane controller pins, the worker claims once with `assigned_only`.

| Wave | Lane | Tickets | Scopes | Gate |
|---|---|---|---|---|
| 0 — now | merger | re-bind + merge PR 2050; batch the 6 PRs without a verdict as their PASSes arrive; 3 FAIL PRs go back through rework | — | G1/G4 |
| 1 — now | BE | #626 · #677 (in tq, approved) · #1413 (approved; tests pushed, not in tq until the BE controller settles its five points and adds `ready`) | `xc-shift`/custody · `transactions` writers · v2 booking module | G3 |
| 2 — after #626 merges | BE | #1043 · #1044 · #1046 · #1023 in parallel | `xc-shift` shift repo · `ops_state` paths · `stock_movements` readers · custody role mapping — **check `scope_prefixes` of all four before pinning**; #1043 and #1044 both name `xc-shift` | G3 |
| 3 — after wave 2 | BE | #1026 → #1027; #1047 after #1043/#1044/#1046 | extractor → drop migrations | G3 |
| 2' — parallel with 2 | BE | #2126 · #2127 (after 2050) · **the #2063 tail: one `class:mechanical` ticket per file (6 K-a + 49 K-b sites), pinned to Flash sessions holding no ticket**; the floor/removal ticket last | one file each — disjoint by construction; check `scope_prefixes` across the set | G4 |
| 3' — parallel | FE | #2129 (after 2050) · "UAT script v1" doc ticket · #1605 | `web/src/screens/…/loyalty` · `docs/` · none | G4/G5 |
| 4 — owner's hands + Claude | #422's assignee or the BE controller | secrets → build-only run → mirror `deploy/staging` → smoke → record | `.github/`, host | G2 |
| 5 — last code ticket | BE | #1474 (fmt gate), pinned to a Claude session, when no other BE ticket is leased or in review | ten directories | G1 |
| 6 — on staging | owner / FE | #1683 · the UAT script · a day of use with the drop migrations applied | — | G5 |
| 7 — release order | owner | `git push origin origin/main:release/prod`; smoke; `STATE.md` | — | — |

**What limits concurrency:** an open PR keeps its ticket's file scope locked through `pr_open` and `in_review` until it merges, so a next ticket in the same files answers `ready_blocked_by=conflict`, and the open-PR guard (claim reason #120) will not hand out a ticket that already has an open PR. The per-worker cap was raised from 1 to 2 on 2026-10-17 at 07:03Z on the owner's order (#564 issuecomment-6231003464), so a worker can hold one ticket in progress next to one in review. Merges free the locks; the merge lane is the main lever. **Supply.** Measured 05:0xZ: Dell `gemini-1,2,4,5,6` idle at the pane (5 of 6), Studio `agy-2…9` idle at the pane (8 of 10; seven of them hold an `in_review` ticket waiting on a reviewer). Unpinned `ready` xchange tickets: 3, every one blocked or colliding. So the bottleneck is **ticket supply and review slots, not workers**: wave 2 is the next supply, and it opens only when #626 merges. The BE controller cuts the #2108/#2063 money tickets now (G4), so wave 2' can start before #626 lands.

**Review.** Six PRs wait for a first verdict while the Dell reviewers each hold two or three slots (`dell-claude-5` holds three: Recognize #162, park108 #1304, xchange #2796). The Claude sessions on Dell and Studio that hold no slot are the review capacity; the controllers point them at a PR with the ticket-specific `/v1/review/next` call.

---

## 5. Owner decisions, as one batch

Each line: what, why it blocks, our recommendation. Idle count attached per the fleet rule: **13 Gemini sessions idle at the pane** (5 Dell + 8 Studio) and **3 unpinned ready tickets, all blocked**, at 05:0xZ.

1. **Deploy-lane secrets (G2 step 1).** `XC_DEPLOY_SSH_KEY` + `XC_DEPLOY_HOST`/`USER`/`KNOWN_HOSTS`/`PUBLIC_URL` on `yuthstyle88/xchange-dev` (staging → Dell) and `108-Plaza/xchange` (prod → do-sgp1). Nothing can be rehearsed without them. **Recommend:** set staging's first, this week; prod's after one green staging push.
2. **Two xcpg reapers (#2832).** A = keep the wired `pg-sweep-stale.sh`, add dead-pid reaping, delete the other; B = switch. **Recommend A**: smallest change, keeps the docker sweep.
3. **RF-7 per-side freshness (#1036).** No answer in 7 days. **Recommend:** not in v1, close `not planned`; the kiosk shows per-line freshness, which is what `rates.md` designed.
4. **OTP sender (I-3).** Customer sign-in needs a real SMS or e-mail sender; today it is `noop`. **Recommend:** pick one provider; the Notification work lands in `108-platform-services`, not xchange.
5. **Identity-document retention period (P-1).** A legal value nobody but the owner can set.
6. **First release without desk OCR?** #542 waits on Recognize #150's children (7 PRs in review). **Recommend:** release without OCR if Recognize is not merged when G1–G5 hold; typed passport fields are v2's behaviour and the verification design keeps them.
7. **Dell load (76/106/128 on 64 threads).** Linux CI, 6 Gemini and 6 Claude sessions share it, and main's deciding `api` run took 57 min there against 25 on a Mac. The merge-lane state names Jam's MacBook as a candidate runner with three blockers (runner 2.338.0 crash, no admin account for Postgres, weak login + Remote Management on). **Recommend:** decide whether that machine joins the Linux-equivalent pool; if not, move the light gates to the macOS pool and accept the main-run time.

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
