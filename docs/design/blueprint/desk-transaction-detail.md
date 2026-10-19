# Blueprint B04: the desk transaction detail `/desk/transactions/:id` (phases 1–2)

> designed-by: claude/5edf7bb5 (subagent of the xchange frontend lane controller) · 2026-10-11 · measured on `main` at `3c761b60`
> Source of the rules: *XC Desk — Blueprint & Data Model v2* §00, §02, §03 B04, §06, §07, §09, §10, §11, §13, §14, §19
> (line numbers are in the tag-stripped text of `docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`:
> `sed 's/<[^>]*>/\n/g' | grep -v '^\s*$'`, written below as `BP:<line>`).
> Built on: `quote-transaction.md` (#551: TX-10 #678, the B04 view; receipts §3.5), `reversal.md` (#552: RV-2 in #682,
> RV-4 #685), `org-roles-branches.md` §3.4 (permissions), `languages.md` (staff audience), `xchange-v3-wave3-staff.md`
> (S-3 #575: `can()`, `command.ts`, `ReasonDialog`), `xchange-v3-wave3-desk.md` (how `/desk` pages are served; DK-H, now part of #543).
> Umbrella: #564. The owner's standing rule: *"logic ห้ามแก้ไข — ปรับได้แค่หน้าตา"*.
> B04 is phase 1–2 (§03 B04 *"1–2"*, BP:141; §13 *"2 • ธุรกรรมและบริหาร"*, BP:661), so the owner's phase lock of 2026-10-11
> (#564 issuecomment-6101191735, phases 4–6 only) does not hold it. Its booking field is phase 4 and is held (D9).
> §9 answers a separate question about B01 `/desk/shift`.

## 0. What binds this design

| # | Source | Text (quoted) | Effect here |
|---|---|---|---|
| B1 | §03 B04 (BP:135-141) | `B04 • /desk/transactions/:id` · TELLER / MANAGER · *"รายละเอียดและใบเสร็จ"* · *"snapshot เรท/สิทธิ; ยอด; booking; audit"* · *"พิมพ์ซ้ำ; เสนอรายการ reversal"* · Transaction, Receipt, Reversal · *"1–2"* | the screen's content, actions and data |
| B2 | §03 layout (BP:268) | *"ทุกตารางมี loading / empty / error / retry"* | four states per section |
| B3 | §02 (BP:63) | *"Server ตรวจทั้ง organization_id, branch scope และ owner ของข้อมูลทุก request; UI ซ่อนปุ่มอย่างเดียวไม่เพียงพอ"* | a hidden button is convenience; the server refuses |
| B4 | §02 TELLER (BP:43-46) | *"ห้ามแก้รายการสำเร็จโดยตรง"* | the screen edits nothing; change only through a reversal request |
| B5 | §06 Transaction (BP:384-386) | *"COMPLETED … แก้ผ่าน reversal/adjustment เอกสารใหม่เท่านั้น"* | no edit control on any field |
| B6 | §06 Reversal (BP:387-389) | *"REQUESTED → APPROVED → POSTED; REJECTED"* · *"MVP ยกเลิกทั้งรายการ"* | the reversal section shows these states; whole transaction only |
| B7 | §07 Receipt (BP:464-466) | *"ใบเสร็จไม่ overwrite; reversal เชื่อมรายการบัญชีตรงข้าม"* | reprint prints the stored document; it never redraws it |
| B8 | §10 (BP:587-589, 602) | `POST /api/transactions/:id/reversal-requests` · B04 · *"เหตุผล; original; วงเงิน/approval policy; ไม่มีการลบทิ้ง"*; *"ทุก mutation ใช้ authenticated actor และ audit"* | the request action; the audit section |
| B9 | §11 (BP:607) | *"แสดง gross และ reversal แยก"* | original figures stay as they are; the reversal is its own section |
| B10 | §14 receipts (BP:706-709) | *"production receipt ออกจาก completed Transaction เท่านั้น"* | receipts listed only as the server stored them |
| B11 | §19 R-1…R-4 (BP:1003-1022; `reversal.md:31-34`) | *"ภายในวันทำการเดียวกัน"* · *"ออกเอกสารยกเลิกใหม่"* · *"FINANCE"* · *"ทางเดียวกับที่จ่ายมา"* | the window is the server's; the cancellation document is a second receipt row |
| B12 | §19 desk receipt (BP:1023-1027; desk D-2) | *"แสดงหลังบันทึกเท่านั้น"* — *"หลังบันทึกแสดงใบเสร็จที่เซิร์ฟเวอร์เก็บ"* | the screen shows only the stored receipt |
| B13 | §19 drawer agent (BP:1078-1081; #564 issuecomment-6074098583, -6074130863) | *"desk เป็นหน้าเว็บ; โปรแกรมเล็กที่ติดตั้งที่เคาน์เตอร์ทำหน้าที่เปิดลิ้นชักเท่านั้น"* | reprint goes through the browser, not the counter agent |
| B14 | §19 L-A4 (BP:1043-1047; `languages.md` L2, L3) | *"หน้าพนักงานและหลังบ้าน TH/EN เท่านั้น"* | TH/EN, no `zh` table |
| B15 | `org-roles-branches.md:146` (§3.4) | `TransactionRead`, `ReceiptReprint`, `ReversalRequest` · B04 · TELLER, BRANCH_MANAGER | buttons through `can()`; no role name in the screen |
| B16 | `quote-transaction.md:227` (§3.5) | *"A reprint prints the stored document again and writes nothing."* | reprint is a read |
| B17 | §00 (BP:20) | *"ชื่อ route และตารางในเอกสารนี้เป็นข้อเสนอ ต้องทำ mapping กับระบบเดิมก่อน migration"* | route names may be mapped; used in §9 (B01), not needed for B04 (D1) |
| B18 | owner's phase lock, 2026-10-11 (#564 issuecomment-6101191735); §13 *"4 • Booking"* (BP:665), *"5 • Marketing / member"* (BP:667) | phases 4–6 are held | the booking link (phase 4) and the benefit snapshot (phase 5, #560) are not in this design's tickets |

## 1. v3 today (measured at `3c761b60`)

| Fact | Evidence |
|---|---|
| No page serves `/desk/transactions/…`; screens are `web/pages/*.html` with `xc-path`/`xc-gate` metas, compiled by `api/build.rs` into `SCREENS` | `api/build.rs:102-125`; `api/build_support/screens.rs:39-90`; the 8 files in `web/pages/` |
| **A parameterised `xc-path` cannot work today.** `ScreenTable::configure` registers `web::resource(&path)`, so actix would accept `/desk/transactions/{id}`, but `serve` then looks the request up by **literal** path and answers the plain 404 | `api/src/presentation/screens.rs:203-213` (registration), `:124` (`d.path == req.path()`), `:72-74` (`holds`, literal too) |
| Built assets are absolute (`base: '/ui/'`), so a page two segments deep loads its assets | `web/vite.config.ts:28` |
| `branch`-gated screens take the staff session's branch, and an owner's `?branch=` | `screens.rs:138-147`; `pages.rs:626-653` (`page_branch`) |
| The permissions exist and are held by TELLER and BRANCH_MANAGER; **no route checks `ReceiptReprint`** | `api/crates/xc-auth/src/domain/access/permission.rs:16-18`; `role_permissions.rs:54-56,77-79`; `git grep -n ReceiptReprint -- api/src` is empty |
| The reversal request has a policy row (`reversalRequest` → `ReversalRequest`) | `api/crates/xc-auth/src/domain/policy/entities/policy.rs:94-96,570` |
| TX-2 #670 is merged (states, lines, payments, receipts); the B04 view is not: no `views/transaction.rs`, no `views/reversal.rs` | `api/migrations/20261111170000_transaction_states.sql`; `ls api/src/presentation/http/v3/views/` |
| TX-10 #678 (open) answers `{ transaction: TransactionDetailDto \| null }`, `VERSIONED = false`; receipts carry `{ number, revision, issuedAt }` — **no file key** | #678 §7-§8 |
| RV-4 #685 (open) adds `reversal: ReversalSummaryDto \| null` to that DTO, with a derived `EXPIRED` | #685 §7 |
| RV-2 (inside #682, open) is the command `POST /api/transactions/:id/reversal-requests`, `{ reason }` 1–500 chars, `Idempotency-Key`; errors `validationFailed`, `outOfScope`, `staleVersion`, `reversalWindowClosed`, `idempotencyConflict` | #682 Part 2 §8 |
| The approval core writes `audit_events` with `object_ref` = the subject's object id (the transaction for an over-limit approval, the reversal for a reversal) | `api/crates/xc-approval/src/application/approval_service.rs:243,322,417`; table `api/migrations/20261010110000_approvals.sql:27` |
| **No transaction command writes an audit event.** `quote-transaction.md` names audit only for the settings commands; TX-4 #672 tests audit only there; TX-5 #873's text has no "audit" | `quote-transaction.md:254`; #672 lines 152, 165 |
| The stored receipt is a file (`receipts.document_ref` → `files(key)`); `/files/{key}` serves files under a `sandbox` CSP (opaque origin, so a parent page cannot call its `print()`) and checks `FileRead`, not `ReceiptReprint` | `quote-transaction.md:214-229`; `api/src/presentation/http/files.rs:23-24` (CSP), applied at `:244`; the `FileRead` check is `policy.rs:589` (`EntryPath::File`) |
| `/devices/receipt` is a *preview* recomputed from the wide row (*"ตัวอย่างก่อนพิมพ์"*), not the stored receipt | `pages.rs:307-455`; TX-8k #1135 |
| The admin's v2-parity detail dialog lists receipt links through `transaction_documents` | `web/src/screens/admin/parts/TransactionDetail.tsx:49-71`; `views/transaction_documents.rs:62-107` |
| `staff_me` answers the actor's effective `permissions[]` | `views/staff_me.rs:33-95`; `web/src/shared/api/generated/staff_me.ts:3` |
| `can()`, `command.ts`, `ReasonDialog` are S-3's `web/src/screens/staff/parts/` (#575, PR 1229 open); not on `main` | `xchange-v3-wave3-staff.md:109,118` |
| The desk's own receipt print (DK-H) is now a part of #543 (open); its print mechanism is its open item O-2 | #544 (closed, merged into #543) |
| `ApiClient` has `view` and `command(method, path, body, key)`; `ViewPoller` polls `since`/version | `web/src/shared/api/client.ts:77,182`; `web/src/shared/live/view-poller.ts:97-107` |

## 2. Decisions

| # | Decision | Source |
|---|---|---|
| D1 | **URL `/desk/transactions/{id}`** (actix's spelling of the Blueprint's `:id`; `id` = the transaction's uuid, as in §10's `/api/transactions/:id/…`). One page `web/pages/desk-transaction.html` (`xc-path` `/desk/transactions/{id}`, `xc-gate` `branch`, as `/desk`), code in `web/src/screens/desk-transaction/**` (a staff folder). Because `serve` matches literal paths only (§1), **TD-1 (BE)** makes the screen table match the request against the declared pattern (`req.match_pattern()`), so the Blueprint's URL is kept as written. Considered and not chosen: `/desk/transactions?id=`, which BP:20 would allow with no api change; the exact URL costs one lookup and keeps B04 identical to the Blueprint, as P01 kept its own. | B1, B17 |
| D2 | **Reads, all one-off, none polled.** On open: `staff_me` (for `can()`), `transaction?id=` (TX-10 + RV-4), `transaction_audit?id=` (TD-2). Again only after the screen's own command succeeds, and on the *refresh* button. No timer. TX-10 is `VERSIONED = false`, so polling it would resend the whole body every tick, which 🔁 forbids; a status that changes elsewhere (FINANCE deciding) shows on the next open or refresh. 🔁 therefore does not apply. | fleet rule 🔁; staff S6 |
| D3 | **Content, in order:** header (number `documentRef`, status, `completedAt`; `cancelledAt` + `cancelReason` when cancelled; the approval request's status while `PENDING_APPROVAL`) · **rate snapshot and amounts** per line (currency, side, quantity, unit, `rateSnapshot`, `thbAmount`, `feeThb`, `settleThb`) · payment (method, reference, account, `amountThb`, `bankFeeThb`, `paidAt`) · receipts (number, revision, `issuedAt`) · reversal (D6) · audit (D7). Every figure is the server's string, shown unchanged (`MoneyText`, `formatRate`); the browser does no arithmetic. Customer identity and slip are not shown: B04 does not list them. | B1, B5, B9 |
| D4 | **Benefits (สิทธิ) and booking are not in phase 1–2.** The benefit snapshot arrives with #560 (phase 5) and the booking link with phase 4; TX-10's DTO carries neither. The screen adds them when those phases open, as a ticket cut then. | B1, B18 |
| D4a | **Amended 2026-10-19: both phases are in** (`quotes.benefit_snapshot` is written by the benefit engine, `marketing-campaigns.md` §4; `bookings.transaction_id` by BK-7). TX-10's `TransactionDetailDto` adds `benefits: [{ ruleId, versionId, benefitType, benefitClass, couponId, amountThb, rateBefore, rateAfter, feeBefore, feeAfter, floorCapped }]` (the `applied` list of the transaction's quote's `benefit_snapshot`, in its stored order; `[]` when there is no quote or nothing applied; money and rates as decimal strings) and `booking: { id, bookingNo } \| null` (the booking whose `transaction_id` is this transaction). Read only: nothing is recomputed. Files `v3/views/transaction.rs`, `dto/v3/transaction_detail.rs`, `generated/transaction.ts` | B1, B18 |
| D5 | **Reprint (พิมพ์ซ้ำ)** is a read: per receipt row, a button shown when `can(me, 'ReceiptReprint')`. It reads **TD-2's `receipt_document?number=`**, which is checked by `ReceiptReprint`, returns the stored document's HTML unchanged, and writes nothing. The screen puts that HTML into a hidden `<iframe sandbox="allow-same-origin allow-modals" srcdoc=…>` (no `allow-scripts`: nothing in the document runs) and calls the frame's `print()`. The browser prints; the counter agent is drawer-only. Revision 1 is the receipt; revision ≥ 2 is the cancellation document (`receipts` CHECK `(kind = 'RECEIPT') = (revision = 1)`, `reversal.md` §3.2), labelled from the revision, so TD-2 does not wait for RV-1's `kind` column. The frame is **one shared component**, `web/src/shared/receipt/ReceiptFrame.tsx` (♻️); the desk's receipt step (DK-H in #543) should print through it too — the lane controller says so on #543. | B1, B7, B12, B13, B15, B16 |
| D6 | **Reversal.** Section shown when `reversal` is not null: status (`REQUESTED`, `APPROVED`, `POSTED`, `REJECTED`, `EXPIRED`), reason, amount, requested by/at, approved at, posted at, document number. The original's figures above never change. The **request** button (*"เสนอรายการ reversal"*) is shown when `can(me, 'ReversalRequest')`, `status = COMPLETED`, and `reversal` is null or `REJECTED` (V-5). It opens S-3's `ReasonDialog` (1–500 characters) and sends `POST /api/transactions/{id}/reversal-requests` `{ reason }` through S-3's `command.ts`, which keeps one `Idempotency-Key` until success and resends the same key and body on retry. Refusals are shown as `t('error.' + code)`. The window (R-1) is the server's to judge: the screen does not compute business dates. **Posting** an APPROVED reversal (`POST /api/reversals/:id/post`, RV-3) is **not** done on B04 (owner, Q-1, 2026-10-11: *"ขั้นในหน้าเดสก์ B03"*). It is a step on the desk B03 (`xchange-v3-wave3-desk.md`, designed separately). B04 shows an `APPROVED` reversal's status with the line *"บันทึกคืนเงินที่หน้าเดสก์"* and no post button. Recorded in §19 (B04 row). | B1, B6, B8, B11, B15; `reversal.md:56-58,133-145` |
| D7 | **Audit** = TD-2's `transaction_audit?id=`: the `audit_events` rows whose `object_ref` is the transaction or one of its reversals, oldest first: `at`, `actorId`, `actorRole`, `action`. The redacted before/after payloads are not sent. Today this shows the approval and reversal events only (§1: no transaction command writes audit). The transaction events arrive with **TX-11** (BE lane: `quote-transaction.md`, AuditLog in the same DB transaction, after #672 and #873), and the screen shows them with no change of its own. | B1, B8 |
| D8 | **Permissions, not roles.** Buttons read `can(me, …)` from `staff_me.permissions` (S-3's `parts/can.ts`, imported, not copied); no role name appears in `web/src/screens/desk-transaction/**`. If `staff_me` fails, every action is hidden and the data still shows. The server refuses anyway (B3). | B3, B15 |
| D9 | **Not here:** FINANCE's approve/reject (A06's generic decide, `/admin/approvals`); the receipt's language (`languages.md` L10); links into B04 from other screens (desk, admin) — those are their own tickets' look. | B1 |
| D10 | **Language TH / EN**, `i18n.ts` exports `th` and `en` only; a stored `xc.lang = 'zh'` shows Thai. | B14 |
| D11 | **States** (B2): loading · `transaction: null` → "not found" (unknown, malformed or other branch's id, TX-10 §8) · a view error → error + retry · audit empty → "no audit events" · reprint `receipt: null` → "document not found", the button stays. | B2 |

## 3. Class diagram

```mermaid
classDiagram
  class DeskTransactionPage { <<web/pages/desk-transaction.html>> xc-path=/desk/transactions/{id}; xc-gate=branch }
  class Entry { <<desk-transaction/main.tsx>> render(TransactionScreen) }
  class TransactionScreen { <<desk-transaction/TransactionScreen.tsx>> load(); refresh(); states }
  class TransactionRoute { <<desk-transaction/route.ts>> idFromPath(pathname) string or null; branchFromSearch(search) string or undefined }
  class HeaderSection { <<desk-transaction/HeaderSection.tsx>> number; status; times; approval }
  class LinesSection { <<desk-transaction/LinesSection.tsx>> rate snapshot and amounts per line }
  class PaymentSection { <<desk-transaction/PaymentSection.tsx>> }
  class ReceiptsSection { <<desk-transaction/ReceiptsSection.tsx>> rows; reprint(number) }
  class ReversalSection { <<desk-transaction/ReversalSection.tsx>> summary; request button }
  class AuditSection { <<desk-transaction/AuditSection.tsx>> events }
  class Texts { <<desk-transaction/i18n.ts>> th; en }
  class ReceiptFrame { <<shared/receipt/ReceiptFrame.tsx  TD-3>> print(html) Promise~void~ }
  class can { <<screens/staff/parts/can.ts  S-3 #575>> can(me, permission) bool }
  class command { <<screens/staff/parts/command.ts  S-3 #575>> }
  class ReasonDialog { <<screens/staff/parts/ReasonDialog.tsx  S-3 #575>> }
  class TransactionDetailDto { <<shared/api/generated/transaction.ts  TX-10 #678 + RV-4 #685>> }
  class ReceiptDocumentData { <<shared/api/generated/receipt_document.ts  TD-2>> }
  class TransactionAuditData { <<shared/api/generated/transaction_audit.ts  TD-2>> }
  class transaction_view { <<api views/transaction.rs  TX-10 #678>> }
  class reversal_requests { <<api commands/exchange/reversals.rs  RV-2 #682>> }
  class receipt_document_view { <<api views/receipt_document.rs  TD-2>> ReceiptReprint; writes nothing }
  class transaction_audit_view { <<api views/transaction_audit.rs  TD-2>> readers = transaction_view's }
  class ScreenTable_serve { <<api presentation/screens.rs  TD-1>> match_pattern lookup }

  DeskTransactionPage --> Entry
  Entry --> TransactionScreen
  TransactionScreen --> TransactionRoute
  TransactionScreen --> HeaderSection
  TransactionScreen --> LinesSection
  TransactionScreen --> PaymentSection
  TransactionScreen --> ReceiptsSection
  TransactionScreen --> ReversalSection
  TransactionScreen --> AuditSection
  TransactionScreen --> Texts
  TransactionScreen --> can
  ReceiptsSection --> ReceiptFrame
  ReceiptsSection --> receipt_document_view : GET on click
  ReversalSection --> ReasonDialog
  ReversalSection --> command
  command --> reversal_requests : POST + Idempotency-Key
  TransactionScreen --> transaction_view : GET on open
  AuditSection --> transaction_audit_view : GET on open
  ScreenTable_serve ..> DeskTransactionPage : serves
```

Plus `desk-transaction/desk-transaction.css` (every static class has a rule: `web/src/shared/ui/screen-look.test.ts`).

| Provides | Used by | State |
|---|---|---|
| `TransactionDetailDto` + `transaction` view | `TransactionScreen` and its sections | TX-10 #678, RV-4 #685 (open) |
| `POST /api/transactions/:id/reversal-requests` | `ReversalSection` via `command.ts` | RV-2 in #682 (open) |
| `can`, `command.ts`, `ReasonDialog` | `TransactionScreen`, `ReversalSection` | S-3 #575 (PR 1229, open) |
| pattern paths in `ScreenTable` | the page | TD-1 |
| `receipt_document`, `transaction_audit` views + the `receiptReprint` policy row | `ReceiptsSection`, `AuditSection` | TD-2 |
| `ReceiptFrame` | `ReceiptsSection`; later the desk's receipt step (#543) | TD-3 |

## 4. Behaviour (the contract the up-front tests pin)

### 4.1 Pattern paths in the screen table (TD-1)

| Case | Result |
|---|---|
| a built page declaring `xc-path` `/desk/transactions/{id}`, `GET /desk/transactions/7f0c…` with a staff session | `200`, the page, `Cache-Control: no-cache`, as every screen |
| the same, signed out | the same redirect as every `branch`-gated screen (`pages::to_login`) |
| owner session, no `?branch=` | `400 Invalid branch`, as every `branch`-gated screen |
| `/desk/transactions`, `/desk/transactions/`, `/desk/transactions/a/b` | the plain 404 |
| `POST /desk/transactions/7f0c…` | the plain 404 |
| every literal screen and every `PAGE_SPECS` path | unchanged; every existing test green and unedited |
| `holds("/desk")` and other literal lookups | unchanged |

### 4.2 `receipt_document?number=` (TD-2)

| Case | Result |
|---|---|
| a receipt number of the caller's branch; caller holds `ReceiptReprint` | `{ receipt: { number, revision, issuedAt, html } }`, `html` = the stored file's bytes as UTF-8, byte-identical |
| revision 2 (a cancellation document) | its own stored HTML, `revision: 2` |
| unknown, malformed, or another branch's number | `{ receipt: null }` |
| caller without `ReceiptReprint` (e.g. RATE_MANAGER, FINANCE) | the seam's policy refusal; no body |
| not signed in | `401 signIn` |
| two calls | the row count of every table, `audit_events` included, is unchanged, and the `receipts` rows are byte-equal before and after (an UPDATE is caught, not only an INSERT) |

### 4.3 `transaction_audit?id=` (TD-2)

| Case | Result |
|---|---|
| a transaction with an over-limit approval requested and decided | two events, oldest first, `object_ref` = the transaction id; fields `at`, `actorId`, `actorRole`, `action` only |
| a transaction with a reversal requested and decided | the reversal's events are included |
| another transaction's events | never included |
| no events | `{ events: [] }` |
| unknown, malformed, or another branch's id | `{ events: null }` |
| readers | exactly the readers of TX-10's `transaction` view (same seam row); a test pins both answer the same callers |

### 4.4 `ReceiptFrame` (TD-3)

| Case | Result |
|---|---|
| `print(html)` | one hidden `iframe`, `sandbox` exactly `allow-same-origin allow-modals`, `srcdoc` byte-identical to `html`; its `contentWindow.print()` called once after load; the frame removed afterwards |
| `html` containing `<script>` | not run (no `allow-scripts`) |
| two prints in a row | two frames, one at a time; no leftover frame |

### 4.5 The screen (TD-3)

| Case | Result |
|---|---|
| `/desk/transactions/<id>` | exactly three GETs on open: `staff_me`, `transaction` `{id}`, `transaction_audit` `{id}`; `?branch=` forwarded when present |
| 10 minutes pass with no click (fake timers) | no further request |
| refresh button | the same three GETs once more |
| `transaction: null` | the not-found state; no action button |
| a view answers an error | error + retry; retry repeats the read |
| `COMPLETED`, lines USD BUY `quantity "1000.0000"`, `rateSnapshot "35.12000000"`, `thbAmount "35120.00"`, `feeThb "0.00"`, `settleThb "35120.00"` | each value shown with its digits unchanged; no `Number(`, `parseFloat`, `toFixed`, `Math.` in `web/src/screens/desk-transaction/**` (grep test) |
| `DRAFT` / `CONFIRMED` | status shown; no receipts, no reprint, no request button |
| `PENDING_APPROVAL` | the approval request's status shown; no request button |
| `CANCELLED` | `cancelledAt` and `cancelReason` shown; no receipts |
| `can ReceiptReprint`, receipts rev 1 | a reprint button per row; click → one GET `receipt_document {number}` → `ReceiptFrame.print(html)` with that `html`; no non-GET request |
| without `ReceiptReprint` | no reprint button |
| `receipt: null` on reprint | "document not found"; the button stays |
| `can ReversalRequest`, `COMPLETED`, `reversal: null` | the request button; click → `ReasonDialog`; confirm → `POST /api/transactions/<id>/reversal-requests`, body `{"reason":"…"}`, an `Idempotency-Key` |
| that POST fails on the network, then retry | the same key and a byte-equal body |
| the POST answers `reversalWindowClosed` / `staleVersion` / `outOfScope` | `t('error.<code>')` shown; nothing re-read |
| the POST succeeds | `transaction` and `transaction_audit` read once each |
| `reversal.status` `REQUESTED`, `APPROVED`, `POSTED`, `EXPIRED` | no request button; the section shows the status, reason, amount, times and (POSTED) document number |
| `reversal.status` `REJECTED` | the section shows it rejected; the request button is shown again |
| `POSTED` | the original's line and payment figures are the same strings as before the reversal; receipts list revision 1 and 2 |
| `reversal.status` `APPROVED` | the status and the line *บันทึกคืนเงินที่หน้าเดสก์*; no post button in any case (the post is a desk B03 step) |
| `staff_me` fails | data shown; no action button |
| any text | through `t()`; `i18n.ts` exports `th` and `en`, no `zh` (`audience.test.tsx` guard); no role name (`TELLER`, `BRANCH_MANAGER`, `FINANCE`, `FRONT`, `BACKEND`, `MANAGER`) in the folder (grep test) |

## 5. Tickets

Each owns disjoint files. Module registration lines (`mod.rs`) are not reserved (lane rule, #564). Up-front tests are written by the ticket author on the ticket's branch from `origin/main` (🧪) before `ready`.

| # | Lane | Ticket | `## File scope` | Starts after |
|---|---|---|---|---|
| TD-1 | api (BE) | pattern paths in the screen table (§4.1): `serve` finds the declaration by the matched resource pattern; literal lookups unchanged | `api/src/presentation/screens.rs` · `api/tests/screen_pattern_path_test.rs` | this design |
| TD-2 | api (BE) | `receipt_document` and `transaction_audit` views (§4.2, §4.3); the `receiptReprint` → `ReceiptReprint` policy row | `api/src/presentation/http/v3/views/receipt_document.rs` · `api/src/presentation/http/v3/views/transaction_audit.rs` · `api/src/presentation/dto/v3/receipt_document.rs` · `api/src/presentation/dto/v3/transaction_audit.rs` · `web/src/shared/api/generated/receipt_document.ts` · `web/src/shared/api/generated/transaction_audit.ts` · `api/crates/xc-auth/src/domain/policy/entities/policy.rs` · `api/crates/xc-auth/tests/area_rows_seam_test.rs` · `api/tests/v3_b04_reads_test.rs` | TX-10 #678 and #682 (RV-1 + RV-2) merged. TD-2's audit view takes TX-10's reader row, and its reversal events need RV-1's `reversals` table and RV-2's FINANCE decision |
| TD-3 | web (FE) | the B04 screen (§4.4, §4.5; D1–D11) and the shared `ReceiptFrame` | `web/pages/desk-transaction.html` · `web/src/shared/receipt/` (new folder, reserves its subtree on purpose) · `web/src/screens/desk-transaction/` (new folder, reserves its subtree on purpose) | TD-1, TD-2, TX-10 #678, RV-4 #685, RV-2 (#682), S-3 #575 merged |

| Ticket | Up-front tests (red until implemented) |
|---|---|
| TD-1 | `api/tests/pages_test.rs:527-550` (`an_address_that_is_almost_a_page_is_not_one`) stays green and unedited, as the guard that literal pages still reject near-misses; every row of §4.1, with a test-built `ScreenTable` holding one pattern page and one literal page (`ScreenTable::from_dist`, as `screen_table_release_test.rs:23` builds one) |
| TD-2 | every row of §4.2 and §4.3 against rows seeded through TX-2's store, a stored receipt file, and `audit_events` written through `ApprovalService`; the seam row in `area_rows_seam_test.rs` |
| TD-3 | every row of §4.4 and §4.5 with mocked `fetch` and fake timers; `npm run check` in `web/` green; a browser check on the test server with screenshots in the PR |

Done criteria shared by every ticket: no existing test edited or deleted; TD-1/TD-2 `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run` green; TD-2/TD-3 `cd web && npm run check` green.

**Already covered elsewhere, not cut again:** the B04 view (TX-10 #678), its reversal field (RV-4 #685), the request command (RV-2 in #682), the post command (RV-3 #684), the receipt preview (TX-8k #1135), `can`/`command`/`ReasonDialog` (S-3 #575).
Searched `gh issue list --state all` for `B04`, `reversal`, `transaction detail`, `reprint`, `desk/transactions`, `desk/shift` (2026-10-11): no screen ticket for B04 exists.

**Note for the lane controller (not an owner question):** §10 asks *"ทุก mutation ใช้ authenticated actor และ audit"* (BP:602), but the transaction design names audit only for settings (`quote-transaction.md:254`), and TX-4 #672 / TX-5 #873 pin no audit row for create, confirm, cancel or complete. This is now designed as **TX-11 #1262** (PR 1261, merged `9f382ea4`): create, confirm, cancel and complete each write one `AuditEvent` in the same transaction. Until it lands, D7's audit shows approval and reversal events only. **Settled:** the BE lane controller rejected TX-10 #678's `[proposed]` reader row (`adminSnapshot` → `CompatRead`), and the view stays on `TransactionRead` (TELLER, BRANCH_MANAGER; #678 issuecomment-6104407930).

## 6. Blueprint conformance (logic is fixed; only the look may change)

| Blueprint item | This design |
|---|---|
| §03 B04 route `/desk/transactions/:id` | D1 (kept as written; TD-1) |
| §03 B04 users TELLER / MANAGER | B15: `TransactionRead`, `ReceiptReprint`, `ReversalRequest` through `can()` and the server |
| §03 B04 content: rate snapshot | D3 (per line); benefits D4 (phase 5) |
| §03 B04 content: amounts | D3 |
| §03 B04 content: booking | D4 (phase 4, held) |
| §03 B04 content: audit | D7, TD-2 |
| §03 B04 action: reprint | D5, TD-2, TD-3 |
| §03 B04 action: propose a reversal | D6 (RV-2) |
| §03 B04 data: Transaction, Receipt, Reversal | TX-10, RV-4, TD-2 |
| §03 layout: loading / empty / error / retry | D11 |
| §02 server checks scope; hiding a button is not enough | D8, B3 |
| §02 TELLER *"ห้ามแก้รายการสำเร็จโดยตรง"*; §06 change only by a new document | B4, B5: no edit control |
| §06 reversal states, whole transaction | D6 |
| §07 receipts never overwritten | D5: the stored document, printed as is |
| §11 gross and reversal separately | D6, §4.5 `POSTED` row |
| §14 production receipt only from a completed transaction | D3, §4.5 `DRAFT`/`CANCELLED` rows |
| §19 R-1…R-4 | D6: the server judges the window; the cancellation document is revision ≥ 2 |
| §19 drawer agent opens the drawer only | D5: the browser prints |
| §19 L-A4 staff screens TH/EN | D10 |
| fleet rule 🔁 | D2: one-off reads, no poll |

## 7. Data migration

None. No table, column, constraint or index changes. TD-1 changes request routing only. TD-2 reads `receipts`, `files`, `reversals` and `audit_events` as they are (TX-2 #670, AP-1, RV-1 #682), and its policy row is code. TD-3 is web only. Nothing to backfill; no deploy order beyond the tickets' merge order; nothing for `down` to lose.

## 8. Owner answers (2026-10-11, asked by claude/5edf7bb5)

| # | Question | Answer | Where it lands |
|---|---|---|---|
| Q-1 | Where does the teller post an approved reversal? (A B04 button · B a desk B03 step · C a new screen) | **B**, *"ขั้นในหน้าเดสก์ B03"* (the owner first chose A, then changed it to B the same day) | D6, §4.5 row, Blueprint §19 B04 row; the desk step is designed in `xchange-v3-wave3-desk.md` |
| Q-2 | B01's URL: `/desk/shift` or `/staff`? (§9) | **A**, *"ใช้ /staff ต่อ"* | §9, Blueprint §19 B01 row |

## 9. B01 `/desk/shift` against `/staff` (separate question)

**What was checked.**
- `xchange-v3-wave3-staff.md` §12 maps B01 onto `/staff`'s shift cards (`:258`). S1 keeps the path `/staff` from the first version (`:27,43`); that version's S1 gave a reason about v2's login card, not about B01 (`git show ec83d41:docs/design/xchange-v3-wave3-staff.md`, line 41).
- `cash-shift-transfer.md` names B01 `/desk/shift` as a source (`:4-5`) and designs no screen path.
- `xc-desk-blueprint-v2-mapping.md:61` records B01 as *exists*, spread over the admin cash menu and `/staff`.
- Blueprint §19 owner rulings (BP:883-1158): the only B01 row is the count by banknote (#617 issuecomment-6081125742, BP:1083-1087); no row places B01's URL.
- #564 and its 21 comments: none places B01 (the one match, issuecomment-6101191735, is the phase lock).
- `git log -S'/desk/shift' origin/main -- docs`: five commits, all designs or the mapping; none records an owner answer.

**Finding before asking:** not decided by the owner; the staff design had mapped B01 to `/staff` on its own. **Answered 2026-10-11: Q-2 A** (*"ใช้ /staff ต่อ"*), recorded in Blueprint §19. The nearest written basis is BP:20: *"ชื่อ route และตารางในเอกสารนี้เป็นข้อเสนอ ต้องทำ mapping กับระบบเดิมก่อน migration"*.

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q-2 | **B01's URL.** The Blueprint says `/desk/shift`; the staff design serves B01's content (counts by banknote, hand-over, close and certify: S9, B6–B7) as cards on `/staff`. | **A.** Keep `/staff` and record a §19 row: route names are proposals mapped to the existing system (BP:20). No work. · **B.** Add `/desk/shift` as a page holding only the shift card (S-5 #578's component, imported), and keep `/staff` as is. One small FE ticket after S-5; two entries to one card. · **C.** Move the shift card out of `/staff` to `/desk/shift`. Re-cuts S-5 and the staff layout. | **A**: every B01 content item, action and permission is there, BP:20 allows the mapping, and B or C adds a second entry or a re-cut without changing what a teller can do. Until answered, nothing changes; S-5 continues as designed. |
