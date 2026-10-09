# XC Desk Blueprint v2 — mapped against XCHANGE v3

> mapped-by: claude/608dff (xchange lane controller) · 2026-10-09 · measured on `main` at
> `3d3e8a40481badab3c92368bd75266ef511e3de3` (`3d3e8a4`)

| | |
|---|---|
| **Source** | The owner-supplied file `XC-Desk-Blueprint-Data-Model-v2.html`, *"Blueprint v2 • 8 ตุลาคม 2026 • XC X TK"*. It calls itself a design proposal: its routes and tables were not checked against an existing system, and it asks for a mapping before any migration (its §01 and §13). This document is that mapping |
| **What this is** | A mapping, **not a decision**. It records, for every blueprint screen, entity, role, locked value and acceptance criterion, what v3 already has on `main`, what is being built, what is missing, and where the blueprint contradicts a decision. It changes nothing in `xchange-v3.md`, `xchange-v3-web.md` or `xchange-v3-known-differences.md` |
| **Why** | The owner chose (2026-10-09) to map the blueprint against the v3 design so they can choose which blueprint phases to fund |
| **Read** | `docs/design/xchange-v3.md`, `docs/design/xchange-v3-web.md`, `docs/design/xchange-v3-known-differences.md`; the route table (`api/src/presentation/http/mod.rs:25-42` and every `configure` it calls); the page table (`api/src/presentation/pages.rs`); every migration in `api/migrations/`; `api/crates/*/src`; `web/src/screens/`; `app/lib/`; the open issues and PRs of `108-Plaza/xchange` (listed 2026-10-09) |

Citations are `path:line` at `3d3e8a4`. A "not found" names what was searched. "In flight" names the issue and the PR,
with the PR's head when it was read; it says nothing about the PR's state after 2026-10-09.

**Decided by the owner, applied below, not reopened here:**

| # | Decision | Date |
|---|---|---|
| O-1 | v2 referrals are not carried into v3. Brokers, referrals and commission live in `108-Plaza/Broker-Platform` | 2026-10-08 |
| O-2 | v3 runs in the cloud; the cash drawer goes through a local agent at the counter (`xchange-v3.md` D11) | 2026-10-07 |
| O-3 | The v3 web is bilingual Thai/English through `web/src/shared/i18n` (`xchange-v3-web.md` W10) | 2026-10-08 |
| O-4 | Identity-Platform only authenticates; it gets no account-management features (`xchange-v3.md` D4) | 2026-10-06 |
| O-5 | v2 data is test data only; v3 starts on an empty database; no data-migration tool (`xchange-v3.md` D6) | 2026-10-06 |
| O-6 | Bookings are ported; staff and the owner create them; no public booking site (`xchange-v3.md` D5) | 2026-10-08 |

O-6 is a v3 design decision recorded as the owner's (D5 cites the owner on #377 and PR 395); it is listed because the
blueprint's customer booking contradicts it.

## 1. How v3 is shaped, in one paragraph

v3 is a port of v2: one Rust server for every branch, behind v2's HTTP contract (`xchange-v3.md` §5.3, *"identical to
v2"*), with v3's own SolidJS screens replacing v2's built pages one path at a time (`xchange-v3-web.md` W1–W9). New screens
read per-screen views at `GET /api/v3/views/{name}` (`api/src/presentation/http/v3/view.rs:86`) and compute-only actions at
`POST /api/v3/compute/{name}` (`api/src/presentation/http/v3/action.rs:53`). The customer surface is the Flutter kiosk
(`app/`, web plus the Play Store app, D8/D9), not a customer account app. There is no organization table: the root of all
data is `branches` (`api/migrations/20261007100000_branches_rates.sql:1-7`). The blueprint is a different product shape
(a central multi-company platform with customer, partner and marketing apps), so most of its phases 3–5 are new ground,
and phases 1–2 overlap v3 heavily but with a different rate model.

## 2. Screens

37 blueprint screens: the 28 of its §03 and the 9 of its §15. "Route" is the blueprint's proposal.

### 2.1 Admin (A) and approvals

| ID | Route | v3 status | Evidence and notes |
|---|---|---|---|
| A01 | `/admin/overview` | **in flight** (#461, PR 524 at `d6758a3`; #469, PR 519 at `757aeab`) | PR 524 adds the view `admin_overview` (period totals, buy/sell split, shift-open strip, latest exchanges) for **one branch**; PR 519 adds `admin_reports` (by payment channel, currency, employee, BoT reference spread). Not in either: an all-branch view, per-branch sync time, booking conversion, commission (→ Broker-Platform), drill-down "to the source with the same filters". The owner can already read any branch by naming it (`api/crates/xc-auth/src/domain/policy/entities/policy.rs:43`) |
| A02 | `/admin/rates` | **partial** | Exists: the rates menu `web/src/screens/admin/rates/menu.ts:9`, which saves through `adminSaveRate` (`web/src/screens/admin/rates/Rates.tsx:158` → `api/src/application/compat/rpc.rs:270`), the views `admin_rates` (`api/src/presentation/http/v3/views/admin_rates.rs:42`) and `rates_reference` (BoT panel, `api/src/presentation/http/v3/views/rates_reference.rs:191`). Missing: reference date selector, signed buy%/sell% adjustment, AUTO/MANUAL per side, preview of impact, draft → simulate → approve → publish. v3 saves one currency's buy and sell at once, immediately, by OWNER or RATES (`policy.rs:82`, `:115`). See conflicts K2–K4 |
| A03 | `/admin/rates/history` | **partial** | Exists: view `admin_rate_history` (`api/src/presentation/http/v3/views/admin_rate_history.rs:117`) over the append-only `rates` table (`api/migrations/20261007100000_branches_rates.sql:18-33`: who, when, request id, BoT reference). Missing: approval outcome, reason, guard that was exceeded (no approval exists) |
| A04 | `/admin/branches` | **partial** | Exists: create a branch (`adminCreateBranch`, `web/src/screens/admin/AdminShell.tsx:298` → `rpc.rs:277`), list (`api/src/presentation/http/v3/views/admin_branches.rs:33`), and a per-branch booking on/off and minutes (`bookingRules` in `branch_settings`, `api/migrations/20261008100000_branch_settings.sql:1-9`; `xchange-v3.md` §5.6.3 `settings`). Missing: code, address, timezone, opening hours, contact channels, status, slot capacity. Service hours are hard-coded 11:00–22:50/23:00 (`xchange-v3.md` D7) — conflict K12 |
| A05 | `/admin/users` | **partial** | Exists: the employees menu `web/src/screens/admin/employees/menu.ts:9` over `POST /api/employees` (`api/src/presentation/http/employees.rs:43`; list, create, reset, disable, enable), audited in `employee_audit` (`api/migrations/20261007150000_accounts.sql:38-44`). Missing: several roles per user, scope wider than one branch for a non-owner, valid-from/to. An employee has one `role` and one `branch_id` (`accounts.sql:18-29`) |
| A06 | `/admin/approvals` | **partial** (cash only) | The only approval flow in v3 is custody: a proposed cash request (OPEN, HANDOVER, CLOSE, SAFE_*, EXTERNAL_*; `api/crates/xc-shift/src/domain/custody/value_objects/request_kind.rs:4-13`), one PENDING per branch (`api/migrations/20261007140000_custody.sql:34`), signed and committed by different roles (`xchange-v3.md` §3 Custody). No generic approval queue for rates, exceptions, reversals, campaigns or payouts. Searched `approval` in `api/migrations`, `api/crates/*/src`, `api/src`, `web/src`, `app/lib`: 0 files |

### 2.2 Desk (B) and finance (F)

| ID | Route | v3 status | Evidence and notes |
|---|---|---|---|
| B01 | `/desk/shift` | **exists** (per currency) | Shift open/close and stock moves: the cash menu `web/src/screens/admin/cash/Cash.tsx:247` (`adminOpenShift`), `:357` (`adminMoveStock`), `:452` (`adminCloseShift`); views `admin_shifts` (`views/admin_shifts.rs:76`), `admin_cash` (`views/admin_cash.rs:134`). Per-currency opening/expected/counted/difference (`api/migrations/20261007110000_shifts.sql:18-26`); a non-zero difference needs a note (`xchange-v3.md` §3 Close shift; test `api/crates/xc-shift/tests/shift_service_test.rs:448`). Handover and count sheets by denomination are custody requests on the staff app (`/staff`, `api/src/presentation/pages.rs:81`; `api/crates/xc-shift/src/domain/custody/value_objects/denomination_row.rs:6-10`). Missing: a persisted count line per banknote (denominations live inside the custody request, not a table) |
| B02 | `/desk/bookings` | **partial** | Exists: the bookings menu `web/src/screens/admin/bookings/menu.ts:10` (list, create, confirm, cancel, member) over `POST /api/bookings` and, for the counter roles, `POST /api/staff/bookings` (`api/src/presentation/http/bookings.rs:78`, `:84`); a booking is used by the desk save and becomes `USED` (`xchange-v3.md` §5.6.3). Missing: time slots, QR lookup, check-in, no-show, hand-off "to service". See K5 |
| B03 | `/desk/exchange` | **exists** (v2's kept page) | `/desk` and `/staff/desk` are v2's pages served by v3 (`pages.rs:67`, `:144`), saving through `saveTransaction` (`api/src/application/compat/actions/save.rs:78`) with all money computed by the server (`api/crates/xc-kernel/src/money/quote.rs:30-60`). v3's own desk screen is Wave 3 (`xchange-v3-web.md` §8 V3-desk), not cut yet. Not in v3: a separate quote step before confirm (the save is quote+confirm in one request), benefits/coupons |
| B04 | `/desk/transactions/:id` | **partial** | Exists: the transactions menu `web/src/screens/admin/transactions/menu.ts:11`, view `admin_transactions` (`views/admin_transactions.rs:49`), detail with slip and receipt (`web/src/screens/admin/parts/TransactionDetail.tsx:1`, `:75`), the receipt document (`GET /documents`, `api/src/presentation/http/documents.rs:40`), reprint by retrying the print job (`api/src/presentation/http/devices.rs:186`). Missing: **reversal**. Searched `revers` and `refund` in `api/crates/xc-exchange/src` and `api/src`: no match. `transactions` has no status column (`api/migrations/20261007190000_transactions.sql:2-36`) |
| B05 | `/desk/cash` | **partial** | Exists: balances per currency per shift and stock movements (cash menu, `views/admin_cash_movements.rs:45`), the safe and external movements of custody (`request_kind.rs:8-12`), a computed shift valuation (`api/src/presentation/http/v3/actions/shift_valuation.rs`; panel #511). Missing: per-banknote stock, available vs reserved, low-stock thresholds, transfer between branches with an in-transit state |
| B06 | `/desk/queue` | **partial** | The kiosk queue is v3's customer queue: `kiosk_requests` WAITING → ACCEPTED → ID_CHECKED → COMPLETED with the accepting employee and work times (`api/migrations/20261007170000_kiosk.sql:4-25`, `20261008110000_kiosk_work_times.sql:4-6`), page `/kiosk-queue.html` (`web/pages/kiosk-queue.html:7`), views `kiosk_queue` (`views/kiosk_queue.rs:42`) and `kiosk_notifications` (`views/kiosk_notifications.rs:38`). The admin queue bar is #471 (no PR when read). Missing: queue numbers per branch and day, call/recall/hold/skip, walk-in tickets that are not a kiosk quote, a link from a booking to a ticket |
| F01 | `/finance/reconciliation` | **partial** | Exists: counted vs expected per currency at shift close (above), cash exceptions inside custody (`custody.sql:61`). Missing: a reconciliation screen across transactions, transfers and payouts; a ledger to reconcile against (§3, CashAccount/Ledger). Payouts → Broker-Platform |
| F02 | `/operations/verification` | **partial** | v3 has the roles DOCUMENTS and DOCUMENT_REVIEW (`api/crates/xc-auth/src/domain/policy/value_objects/role.rs:14-15`) and the documents lookup per exchange (`documents.rs:40-46`, the passport file of every exchange). There is no verification case, no queue of cases, no request-for-update. Searched `verification` in the paths above: 2 files, neither a case model |

### 2.3 Public display (P) and customer app (C)

| ID | Route | v3 status | Evidence and notes |
|---|---|---|---|
| P01 | `/rates?branch=:code` | **partial** | The public rates of a branch exist as the kiosk action `rates` (`POST /api/kiosk`, `api/src/presentation/http/kiosk.rs:75`; shape `api/crates/xc-kiosk/src/domain/kiosk/value_objects/public_rates.rs:9-16`: branch name, open flag, rates of the offered currencies, payment rules without the account) and are shown by the Flutter kiosk (`app/lib/features/kiosk/presentation/pages/select_page.dart:37-42`), in 16 languages (`app/assets/l10n/kiosk.json`, `langs`). Missing: a full-screen landscape board, updated-at and reference date on screen, stale badge (searched `stale` in `api/crates`, `api/src`, `app/lib`: only device-agent job expiry), booking QR |
| C01 | `/app` | **gap** | No customer account app. The kiosk is anonymous and branch-bound. See K15 |
| C02 | `/app/book` | **conflicts with a decision** (O-6) | D5: no public booking site; staff and the owner create bookings |
| C03 | `/app/bookings/:id` | **conflicts with a decision** (O-6) | Same. v3 dropped v2's `status_token_hash`, the public reader of a booking (`xchange-v3.md` §5.6.2) |
| C04 | `/app/history` | **gap** | No customer account. The kiosk shows one published receipt for less than 10 minutes (`xchange-v3.md` §3 Kiosk) |
| C05 | `/app/member` | **gap** (tiers) | `members` is a verified-member flag with a member code per branch and customer (`api/migrations/20261008130000_bookings.sql:1-11`), no tier, no points, no coupons |
| C06 | `/app/referral` | **out of v3 — Broker-Platform** | O-1 |
| C07 | `/app/profile` | **gap** | No consent model. Searched `consent` in the paths above: 0 files |
| C08 | `/app/branches` | **gap** | No branch profile (A04) to show |
| C09 | `/app/journey` | **gap** | No planner draft, no pass. `tx_drafts` (`custody.sql:53`) is the counter's own draft, not the customer's |
| C10 | `/app/verification` | **gap** | No verification case (F02) |
| C11 | `/app/rate-alerts` | **gap** | Searched `alert` in the paths above: matches are UI alert roles and device states, no rate alert |
| C12 | `/app/assistant` | **gap** | — |
| C13 | `/app/notifications` | **gap** (customer side) | `kiosk_notifications` is for signed-in staff (`views/kiosk_notifications.rs:38`) |
| C14 | `/app/sign-in`, `/app/sign-up` | **gap**; constrained by O-4 | Customer sign-in may use Identity-Platform for authentication only; profile, consent and membership must live in v3 (or another platform), never in Identity |

### 2.4 Partner (R) and marketing (M)

| ID | Route | v3 status | Notes |
|---|---|---|---|
| R01 | `/partner` | **out of v3 — Broker-Platform** | O-1 |
| R02 | `/partner/referrals` | **out of v3 — Broker-Platform** | O-1 |
| R03 | `/partner/bookings/new` | **out of v3 — Broker-Platform** (the portal) | The booking it creates would be a v3 booking; see question Q8 for the seam. Also blocked by O-6 for a booking not made by staff |
| R04 | `/partner/commissions` | **out of v3 — Broker-Platform** | O-1 |
| M01 | `/marketing/campaigns` | **gap** | Searched `campaign`, `coupon`: 0 files |
| M02 | `/marketing/campaigns/:id` | **gap** | — |
| M03 | `/marketing/members` | **gap**; one input **in flight** | #466 (PR 520 at `12be307`) adds the admin customers view with completed count, volume and last date per customer — the "frequency/value" inputs of a segment, per branch. No tier, consent or segment. Searched `segment` in `web/src`: matches are UI path segments |
| M04 | `/marketing/performance` | **gap**; attribution half → Broker-Platform | Reports by channel/currency/employee are #469 (PR 519); source → booking → completion attribution belongs to Broker-Platform |

**Count of the 37:** exists 2 (B01, B03) · in flight 1 (A01) · partial 12 (A02–A06, B02, B04, B05, B06, F01, F02, P01) · gap 15 (C01, C04, C05, C07–C14, M01–M04; M03 has one input in flight and M04 an attribution half that is Broker-Platform's) · out of v3 — Broker-Platform 5 (C06, R01–R04) · conflicts with a decision 2 (C02, C03).

### 2.5 What v3 has that the blueprint does not mention

These exist and any blueprint phase must fit around them: the customer kiosk with its own quote → wait → accept →
identity check → complete machine (`xchange-v3.md` §3 Kiosk); custody phases CLOSED → READY → OPEN → CUT_OFF → COUNTING
(`custody.sql:4`); DEMO mode beside LIVE on customers and exchanges (`customers.sql:14`, `transactions.sql:7`); non-cash
counter payment methods with a required slip (`kiosk.sql:17-18`, `xchange-v3.md` §3 Payment) and per-branch payment
settings (`web/src/screens/admin/payments/menu.ts:11`, `api/src/presentation/http/payment_methods.rs:38`); receipt
printers and the counter drawer agent (O-2, `api/src/presentation/http/device_agent.rs:47-64`); the global document number
`XC-yyyymmdd-NNNNNN` (`xchange-v3.md` §3 Document ref).

## 3. Data model and roles

### 3.1 Entities

42 blueprint entity groups: 33 of its §07 and 9 of its §16.

| Blueprint entity | v3 status | v3 table / type, and what differs |
|---|---|---|
| Organization | **gap** | No organization table; searched `organization` in migrations and code: 0 files. One server serves one company's branches (D11). See Q7 |
| Branch | **partial** | `branches` (`20261007100000_branches_rates.sql:1-7`: `id`, `name`, `created_at/by`, `request_id`); `bookingRules` in `branch_settings` (`20261008100000_branch_settings.sql:1-9`). No code, address, timezone, opening hours, booking flag column, current publication pointer |
| User / Role / UserRole | **partial** | `owners`, `employees` (`20261007150000_accounts.sql:1-29`): one role and one branch per employee; `Role` (`role.rs:5-16`). No auth subject, no UserRole rows, no validity window |
| Customer | **partial**, **conflicts with v3 design** | `customers` (`20261007120000_customers.sql:6-18`): per branch, unique on (branch, passport, nationality, mode), id `CX-…`. Blueprint wants one customer per company across branches (K6) |
| CustomerIdentity / CustomerContact | **gap** | Passport number and member phone are plain columns (`customers.sql:10`, `bookings.sql:6`). No encrypted value, lookup hash, verified contact or merge |
| Consent | **gap** | — |
| Partner / PartnerAgreement, PartnerUser | **out of v3 — Broker-Platform** | `rates.partner_name` / `partner_cost` (`branches_rates.sql:24-25`) is a rate-cost partner of a currency, not a referral partner |
| ReferralCode, ReferralAttribution | **out of v3 — Broker-Platform** | Seam: Q8 |
| Currency / Denomination | **partial** | `currencies` (`branches_rates.sql:11-16`: code, Thai/English name, symbol). No `minor_unit` (every amount has two decimals, `xchange-v3.md` §3 Amount). Denominations exist only as a value object in custody count sheets (`denomination_row.rs:6-10`) |
| RateReferenceBatch / RateReferenceLine | **partial** | The BoT reference is fetched when a rate is saved and stored inside that rate row as `reference jsonb` (`branches_rates.sql:26`; `api/crates/xc-rate/src/infrastructure/rate/bot_rate_source.rs:14-19`, 6 currencies). No batch table, no source date column, no raw checksum |
| RatePolicyVersion | **gap** | No percentage policy; searched `adjust_pct`, `percent`: no rate policy |
| RateDraftLine | **gap** | A save is immediate; no draft |
| RatePublication / PublishedRateLine | **partial** | `rates` is append-only, one row per currency save; the newest row is current (`api/crates/xc-rate/tests/rates_test.rs:197`). No publication set, no version per branch, no atomic pointer over all currencies; `NUMERIC(14,4)` per 1 unit (K2, K3) |
| Quote / QuoteLine | **partial** | Kiosk quotes are stored with a 5-minute life (`kiosk.sql:4-25`; `api/crates/xc-kiosk/tests/kiosk_machine_test.rs:436`). Desk quotes are computed inside the save and not stored (`quote.rs:30`). No benefit snapshot |
| BookingSlot | **gap** | — |
| Booking / BookingEvent | **partial** | `bookings` (`bookings.sql:13-37`): PENDING / CONFIRMED / USED / CANCELLED (EXPIRED shown, not stored), rate locked at confirm, `transaction_ref` → `transactions.document_ref`. No slot, no event table, no QR token, no source channel |
| Transaction / TransactionLine | **exists** (one line) | `transactions` (`transactions.sql:2-36`): one currency per exchange, decimal money, idempotent on (`branch_id`, `mode`, `submission_id`) + `payload_hash`. No lines, no cost basis, no realized margin, no status (saved = complete) |
| Receipt / Reversal | **partial** / **gap** | Receipt = file of kind RECEIPT (`transactions.sql:39-41`) plus receipt images. Reversal: none (§2.2 B04) |
| Shift / CashCount / CashCountLine / Handover | **exists** (per currency) | `shifts`, `shift_balances` (`shifts.sql:1-26`, one OPEN shift per branch `:16`); handover and counts are custody requests and signatures (`custody.sql:22-43`). No count line per banknote as a row |
| CashAccount / LedgerPosting / LedgerEntry | **gap** | Balances are a running `expected` per shift and currency (`shifts.sql:22`; `xchange-v3.md` §4). No double-entry ledger; searched `ledger` in migrations: 0 |
| InventoryLot / InventoryMovement | **partial** | `stock_movements` (`shifts.sql:28-41`) records manual IN/OUT; exchanges change `expected` directly. No lots, no unit cost, no weighted average |
| CashTransfer / CashTransferLine | **partial** | Custody SAFE_IN/OUT and EXTERNAL_IN/OUT within a branch (`request_kind.rs:8-12`). No transfer between branches, no in-transit |
| StockReservation | **gap** | Blueprint phase 6 |
| Campaign / CampaignVersion / CampaignBranch | **gap** | — |
| BenefitRule / Coupon / BenefitRedemption | **gap** | — |
| CommissionPolicyVersion / Accrual / Adjustment | **out of v3 — Broker-Platform** | O-1 |
| PayoutBatch / Allocation / PaymentAttempt | **out of v3 — Broker-Platform** | O-1 |
| TierRuleVersion / Membership / MembershipHistory | **partial** (membership flag only) | `members` (`bookings.sql:1-11`) |
| SegmentDefinition / SegmentSnapshot | **gap** | — |
| ReferralReward | **out of v3 — Broker-Platform** | O-1 |
| ApprovalRequest / ApprovalDecision | **partial** (cash only) | `custody_requests`, `custody_signatures` (`custody.sql:22-43`) |
| AuditEvent | **partial** | Per area, not one table: `employee_audit` (`accounts.sql:38`), `ops_events` (`custody.sql:69`), `branch_setting_history` (`branch_settings.sql:10-20`), append-only `rates`, `device_events` (`20261007180000_devices.sql:16`) |
| OutboxEvent / IdempotencyRecord | **partial** | Idempotency per table (request ids and payload hashes on rates, shifts, movements, transactions, bookings; `abandoned_requests`). No outbox: post-save work runs in the same DB transaction (`xchange-v3.md` §1) |
| VerificationCase | **gap** | — |
| VerificationDocument | **partial** | `files` + `transaction_files` kind PASSPORT (`20261007160000_files.sql:1`, `transactions.sql:39-41`); no case, retention class or expiry |
| PlannerDraft | **gap** | — |
| QueueTicket / QueueEvent | **partial** | `kiosk_requests` (above); events in the kiosk audit line (`api/crates/xc-kiosk/src/domain/kiosk/value_objects/audit_line.rs`). No ticket number per day |
| BranchServiceStatus | **partial** | `ops_state.phase` (`custody.sql:1-11`) and the kiosk's `open` flag (`public_rates.rs:12`); no counters count or wait estimate |
| RateAlert / RateAlertEvent | **gap** | — |
| Notification / NotificationDelivery | **partial** (staff only) | `kiosk_notifications` view; no stored notification, no delivery |
| PointRuleVersion / PointAccount / PointEntry | **gap** | — |
| AssistantConversation / AssistantMessage | **gap** | — |

### 3.2 Roles

v3's roles are `Role` (`api/crates/xc-auth/src/domain/policy/value_objects/role.rs:5-16`: OWNER plus nine staff roles),
asked through `Policy::allow(actor, EntryPath, Action)` and `Policy::branch` (`policy.rs:17`, `:43`) on the entry paths
of `entry_path.rs:5-22`. A staff caller is always answered for their own branch, whatever branch they name (tests
`api/tests/v3_view_test.rs:584`, `api/crates/xc-auth/tests/policy_test.rs:147`) — the blueprint's "server checks branch
scope on every request" already holds for branch; there is no organization scope to check.

| Blueprint role | Nearest v3 role | Gap |
|---|---|---|
| EXECUTIVE | OWNER | No approval policy by type and amount; the owner is the only all-branch actor |
| ADMIN | OWNER | Branch and employee management is owner-only (`EntryPath::OwnerRoute`, `policy.rs:30`); no admin who is not the owner |
| RATE_MANAGER | RATES (`policy.rs:82`, `:115`) | No "assigned branches"; no out-of-bound approval |
| BRANCH_MANAGER | MANAGER, with BACKEND for shift open/close on `Rpc` (`xchange-v3.md` §6) | — |
| TELLER | FRONT (the custodian who saves, `xchange-v3.md` §3 Custody gate) | — |
| FINANCE | FINANCE (`adminMoveStock`) | No commission or payout work (Broker-Platform) |
| MARKETING | none | New role |
| PARTNER | none | Broker-Platform |
| CUSTOMER | none | The kiosk caller is anonymous; `members` are not accounts |
| VERIFICATION_REVIEWER | DOCUMENTS / DOCUMENT_REVIEW (`role.rs:14-15`) | No verification case to review |
| — | MONITOR, SYSTEM | v3 roles with no blueprint counterpart (custody signers, device work) |

## 4. Locked values and acceptance criteria

### 4.1 Rate engine (blueprint §05) and values to lock (§12)

| Blueprint value | Does v3 guarantee it? | Evidence |
|---|---|---|
| BUY = shop buys foreign currency; SELL = shop sells | **yes** | `api/crates/xc-kernel/src/money/side.rs:5-10` |
| Price is THB per `unit_size` (100 JPY ≠ 1 JPY) | **no — v3 is per 1 unit** | BoT JPY divided by 100 on read (`bot_rate_source.rs:215-220`), shown as such (`web/src/screens/admin/rates/Rates.tsx:190`). K2 |
| Buy and sell references kept apart, with source, date, fetched time; unavailable ⇒ refuse, never guess | **partial** | Reference rows stored with the save (`rates_test.rs:244`); a failing source refuses the save (`rates_test.rs:362`). No separate source date column |
| auto = base × (1 + pct/100), signed | **no** | No rate policy (§3.1) |
| MANUAL per side, global adjust skips MANUAL | **no** | Every v3 rate is typed by hand |
| rate > 0; after rounding buy ≤ sell | **yes** | `CHECK (buy > 0)`, `CHECK (sell >= buy)` (`branches_rates.sql:22-23`); scenario fixture `rates_test.rs:91` |
| Out-of-bound ⇒ approval | **no** | No bounds, no approval |
| Publication all-or-nothing | **no** (per currency) | Each save writes one currency; concurrent saves of one row: exactly one wins (`rates_test.rs:543`) |
| DECIMAL only; rate scale 8; % scale 4 | **partial** | Decimal everywhere (`rust_decimal`, `NUMERIC`), but rates are `NUMERIC(14,4)` with at most 4 decimals (`xchange-v3.md` §3 Amount). K3 |
| THB rounded to 2 places HALF_UP at the final amount | **yes** (on the principal; fee is already 2 places) | `quote.rs:36-37` (`MidpointAwayFromZero`); test `api/crates/xc-kernel/tests/quote_test.rs:144` |
| Foreign amounts per currency minor unit and banknotes | **no** | Two decimals for every currency (`xchange-v3.md` §3 Amount) |
| JS float never authoritative | **yes** for v3 screens | W7 (`xchange-v3-web.md` §2): screens do no money arithmetic, new views send decimal strings. The kept v2 pages still receive JSON numbers (`xchange-v3.md` §5.3) |
| Immutable publication; quote references a rate line id and snapshot | **partial** | `rates` rows are never updated; a saved exchange stores its `rate` and `reference_rate` (`transactions.sql:17-18`), not a rate row id |
| Stale policy limits or stops quoting | **no** | — |
| Commission / referral values | **out of v3 — Broker-Platform** | O-1 |
| Member tiers, rolling 12 months | **no** | — |
| Promotion stacking | **no** | — |
| Booking: queue first; partner bookings confirmed by the customer; slot length, cutoff, no-show | **partial** | Booking life 5–1440 minutes per branch (`bookingRules`, `xchange-v3.md` §5.6.3 `settings`); no slots, cutoff or no-show; no partner bookings |
| Cash & cost: weighted average, shift counts | **partial** | Shift counts yes (`shift_service_test.rs:448`); no cost method |
| Brand / privacy: PII by role, retention, export permission | **partial** | Files readable per role (`policy.rs:183`, `:196`); backup and log retention (`xchange-v3.md` §7b); no retention per document class |

### 4.2 Acceptance criteria per blueprint phase (§13)

| Phase | Criterion | v3 | Evidence |
|---|---|---|---|
| 1 | auto follows signed % | **no** | — |
| 1 | MANUAL side kept | **no** (n/a: all manual) | — |
| 1 | cross-unit computed correctly | **no** (single unit) | K2 |
| 1 | the same publication on every page | **partial** | One `rates` table read by desk, kiosk and admin; the desk save refuses a rate other than the counter rate (`api/tests/booking_save_step_test.rs:293`); the kiosk re-checks at confirm (`kiosk_machine_test.rs:530`) |
| 1 | stale shown clearly | **no** | — |
| 1 | out-of-bound cannot publish without approval | **no** | — |
| 2 | a repeated complete adds nothing | **yes** | `api/tests/save_transaction_test.rs:757`; shift requests `shift_service_test.rs:1136` |
| 2 | concurrent exchanges never oversell stock | **yes** | `save_transaction_test.rs:793`; `shift_service_test.rs:1033`, `:1080`, `:1177` |
| 2 | reversal reconciles | **no** | no reversal |
| 2 | dashboard drills to the source and matches cash/ledger | **in flight** / **no** | #461 (PR 524), #469 (PR 519); no ledger |
| 3 | all four (partner scope, single attribution, adjustment after payout, payout retry) | **out of v3 — Broker-Platform** | O-1 |
| 4 | last slot goes to one of two | **no** | no slots |
| 4 | partner confirmation complete | **no** | — |
| 4 | booking linked to its transaction | **yes** | `bookings.transaction_ref` (`bookings.sql:32`, `:36`); `booking_save_step_test.rs:266`, `:333`, `:364` |
| 4 | QR exposes no PII | **partial** | No QR; the member code is stored only as a hash (`bookings.sql:7`) |
| 4 | rescheduling never takes two slots | **no** | — |
| 5 | all five | **no** | — |
| 6 | reservation expiry returns stock | **no** | — |
| 6 | complete uses the right quote lock | **partial** | A confirmed booking locks the counter rate until its expiry (`api/crates/xc-exchange/tests/booking_service_test.rs:1153`) and the save keeps it (`booking_save_step_test.rs:266`); kiosk quotes expire after 5 minutes (`kiosk_machine_test.rs:436`) |
| 6 | network retry creates nothing twice | **yes** | `save_transaction_test.rs:757`, `booking_service_test.rs:903` |
| 6 | app reads the same data as the web | **yes** | The kiosk and the web read one server |

The Journey Pass criteria of the blueprint's §18 (planner, pass, verification, queue numbers, points, alerts, three
languages) are **not** guaranteed: none of those features exists. The one partial analogue is that a kiosk customer can
quote and confirm but only staff accept, check identity and complete (`api/crates/xc-auth/tests/policy_kiosk_test.rs:122`).

## 5. Conflicts

| # | Blueprint | v3 / decision | Kind |
|---|---|---|---|
| K1 | Routes `/admin/*`, `/desk/*`, `/app/*`; API `/api/rate-drafts/:id/…`, `/api/quotes`, `/api/transactions/:id/complete` | v2's contract kept (`xchange-v3.md` §5.3); new reads at `/api/v3/views/{name}`, new compute at `/api/v3/compute/{name}`; admin menus inside `/` (`xchange-v3-web.md` §12.13) | naming — v3's scheme should win; the blueprint's names are proposals |
| K2 | Rate is THB per `unit_size` | Rate is THB per 1 unit (`bot_rate_source.rs:215`) | v3 model |
| K3 | Rate `DECIMAL(20,8)` | `NUMERIC(14,4)`, at most 4 decimals (`branches_rates.sql:22`, `xchange-v3.md` §3) | v3 model. With per-1-unit rates, 4 places leave few significant digits for low-value currencies (1 KRW is a few hundredths of a baht), so K2 and K3 must be decided together (Q2) |
| K4 | Rate workflow DRAFT → PENDING_APPROVAL → APPROVED → PUBLISHED → SUPERSEDED; % policy | Immediate manual save by OWNER/RATES with an optimistic check and BoT reference (`xchange-v3.md` §3 Save rate) | v3 model (a port of v2, W9) |
| K5 | Booking = a queue reservation with a slot; indicative quote; final rate at the transaction; rate lock only in phase 6 | A CONFIRMED booking **locks the counter rate** until its expiry and skips the rate check (`xchange-v3.md` §5.6.3, test `booking_save_step_test.rs:266`) | v3 model, opposite meaning (Q5) |
| K6 | One customer per company across branches; `customer_no`; repeat customers counted across branches | Customer per branch, keyed by passport (`customers.sql:17`; `xchange-v3.md` §3 Customer identity) | v3 model (Q6) |
| K7 | Customer self-booking (C02/C03), partner booking (R03) | No public booking site; staff create (O-6) | **decision** |
| K8 | Partner portal, referrals, commission, payouts (R01–R04, C06, §07 partner/commission/payout entities, phase 3) | Broker-Platform owns them (O-1) | **decision** |
| K9 | Customer sign-in, profile and consent (C14, C07) | Identity only authenticates (O-4) | **decision** — authentication may come from Identity; everything else is v3's or another platform's |
| K10 | "Map the old system, then migration rehearsal with trial data"; JPY rate conversion at migration (§13, §17) | v3 starts empty, no import (O-5) | **decision** — those steps are not needed |
| K11 | TH/EN/ZH on every UI | v3 web TH/EN (O-3); the kiosk already has 16 languages including Chinese (`app/assets/l10n/kiosk.json`) | **decision** for staff screens; a new customer app is open (Q9) |
| K12 | Per-branch opening hours, holidays, slot capacity | Hours hard-coded 11:00–22:50/23:00 (`xchange-v3.md` D7, "for parity") | v3 design |
| K13 | `organization_id` on every row | One company, branches as root (D11) | v3 shape (Q7) |
| K14 | Transaction DRAFT → PENDING_APPROVAL → CONFIRMED → COMPLETED; edits only by reversal | Saved = complete in one DB transaction (`xchange-v3.md` §1); no reversal | v3 model; reversal is a gap, not a contradiction |
| K15 | Customer app = mobile web (C01–C14) | Customer surface = the Flutter kiosk, web + Play Store `com.spacing.xchange` (D8, D9, W3) | product shape (Q9) |
| — | No online payment in scope | v3 takes no online payment either; PromptPay, card, Alipay and transfer are counter methods with a slip | agrees |
| — | Cloud web + counter | v3 runs in the cloud with a counter drawer agent (O-2) | agrees |

## 6. Gap summary by blueprint phase

Every row that adds a table needs, in its design doc, the fleet's **data-migration section** (what existing rows become,
backfill or why not, deploy order, what `down` loses). O-5 makes the first install empty, but once v3 is installed the
rows of `rates`, `customers`, `bookings` and `members` exist and each change below touches them. Every row needs a
**class diagram merged before its tickets** (`xchange-v3.md` §10 is the pattern) and up-front failing tests per ticket.

| Phase | What it means for v3 | Size | Depends on |
|---|---|---|---|
| **1 — rates and storefront** | New tables: rate policy versions, rate drafts and draft lines, rate publications + lines + a current pointer per branch, approval requests and decisions; branch profile columns. New screens: A02 rebuilt around draft/simulate/approve/publish, A06 approvals queue, P01 landscape board (can read the kiosk's public `rates`), stale display. Changes the save-rate path every kept screen uses, and the kiosk's rate check | **L** | Q1–Q4; K12 (hours) if branch hours are wanted |
| **2 — transactions and management** | Being built: A01 (#461), reports (#469), customers (#466), shift calculators (#511). New: reversal (request → approve → post) on `transactions`; cost basis and realized margin (inventory lots + weighted average) and a ledger to reconcile against; transfers between branches with in-transit; an all-branch executive view; F01 screen. The ledger is the largest piece and touches every money path | **L** (ledger/cost **XL**) | Q10 (cost method); phase 1 publications if quotes must cite a rate line |
| **3 — partner / referral** | Out of v3 (O-1). v3 side is only the seam: an attribution reference on a booking or transaction, and a way for Broker-Platform to learn of completed and reversed exchanges (v3 has no outbox today) | **S–M** (v3 side) | Q8; Broker-Platform's design |
| **4 — booking and customer app** | Blocked by O-6 for the customer half. With O-6 reopened: customer accounts (authentication via Identity, O-4), customer profile and consent, booking slots with atomic capacity, QR check-in and no-show, queue tickets with numbers and call/recall/skip (B06), branch finder, journey pass, verification cases and the reviewer screen (C10, F02), notifications. A new customer app | **XL** | Q4–Q6, Q9; phase 1 branch profile |
| **5 — marketing and member** | Campaigns with versions, benefit rules, coupons and redemptions with atomic budget; tiers and points ledgers; segments; rate alerts; consent-gated eligibility; simulator that must equal real quotes | **L** | Phase 4 customer accounts and consent; phase 2 margin (for contribution); phase 1 publications (for simulated rates) |

### Questions only the owner can answer

| # | Question | Options | Controller's recommendation |
|---|---|---|---|
| Q1 | Rate model | A. keep v3's direct manual rates and add % policy + approval **on top**, producing the same `rates` rows · B. replace with the blueprint's draft → approve → publish | **A** — the kept screens, the kiosk and the save's rate check all read `rates`; A adds without breaking them |
| Q2 | Rate unit and precision (K2, K3) | A. per 1 unit, raise precision · B. per `unit_size`, keep 4 places · C. per 1 unit, 4 places (today) | A or B, decided together; C limits low-value currencies |
| Q3 | Which rate changes need approval, and who approves (bounds per currency? per branch?) | — | none without the business rule |
| Q4 | Reopen D5 for customer self-booking and a customer app? | A. keep D5 (staff bookings only; skip the customer half of phase 4) · B. reopen | none — a business choice; phase 4 cannot start without it |
| Q5 | What does a booking promise (K5)? | A. a locked rate until expiry (v3 today) · B. a queue place only, rate at the counter (blueprint) | none; A is what staff use now |
| Q6 | One customer across branches (K6)? | A. per branch, by passport (v3 today) · B. one per company | **B if phase 4 or 5 is funded** — tiers, repeat customers and segments need it, and the change is cheapest before real data exists |
| Q7 | Will one v3 server ever hold more than one company (K13)? | A. no — one company per server · B. yes — add `organization_id` | **A**, unless a second company is planned |
| Q8 | Seam to Broker-Platform: what does Broker need on a booking or exchange (an attribution id?), and does it pull completed/reversed exchanges or does v3 push them? | — | ask the Broker-Platform controller, then the owner |
| Q9 | Customer app: languages (TH/EN/ZH?) and technology (extend the Flutter `app/`, or web) | — | none |
| Q10 | Cost method for margin (weighted average per branch and currency?) | — | needs the accounting owner; the blueprint itself leaves it open |
