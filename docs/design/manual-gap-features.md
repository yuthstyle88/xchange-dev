# Features from the shop manual that v3 does not have

Status: **plan, for the owner to order**. Each feature gets its own design file, with a class diagram and a behaviour
table, before any ticket is cut.

## 0. Decision, owner's words, reason

**Decision.** v3 adds the features in the shop's XCHANGE manual (Nov 2026 edition, the system at rate.xc888.cc) that v3
does not have and that Blueprint v2 never names. Where the manual and the Blueprint both cover a thing and disagree,
the Blueprint stays and nothing in it changes. That covers the two-person count sign-off, counts by note type, and
calling the figure "contribution" rather than profit.

**Owner's words** (2026-11-19, in a Claude session):
- *"เพราะตาม blueprint เราทำไปครบแล้ว"*
- *"แล้ว เสนอ เพิ่ม ฟีเจอร์ใหม่"*
- *"เราจะเพิ่มใน สิ่งที่ไม่มี"*

**Reason.** The Blueprint is complete in v3, and its logic is fixed (owner, 2026-10-09). The manual describes how the
shop works today. The gap is therefore new features, not Blueprint rework. The checks behind this:
- a keyword search of `docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`;
- a code survey of `api/` and `web/src` at `f764777b2`.

## 1. What is missing (measured at `f764777b2`)

| # | Feature (manual §) | v3 today | Blueprint v2 |
|---|---|---|---|
| F1 | Sell-back to another shop + owner fund (1.11, 2.3) | none | not named |
| F2 | USD note-type codes USDL / USDS / USD1 chosen on every buy and sell, with no default; USDS cannot be sold (1.7, 1.8) | note types exist only as admin denomination groups; the desk does not ask for one | note types are named for counts only |
| F3 | USDT with a manual rate: editable, 25–45 THB, up to 4 decimals, the entering user recorded; allowed when the board shows ASK (1.9) | no USDT, no manual rate | not named |
| F4 | A top-up into the drawer must name its source (safe / owner / bank) (1.6) | the transfer kind plus an optional note | not named |
| F5 | Keypad buy screen: a flag grid of currencies with rates, a large keypad, a review step and then the receipt (1.7) | a 5-step form | the screens are B01–B05; the look may change |
| F6 | Shift summary: day, morning and evening cards; compare 7/14/30/60/90 days or a custom range (≤ 120 days, ≤ 400 back); bar chart (2.1) | a staff report with no compare and no chart | not named |
| F7 | Admin backup download (JSON without images or passport numbers) (2.5) | server backup only | not named |
| F8 | LINE: per-order notify + 20:30 summary, rate sign as PNG + A4 PDF resent on change, sell-back orders to a LINE group (1.12, 2.2, 2.3) | no LINE integration | not named |
| F9 | Sign in with Telegram (Telegram ID bound by an admin) (1.1, 2.5) | username + password only | not named |
| F10 | Photo at clock-in, clock-out and close (1.4, 1.14) | clock with no photo | not named |
| F11 | Finance documents: payment voucher, purchase order, tax invoice upload, monthly input-VAT summary (2.4) | none | not named |
| F12 | Google Sheet "บันทึกยอดรายกะ" filled every ~30 min (1.15, 2.6) | none; reports read the DB | not named |

### 1.1 What v3 may already do in its own way: the old system was part manual

Owner, 2026-11-19: *"บางอย่างอาจมีแล้ว แต่ ใน pdf อาจยังทำงาน กึ่ง manual อยู่ เลยต้อง ไป เอางานจาก google sheet"*.
Then: *"เนื่องจาก ระบบเดิมไม่รองรับ จึงแยกงานไปทำ ที่ google sheet"*.
Then, when asked to open the Sheet: *"เราไม่ไปอ่าน google sheet ดิ"*.
The manual is the old system's workflow. Several of its steps exist only because that system could not hold part of
the work, so it went to paper or to a Google Sheet. **v3 does not read the Sheet and does not copy its shape.** The
work the Sheet carried is taken from the manual's own description of it, and v3 records that work in its own data.
The reason is inferred from the owner's words and checked against the code: the Sheet was a workaround for the old
system's limits. v3 already holds the same facts, the counts, differences, exceptions and transfers, so the Sheet's
layout is not a requirement.
**v3 already has an auditor sheet:** F01 reconcile-and-certify per branch-day (`api/src/application/finance/reconciliation.rs`,
`sheet_parts.rs`; screen `web/src/screens/admin/reconciliation/`). It holds counts, transfers, exceptions, integrity
and certification, which is the work the manual's "บันทึกยอดรายกะ" tab did (§1.15, §2.6).

**Before a feature here is designed, check what v3 already records.** The rows below are
the ones where v3 holds the data already, so the right work is probably a screen or a report, not new logic.

| Manual step (manual §) | Why the old system needed it | What v3 already records | So |
|---|---|---|---|
| Staff type the counted cash into the Sheet's yellow cells (1.14, 1.15) | the close count was not kept in the system | the CLOSE count per currency, `counted − expected`, and a `CashException` for any difference (`xc-cash/.../count_service.rs`) | **F12 is not built.** F01 is the auditor's view in v3; any field the manual's auditor checks and F01 lacks is added to F01 |
| An auditor checks that the yellow cells are filled, the difference, and the source of each top-up (2.6) | the checks were done by eye in the Sheet | count verify by a second person, cash exceptions, transfers with their kind | a report, not a new flow |
| "เงินลงลิ้นชัก" (cash into the drawer) is written on paper (1.4, 1.6) | the old screen had a step the shop did not want | the OPEN count posts `SAFE → DRAWER`; a mid-shift top-up is a `SAFE_TO_DRAWER` or `EXTERNAL_IN` transfer | nothing to build |
| A top-up names where it came from (1.6) | a free-text field in the old system | the transfer kind already says safe or outside; only "owner" has no place of its own | **F4 shrinks:** an `EXTERNAL_IN` needs a "from the owner / from the bank" choice, and once F1 lands, an `OWNER_FUND → DRAWER` move |
| The shift summary per day and per shift (2.1) | the old dashboard read the Sheet | `staff-report` per branch-day and per shift; `admin/reports` by date range | **F6 shrinks** to the compare ranges, the chart and the sell-back figures |
| The opening float carries over from the last close when nobody records one (1.5) | there was no ledger | the drawer's balance is the ledger; the close posts `DRAWER → SAFE` and the next OPEN draws from the safe | nothing to build; this is a Blueprint rule (B) |

## 2. What to build: only what is truly missing

Owner, 2026-11-19: *"เราจะเพิ่มแค่สิ่งที่มันขาดจริง ๆ"* · *"ส่วนไหนที่ ทำทดแทนกันแล้วไม่ต้องไปทำ ซ้ำ"*.
**Rule:** where v3 already does the job another way, it is not rebuilt in the manual's shape. Each row below was
checked against the code at `f764777b2`.

### 2.1 Already replaced in v3: do not build

| Manual | What v3 does instead (evidence) |
|---|---|
| F12 Google Sheet "บันทึกยอดรายกะ", yellow cells, the auditor's checks (1.15, 2.6) | close count per currency, `counted − expected`, `CashException` on any difference, second-person verify (`xc-cash/.../count_service.rs`); F01 reconcile-and-certify per branch-day (`api/src/application/finance/reconciliation.rs`, `web/src/screens/admin/reconciliation/`) |
| "เงินลงลิ้นชัก" written on paper (1.4, 1.6) | the OPEN count posts `SAFE → DRAWER` (`ledger-inventory.md` §4.4) |
| Opening carried over from the last close (1.5) | the close posts `DRAWER → SAFE` and the next OPEN draws from the safe (Blueprint rule) |
| Expected cash by formula at close (1.14) | the drawer's ledger balance per currency (`ledger_postings.rs` `expected_in`), which equals the shop's formula once sell-back THB goes to the owner fund (F1) |
| F5 keypad screen (1.7) | the desk's 5-step form does the same buy and sell; a keypad is a look, not a missing function |
| F6's per-day and per-shift summary (2.1) | `staff-report` per branch-day and per shift; `admin/reports` by date range; `admin/executive` (the compare ranges and chart are not here: §2.3) |
| F7 backup (2.5) | server backup (`deploy/server/backup.sh` + timer); xlsx export, permission-gated (`api/src/presentation/http/v3/xlsx.rs`) |
| Top-up from the safe or from outside (1.6) | transfers `SAFE_TO_DRAWER`, `EXTERNAL_IN` (`xc-cash/src/domain/transfer/cash_transfer.rs:9-15`) |
| Rate board on a TV, 9:16 (1.12) | `web/src/screens/customer/rate-display/` (1080×1920 portrait, fullscreen) |
| "No published rate, do not trade" (1.7) | the desk refuses with `noPublishedRate` (`web/src/screens/desk/steps/rate/i18n.ts:17`) |
| Receipt `XC-YYYYMMDD-NNNNNN`, 80 mm print (1.10) | `xc-exchange/.../document_ref.rs:14-22`; the receipt step prints 80 mm |
| Staff accounts: add, temporary password, reset, suspend (2.5) | `web/src/screens/admin/users/`, `user_service.rs` |

### 2.2 The backend exists; only the desk is not wired

| Manual | v3 today | The work |
|---|---|---|
| F2 USD note type (USDL / USDS / USD1) chosen on every buy and sell (1.7, 1.8) | quotes already take `denominationGroup`, and published rates are per group (`api/src/application/workflows/exchange/quote.rs:100,236`); the desk always sends `denominationGroup: null` (`web/src/screens/desk/desk-store.ts:329`) | a required group select on the desk for a currency that has groups; no backend change |

### 2.3 Truly missing: build

| Feature | Why nothing in v3 covers it |
|---|---|
| **F1** Sell-back to another shop + owner fund | no account kind, posting or flow (`sell-back-owner-fund.md` §0) |
| **F3** USDT with a rate the teller types (25–45 THB, 4 dp, who typed it) | no USDT, and every desk rate comes from a published line |
| **F4** Top-up "from the owner" | no owner place until F1; after F1 it is an `OWNER_FUND → DRAWER` move |
| **F6** (part) Compare 7/14/30/60/90 days or a custom range (≤ 120 days, ≤ 400 back) with a bar chart | the reports filter by one range and draw no chart |
| **F8** LINE: order notify, 20:30 summary, rate sign PNG/PDF, sell-back orders | no LINE integration at all |
| **F9** Sign in with Telegram | username and password only |
| **F10** Photo at clock-in, clock-out and close | the clock takes no photo |
| **F11** Payment voucher, purchase order, tax-invoice upload, input-VAT summary | none (F01 reconciles cash and does not hold documents) |

**Order:** F1 → F2 (wiring) → F3 → F4 come first, because they are counter work done every shift and need no outside
service. The F6 compare and chart follow, since they need F1's sell-back figures. F8 and F9 come next, since each needs
a token the owner holds. F10 and F11 are last: F10 stores personal photos and needs a retention rule, and F11 touches
tax and may belong to AccountZing.

## 3. Questions for the owner (one per feature, asked when its design starts)

- **F2:** are USDL, USDS and USD1 set up as the admin's denomination groups for USD (the desk will list whatever groups exist)?
- **F3:** are the bounds 25–45 THB fixed, or a setting?
- **F8:** which LINE account and group? A LINE Messaging API channel is needed.
- **F9:** which Telegram bot? Telegram login needs a bot token and a domain.
- **F10:** how long are photos kept, and who may see them?
- **F11:** does this belong in xchange or in AccountZing? (`ledger-inventory.md` §7 already proposes a GL summary to AccountZing.)
