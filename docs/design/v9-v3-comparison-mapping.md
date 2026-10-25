# XCHANGE V.9 compared with v3 — CMP-xx mapped to v3

> mapped-by: claude/bc8fd29a (xchange BE lane controller) · 2026-10-25 · measured on `main` at `38871dcb1`

| | |
|---|---|
| **Source** | The owner-supplied report, committed as [`docs/blueprint/XCHANGE-V9-V3-Comparison-20261025.pdf`](../blueprint/XCHANGE-V9-V3-Comparison-20261025.pdf) (sha256 `5fdaf91a228252a1…`, 55 pages, 51 topics CMP-01…CMP-51). It is a read-only UI review dated 2026-10-25 on MAIN / XC-B. It says itself that a button on a screen does not prove the backend enforces it, and that "not found" means "not found in the screens surveyed", not "absent from the code". |
| **What this is** | For each CMP topic: the v3 screen and backend view that already cover it, what changes so the screen and the order of work match V.9, and which side owns the change. A mapping, not a ticket list. |
| **Owner's decision** | Asked in claude/bc8fd29a's session, 2026-10-25, answered there: first *"Keep the Blueprint (O-7) as the requirement and use V.9 only as a look/UX reference. Write the CMP-xx → v3 mapping as a doc PR, and propose tickets only for confirmed gaps."*, then *"เราต้องแก้ให้ ตรง V9"* ("we must fix it to match V9"), then, choosing between three readings: **"หน้าจอ+ลำดับงานตาม V9"** (screens and the order of work follow V.9). |
| **Read** | `web/src/screens/` (every folder), `api/src/presentation/http/v3/views/` (every view), `docs/design/blueprint/*.md`, `docs/design/xc-desk-blueprint-v2-mapping.md` (decisions O-1…O-7) |

## 1. The rule this mapping follows

**Screens and the order of work follow V.9. The server rules stay v3's.** Concretely:

| Follows V.9 | Stays v3 (not changed by this mapping) |
|---|---|
| Menu grouping, page layout, the order of steps on Desk, Kiosk, queue, booking and pricing, labels, what is shown together on one screen | The open-shift gate on Desk (`CMP-02`, `CMP-05`), counting before verification, the count-difference gate and FINANCE approval |
| | Approval through `xc-approval`; a rate publishes by policy or after approval (`pricing-tiers.md` P-3: V.9's *approve-is-publish* is not brought in) |
| | Idempotency of every write, branch scope, permissions checked by the server (hiding a button is not a permission) |
| | **O-1:** brokers, referral codes, commission and payout live in `108-Plaza/Broker-Platform`; v3 holds only the attribution and exchange-feed seam (`docs/design/blueprint/broker-seam.md`) |
| | Figures with no source yet answer "unavailable" with a reason, never 0 (`executive-finance.md` D-2) |
| | USDT trading and Telegram identity are not in v3's scope (no Blueprint section; nothing in the code) |

A V.9 screen that needs one of the right-hand rules changed is **not** covered by this decision. It goes back to the owner.

## 2. The mapping

Columns: **v3 today** = the screen folder under `web/src/screens/` and the backend view under `api/src/presentation/http/v3/views/` · **To match V.9** = the screen change · **Side** = FE (web/app screens), BE (server), BR (Broker-Platform), — (no change) · **Status**: `look` = v3 has it, only the screen changes · `seam` = belongs to another platform · `out` = outside v3's scope · `awaits` = the screen exists but its data source does not yet · `test` = needs a real-device or UAT check, not code.

| CMP | Topic | v3 today | To match V.9 | Side | Status |
|---|---|---|---|---|---|
| 01 | Start of day, overview | `admin/overview` · `admin_overview` | Put the queue and "start transaction" on the main path; show branch, shift and connection before any work; unknown shows as unknown, not 0 | FE | look |
| 02 | Open shift, cash | `staff/shift` · `my_shift`, `cash_counts` | Show who must act next (count → verify) on the shift card | FE | look |
| 03 | Customer kiosk | `app/` (Flutter kiosk), `kiosk-currencies` · `kiosk_currencies` | One page: direction → currency (with flag) → amount on a keypad → channel → total, as V.9; test portrait and landscape on the real tablet | FE | look + test |
| 04 | Customer queue | `kiosk-queue`, `admin/queue` · `queue_board`, `kiosk_queue` | Taking a ticket opens Desk pre-filled; show who took it and how long it waits | FE | look |
| 05 | Transaction, 5 steps | `desk`, `desk-transaction` · `desk_config`, `transaction` | Rate → Passport → Booking → Payment → Receipt as V.9's five steps, keeping data when going back; the open-shift gate stays | FE | look + test |
| 06 | Online booking | `admin/booking-slots`, `desk` · `desk_bookings`, `admin_booking_slots` | A booking list for staff separate from the slot policy, opening Desk from one booking number | FE | look |
| 07 | Customer from a broker | — | — | BR | seam (O-1) |
| 08 | Customers, members | `admin/customers`, `admin/members` · `admin_customers`, `staff_customers` | Fast search; reuse permitted data on Desk | FE | look |
| 09 | Member requests | `operations/verification`, `admin/members` · `verification_queue`, `verification_case` | A V.9-style request → passport check → approve → member-number search path, beside v3's member groups | FE | look |
| 10 | Rates for staff | `staff/rates`, `admin/rates` · `staff_published_rates`, `rate_drafts` | Separate "read rates" from "edit and approve"; show the effective version and time | FE | look |
| 11 | Currencies the customer sees | `admin/denomination-groups`, `kiosk-currencies` · `denominations`, `kiosk_currencies` | One page for "currencies on offer + display order", apart from denomination groups | FE | look |
| 12 | Rate board and queue QR | `customer/rate-display` (public `/rates?branch=`), `admin/notices` · `published_rates` | A preview of the shop board from the back office with its data freshness | FE | look |
| 13 | Exchange list | `admin/transactions` · `admin_transactions`, `transaction_audit` | Link queue / booking / customer / receipt numbers into one record | FE | look |
| 14 | Slips and evidence | `admin/slips`, `documents` · `admin_slips`, `transaction_documents` | Show evidence status and transaction number together | FE | look |
| 15 | Payment-method settings | `admin/payments` · `admin_payments` | One name per channel on every screen, with its evidence rule | FE | look |
| 16 | Finance and evidence | `admin/reconciliation` · `finance_reconciliation` | One finance page joining transactions, evidence, totals, differences and verifier; cash apart from bank | FE | look |
| 17 | Reports | `admin/reports`, `staff-report` · `admin_reports`, `staff_daily_report` | Drill from totals to transactions and evidence with one filter set | FE | look |
| 18 | Brokers and referrals | `referral_attributions` (capture only) | — | BR | seam (O-1) |
| 19 | Commission and payout rounds | — | — | BR | seam (O-1) |
| 20 | Staff and permissions | `admin/users` (`UsersScreen`, `LoginHistory`, `UserAudit`, `PermissionMatrix`) · `admin_users`, `admin_login_history`, `admin_user_audit`, `admin_permission_matrix` | The four parts exist; arrange them as V.9 does and use role names shop staff understand | FE | look |
| 21 | Staff account entry | `admin/users`, `staff` · `staff_me` | One entry page linking users and staff, saying which work is staff's and which is the owner's | FE | look |
| 22 | Devices and printing | `devices` · `devices`, `device_jobs`, `device_events` | Separate "saved", "print queued" and "printed"; reprint keeps the number; test the real NEOCAL printer | FE | look + test |
| 23 | Executive overview | `admin/executive` · `executive_overview` | Group by sales / funds / profit / exceptions with sources; drill down to branch | FE | look |
| 24 | Per-branch results | `admin/executive`, `admin/overview` · `executive_overview` | A per-branch page with the shared date filter | FE | look |
| 25 | HQ profit confirmation | `admin/executive` (`realized_margin_thb` = unavailable, `awaitsLedger`) | — until LG-4 gives realized margin | — | awaits |
| 26 | HQ commission approval | `admin/approvals` (generic) | — | BR | seam (O-1) |
| 27 | HQ payout history | — | — | BR | seam (O-1) |
| 28 | HQ funds and stock | `admin/cash` · `admin_cash`, `branch_stock` | Cash, bank and stock with units and cost kept apart | FE | look; cost part awaits LG-4 |
| 29 | Branch system and connection | `admin/branches` · `admin_branches`, `branch_service_status` | A system-health view separate from branch settings | FE | look |
| 30 | HQ approval history | `admin/approvals` · `approval_requests` (decided tab exists) | Search from a request number to its transaction; cross-branch scope as it really is | FE | look |
| 31 | USDT orders | — | — | — | out |
| 32 | Telegram identity | — | — | — | out |
| 33 | USDT settings | — | — | — | out |
| 34 | Cash-transfer limits | `admin/cash-limits` · `cash_limits`, `cash_transfer_policy` | Link from the shift's transfer step | FE | look (v3 only) |
| 35 | Count-difference limit | `admin/cash-limits` (`CashLimits.tsx`) · `cash_limits` | Show the limit's guidance at shift close | FE | look (v3 only) |
| 36 | Organisation starting prices | `admin/starting-prices` · `org_starting_prices` | — | — | look (v3 only) |
| 37 | Suggested prices, 4 tiers | `admin/pricing` (`PricingGrid`, `Recommendations`, `TierEditor`) · `pricing_board` | V.9's grid with per-currency expand; v3 approval flow kept | FE | look |
| 38 | Campaigns | `admin/campaigns` · `campaigns` | — | — | look (v3 only) |
| 39 | Campaign results | `admin/campaign-performance` · `campaign_performance` | — | — | look (v3 only) |
| 40 | Points and tiers | `admin/loyalty` · `loyalty_rule_versions` | Wording to match the member pages | FE | look |
| 41 | Consent and retention | `admin/consent` · `consent_texts` | — | — | look (v3 only) |
| 42 | Branch alert thresholds | `admin/alert-settings` · `branch_alert_settings` | — | — | look (v3 only) |
| 43 | Rate alert thresholds | `admin/rate-alert-settings` · `rate_alert_settings` | Link an alert to its price request | FE | look |
| 44 | Notification inbox | `admin/notices` | Separate "needs action" from information, with owner, status and a link | FE | look |
| 45 | Staff guide | `staff-guide` | Steps and labels as the V.9-order screens use them | FE | look |
| 46 | Pricing: per currency | `admin/pricing` (`PricingRow`, `TierEditor`) | Buy/sell × 4 tiers with spread, suggested, approved and last approver in the currency's row | FE | look |
| 47 | Pricing: MAIN and branches | `admin/pricing` · `pricing_board`, `org_starting_prices` | Show the base, the branch adjustment (bp) and branch-only currencies before approval | FE | look |
| 48 | Pricing: market and policy | `admin/pricing` (`MarketSources`, `PricingPolicy`) · `rates_reference`, `rate_policy` | Market freshness (TT / Super Rich), fallback reason and minimum spread in shop wording | FE | look |
| 49 | Pricing: Follow and approve | `admin/pricing` (`BulkApprove`) · `rate_policy` (in-bound publishes by policy) | A "Follow" control mapped onto v3's policy publish; approval stays `xc-approval` (P-3) | FE | look; confirm with owner if Follow must publish outside policy |
| 50 | Pricing: board tiers and order | `admin/pricing`, `customer/rate-display` | Choose board tiers 1/4 without removing 2/3; currency order apart from price | FE | look |
| 51 | Pricing: orders vs market | `admin/pricing` (`OrderRefs`) · `published_rate_lines.source_snapshot` | V.9's columns (order, tier, THB difference vs TT); difference vs market is not profit | FE | look |

## 3. What this gives

- **No v3 backend gap is confirmed.** Every `look` row is served by a view that exists on `main` (listed per row). If a V.9 screen needs a field a view does not answer, that is found while writing the FE ticket and becomes a BE ticket then.
- **Screen work (FE) belongs to the xchange FE lane.** Its controller proposes the tickets to the owner, per the owner's rule that every new xchange issue is proposed first (#564 issuecomment-6355753594).
- **Not v3 work:** CMP-07, 18, 19, 26, 27 (Broker-Platform, O-1); CMP-31, 32, 33 (out of scope); CMP-25 waits on LG-4.
- **Needs a real device or UAT, not code:** CMP-03 (tablet), CMP-05 (a branch with an open shift), CMP-22 (NEOCAL printer).
- **The one row that could touch a server rule:** CMP-49. A "Follow market" that publishes outside the approved policy would change P-3, which this decision does not cover.
