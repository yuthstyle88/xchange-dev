# Blueprint ledger, inventory and cost (#553)

> Design only. No code, no migration, no `ready`. Written by claude/e5ab6c on the lane controller's assignment
> (#564 issuecomment-6074257078). The code tickets are cut by the lane controller after a `Verdict: PASS`.

**Source of the logic.** *XC Desk — Blueprint & Data Model v2* (`docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`,
#565). The owner's rule (#564, 2026-10-09): **the Blueprint's logic is fixed, and only the look may change**.

| What | Blueprint |
|---|---|
| Entities | §07 `CashAccount(id, org_id, branch_id nullable, currency, kind)`, `LedgerPosting(id, source_type, source_id, status, posted_at)`, `LedgerEntry(id, posting_id, account_id, quantity_signed DECIMAL(20,4), valuation_thb DECIMAL(20,2))` — *"posting สมดุลต่อ currency รวม clearing accounts; valuation balanced ตาม accounting policy"*; `InventoryLot(id, branch_id, currency, denomination_id nullable, acquired_at, unit_cost_thb DECIMAL(20,8))`, `InventoryMovement(id, lot_id, posting_id, transaction_line_id nullable, quantity_signed, movement_type)` — *"ต้นทุนเสนอ weighted average ต่อสาขา/สกุล; freeze cost allocation ที่ transaction; อย่ารวม FIFO กับ weighted average"* |
| Relations | §08 *"Posting → Entry → Account 1:N → N:1; posting เชื่อม transaction/transfer/payout/reversal"* |
| Atomicity | §09 complete exchange: *"… post ledger/inventory … ใน DB transaction เดียว"*; reversal: *"posting ตรงข้าม … ไม่ลบ original"* |
| Numbers | §11 ส่วนต่างรับรู้ (realized margin): *"สำหรับขาย: เงิน THB รับ − allocated FX cost; สำหรับซื้อเงินต่างประเทศไม่รับรู้กำไร"*; *"inventory ยังไม่ขายแสดง valuation แยก"*; เงินพร้อมใช้งาน = *"on_hand − active reservations; เงิน in-transit แยก; cash variance = counted − expected"* |
| Open values | §12 Cash & cost: *"weighted average เป็นข้อเสนอ … method ตามระบบเดิมและการบัญชี; วงเงิน; ผู้รับรอง; negative stock policy"* |

**Measured on** `108-Plaza/xchange` `main` at `af865a3`. It stands on:
- `rates.md` (units, `ForeignAmount`, AP-1 approvals);
- the cash design `cash-shift-transfer.md` (#582, merged `af865a3`), whose posting port §4.6 this design owns the final
  shape of;
- the quote/transaction design (#551, PR 608), whose completion calls the port;
- the outbox OB-1 (`broker-seam.md`, merged `e6c7fd0`).

---

## 0. The owner's answers (2026-10-09, in session claude/e5ab6c, AskUserQuestion)

Recorded on #553 (issuecomment-6074305056, issuecomment-6074772982).

| # | Question | Answer (verbatim) | What this design does |
|---|---|---|---|
| B-1 | Where the books live | *"แยกสองชั้น"* | v3 holds the operational sub-ledger (quantities, lots, cost, margin) in the same DB transaction as the business change; AccountZing receives a THB GL summary through the outbox (§7) |
| Q10 | Cost method | *"Weighted average"* | a running weighted average per (branch, currency), frozen on each transaction line (§4) |
| N-1 | Negative stock | *"ผู้จัดการอนุมัติให้ติดลบได้"* | a move that would take a balance below zero is refused `stockInsufficient` unless BRANCH_MANAGER approves it through AP-1 (§5.1) |
| V-1 | Who signs off a count difference | *"FINANCE เมื่อเกินวงเงิน"* | a difference above a THB limit needs FINANCE through AP-1; at or below, #582's verifier is enough (§5.2) |

**Still the owner's** (§11): the AccountZing event and account mapping (AccountZing `docs/DO_NOT_TOUCH.md`: *"Chart
of accounts / account codes"* and the posting contract need the owner's approval), the two limits' values.

---

## 1. v3 today (measured)

| Fact | Where |
|---|---|
| **No ledger.** Stock is one number per (shift, currency): `shift_balances(opening, expected, counted, difference)` | `api/migrations/20261007110000_shifts.sql:18-26` |
| **An exchange moves `expected` directly**: foreign `+amount` on BUY, `−amount` on SELL; THB only when the method is cash (`−paid` BUY, `+received` SELL) | `xc-shift/src/domain/shift/value_objects/stock_delta.rs:55-87`; `xc-shift/src/application/shift/service.rs:384-415` |
| **Never negative.** `ensure_non_negative` refuses (`ShiftError::Insufficient`) | `service.rs:384-415` |
| **Manual moves** write `stock_movements(direction IN/OUT, amount, reason)`; exchanges write none | `20261007110000_shifts.sql:28-42`; `service.rs:305` |
| **The safe** is a JSON blob, `ops_state.safe` | `api/migrations/20261007140000_custody.sql:8` |
| **No cost.** Nothing stores what foreign currency cost. `profit_summary` is a calculator over figures the user types (`foreignCost`, `realized`) | `api/src/presentation/http/v3/actions/profit_summary.rs:20-60` |
| **The cash design's seam.** `CashPostings::post_in` / `expected_in` with `CashPlace { Drawer(ShiftId), Safe(BranchId), InTransit(TransferId), Outside }`, implemented for now by `InterimCashPostings` on the tables above; *"#553 replaces it with ledger postings without touching any caller"* | `docs/design/blueprint/cash-shift-transfer.md` §4.6 (main `af865a3`) |
| **AccountZing** is a THB-only double-entry GL (`journal_entry_lines.amount numeric(18,4)`, no currency or quantity), an async consumer of events (`POST /v1/events`, idempotent on `(tenant_id, event_id)`) | `108-Plaza/AccountZing-Platform` `migrations/20260615000003_ledger.sql`, `README.md` |

---

## 2. Decisions

| # | Decision | Why |
|---|---|---|
| L-1 | **The ledger is v3's operational sub-ledger in quantities**, one currency per entry, balanced per currency per posting (§07). THB valuation is carried on every entry. It is not a GL: no revenue or expense accounts beyond what §11 needs. | B-1; AccountZing is the GL |
| L-2 | **`CashPostings` keeps its two methods and callers**; its `PostingRequest` gains the source line and a `ForeignAmount` instead of `Money` (3-place currencies, `rates.md` D-2). `LedgerCashPostings` replaces `InterimCashPostings`. | the cash design §4.6 leaves the final shape to #553 |
| L-3 | **Weighted average is a running position per (branch, currency)**: `inventory_positions(quantity, cost_thb)`. Each acquisition is an `InventoryLot` (its unit cost is the evidence); each change is an `InventoryMovement` carrying the cost it applied. The average is `cost_thb ÷ quantity`, never a per-lot pick. | Q10 and §07 *"อย่ารวม FIFO กับ weighted average"* |
| L-4 | **Shift balances are derived, not stored.** A drawer's expected balance is the sum of its account's entries. `shift_balances` and `stock_movements` stop being written once the cutover lands, and are dropped in the cleanup block. | the ticket's §4: *"Show how today's shift balances are derived from postings"* |
| L-5 | **Locking order, a total order**: a posting that touches **one** branch runs inside that branch's `BranchTx`; a posting that touches **two** branches (a BRANCH transfer's receipt, §4.4) runs inside `BranchTx::run_pair(a, b, f)`, which locks both `branch_versions` rows in `branch_id` order and moves **both** versions (LG-0). Inside either, `inventory_positions` rows are locked `FOR UPDATE` in `(branch_id, currency)` order. No other lock is taken. | one total order ⇒ two opposite receipts (A→B under B, B→A under A) lock `(A,…)` before `(B,…)` in both and cannot deadlock; and every branch whose figures change moves its version, so a view answering `changed: false` from a branch's version (web design §4.2) never hides a receipt |
| L-6 | **No row is updated or deleted** in `ledger_postings`, `ledger_entries`, `inventory_lots`, `inventory_movements`. A correction is a new posting (`source_type = 'REVERSAL'`, #552). `inventory_positions` is the only mutable row, a cache of the movements' sums, checked by a reconciliation query. | §07 *"ประวัติ immutable"*; §09 reversal *"ไม่ลบ original"* |

---

## 3. Tables (Blueprint §07 names kept; `organization_id`, composite FKs and timestamps per #550)

### 3.1 Accounts and postings (migration 1)

```
cash_accounts(
  id               uuid PK,
  organization_id  uuid NOT NULL,
  branch_id        text NULL,                 -- NULL for an organisation-wide account (§07 "branch_id nullable")
  currency         char(3) NOT NULL,          -- FK currencies
  kind             text NOT NULL CHECK (kind IN ('DRAWER','SAFE','IN_TRANSIT','CUSTOMER','BANK','VARIANCE')),
  shift_id         text NULL,                 -- set for DRAWER only (#582 C1: a drawer is a teller's shift)
  created_at       timestamptz NOT NULL,
  CHECK ((kind = 'DRAWER') = (shift_id IS NOT NULL)),
  UNIQUE NULLS NOT DISTINCT (organization_id, branch_id, currency, kind, shift_id)   -- PostgreSQL ≥ 15; CI runs 16
)
ledger_postings(
  id               uuid PK,
  organization_id  uuid NOT NULL,
  branch_id        text NOT NULL,
  source_type      text NOT NULL CHECK (source_type IN ('EXCHANGE','TRANSFER','COUNT','OPENING','REVERSAL','ADJUSTMENT')),
  source_id        text NOT NULL,
  source_key       text NOT NULL,             -- e.g. 'complete', 'dispatch', 'receive'; one posting per (source, key)
  status           text NOT NULL CHECK (status IN ('POSTED')),   -- §07 has status; only POSTED exists (L-6)
  posted_at        timestamptz NOT NULL,
  posted_by        text NOT NULL,
  UNIQUE (organization_id, source_type, source_id, source_key)
)
ledger_entries(
  id               bigint GENERATED ALWAYS AS IDENTITY PK,
  posting_id       uuid NOT NULL REFERENCES ledger_postings(id),
  account_id       uuid NOT NULL REFERENCES cash_accounts(id),
  currency         char(3) NOT NULL,          -- = the account's; repeated so the balance check is one GROUP BY
  quantity_signed  numeric(20,4) NOT NULL CHECK (quantity_signed <> 0),
  valuation_thb    numeric(20,2) NOT NULL,
  created_at       timestamptz NOT NULL
)
CREATE INDEX ledger_entries_account ON ledger_entries (account_id, id);
```

- **Balanced per currency** (§07): for every posting and every currency, `Σ quantity_signed = 0`. A deferred
  constraint trigger checks it at commit (`ledger_posting_balanced`), so a posting written in several statements is
  checked once, at the end.
- **Valuation balanced per accounting policy**: every entry pair is valued alike on both sides (a foreign pair at the
  cost it moves, a THB pair at face value), so **for every posting `Σ valuation_thb = 0`**. The margin is not in the
  valuations; it lives on #551's transaction line (`realized_margin_thb`, §4.3). The trigger checks quantities;
  valuations are checked by §6 check 5.
- **Accounts are found or created** by `(org, branch, currency, kind, shift)` inside the posting's transaction
  (`INSERT … ON CONFLICT DO NOTHING RETURNING`, then `SELECT`).

### 3.2 Inventory (migration 1)

```
inventory_positions(
  organization_id  uuid NOT NULL,
  branch_id        text NOT NULL,
  currency         char(3) NOT NULL,
  quantity         numeric(20,4) NOT NULL,    -- may be negative only after an approved negative move (N-1)
  cost_thb         numeric(20,2) NOT NULL,    -- total cost of `quantity` at weighted average
  version          bigint NOT NULL,
  PRIMARY KEY (organization_id, branch_id, currency)
)
inventory_lots(
  id               uuid PK,
  organization_id  uuid NOT NULL,
  branch_id        text NOT NULL,
  currency         char(3) NOT NULL,
  denomination_id  bigint NULL,               -- NULL = group ALL (rates A4; #551 T4)
  acquired_at      timestamptz NOT NULL,
  quantity         numeric(20,4) NOT NULL CHECK (quantity > 0),
  unit_cost_thb    numeric(20,8) NOT NULL CHECK (unit_cost_thb >= 0),
  source_posting_id uuid NOT NULL REFERENCES ledger_postings(id)
)
inventory_movements(
  id               bigint GENERATED ALWAYS AS IDENTITY PK,
  organization_id  uuid NOT NULL,
  branch_id        text NOT NULL,
  currency         char(3) NOT NULL,
  lot_id           uuid NULL REFERENCES inventory_lots(id),   -- set for an acquisition; NULL for a disposal at average
  posting_id       uuid NOT NULL REFERENCES ledger_postings(id),
  transaction_line_id uuid NULL,                              -- #551's transaction_lines(id)
  movement_type    text NOT NULL CHECK (movement_type IN ('ACQUIRE','DISPOSE','TRANSFER_OUT','TRANSFER_IN','ADJUST')),
  quantity_signed  numeric(20,4) NOT NULL CHECK (quantity_signed <> 0),
  cost_thb_signed  numeric(20,2) NOT NULL,                    -- the cost this movement applied, frozen (§07)
  average_after    numeric(20,8) NULL,                        -- the position's average after it; NULL when quantity ≤ 0
  created_at       timestamptz NOT NULL
)
```

- **THB is not inventory.** Positions exist for foreign currencies only, as `StockDelta` treats THB today
  (`stock_delta.rs:71-73`).
- **Where inventory lives vs where cash lies.** Inventory and cost are per **branch** (Q10: *"ต่อสาขา/สกุล"*). The
  ledger says which **place** in the branch holds it (drawer, safe, in transit). A move between places inside a branch
  changes no position and no cost.

### 3.3 Limits (migration 1; N-1, V-1)

```
cash_limits(
  organization_id  uuid NOT NULL,
  branch_id        text NOT NULL,
  count_difference_limit_thb numeric(20,2) NULL CHECK (count_difference_limit_thb IS NULL OR count_difference_limit_thb >= 0),
  updated_by text NOT NULL, updated_at timestamptz NOT NULL,
  PRIMARY KEY (organization_id, branch_id)
)
```

`NULL` (or no row) = **every** non-zero difference goes to FINANCE: a missing limit must not let a large difference
pass on the verifier alone. `[proposed]`, the opposite default of #551's approval limit, because there the safe
default is "nothing waits" and here it is "someone checks".

---

## 4. Postings

### 4.1 The port (final shape of the cash design's §4.6)

```rust
// api/crates/xc-cash/src/domain/posting.rs  (CA-3 creates it; LG-2 changes it to this shape)
pub enum CashPlace { Drawer(ShiftId), Safe(BranchId), InTransit(TransferId), Customer, Bank }
pub struct PostingRequest {
    pub from: CashPlace, pub to: CashPlace,
    pub currency: CurrencyCode, pub quantity: ForeignAmount,   // THB as ForeignAmount with minor unit 2
    pub source: SourceRef,                                      // (source_type, source_id, source_key)
    pub line: Option<TransactionLineId>,                       // #551's line, for cost freezing
}
pub trait CashPostings: Send + Sync {
    fn post_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, req: &'a [PostingRequest],
                   allow_negative: Option<ApprovalRequestId>) -> BoxFuture<'a, AppResult<PostingResult>>;
    fn expected_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, place: &'a CashPlace)
                   -> BoxFuture<'a, AppResult<Balances>>;
}
pub struct PostingResult { pub posting_id: PostingId, pub costs: Vec<LineCost> }   // per request with a `line`
pub struct LineCost { pub line: TransactionLineId, pub cost_basis_thb: Money, pub realized_margin_thb: Option<Money> }
```

- `Outside` of the cash design becomes `Customer` (an exchange's counterparty) or `Bank` (non-cash payments and
  external moves). The cash design's `EXTERNAL_IN/OUT` kinds map to `Bank`.
- `allow_negative` names the approved AP-1 request that lets this posting take a balance below zero (§5.1); `None`
  refuses with `stockInsufficient`.
- `LedgerCashPostings` (`xc-cash/src/infrastructure/ledger_postings.rs`) is the implementation. `InterimCashPostings`
  is removed in the cutover ticket (LG-4).

### 4.2 Shift balances, derived (L-4)

| Today (`shift_balances`) | From the ledger |
|---|---|
| `opening` | the shift's OPEN count posting (`SAFE → DRAWER(shift)`, or the hand-over's `DRAWER(previous) → DRAWER(shift)`) |
| `expected` | `Σ quantity_signed` of the entries on `DRAWER(shift)` for that currency (`expected_in`) |
| `counted` / `difference` | #582's `cash_count_totals` (already its own table); a non-zero difference posts `DRAWER ↔ VARIANCE` (§5.2) |

`v2_shift_cases.json` and `v2_stock_cases.json` (the cash design §5.1 keeps them) must give the same answers through
`expected_in`: the cutover ticket runs them against `LedgerCashPostings`.

### 4.3 An exchange (§09, called by #551's `CompleteExchange`)

One posting, `source_type EXCHANGE`, `source_key complete`, per currency balanced:

| Side | Foreign entries | THB entries (cash) | THB entries (non-cash) |
|---|---|---|---|
| **BUY** (shop buys `q` foreign, pays `paid` THB) | `DRAWER +q`, `CUSTOMER −q` | `DRAWER −paid`, `CUSTOMER +paid` | `BANK −paid`, `CUSTOMER +paid` |
| **SELL** (shop sells `q` foreign, receives `received` THB) | `DRAWER −q`, `CUSTOMER +q` | `DRAWER +received`, `CUSTOMER −received` | `BANK +received`, `CUSTOMER −received` |

**Cost** (Q10, weighted average, per branch and currency, under the position's row lock):
- **BUY** acquires: a new `inventory_lots` row with `unit_cost_thb = thb_amount ÷ q` (the principal before fee, 8
  places HALF_UP); the position becomes `quantity + q`, `cost_thb + thb_amount`. The line's `cost_basis_thb =
  thb_amount`, and `realized_margin_thb` is NULL (§11 *"สำหรับซื้อ … ไม่รับรู้กำไร"*).
- **SELL** disposes at average: `cost = round_half_up(q × cost_thb ÷ quantity, 2)`; the position becomes
  `quantity − q`, `cost_thb − cost`. The line's `cost_basis_thb = cost`, and `realized_margin_thb = thb_amount − cost`
  (§11 *"เงิน THB รับ − allocated FX cost"*). The fee is income of its own (`fee_thb`) and is not in the margin, so
  §11's Contribution does not count it twice.
- **Valuation**: both foreign entries carry the cost moved (`DRAWER −cost`, `CUSTOMER +cost` on a SELL; `±thb_amount`
  on a BUY); both THB entries carry their face value. The posting's valuations sum to 0 (§6 check 5). The margin is
  recorded on the line and reconciled by §6 check 4.
- **Who sends which leg.** #551's `CompleteExchange` sends both requests: the foreign leg, and the THB leg with the
  place its payment method gives (`Drawer` for Cash, `Bank` otherwise). `LedgerCashPostings` derives nothing.
  (PR 608 §4.5 is aligned to this at its next revision.)
- **The last unit**: when a SELL takes the position to exactly zero, `cost` is the whole remaining `cost_thb`, so no
  rounding crumb is left behind.

### 4.4 Transfers, counts, opening (#582's flows, re-pointed)

| Flow (#582) | Posting |
|---|---|
| OPEN count verified | `SAFE → DRAWER(shift)` per currency counted (`source_type OPENING`) |
| hand-over accepted | `DRAWER(from) → DRAWER(to)` |
| drawer ↔ safe | `DRAWER ↔ SAFE` |
| BRANCH transfer **dispatched** | ledger: `SAFE(from) → IN_TRANSIT` (§07 *"ledger ย้ายเข้าสู่ in-transit account ตอน dispatch"*). The `IN_TRANSIT` account belongs to the **sender** (`cash_accounts.branch_id = from_branch_id`, one per currency). Inventory: **nothing** — the money in transit is still the sender's, inside the sender's position |
| BRANCH transfer **received** (`qty_received`) | runs in `BranchTx::run_pair(from, to, …)` (L-5). Ledger: `IN_TRANSIT(from) → SAFE(to)` for `qty_received`. Inventory: `TRANSFER_OUT` on the sender for `qty_received` at the sender's average at that moment, and `TRANSFER_IN` on the receiver as a lot at that same unit cost (cost travels with the money) |
| a received quantity **short** of the sent one | the shortfall stays on the sender's `IN_TRANSIT`, and in the sender's position, until FINANCE reconciles (§06 *"difference เป็น exception"*). Reconcile posts `IN_TRANSIT(from) → VARIANCE(from)` and an inventory `ADJUST` on the sender for `−shortfall` at its average |
| EXTERNAL in / out | `BANK ↔ SAFE` |
| count difference verified | `DRAWER ↔ VARIANCE` for `counted − expected`; inventory `ADJUST` at average |

### 4.5 Negative stock (N-1)

- Any posting that would take `DRAWER` or `SAFE` below zero for a currency, or a foreign position below zero, is
  refused `stockInsufficient` unless `allow_negative` names an APPROVED request of type `NEGATIVE_STOCK` for this
  source (§5.1).
- **Cost below zero.** The part sold beyond the position is costed at the position's current average (or, if the
  position is exactly zero, at the last movement's `average_after`). When a later acquisition brings the position back
  above zero, the difference between that provisional cost and the acquisition's unit cost on the negative quantity
  is posted as an `ADJUST` movement (and a `realized_margin_thb` correction line on the adjustment posting), so the
  average restarts from real cost. `[proposed]`; the Blueprint leaves it to *"negative stock policy"* (§12).

---

## 5. Approvals (AP-1's traits, `rates.md` §5.2)

### 5.1 `NEGATIVE_STOCK` (N-1)

`NegativeStockSubject` + `NegativeStockPolicy` (`xc-cash/src/application/ledger_approval/negative_stock.rs`):
- requested by the caller that got `stockInsufficient` (the completion of #551, a transfer dispatch of #582) with the
  shortfall per currency in `proposed_payload`;
- `may_decide` ⇔ BRANCH_MANAGER of the branch; `allows_self_decision` = false;
- `version_hash` = the source's state (the transaction's lines and status, or the transfer's lines and status);
- `on_decided`: APPROVED does nothing by itself; the caller retries with `allow_negative = Some(request)` (an
  exchange: `complete` again with the same key's new attempt; the desk shows "waiting for approval").

### 5.2 `COUNT_DIFFERENCE` (V-1)

`CountDifferenceSubject` + `CountDifferencePolicy` (`xc-cash/src/application/ledger_approval/count_difference.rs`):
- requested when #582's `CashCountService::verify` finds `|Σ difference valued at average| > count_difference_limit_thb`
  (or any non-zero difference when the limit is NULL, §3.3);
- `may_decide` ⇔ FINANCE (organisation-wide or of the branch); `allows_self_decision` = false;
- until APPROVED, the count stays unverified, and the `DRAWER ↔ VARIANCE` posting is not written;
- **the hook is in #582's file** (`CashCountService::verify`, CA-3): it calls `CountDifferenceGate::check_in(tx, count)`
  from this design. That one call is LG-5's edit to a CA-3 file, after CA-3 has merged.

---

## 6. Reconciliation (the checks the design promises)

`api/crates/xc-cash/src/application/reconcile.rs`, run by a test and by an admin view:
1. every posting balances per currency (`GROUP BY posting_id, currency HAVING sum(quantity_signed) <> 0` is empty);
2. every `inventory_positions` row equals the sum of its movements (quantity and cost);
3. per branch and foreign currency: Σ of the balances of that branch's own `DRAWER`, `SAFE` and `IN_TRANSIT` accounts
   (`cash_accounts.branch_id` = the branch) = the branch's position quantity. It holds during transit (the money is on
   the sender's `IN_TRANSIT` and in the sender's position) and after a short receipt until reconcile (the shortfall
   is on both until the `VARIANCE` posting and the `ADJUST` take it off both);
4. Σ `realized_margin_thb` of the period's lines = Σ `thb_amount` of SELL lines − Σ their `cost_basis_thb`;
5. every posting's `Σ valuation_thb` = 0 (§3.1).

A failure is a defect, never "corrected" by an update: it is reported, and an `ADJUSTMENT` posting is the only fix,
by FINANCE.

---

## 7. The GL summary to AccountZing (B-1) — a proposal for the owner, not decided

AccountZing ingests events through `POST /v1/events` with the envelope of its SPEC-001 §4.1 (`event_id`,
`event_type`, `schema_version`, `source_system`, `occurred_at`, `payload`). It has no FX event type today
(SPEC-001 §4.2), and its chart of accounts (§9.1) has no FX accounts. Both are on its `DO_NOT_TOUCH.md`.

**Proposed** (needs the owner's approval before any ticket):
- one event per branch per business day (branch timezone, #550), `event_type = "XCHANGE_DAY_SUMMARY"`,
  `source_system = "XCHANGE"`, `event_id` = uuid v5 of `(org, branch, date)`;
- payload, all THB, from §4's postings of that day: FX bought at cost, FX sold at cost, THB received on SELL, THB paid
  on BUY, fee income, bank fees (#551 `TransactionPayment.bank_fee_thb`), realized margin, cash variance, closing FX
  inventory at cost per currency;
- AccountZing accounts it would need (codes for the owner and AccountZing's maintainer to choose): FX inventory
  (asset), FX gain (revenue), fee income, bank-fee expense, cash over/short;
- delivered by an OB-1 handler (`accountzing.day_summary`), written in the transaction that closes the branch day
  (#582's `BranchDay` cut-off).

Until the owner approves, **no** AccountZing ticket is cut; the v3 side (LG-1…LG-5) does not depend on it.

---

## 8. Class diagram

```mermaid
classDiagram
  class CashPostings { <<trait xc-cash domain/posting.rs (CA-3; shape LG-2)>> +post_in() +expected_in() }
  class LedgerCashPostings { <<xc-cash infrastructure/ledger_postings.rs>> }
  class InterimCashPostings { <<xc-cash infrastructure/interim_postings.rs (CA-3; removed LG-4)>> }
  class LedgerRepository { <<trait xc-cash domain/ledger/repository.rs>> +account_in() +insert_posting_in() +balance_in() }
  class PgLedgerRepository { <<xc-cash infrastructure/ledger/repository.rs>> }
  class Posting { <<xc-cash domain/ledger/posting.rs>> +entries +is_balanced() }
  class CashAccount { <<xc-cash domain/ledger/account.rs>> +kind +shift }
  class WeightedAverage { <<xc-cash domain/inventory/weighted_average.rs>> +acquire(q, cost) +dispose(q) LineCost }
  class InventoryRepository { <<trait xc-cash domain/inventory/repository.rs>> +position_for_update_in() +record_in() }
  class PgInventoryRepository { <<xc-cash infrastructure/inventory/repository.rs>> }
  class NegativeStockSubject { <<xc-cash application/ledger_approval/negative_stock.rs>> }
  class CountDifferenceSubject { <<xc-cash application/ledger_approval/count_difference.rs>> }
  class CountDifferenceGate { <<xc-cash application/ledger_approval/count_difference.rs>> +check_in() }
  class Reconcile { <<xc-cash application/reconcile.rs>> +run() Report }
  class CompleteExchange { <<xc-exchange application/transaction/complete.rs (#551 TX-5)>> }
  class CashCountService { <<xc-cash application/count (#582 CA-3)>> }
  class ApprovalService { <<xc-approval AP-1>> }

  LedgerCashPostings ..|> CashPostings
  InterimCashPostings ..|> CashPostings
  LedgerCashPostings --> LedgerRepository
  LedgerCashPostings --> InventoryRepository
  LedgerCashPostings --> WeightedAverage
  PgLedgerRepository ..|> LedgerRepository
  PgInventoryRepository ..|> InventoryRepository
  LedgerRepository --> Posting
  Posting --> CashAccount
  CompleteExchange --> CashPostings : post_in
  CashCountService --> CountDifferenceGate : check_in
  CountDifferenceGate --> ApprovalService
  NegativeStockSubject ..|> ApprovalSubject
  CountDifferenceSubject ..|> ApprovalSubject
  Reconcile --> LedgerRepository
  Reconcile --> InventoryRepository
```

### 8.1 Who provides, who uses

| Provided by | Used here | If it has not landed |
|---|---|---|
| #582 CA-2/CA-3: the `xc-cash` crate, `CashPostings`, `InterimCashPostings`, counts | §4.1, §4.4, §5.2 | LG-1 starts after CA-3 |
| #582 CA-5 (#621): `TransferFlow::receive` | calls `post_in` for a receipt, which touches two branches | CA-5 stays as written (lane controller's ruling, 2026-10-09): until LG-4, `InterimCashPostings` writes only the receiver's tables, so its single-branch `BranchTx` is correct. **LG-4 switches `receive` to `BranchTx::run_pair(from, to, …)`** (LG-0, L-5) in the same PR that switches the postings |
| `rates.md` RU-1: `ForeignAmount`, `MinorUnit` | `PostingRequest.quantity` | merged before CA-3 |
| AP-1 (#596) | §5 | LG-3 starts after AP-1 |
| #551 TX-2/TX-5: `transaction_lines`, `CompleteExchange` | `LineCost` written on the line | LG-4 (cutover) starts after TX-5 |
| #550 OR-1: `organization_id`, branch timezone | every table; the business day of §7 | merged before CA-1 |

---

## 9. Data migration (block `20261015*`, #564 issuecomment-6074369626; drops in the cleanup block)

v3 holds test data (D6); v3 migrations are approved until the first install. The migrations still run correctly on a
database with rows.

**1. `api/migrations/20261015100000_ledger.sql`** (LG-1)
- *Changes:* creates `cash_accounts`, `ledger_postings`, `ledger_entries` (+ the balance trigger),
  `inventory_positions`, `inventory_lots`, `inventory_movements`, `cash_limits`.
- *Existing rows:* none touched.
- *Deploy:* with LG-1's binary; an older binary never reads these tables.
- *`down`:* drops them; nothing has been posted yet (postings start at LG-4).

**2. `api/migrations/20261015110000_ledger_opening_balances.sql`** (LG-4, the cutover)
- *Backfill:* for every OPEN shift, one `OPENING` posting per currency moving its current `shift_balances.expected`
  from a `SAFE` account into its `DRAWER` account; for every branch, one `OPENING` posting per currency moving
  `ops_state.safe`'s amounts into `SAFE`; for every foreign currency with a positive balance, an `inventory_lots` row
  whose unit cost is **the branch's current published BUY rate ÷ unit_size** (`rates.md` §4.8) and the matching
  position. `[proposed]`: v3 has no cost history (§1); test data only (D6); the alternative is a cost the owner types.
- *Count first:*

  ```sql
  SELECT count(*) FROM shifts WHERE status = 'OPEN';
  SELECT branch_id, code, sum(expected) FROM shift_balances b JOIN shifts s ON s.id = b.shift_id
   WHERE s.status = 'OPEN' GROUP BY 1, 2;
  SELECT branch_id, safe FROM ops_state;
  ```
- *Deploy:* with LG-4's binary, which switches `CashPostings` to `LedgerCashPostings`. The previous binary must not run
  after it (it would write `shift_balances` that nothing reads any more).
- *`down`:* refused (`RAISE`) once any posting other than `OPENING` exists, because a later SELL has disposed against
  the opening lots. Otherwise it deletes, in this order: the `OPENING` postings' `inventory_movements` and
  `ledger_entries`, then their `inventory_lots`, then the postings, then every `inventory_positions` row (all of them
  came from the backfill).

**3. `api/migrations/20261030150000_drop_shift_balances.sql`** (LG-6) — cleanup block, slot assigned on #564
(issuecomment-6074943261).
- *Changes:* drops `shift_balances`, `stock_movements` and `ops_state.safe`, after every reader reads the ledger.
- *Precheck* (must be 0, else `RAISE`): the reconciliation of §6 run as SQL.
- *`down`:* re-creates them empty.

---

## 10. Tickets

| # | Ticket | Owns | Starts after |
|---|---|---|---|
| LG-0 | `BranchTx::run_pair(a, b, f)`: one transaction, both `branch_versions` rows created if absent and locked `FOR UPDATE` in `branch_id` order, both versions bumped on commit; `a == b` behaves as `run` | `api/crates/xc-kernel/src/branch_tx.rs` (one new method beside `run`) and its tests | this design |
| LG-1 | tables and domain: accounts, postings, entries, the balance trigger; positions, lots, movements; `WeightedAverage` | `xc-cash/src/domain/{ledger,inventory}/**`, `xc-cash/src/infrastructure/{ledger,inventory}/**` (new); migration 1 | #582 CA-3 |
| LG-2 | `LedgerCashPostings` and the port's final shape (§4.1) | `xc-cash/src/domain/posting.rs` (shape), `xc-cash/src/infrastructure/ledger_postings.rs` (new) | LG-0, LG-1 |
| LG-3 | the two approval subjects and `CountDifferenceGate`; `cash_limits` commands | `xc-cash/src/application/ledger_approval/{negative_stock,count_difference}.rs`, `http/v3/commands/ledger/limits.rs` (outside CA-8's `commands/cash/**`) | LG-1, AP-1 |
| LG-4 | cutover: wire `LedgerCashPostings` in bootstrap, remove `InterimCashPostings`; `TransferFlow::receive` onto `run_pair`; migration 2 | `api/src/bootstrap/mod.rs` (one line), `interim_postings.rs` (deleted), the file of `TransferFlow::receive` under `xc-cash/src/application/transfer/` (#582 CA-5's; the `BranchTx` call only); migration 2 | LG-0, LG-2, #551 TX-5, #582 CA-5, #582 CA-7 |
| LG-5 | the count-difference hook in `CashCountService::verify` | `xc-cash/src/application/count/count_service.rs` (cash design §5, `CashCountService`; the one call only) | LG-3, CA-3 |
| LG-6 | readers on the ledger; migration 3 | per reader (`views/admin_cash.rs`, `shift_valuation.rs`, the staff report), one ticket per file; migration 3 last | LG-4 |
| LG-7 | reconciliation (§6) and its admin view | `xc-cash/src/application/reconcile.rs`, `views/cash_reconcile.rs` | LG-2 |

### 10.1 Up-front tests (🧪)

| Ticket | Behaviour locked |
|---|---|
| LG-0 | `run_pair(A, B)` and `run_pair(B, A)` started at once both commit (no deadlock, measured with a barrier, repeated); both versions move by one on commit and neither on rollback; `run_pair(A, A)` = `run(A)` |
| LG-1 | an unbalanced posting (per currency) is refused at commit; the same `(source, key)` twice is one posting; `WeightedAverage`: buy 100 USD at 3,200 then 100 at 3,300 ⇒ average 32.50; selling 50 costs 1,625.00; selling the last unit takes the whole remaining cost |
| LG-2 | `v2_shift_cases.json` and `v2_stock_cases.json` give the same `expected` through `LedgerCashPostings` as through `InterimCashPostings`; a BUY posts a lot and NULL margin; a SELL freezes `cost_basis_thb` and `realized_margin_thb = thb_amount − cost`; non-cash THB goes to `BANK`, not the drawer; a move inside a branch changes no position |
| LG-3 | a shortfall without approval ⇒ `stockInsufficient`; with an APPROVED `NEGATIVE_STOCK` request ⇒ posted and the position goes negative; a teller cannot approve; a difference above the limit waits for FINANCE; a NULL limit sends every non-zero difference to FINANCE |
| LG-4 | migration 2 on a seeded DB: every OPEN shift's expected equals its drawer balance after the backfill; reconciliation (§6) is clean; a BRANCH transfer received through `TransferFlow::receive` moves both branches' versions, and two opposite receipts at once both complete |
| LG-7 | each of the five checks fails on a hand-broken row and passes on a clean ledger; check 3 holds for a BRANCH transfer DISPATCHED and not received (sender's `IN_TRANSIT` and position both carry it), after a short receipt before reconcile, and after reconcile (`VARIANCE` + `ADJUST`); check 5 holds for a SELL with a margin; **a receipt moves the sender's and the receiver's `branch_versions`**; two opposite receipts (A→B and B→A, both USD) run at once both complete |

---

## 11. Must ask the owner (not decided here)

| # | Question | Blocks |
|---|---|---|
| Q-1 | The AccountZing event and accounts of §7 (AccountZing `DO_NOT_TOUCH.md`) | only the AccountZing ticket, which is not cut until answered |
| Q-2 | `count_difference_limit_thb` per branch (unset = every difference goes to FINANCE) | nothing; ADMIN/FINANCE sets it |
| Q-3 | The opening cost of today's stock at cutover (§9 migration 2): the published BUY rate, or a cost the owner types | LG-4's backfill only |
| Q-4 | §4.5's provisional-cost rule for negative stock | LG-2 |

---

## 12. Blueprint conformance

| Blueprint | Here | Status |
|---|---|---|
| §07 CashAccount, LedgerPosting, LedgerEntry fields | §3.1 (+`shift_id` on DRAWER accounts, `source_key`) | kept + additions for #582's drawer-per-teller |
| §07 *"posting สมดุลต่อ currency รวม clearing accounts"* | the `CUSTOMER`/`BANK` clearing kinds; the deferred trigger | kept |
| §07 InventoryLot/Movement; weighted average per branch/currency; frozen at the transaction | §3.2, §4.3 | kept (Q10) |
| §07 *"อย่ารวม FIFO กับ weighted average"* | the average is never a per-lot pick (L-3) | kept |
| §09 post ledger/inventory in the completion's DB transaction | `post_in(tx, …)` inside `CompleteExchange` | kept |
| §09 reversal: opposite posting, no delete | L-6 | kept (#552 writes it) |
| §11 realized margin, separate valuation, in-transit separate, variance | §4.3, §4.4, §6 | kept |
| §12 negative stock policy, certifier | N-1, V-1 | owner-decided |
| — | AccountZing summary | proposal, owner-gated (§7) |
