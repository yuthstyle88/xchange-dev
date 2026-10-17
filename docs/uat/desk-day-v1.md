# UAT Script v1 — A Desk Day on Staging (Release Gate G5)

## Purpose

This script verifies release gate **G5** (`docs/production-readiness.md` §2 G5): a human operator can run one complete branch desk day on v3 from start to finish—signing in, opening a shift with an opening count, quoting, executing both a BUY and a SELL exchange, printing/previewing receipts, reversing a mistaken exchange, handing over cash to a second staff member, closing the shift with a closing count, reading the daily close shift report, and verifying that the shift report totals and the executive overview agree with the branch ledger.

Run the steps below in order on staging. At every step, compare the screen against **Expected screen result** and fill in `Result: pass / fail / note`.

---

## Before you start

- **Staging base URL**: `https://<staging-host>` (replace `<staging-host>` with the staging domain from the deployment runbook).
- **Credentials & secrets**: **Do not paste passwords, tokens, or real customer PII into this document or into GitHub comments.** Use the test accounts stored in the owner's staging password vault / secret manager (see the owner's staging credentials entry referenced by `docs/production-readiness.md`).
- **Required test accounts (3 distinct users)**:
  1. **Cashier A** — branch staff account at the test branch with shift, desk, and reversal-request permissions (`ShiftOperate`, `ReversalRequest`, `ReceiptReprint`).
  2. **Cashier B (Second staff member)** — a second branch staff / supervisor account at the **same branch** with `ShiftOperate` and `ShiftCloseCertify` (required to receive the cash handover and to verify opening/closing counts, since `ShiftCard` prevents a user from verifying their own count or handing over to themselves).
  3. **Executive / Manager / Finance** — back-office account scoped to the organisation/branch with `OverviewRead`, `ApprovalsDecide`, `BranchDayOperate`, and `FinanceReconcile`.
- **Target branch**: Use one staging branch (e.g. `XC-B01`) with a clean business day (no open shift and no prior transactions on today's `Asia/Bangkok` date).
- **Pre-published currencies, denominations, and rates**:
  1. In `web/pages/admin.html` (`web/src/screens/admin/denomination-groups/`), ensure denominations exist for `THB` (e.g. `1000`, `500`, `100`) and `USD` (e.g. `100`, `50`).
  2. In `web/pages/admin.html` → menu **`เรทและสกุลเงิน` / `Rates & Currencies`** (`#rates`, `web/src/screens/admin/rates/`), publish a live rate version for the test branch before starting:
     - **`USD`** (unit size `1`): Shop Buy (`ร้านรับซื้อ`) = **`33.00`**, Shop Sell (`ร้านขาย`) = **`33.50`**
     - **`JPY`** (unit size `100`): Shop Buy (`ร้านรับซื้อ`) = **`22.00`**, Shop Sell (`ร้านขาย`) = **`22.50`**
  3. On `web/pages/staff.html` (`web/src/screens/staff/floor/`), ensure **Cashier A** and **Cashier B** are clocked in (`ลงเวลาเข้า` / `Clock In`) and the branch day phase (`data-testid="floor-phase"`) is **`OPEN`** (if `READY`, click `เริ่มเปิดบริการ` / `Start Service` and confirm).

---

## The day

### Step 1 — Sign in (`login.html` → `staff.html`) <!-- web/pages/login.html, web/src/screens/login/, web/pages/staff.html, web/src/screens/staff/ -->

- **Page / Menu**: `/login` (`web/pages/login.html`, `web/src/screens/login/`) → redirects to `/staff` (`web/pages/staff.html`, `web/src/screens/staff/`).
- **Action**:
  1. Open `https://<staging-host>/login` in Browser Window 1.
  2. Enter **Cashier A**'s username in `identifier` and password in `password`, then click the submit button (`เข้าสู่ระบบ` / `Sign in`).
- **Expected screen result**:
  - The login form submits without showing an alert in `#message` (`role="alert"`).
  - The browser navigates to `/staff` (`web/pages/staff.html`, `web/src/screens/staff/StaffPage.tsx`) inside `AppShell`.
  - The header shows the branch operating hours (`data-testid="staff-hours"`), the refresh button (`รีเฟรช` / `Refresh`, `data-testid="staff-refresh"`), and the cards for Floor (`data-card="floor"`, `web/src/screens/staff/floor/`) and My Shift (`กะของฉัน` / `My Shift`, `data-card="shift"`, `web/src/screens/staff/shift/`).
- `Result: pass / fail / note`:

---

### Step 2 — Open shift (opening count) (`staff.html`) <!-- web/pages/staff.html, web/src/screens/staff/shift/ -->

- **Page / Menu**: `/staff` → card **`กะของฉัน` / `My Shift`** (`web/pages/staff.html`, `web/src/screens/staff/shift/ShiftCard.tsx`).
- **Action**:
  1. As **Cashier A**, in the **`กะของฉัน` / `My Shift`** card (`web/src/screens/staff/shift/`), click **`นับเปิดกะ` / `Open Count`** (`data-testid="shift-open-count"`).
  2. In the opening count form (`data-testid="shift-count-form"`), enter denomination quantities totaling:
     - **`THB`**: **`500,000.00`** (e.g. quantity `500` on the `THB 1000` denomination row `input[data-denomination-id]`)
     - **`USD`**: **`2,000.00`** (e.g. quantity `20` on the `USD 100` denomination row `input[data-denomination-id]`)
  3. Click **`บันทึก` / `Save`** (`data-testid="shift-count-save"`).
  4. *(Second person)* In Browser Window 2 signed in as **Cashier B** on `/staff` (`web/pages/staff.html`, `web/src/screens/staff/shift/`), locate the **`รายการนับที่รอยืนยัน` / `Counts to Verify`** section, find **Cashier A**'s `OPEN` count item (`data-count-id`), and click **`ยืนยัน` / `Verify`** (`data-testid="shift-verify"`).
- **Expected screen result**:
  - After **Cashier A** saves: `ShiftCard` displays the notice **`บันทึกแล้ว` / `Saved`** (`data-testid="shift-notice"`), closes the count form, and switches the shift action buttons from **`นับเปิดกะ` / `Open Count`** to **`นับปิดกะ` / `Close Count`** (`data-testid="shift-close-count"`) and **`ส่งมอบเงิน` / `Hand Over`** (`data-testid="shift-handover"`).
  - After **Cashier B** clicks **`ยืนยัน` / `Verify`**: `ShiftCard` shows **`บันทึกแล้ว` / `Saved`** (`data-testid="shift-notice"`) and removes the `OPEN` count from **`รายการนับที่รอยืนยัน` / `Counts to Verify`**.
  - In `web/pages/admin.html` → menu **`เงินสดและกะ` / `Cash & Shifts`** (`#cash`, `web/src/screens/admin/cash/`), the status banner shows **`กะกำลังเปิด` / `Shift open`**, `เงินสดตั้งต้น` (`Opening Cash`) = **`500,000.00`**, and the stock table shows Opening Balance (`ยกมาตอนเปิดกะ`) `THB = 500,000.00` and `USD = 2,000.00`.
- `Result: pass / fail / note`:

---

### Step 3 — Quote (`desk` Step 01 Rate) <!-- web/src/screens/desk/, web/src/screens/desk/steps/rate/ -->

- **Page / Menu**: `/staff/desk` or `/desk` → Step **`01 เลือกเรท` / `01 Rate`** (`web/src/screens/desk/DeskPage.tsx`, `web/src/screens/desk/steps/rate/step.tsx`).
- **Action**:
  1. As **Cashier A**, open `/staff/desk` (or `/desk`).
  2. On Step **`01 เลือกเรท` / `01 Rate`** (`เลือกเรทและจำนวนเงิน` / `Choose the rate and amount`):
     - Select `ประเภทรายการ` (`Transaction type`, `select[name="type"]`) = **`ร้านรับซื้อ (BUY)` / `Shop buys (BUY)`** (`BUY`).
     - Select `สกุลเงิน` (`Currency`, `select[name="currency"]`) = **`USD`**.
     - Type `1000` into `จำนวนเงิน` (`Amount`, `input[name="amount"]`).
- **Expected screen result**:
  - No `ยังไม่มีกะเปิด` (`No shift is open`) warning banner is shown.
  - The counter rate line (`.desk-rate-line`) updates from the server quote to show **`33.00 THB ต่อ 1 USD`** (`33.00 THB per 1 USD`).
  - The right-hand summary panel (`aside.desk-aside`, `web/src/screens/desk/Summary.tsx`) shows the live quoted settlement amount of **`33,000.00 THB`**.
- `Result: pass / fail / note`:

---

### Step 4 — Exchange: BUY and SELL (at least one each) (`desk` Steps 01–04) <!-- web/src/screens/desk/, web/src/screens/desk/steps/customer/, web/src/screens/desk/steps/booking/, web/src/screens/desk/steps/payment/ -->

- **Page / Menu**: `/staff/desk` or `/desk` → Steps **`01 เลือกเรท` (`Rate`) → `02 ลูกค้า` (`Customer`) → `03 การจอง` (`Booking`) → `04 ชำระเงิน` (`Payment`)** (`web/src/screens/desk/steps/rate/`, `web/src/screens/desk/steps/customer/`, `web/src/screens/desk/steps/booking/`, `web/src/screens/desk/steps/payment/`).
- **Action**:
  1. **Complete Exchange 1 — BUY (`1,000.00 USD` @ `33.00` = `33,000.00 THB`)**:
     - From the quote in Step 3 (`BUY`, `USD`, `1000`), click **`ถัดไป · พาสปอร์ต / ลูกค้า →` / `Next · Passport / customer →`**.
     - On Step 02 (`web/src/screens/desk/steps/customer/step.tsx`), attach a valid `.jpg`/`.png` passport image (`input[name="passport"]`), fill `name` (`JOHN SMITH`), `passportNo` (`AA1234567`), `nationality` (`USA`), check `identityChecked`, and click submit (or click **`ข้ามเอกสาร` / `Skip`** for walk-in placeholder).
     - On Step 03 (`web/src/screens/desk/steps/booking/step.tsx`), verify read-only `name` and `staff` are populated and click submit to advance to Step 04.
     - On Step 04 (`web/src/screens/desk/steps/payment/step.tsx`), select `method` = **`Cash`**, verify `account` is filled, check the confirmation checkbox (`input[name="paymentChecked"]`), click submit, and accept the browser cash-count confirmation dialog (`window.confirm`).
  2. **Complete Exchange 2 — SELL (`500.00 USD` @ `33.50` = `16,750.00 THB`, kept)**:
     - On Step 05 (`web/src/screens/desk/steps/receipt/step.tsx`), click **`ทำรายการใหม่` / `New transaction`** to return to Step 01.
     - Select `type` = **`ร้านขาย (SELL)` / `Shop sells (SELL)`** (`SELL`), `currency` = **`USD`**, `amount` = **`500`** (quoted rate shows **`33.50 THB ต่อ 1 USD`**, net **`16,750.00 THB`**), then step through Customer → Booking → Payment (`Cash`) and submit.
  3. **Complete Exchange 3 — SELL (`100.00 USD` @ `33.50` = `3,350.00 THB`, to be reversed in Step 6)**:
     - Click **`ทำรายการใหม่` / `New transaction`**, select `type` = **`SELL`**, `currency` = **`USD`**, `amount` = **`100`** (quoted rate **`33.50 THB ต่อ 1 USD`**, net **`3,350.00 THB`**), step through Customer → Booking → Payment (`Cash`), and submit. Record Exchange 3's transaction ID / document number (`เลขที่เอกสาร`).
- **Expected screen result**:
  - For each of the three exchanges, Step 04 displays the exact net THB settlement (`33,000.00`, `16,750.00`, and `3,350.00`) and enables the submit button once `paymentChecked` is ticked.
  - After submitting each exchange, the screen advances to Step **`05 ใบเสร็จ` / `05 Receipt`** (`web/src/screens/desk/steps/receipt/step.tsx`) and transitions from `กำลังบันทึกรายการและไฟล์แนบ…` (`Saving the transaction and its attachments…`) to **`บันทึกเอกสารแล้ว` / `Documents saved`** with a generated **`เลขที่เอกสาร` / `Document number`** (e.g. `EXYYYYMMDD....`).
- `Result: pass / fail / note`:

---

### Step 5 — Receipt (print or preview) (`desk` Step 05 Receipt, `desk-transaction.html`, `receipt-sample.html`) <!-- web/src/screens/desk/steps/receipt/, web/pages/desk-transaction.html, web/src/screens/desk-transaction/, web/pages/receipt-sample.html, web/src/screens/receipt-sample/ -->

- **Page / Menu**:
  - `/staff/desk` or `/desk` → Step **`05 ใบเสร็จ` / `05 Receipt`** (`web/src/screens/desk/steps/receipt/step.tsx`),
  - `/desk/transactions/{id}` (`web/pages/desk-transaction.html`, `web/src/screens/desk-transaction/ReceiptsSection.tsx`), and
  - `/receipt-sample` (`web/pages/receipt-sample.html`, `web/src/screens/receipt-sample/ReceiptSample.tsx`).
- **Action**:
  1. On Step 05 (`web/src/screens/desk/steps/receipt/step.tsx`) after saving Exchange 3, click **`พิมพ์ใบเสร็จ NEOCAL 80 มม.` / `Print receipt (NEOCAL 80 mm)`** to open the browser print preview, then close the print dialog.
  2. Open `/desk/transactions/{id}` (`web/pages/desk-transaction.html`, `web/src/screens/desk-transaction/TransactionScreen.tsx`) for Exchange 3 (either directly by URL or via `เปิดรายการในหน้าแอดมิน` → `#transactions`, `web/src/screens/admin/transactions/`).
  3. In the **`เอกสาร` / `Documents`** section (`data-section="receipts"`, `web/src/screens/desk-transaction/ReceiptsSection.tsx`), click **`พิมพ์ซ้ำ` / `Reprint`** (`button[data-action="reprint"]`) on Revision `1 · ใบเสร็จ` (`1 · Receipt`).
  4. Optionally open `/receipt-sample` (`web/pages/receipt-sample.html`, `web/src/screens/receipt-sample/`) to compare the 80 × 90 mm NEOCAL thermal layout.
- **Expected screen result**:
  - On `/desk/transactions/{id}` (`web/pages/desk-transaction.html`), the **`เอกสาร` / `Documents`** table lists Revision **`1 · ใบเสร็จ` (`1 · Receipt`)** with its document number (`เลขที่เอกสาร`) and timestamp (`ออกเมื่อ` / `Issued at`).
  - Clicking **`พิมพ์ซ้ำ` / `Reprint`** loads the stored receipt HTML via `receipt_document` into `ReceiptFrame` and triggers the print preview showing the branch details, document number, side (`SELL`), currency (`USD`), amount (`100.00`), rate (`33.50`), and settlement (`3,350.00 THB`) without any error banner.
- `Result: pass / fail / note`:

---

### Step 6 — Reversal of one exchange (`desk-transaction.html` → `admin.html#approvals` → `desk` Pending Refunds) <!-- web/pages/desk-transaction.html, web/src/screens/desk-transaction/, web/pages/admin.html, web/src/screens/admin/approvals/, web/src/screens/desk/steps/refunds/, web/src/screens/admin/cash/ -->

- **Page / Menu**:
  1. `/desk/transactions/{id}` (`web/pages/desk-transaction.html`, `web/src/screens/desk-transaction/TransactionScreen.tsx` & `ReversalSection.tsx`)
  2. `/#approvals` (`web/pages/admin.html` → menu **`คำขออนุมัติ` / `Approvals`**, `web/src/screens/admin/approvals/Approvals.tsx`)
  3. `/staff/desk` or `/desk` → **`คืนเงินที่รอบันทึก` / `Pending Refunds`** (`web/src/screens/desk/steps/refunds/RefundsStep.tsx`)
  4. `/#cash` (`web/pages/admin.html` → menu **`เงินสดและกะ` / `Cash & Shifts`**, `web/src/screens/admin/cash/`)
- **Action**:
  1. **Exchange to reverse**: Reverse **Exchange 3** (the **`100.00 USD` SELL** for **`3,350.00 THB`** Cash from Step 4.3).
  2. On `/desk/transactions/{id}` (`web/pages/desk-transaction.html`) for **Exchange 3**, click **`เสนอรายการ reversal` / `Request a reversal`** (`button[data-action="request-reversal"]`), enter the reason (e.g. `Customer entered wrong USD amount`) in `ReasonDialog`, and confirm.
  3. As **Executive / Manager**, open `web/pages/admin.html` and click **`คำขออนุมัติ` / `Approvals`** (`#approvals`, `web/src/screens/admin/approvals/`). Find the pending reversal request for **Exchange 3**, enter a decision reason, and click **`อนุมัติ` / `Approve`**.
  4. Return to `/desk/transactions/{id}` (`web/pages/desk-transaction.html`) and click **`รีเฟรช` / `Refresh`** (`button[data-action="refresh"]`) to verify the approved state.
  5. As **Cashier A** on `/staff/desk` or `/desk` (`web/src/screens/desk/DeskPage.tsx`), click **`คืนเงินที่รอบันทึก` / `Pending Refunds`** in the top step bar (`web/src/screens/desk/steps/refunds/task.tsx`). On the row for **Exchange 3** (`data-reversal`), click **`บันทึกคืนเงิน` / `Post Refund`** (`button[data-action="post"]`), click **`ยืนยัน` / `Confirm`** in the confirmation dialog, and then click **`พิมพ์ใบคืนเงิน` / `Print Refund Receipt`** (`button[data-action="print"]`).
  6. Refresh `/desk/transactions/{id}` (`web/pages/desk-transaction.html`) and inspect `web/pages/admin.html` → **`เงินสดและกะ` / `Cash & Shifts`** (`#cash`, `web/src/screens/admin/cash/`).
- **Expected screen result**:
  - **After requesting (Step 6.2)**: `/desk/transactions/{id}` shows status message **`ส่งคำขอคืนรายการแล้ว` / `Reversal requested`**, the **`การคืนรายการ` / `Reversal`** section (`data-section="reversal"`, `web/src/screens/desk-transaction/ReversalSection.tsx`) appears with `สถานะ` (`Status`) = **`REQUESTED`** and `จำนวนเงิน` (`Amount`) = **`3,350.00`**, and the request button disappears.
  - **After approving (Step 6.3–6.4)**: In `#approvals`, the card status changes to **`อนุมัติแล้ว` / `Approved`**. On `/desk/transactions/{id}`, the Reversal section shows `Status` = **`APPROVED`** and note **`บันทึกคืนเงินที่หน้าเดสก์` / `Record the refund at the desk`**.
  - **What the receipt must show afterwards (Step 6.5–6.6)**:
    - In **`คืนเงินที่รอบันทึก` / `Pending Refunds`** (`web/src/screens/desk/steps/refunds/RefundsStep.tsx`), posting removes the row from the pending list and displays the new refund document number (`.desk-refunds-doc`) with the **`พิมพ์ใบคืนเงิน` / `Print Refund Receipt`** button.
    - On `/desk/transactions/{id}` (`web/pages/desk-transaction.html`), the **`การคืนรายการ` / `Reversal`** section now shows `สถานะ` (`Status`) = **`POSTED`**, `บันทึกเมื่อ` (`Posted at`) populated, and `เลขที่เอกสารคืนเงิน` (`Refund document number`) matching the refund receipt; the **`เอกสาร` / `Documents`** section (`ReceiptsSection.tsx`) now shows **two** rows: Revision **`1 · ใบเสร็จ` (`1 · Receipt`)** and Revision **`2 · เอกสารคืนเงิน` (`2 · Refund document`)**.
  - **What the drawer must show afterwards (Step 6.6)**:
    - In `web/pages/admin.html` → **`เงินสดและกะ` / `Cash & Shifts`** (`#cash`, `web/src/screens/admin/cash/`), the `3,350.00 THB` refund payout and `+100.00 USD` return are reflected in the open shift's drawer balances (`คงเหลือตามระบบ` / `System Expected`):
      - **`THB`**: Opening `500,000.00` + Cash In `20,100.00` − Cash Out `36,350.00` (BUY `33,000.00` + Refund `3,350.00`) = **`483,750.00 THB`** (Net Movement `-16,250.00`).
      - **`USD`**: Opening `2,000.00` + BUY `1,000.00` − SELL `600.00` + Refund `100.00` = **`2,500.00 USD`** (Net Movement `+500.00`).
- `Result: pass / fail / note`:

---

### Step 7 — Handover to a second staff member (`staff.html`) <!-- web/pages/staff.html, web/src/screens/staff/shift/ -->

- **Page / Menu**: `/staff` → card **`กะของฉัน` / `My Shift`** (`web/pages/staff.html`, `web/src/screens/staff/shift/ShiftCard.tsx`).
- **Second person required**: **Yes — this step requires both Cashier A (sender) and Cashier B (second staff member, receiver) signed in at the same branch.**
- **Action**:
  1. In Browser Window 1 (**Cashier A** on `/staff`), in the **`กะของฉัน` / `My Shift`** card (`web/src/screens/staff/shift/ShiftCard.tsx`), click **`ส่งมอบเงิน` / `Hand Over`** (`data-testid="shift-handover"`).
  2. In the handover form (`data-testid="shift-handover-form"`):
     - Select **Cashier B** from the **`ผู้รับมอบ` / `Receiver`** dropdown (`data-testid="shift-receiver"`).
     - Enter a handover quantity on a denomination row (e.g. `10` on `THB 1000` = `10,000.00 THB`, or the full drawer denominations if handing over the whole drawer, then returning or accounting for it before close; for a zero-difference close at `483,750.00 THB` / `2,500.00 USD`, either have **Cashier B** click **`ปฏิเสธ` / `Reject`** to test rejection without moving drawer stock, or have **Cashier B** click **`รับมอบ` / `Accept`** and hand the same `10,000.00 THB` back from Cashier B's shift to Cashier A so net handover movement on Cashier A's shift is `0.00`).
     - Click **`บันทึก` / `Save`** (`data-testid="shift-handover-save"`).
  3. *(Second person)* In Browser Window 2 (**Cashier B** on `/staff`, `web/pages/staff.html`), click **`รีเฟรช` / `Refresh`** (`data-testid="staff-refresh"`). In **`กะของฉัน` / `My Shift`** under **`รายการส่งมอบถึงฉัน` / `Handovers to Me`**, locate the pending handover (`data-handover-id`) showing **Cashier A**'s shift ID and click **`รับมอบ` / `Accept`** (`data-testid="shift-accept"`). (If testing round-trip so Cashier A's drawer returns to `483,750.00 THB`, send the matching `10,000.00 THB` handover back to **Cashier A** and have **Cashier A** click **`รับมอบ` / `Accept`**.)
- **Expected screen result**:
  - When **Cashier A** submits the handover form: `ShiftCard` shows **`บันทึกแล้ว` / `Saved`** (`data-testid="shift-notice"`) and closes the handover form (`data-testid="shift-handover-form"`).
  - On **Cashier B**'s `/staff` screen: the **`รายการส่งมอบถึงฉัน` / `Handovers to Me`** section (`web/src/screens/staff/shift/ShiftCard.tsx`) displays the pending item (`data-handover-id`) with **`รับมอบ` / `Accept`** (`data-testid="shift-accept"`) and **`ปฏิเสธ` / `Reject`** (`data-testid="shift-reject"`).
  - When **Cashier B** clicks **`รับมอบ` / `Accept`**: `ShiftCard` shows **`บันทึกแล้ว` / `Saved`** (`data-testid="shift-notice"`) and removes the item from **`รายการส่งมอบถึงฉัน` / `Handovers to Me`** (`pendingRequests` returns to `0`).
- `Result: pass / fail / note`:

---

### Step 8 — Close shift (closing count) (`staff.html`) <!-- web/pages/staff.html, web/src/screens/staff/shift/, web/src/screens/staff/floor/ -->

- **Page / Menu**: `/staff` → card **`กะของฉัน` / `My Shift`** (`web/pages/staff.html`, `web/src/screens/staff/shift/ShiftCard.tsx`) and Floor card (`web/src/screens/staff/floor/FloorCard.tsx`).
- **Action**:
  1. As **Cashier A** on `/staff` (`web/pages/staff.html`), in **`กะของฉัน` / `My Shift`**, click **`นับปิดกะ` / `Close Count`** (`data-testid="shift-close-count"`).
  2. In the closing count form (`data-testid="shift-count-form"`), enter the counted denomination quantities matching the expected drawer balances:
     - **`THB`**: **`483,750.00`** (e.g. `483` × `1000`, `1` × `500`, `2` × `100`, `1` × `50`)
     - **`USD`**: **`2,500.00`** (e.g. `25` × `100`)
  3. Click **`บันทึก` / `Save`** (`data-testid="shift-count-save"`).
  4. *(Second person)* In Browser Window 2 as **Cashier B** (with `ShiftCloseCertify`) on `/staff` (`web/pages/staff.html`, `web/src/screens/staff/shift/`), refresh, find **Cashier A**'s `CLOSE` count under **`รายการนับที่รอยืนยัน` / `Counts to Verify`** (`data-count-id`), and click **`ยืนยัน` / `Verify`** (`data-testid="shift-verify"`).
  5. If closing out the branch business day, as a user with `BranchDayOperate` in the Floor card (`web/src/screens/staff/floor/FloorCard.tsx`), click **`ตัดยอดปิดวัน` / `Cutoff`** (`data-testid="floor-cutoff"`).
- **Expected screen result**:
  - After **Cashier A** saves the closing count: `ShiftCard` displays **`บันทึกแล้ว` / `Saved`** (`data-testid="shift-notice"`), closes the count form, and the shift action buttons switch back to **`นับเปิดกะ` / `Open Count`** (`data-testid="shift-open-count"`).
  - After **Cashier B** clicks **`ยืนยัน` / `Verify`**: `ShiftCard` displays **`บันทึกแล้ว` / `Saved`** and removes the `CLOSE` count from **`รายการนับที่รอยืนยัน` / `Counts to Verify`**.
  - In `web/pages/admin.html` → **`เงินสดและกะ` / `Cash & Shifts`** (`#cash`, `web/src/screens/admin/cash/`), the shift history table (`ประวัติกะ` / `Shift History`) shows the shift as **`ปิดแล้ว` / `Closed`** with closing difference **`ตรงตามระบบ` / `Matches System`** (`0.00`).
- `Result: pass / fail / note`:

---

### Step 9 — Shift report (`staff-report.html`) <!-- web/pages/staff-report.html, web/src/screens/staff-report/ -->

- **Page / Menu**: `/staff/report` (`web/pages/staff-report.html`, `web/src/screens/staff-report/StaffReport.tsx`), reached by clicking **`รายงานปิดวัน` / `Daily close report`** (`.xc-staff-report-link`) in the `/staff` header (`web/src/screens/staff/StaffPage.tsx`) or opening `/staff/report` directly.
- **Action**:
  1. Open `/staff/report` (`web/pages/staff-report.html`) as a user with `FinanceReconcile` or `BranchDayOperate`.
  2. Verify the date picker (`วันที่` / `Day`, `input[type="date"]`) is set to today's business date (`YYYY-MM-DD`).
  3. Inspect the metrics section (`.staff-report-metrics`), the currency table (`.staff-report-table`), the handover/checks section (`.staff-report-checks`), and the signers section (`.staff-report-signers`).
- **Expected screen result**:
  - Heading (`header.staff-report-heading`) shows **`DAILY CLOSE · รายงานปิดวัน`** (`DAILY CLOSE · Daily close report`), the branch name and date (`{site} · {day}`), and status line **`.staff-report-status`** = **`ปิดกะแล้ว · รอกระทบยอด · สาขา <branch>`** (`Shifts closed · awaiting reconciliation`) before Finance certification (or **`กระทบยอดแล้ว`** / `Reconciled` after certification).
  - Metrics section (`.staff-report-metrics`, `web/src/screens/staff-report/StaffReport.tsx`) shows:
    - **`รายการสำเร็จ` (`Completed exchanges`, `completed`)**: **`3`**
    - **`มูลค่าแลก · ร้านซื้อ` (`Exchange volume · shop buys`, `buyVolume`)**: **`33,000.00 THB`**
    - **`มูลค่าแลก · ร้านขาย` (`Exchange volume · shop sells`, `sellVolume`)**: **`20,100.00 THB`** (`16,750.00 + 3,350.00`)
    - **`กลับรายการแล้ว` (`Reversed`, `reversedVolume`)**: **`3,350.00 THB`**
    - **`รายการธุรกรรมค้าง` (`Unfinished exchanges`, `pendingTransactions`)**: **`0`**
  - Currency table (`.staff-report-table`) shows:
    - **`THB`**: `ลิ้นชักนับจริงปิดวัน` (`Drawer counted at close`, `drawerCounted`) = **`483,750.00`**, `ผลต่างลิ้นชักรวม` (`Drawer difference`, `drawerDifference`) = **`0.00`**
    - **`USD`**: `ลิ้นชักนับจริงปิดวัน` (`Drawer counted at close`, `drawerCounted`) = **`2,500.00`**, `ผลต่างลิ้นชักรวม` (`Drawer difference`, `drawerDifference`) = **`0.00`**
  - Checks section (`.staff-report-checks`) shows **`งานรับ–ส่งรอยืนยัน 0 รายการ · เหตุผลต่างที่ยังเปิด 0 รายการ`** (`pendingRequests = 0`, `openExceptions = 0`), **`ลูกค้ารอ Monitor ตรวจ 0 รายการ`** (`openHolds = 0`), and **`เงินระหว่างนำส่ง: ไม่มี`** (`inTransit` empty).
  - Signers section (`.staff-report-signers`) lists **Cashier A (`ผู้นับ` / `counted`)** and **Cashier B (`ผู้รับรอง` / `verified`)**.
- `Result: pass / fail / note`:

---

### Step 10 — Executive overview and ledger agreement (`admin.html#executive` & `admin.html#reconciliation`) <!-- web/pages/admin.html, web/src/screens/admin/executive/, web/src/screens/admin/reconciliation/ -->

- **Page / Menu**:
  1. `web/pages/admin.html` → menu **`ภาพรวมผู้บริหาร` / `Executive overview`** (`/#executive`, `web/src/screens/admin/executive/Executive.tsx`, `web/src/screens/admin/executive/menu.ts`).
     - **How to reach it from the menu**: Sign in as an **Executive** (holding permission `OverviewRead`), open `https://<staging-host>/` (`web/pages/admin.html`), and in the left sidebar navigation under the **`data`** group click **`🏢 ภาพรวมผู้บริหาร` / `🏢 Executive overview`** (or navigate to `/#executive`). Set the top shell period filter (`from` / `to`) to today's date (`YYYY-MM-DD`) and tick the test branch in the **`สาขา` / `Branches`** filter fieldset (`fieldset.executive-branch-filter`).
  2. `web/pages/admin.html` → menu **`กระทบยอดและรับรอง` / `Reconcile & certify`** (`/#reconciliation`, `web/src/screens/admin/reconciliation/Reconciliation.tsx`, `web/src/screens/admin/reconciliation/i18n.ts`) to inspect the branch ledger sheet (`finance_reconciliation`) and compare every figure against `/staff/report` and `/#executive`.
- **Action**:
  1. Open `/#executive` (`web/pages/admin.html`, `web/src/screens/admin/executive/Executive.tsx`), filter to today's business date and the test branch, and read the KPI tiles (`section.executive-kpis`), the branch row (`table.executive-branches tr[data-branch]`), and the alerts section (`section.executive-alerts`).
  2. Open `/#reconciliation` (`web/pages/admin.html`, `web/src/screens/admin/reconciliation/Reconciliation.tsx`) for the same branch and date, inspect the ledger reconciliation parts (`data-part="transactions"`, `data-part="counts"`, `data-part="exceptions"`, `data-part="integrity"`, `data-part="reversalsPending"`), and click **`รับรอง` / `Certify`**.
  3. Compare each named figure below across **Shift Report (`/staff/report`)**, **Executive Overview (`/#executive`)**, and **Branch Ledger / Reconciliation (`/#reconciliation`)**.
- **Expected screen result**:
  - Every row in the comparison table below must match across all three screens for the test branch and business date:

| Figure to compare | Shift Report (`/staff/report`, `web/src/screens/staff-report/`) | Executive Overview (`/#executive`, `web/src/screens/admin/executive/`) | Ledger / Finance Reconciliation (`/#reconciliation`, `web/src/screens/admin/reconciliation/`) | Expected value (worked example) |
| --- | --- | --- | --- | --- |
| **Gross Buy Volume (THB)** | `มูลค่าแลก · ร้านซื้อ` (`buyVolume`) | KPI tile `ยอดซื้อ` (`data-kpi="buy_volume_thb"`) | `รายการแลกเปลี่ยน` (`data-part="transactions"`) → `ซื้อ (บาท)` (`buyThb`) | **`33,000.00`** |
| **Gross Sell Volume (THB)** | `มูลค่าแลก · ร้านขาย` (`sellVolume`) | KPI tile `ยอดขาย` (`data-kpi="sell_volume_thb"`) | `รายการแลกเปลี่ยน` (`data-part="transactions"`) → `ขาย (บาท)` (`sellThb`) | **`20,100.00`** |
| **Total Branch Exchange Volume (THB)** | `buyVolume + sellVolume` (`33,000.00 + 20,100.00`) | Branch row `ยอด` (`colVolume`, `volumeThb`) | Sum of `buyThb + sellThb` in `data-part="transactions"` | **`53,100.00`** |
| **Posted Reversed Volume (THB)** | `กลับรายการแล้ว` (`reversedVolume`) | KPI sub-labels `คืนเงินแล้ว` (`reversed`: `buyReversedThb = 0.00`, `sellReversedThb = 3,350.00`) | `รายการแลกเปลี่ยน` (`data-part="transactions"`) → `กลับรายการแล้ว (บาท)` (`reversedThb`) | **`3,350.00`** |
| **Completed Exchange Count** | `รายการสำเร็จ` (`completed`) | Drill-down `ดูรายการ` (`#transactions`) completed count for the period | `รายการแลกเปลี่ยน` (`data-part="transactions"`) → `จำนวนรายการ` (`count`) | **`3`** |
| **Closing THB Cash on Hand** | Currency table `THB` `ลิ้นชักนับจริงปิดวัน` (`drawerCounted`) + `เซฟนับจริงปิดวัน` (`safeCounted`) | Branch row `เงินคงเหลือ` (`colCash`, `availableCash`) | `การตรวจนับ` (`data-part="counts"`) `CLOSE` line `THB counted/expected (difference)` | **`483,750.00`** (diff **`0.00`**) |
| **Closing USD Stock** | Currency table `USD` `ลิ้นชักนับจริงปิดวัน` (`drawerCounted`) | — (THB-only cash column) | `การตรวจนับ` (`data-part="counts"`) `CLOSE` line `USD counted/expected (difference)` | **`2,500.00`** (diff **`0.00`**) |
| **Cash in Transit** | `เงินระหว่างนำส่ง: ไม่มี` (`inTransit`) | Branch row `ระหว่างทาง` (`inTransit`) = **`0.00`** | `การโอนเงินสด` (`data-part="transfers"`) `inTransit` empty | **`0.00` / `ไม่มี`** |
| **Shift Verification & Exceptions** | `งานรับ–ส่งรอยืนยัน 0 รายการ · เหตุผลต่างที่ยังเปิด 0 รายการ` | Branch row `ปิดกะ`: `เปิดอยู่ 0`, `ปิดตรวจแล้ว 1`, `ปิดยังไม่ตรวจ 0`; `ต้องดำเนินการ`: `ไม่มีรายการที่ต้องดำเนินการ` | `ข้อยกเว้นที่ยังเปิดอยู่` (`exceptions`) = `ไม่มี`; `การตรวจความถูกต้องของบัญชี` (`integrity`) = clean | **0 open / 1 verified / 0 alerts** |

  - After clicking **`รับรอง` / `Certify`** in `/#reconciliation`, a certification row (`data-certification`, `data-stale="false"`) appears with **`ตรงกับตัวเลขปัจจุบัน` / `Matches the current figures`**, and reloading `/staff/report` (`web/pages/staff-report.html`) updates the report status line to **`กระทบยอดแล้ว` / `Reconciled`**.
- `Result: pass / fail / note`:

---

## If a step fails

1. **Record the failure immediately** as a comment on issue **`108-Plaza/xchange#2835`** with:
   - The **step number** (e.g. `Step 6 — Reversal of one exchange`),
   - A **screenshot** of the full browser window (showing the URL bar and any error banner or console/network status), and
   - The **exact timestamp** (in `Asia/Bangkok` or UTC) when the failure occurred.
2. **Continue if independent**: If the next step does not depend on the failed step (for example, a print-preview formatting issue in Step 5 does not block Step 6), continue running the remaining steps so a single UAT run surfaces all issues at once. If a blocker prevents subsequent steps (for example, Step 2 shift open fails so Step 3 cannot quote), stop at that step and report it on `#2835`.

---

## Sign-off

Fill in this block when posting the completed run on `#2835`:

- **Tester (name / GitHub handle)**:
- **Date (`YYYY-MM-DD`, `Asia/Bangkok`)**:
- **Staging URL tested**: `https://<staging-host>`
- **Branch tested**:
- **Build `version` (from `GET /health/ready`)**:
- **Build `build` / commit SHA (from `GET /health/ready`)**:
- **Overall outcome**: `PASS` / `FAIL`
