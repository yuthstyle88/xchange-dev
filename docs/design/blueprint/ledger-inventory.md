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
| L-5 | **Locking order, a total order**: a posting that touches **one** branch runs inside that branch's `BranchTx`; a posting that touches **two** branches (a BRANCH transfer's receipt, §4.4) runs inside `BranchTx::run_pair(a, b, f)`, which locks both `branch_versions` rows in the one order `MAIN` first, then ascending `branch_id` (compatible with `lock_also`'s "MAIN first", `branch_tx.rs`; found while writing LG-0's tests, #661) and moves **both** versions (LG-0). A v3 command that writes two branches gets the same lock: it declares `Locks::Pair`, and `CommandEndpoint` opens `run_pair` for it (`rates.md` §6 R-TX and Locks, #701); it never calls `lock_also`. Inside either, `inventory_positions` rows are locked `FOR UPDATE` in `(branch_id, currency)` order. No other lock is taken. | one total order ⇒ two opposite receipts (A→B under B, B→A under A) lock `(A,…)` before `(B,…)` in both and cannot deadlock; and every branch whose figures change moves its version, so a view answering `changed: false` from a branch's version (web design §4.2) never hides a receipt |
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
  quantity_signed  numeric(20,4) NOT NULL CHECK (quantity_signed <> 0 OR movement_type = 'ADJUST'),  -- a cost-only ADJUST (Q-4 correction, #924) carries 0
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
    pub reverses: Option<TransactionLineId>,                   // #552: value this leg at that line's frozen cost (§4.6)
}
pub trait CashPostings: Send + Sync {
    fn post_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, req: &'a [PostingRequest],
                   allow_negative: Option<ApprovalRequestId>) -> BoxFuture<'a, AppResult<PostingResult>>;
    fn expected_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, place: &'a CashPlace)
                   -> BoxFuture<'a, AppResult<Balances>>;
}
pub struct PostingResult { pub posting_id: PostingId, pub costs: Vec<LineCost>,    // per request with a `line`
                          pub reversed: Vec<ReversedCost> }                         // per request with `reverses` (§4.6)
pub struct LineCost { pub line: TransactionLineId, pub cost_basis_thb: Money, pub realized_margin_thb: Option<Money> }
pub struct ReversedCost { pub reverses: TransactionLineId, pub cost_thb: Money, pub difference_thb: Money }
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

### 4.2.1 Reading balances back: `ShiftBalances` (LG-6, added 2026-10-10)

**Why.** §4.2 says how a shift's figures derive from the ledger, but nothing in §4.1 reads them back. `CashPostings::expected_in` returns one place's balance only.
- Measured on `main` `875f7e8d`, four readers need the same four figures per (shift, currency):
  - `views/admin_cash.rs:174`, `:251`, `:317`, `:335`;
  - `views/admin_shifts.rs:142`;
  - `views/devices.rs:109`, `:133`;
  - `xc-shift/src/infrastructure/shift/shift_repository.rs:118-180` (`load_shift_balances`).
- Without a shared reader, each LG-6 child would copy §4.2's derivation into its own SQL. By ♻️ that is a copy, and the four copies would drift.

```rust
// api/crates/xc-shift/src/domain/shift/repositories/shift_balances.rs   (new, LG-6.0)
pub struct ShiftBalance {
    pub currency: CurrencyCode,
    pub opening: Money,               // the OPENING posting, or the hand-over's, into DRAWER(shift)
    pub expected: MoneyDelta,         // Σ quantity_signed on DRAWER(shift), SIGNED: negative after an approved NEGATIVE_STOCK sale
    pub moved_in: Money,              // Σ positive entries on DRAWER(shift) from postings whose source_type ∉ {EXCHANGE, OPENING}
    pub moved_out: Money,             // Σ |negative| entries on the same set
    pub counted: Option<Money>,       // the shift's verified CLOSE (or HANDOVER_OUT) count: cash_count_totals.counted
    pub difference: Option<MoneyDelta>, // the same row's difference
}
pub trait ShiftBalances: Send + Sync {
    /// One row per currency with a DRAWER(shift) account or a count line; ordered THB first, then A–Z.
    fn shift_balances<'a>(&'a self, shift: &'a ShiftId) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>>;
    /// Many shifts at once (added 2026-10-15 for the admin shifts list's N+1): every requested id is a key, an
    /// empty list when the shift has no rows; each value equals `shift_balances` for that shift. The default loops
    /// over `shift_balances`, so test doubles need no change; `PgLedgerShiftBalances` overrides it with one read.
    fn shift_balances_many<'a>(&'a self, shifts: &'a [ShiftId])
        -> BoxFuture<'a, AppResult<BTreeMap<ShiftId, Vec<ShiftBalance>>>>;
}
// api/crates/xc-cash/src/infrastructure/ledger/shift_balances.rs   (new, LG-6.0): PgLedgerShiftBalances implements it
```

**`shift_balances_many` (2026-10-15).** `views/admin_shifts.rs:134` called `shift_balances` once per listed shift. It reads the page's shifts in one call. Ticket: admin shifts batched balances (BE controller claude/7ca80c).

- **Why the trait is in `xc-shift`.** `xc-cash` depends on `xc-shift` (`xc-cash/Cargo.toml` on PR 871), so `xc-shift`'s `PgShiftRepository` cannot name an `xc-cash` type. The port sits beside `ShiftRepository` and `xc-cash` implements it. `PgShiftRepository` takes an `Arc<dyn ShiftBalances>`, and `api`'s views take the same `Arc` from `AppState` (one bootstrap line each, not reserved).
- **The one source.** `expected` is the same sum as `LedgerCashPostings::expected_in(Drawer(shift))`; the two must not drift. LG-6.0's up-front tests compare them on the `v2_shift_cases.json` cases, where every drawer is ≥ 0.
- **`expected` is signed (amended 2026-10-12, lane controller, #1039).** The 2026-10-10 ruling on #924 kept `expected_in` returning `Balances` (non-negative, a negative place reads as 0) **until LG-6**, and gave LG-6.0 the job of carrying the sign. `expected` is therefore `MoneyDelta` (`xc-kernel/src/money/money_delta.rs:9`): after an approved `NEGATIVE_STOCK` sale a drawer reads its negative value here, while `expected_in` still reads 0 for it. The equality above holds for every drawer ≥ 0; for a negative drawer `ShiftBalance.expected` is the signed sum and `expected_in` is 0. Readers r1–r4 display the signed value. `opening`, `moved_in`, `moved_out` and `counted` stay `Money` (each is non-negative by construction).
- **A shift with no posting yet** answers its count rows only. A shift with neither answers `[]`, which today's readers already treat as "no balances".
- **No new table.** It reads `cash_accounts`, `ledger_postings`, `ledger_entries` (§3.1), `cash_counts` and `cash_count_totals` (`20261012100000_cash.sql:120-161`).
- **Fallback if LG-6.0 lands late (📐).** A consumer (r1–r4) may start before #1039 merges by declaring, in its own file, the exact `ShiftBalance` struct and `ShiftBalances` trait of the block above, with the comment `// stub: moves to xc-shift/src/domain/shift/repositories/shift_balances.rs (LG-6.0 #1039)`. Its tests use a test double of the trait. It never declares `PgLedgerShiftBalances` or any SQL on the ledger tables, and never touches LG-6.0's files. When #1039 merges, the consumer's PR (or its rebase) deletes the stub and imports the real path. The tickets keep `Blocked by #1039` until the lane controller chooses to lift it on this fallback.
- **The safe** is read through `CashPostings`, not `ShiftBalances`. Two cases must print differently (lane controller claude/7ca80c's ruling of 2026-10-15 on #1044): a safe that was **never** posted to prints `null` (v2's "never counted in", `snapshot/custody.json:175`), and a safe that was posted to and now nets to zero prints zeros. `expected_in` cannot tell these apart, because it drops sums ≤ 0 (`ledger_postings.rs:1176-1179`), so a third port method answers it:

  ```rust
  // api/crates/xc-cash/src/domain/posting.rs, on `CashPostings` (added 2026-10-15 for LG-6.r5 #1044)
  fn activity_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, place: &'a CashPlace)
      -> BoxFuture<'a, AppResult<Option<Vec<(CurrencyCode, MoneyDelta)>>>>;
  ```

  - **Read-only.** It never creates an account. `LedgerRepository::account_in` finds **or creates** an account, so it must not be used here: an account it created would turn "never posted" into "has an account". A missing account is `None`, and that is the whole point of the method.
  - **`None`** means the place has no account entry at all, in any currency. **`Some(vec)`** holds one entry per currency that has ever had an entry on that place, each the signed net (`Σ quantity_signed`, the same sum as `expected_in` before its clamp), **zeros included**, sorted by currency code so the output is stable.
  - **Not clamped.** A negative net is returned as it is. The caller decides what a negative means. This is consistent with LG-6.r4's rule that a negative drawer is an error at the shift reader (§4.2.1, "How production builds a `PgShiftRepository`").
  - **One caller:** the snapshot's SAFE output (`compat/v2_json.rs` / `snapshot.rs`, LG-6.r5). It prints `null` for `None` and the per-currency map otherwise, and never reads the ledger tables itself.
  - **Default body for test doubles.** The trait gives `activity_in` a default that answers `AppError::Internal("activity_in is not implemented by this CashPostings")`, so the existing test double (`api/tests/tx5_common/mod.rs:94`) compiles unchanged. `LedgerCashPostings` (`ledger_postings.rs`) overrides it with one read of `cash_accounts` ⨝ `ledger_entries` for the place's account kind and owner.

**Not covered by `ShiftBalances`: the `stock_movements` readers.** `views/admin_cash_movements.rs` (`:64-92`) lists `stock_movements` rows with a free-text `reason`. So do `ShiftRepository::movements`/`shift_movements`/`find_movement_by_request_id` (`shift_repository.rs:397`, `:444`, `:631`), called by `compat/snapshot.rs:101`, `v3/actions/shift_valuation.rs:85`, `compat/abandon_request.rs:198` and `service.rs:253`, `:434`. `ledger_postings` has no reason (§3.1), and after CA-10 the manual moves it lists are `cash_transfers` (#582).
- **Ruled A by the lane controller (2026-10-10, #553 issuecomment after 6095872028):** after CA-10 #626, these readers list `cash_transfers` with their dispatch/receive state. `adminMoveStock` (`ShiftService::insert_movement`) retires with CA-10. The reason stays on the row where it is written, and `ledger_postings` gets no copied column.
- LG-6.r6 does this. LG-6.m waits for it.

**The shift writers.** `xc-shift`'s `ShiftService` still writes `shift_balances` through `ShiftRepository` (`insert_shift` `service.rs:229`, `update_expected` `:307`, `:411`, `close_shift` `:360`), separately from the posting port. Its `stock_movements` write (`insert_movement` `:305`) retires with CA-10 (the ruling above). The cash design keeps `ShiftService` (`cash-shift-transfer.md:364`).
- After LG-4 those writes feed tables nothing reads. LG-6.r4 removes the `shift_balances` writes and points `ShiftRepository`'s balance read (`load_shift_balances`) at `ShiftBalances`.
- Each removed write must already be posted through `CashPostings` (LG-2/LG-4). A write with no ledger counterpart stops the child; it does not get a new posting there.

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

### 4.6 A reversal (#552, `reversal.md` V-6)

A reversal values its foreign leg at the **original line's frozen cost**, not at today's average. The caller says so
with `reverses`; `LedgerCashPostings` reads that line's `cost_basis_thb` (call it `C`) and its `shop_side`, inside
the posting's transaction:

- **Reversing a BUY** (the shop gives back the `q` it bought): `DISPOSE` of `q` at `C`, not at the average. The position
  becomes `quantity − q`, `cost_thb − C`. The foreign entries carry `±C`.
- **Reversing a SELL** (the shop takes back the `q` it sold): `ACQUIRE` a lot of `q` with `unit_cost_thb = C ÷ q`
  (8 places HALF_UP) and `cost_thb_signed = +C` exactly. The position becomes `quantity + q`, `cost_thb + C`.
- **The last unit:** when a reversed BUY takes the position to exactly zero, the whole remaining `cost_thb` is moved,
  as §4.3's last-unit rule, so no crumb is left. `difference_thb = moved − C`; it is `0` in every other case.
- **Negative stock:** with an approved `NEGATIVE_STOCK` request (§5.1) the dispose is still at `C`; no average is
  needed.
- **What it returns:** one `ReversedCost` per request with `reverses`, and **no `LineCost`**. `LedgerCashPostings`
  writes nothing on `transaction_lines`, so the original's figures are never rewritten (`reversal.md` V-2). The caller
  records `difference_thb` on its own row.
- **Refusals** (`validationFailed`): `line` and `reverses` both set; `reverses` on a THB request; `reverses` with a
  `source_type` other than `REVERSAL`; a referenced line whose `cost_basis_thb` is NULL (not completed through the
  ledger).
- THB requests carry neither field and move at face value, as in §4.3.

Owned by **LG-2**, with the rest of the port's final shape (§10).

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

`api/crates/xc-cash/src/application/reconcile/` (§6.1), run by a test and by F01's integrity row:
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

### 6.1 Shape: one core, one file per check (LG-7, added 2026-10-10)

The five checks are five variants of one job, so they share one core and each lives in its own file (♻️). The
first two lines of this section's old text named a single `reconcile.rs`; the module path `application::reconcile`
is unchanged.

```rust
// api/crates/xc-cash/src/application/reconcile/mod.rs   (LG-7.0)
pub struct ReconcileScope { pub organization_id: String, pub branch: Option<BranchId>, pub period: Option<(NaiveDate, NaiveDate)> }
pub struct Offender { pub key: String, pub detail: String }     // key: posting id, or "<branch>:<currency>", …
pub struct CheckResult { pub id: u8, pub ok: bool, pub offenders: Vec<Offender> }
pub struct Report { pub checks: Vec<CheckResult> }            // in id order
pub trait ReconcileCheck: Send + Sync {
    fn id(&self) -> u8;                                          // 1..=5, the numbers of §6
    fn run_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, scope: &'a ReconcileScope)
        -> BoxFuture<'a, AppResult<Vec<Offender>>>;
}
pub struct Reconcile { /* Vec<Arc<dyn ReconcileCheck>> */ }
impl Reconcile {
    pub fn new(checks: Vec<Arc<dyn ReconcileCheck>>) -> Self;
    pub fn standard() -> Self;                                   // the five, one line each
    pub async fn run(&self, pool: &PgPool, scope: &ReconcileScope) -> AppResult<Report>;
}
// api/crates/xc-cash/src/application/reconcile/{balanced, positions, places, margin, valuation}.rs   (LG-7.1–7.5)
```

- **One snapshot.** `run` opens one `REPEATABLE READ READ ONLY` transaction and runs every check in it, so check 3
  cannot see a posting without its position movement. It writes nothing.
- **Each check is one SQL statement**, a `pub const QUERY: &str` in its file, that returns the offender rows (empty =
  passed). LG-6.m's migration precheck runs the same five statements; a migration cannot import Rust, so it carries a
  copy, frozen at the version it ships.
- **What ties the copy to the constants (LG-6.m #1047).** The precheck runs the five copies as five separate
  statements, and on the first one that returns a row it `RAISE`s `'reconcile check <id> failed'`. LG-6.m's test seeds
  six databases: one clean, and one per check with a single hand-broken row that only that check catches (the same
  cases as LG-7.1–7.5's tests). On each, it runs both the migration precheck and `Reconcile::standard().run`, and it
  asserts the same outcome per check: the clean seed passes both, and seed *k* fails the precheck naming check *k*
  while `run` reports check *k*, and only check *k*, as failed. A copy that drifts from its `QUERY` disagrees on at
  least one seed, so the test goes red.
- **Registration.** Each check's ticket adds its `pub mod` line and its one line in `standard()`. Those one-line
  registrations are not reserved (lane controller's rule on #564), so the five check tickets touch disjoint files.
- **Ordering and errors.** `Report.checks` is in id order whatever order `standard()` lists them. A check whose query
  fails fails `run` with `AppError::Database`; a failed check is `ok: false` with its offenders, never an error.
- **Who shows it.** No separate admin view: F01's integrity row (`executive-finance.md` §5, FN-2) reads
  `Reconcile::run` for the branch behind `FinanceReconcile`. The old `views/cash_reconcile.rs` line of §10 is dropped.
- **The org-wide run** (`branch: None`) has no screen of its own: it is the reconcile job's own output and LG-6.m's
  precheck (lane controller's ruling on PR 1053, 2026-10-10: only this design and #927 had asked for a separate view;
  no Blueprint item or owner decision did). An org-wide screen, if the owner asks for one, is a new ticket.

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
  class CashPostings { <<trait xc-cash domain/posting.rs (CA-3; shape LG-2)>> +post_in() +expected_in() +activity_in(place) Option~Vec~(CurrencyCode, MoneyDelta)~~ }
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
  class Reconcile { <<xc-cash application/reconcile/mod.rs (LG-7.0)>> +run(pool, scope) Report +standard() }
  class ReconcileCheck { <<trait xc-cash application/reconcile/mod.rs>> +id() +run_in(tx, scope) Offender[] }
  class ReconcileChecks { <<reconcile/balanced, positions, places, margin, valuation (LG-7.1-7.5)>> +QUERY }
  class Report { <<xc-cash application/reconcile/mod.rs>> +checks CheckResult[] }
  class CheckResult { <<xc-cash application/reconcile/mod.rs>> +id u8 +ok bool +offenders Offender[] }
  class CompleteExchange { <<xc-exchange application/transaction/complete.rs (#551 TX-5)>> }
  class CashCountService { <<xc-cash application/count (#582 CA-3)>> }
  class ApprovalService { <<xc-approval AP-1>> }
  class ShiftBalances { <<trait xc-shift domain/shift/repositories/shift_balances.rs (LG-6.0)>> +shift_balances(shift) ShiftBalance[] }
  class PgLedgerShiftBalances { <<xc-cash infrastructure/ledger/shift_balances.rs (LG-6.0)>> }
  class ShiftBalancesReaders { <<views admin_cash, admin_shifts, devices; xc-shift ShiftRepository (LG-6.r1-r4)>> }

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
  Reconcile --> ReconcileCheck
  Reconcile --> Report : run
  Report --> CheckResult
  ReconcileChecks ..|> ReconcileCheck
  PgLedgerShiftBalances ..|> ShiftBalances
  ShiftBalancesReaders --> ShiftBalances : shift_balances
```

### 8.1 Who provides, who uses

| Provided by | Used here | If it has not landed |
|---|---|---|
| #582 CA-2/CA-3: the `xc-cash` crate, `CashPostings`, `InterimCashPostings`, counts | §4.1, §4.4, §5.2 | LG-1 starts after CA-3 |
| LG-1: `cash_accounts`, `ledger_postings`, `ledger_entries`; #582 CA-1: `cash_counts`, `cash_count_totals` | `PgLedgerShiftBalances` (§4.2.1) | LG-6.0 starts after LG-4 |
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

**2. `api/migrations/<a timestamp after main's newest>_ledger_opening_balances.sql`** (LG-4, the cutover)
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
| LG-0 | `BranchTx::run_pair(a, b, f)`: one transaction, both `branch_versions` rows created if absent and locked `FOR UPDATE` in the same order (`MAIN` first, then ascending `branch_id`), both versions bumped on commit; `a == b` behaves as `run` | `api/crates/xc-kernel/src/branch_tx.rs` (one new method beside `run`) and its tests | this design |
| LG-1 | tables and domain: accounts, postings, entries, the balance trigger; positions, lots, movements; `WeightedAverage` | `xc-cash/src/domain/{ledger,inventory}/**`, `xc-cash/src/infrastructure/{ledger,inventory}/**` (new); migration 1 | #582 CA-3 |
| LG-2 | `LedgerCashPostings` and the port's final shape (§4.1), including the reversal rule (§4.6) | `xc-cash/src/domain/posting.rs` (shape), `xc-cash/src/infrastructure/ledger_postings.rs` (new) | LG-0, LG-1 |
| LG-3 | the two approval subjects and `CountDifferenceGate`; `cash_limits` commands | `xc-cash/src/application/ledger_approval/{negative_stock,count_difference}.rs`, `http/v3/commands/ledger/limits.rs` (outside CA-8's `commands/cash/**`) | LG-1, AP-1 |
| LG-4 | cutover, in three ordered commits (lane controller's rulings on #929). **1:** build `LedgerCashPostings` wherever the port is built, remove `InterimCashPostings`, `TransferFlow::receive` onto `run_pair`, migration 2. **2:** the OPEN count's OPENING `SAFE → DRAWER` and the hand-over's `DRAWER → DRAWER` (§4.4 rows 1, 2), a count's expected read from the ledger, `adminOpenShift` (refused when the safe cannot cover it) and `adminMoveStock` post. **3:** `CashPlace::Variance(branch)`; a count's difference and `adminCloseShift`'s post `DRAWER ↔ VARIANCE`, and a short transfer's reconcile (`reconcile_on`) posts `IN_TRANSIT → VARIANCE(sender)`, each an `ADJUST` at average, never a `DISPOSE`/`ACQUIRE` (§4.4 rows 6, 7) | `xc-cash/src/infrastructure/interim_postings.rs` (deleted), `xc-cash/src/infrastructure/{mod.rs, ledger_postings.rs}`, `xc-cash/src/domain/posting.rs`, `xc-cash/src/application/count/count_service.rs`, `xc-cash/src/application/handover/handover_service.rs`, `xc-cash/src/application/transfer/transfer_flow.rs`, `api/src/presentation/http/v3/commands/complete.rs`, `api/src/application/workflows/exchange/complete.rs` (a comment), `api/src/application/compat/rpc.rs` (`shift_action`); migration 2 | LG-0, LG-2, #551 TX-5, #582 CA-5, #582 CA-7 |
| LG-5 | the count-difference hook in `CashCountService::verify` | `xc-cash/src/application/count/count_service.rs` (cash design §5, `CashCountService`; the one call only) | LG-3, CA-3 |
| LG-6.0 | `ShiftBalances` (§4.2.1) | `xc-shift/src/domain/shift/repositories/shift_balances.rs`, `xc-cash/src/infrastructure/ledger/shift_balances.rs` (both new) | LG-4 |
| LG-6.r1–r3 | `admin_cash`, `admin_shifts`, `devices` read `ShiftBalances` | one view file each | LG-6.0 |
| LG-6.r4 | `ShiftRepository`'s balances read `ShiftBalances`; `ShiftService`'s `shift_balances` writes removed (§4.2.1) | `xc-shift/src/infrastructure/shift/shift_repository.rs`, `xc-shift/src/application/shift/service.rs`, `xc-shift/src/domain/shift/repositories/shift_repository.rs` | LG-6.0, CA-1's opener lookup (#1025) |
| LG-6.r5 | `ops_state.safe` neither read nor written; whoever still shows the safe after CA-10 (today `compat/v2_json.rs:493` and `CustodyPages`' floor facts) reads `CashPostings::activity_in(Safe(branch))` from `api` (`null` when `None`, the map otherwise; §4.2.1), because `xc-shift` cannot call `xc-cash` | `xc-shift/src/infrastructure/custody/{custody_pages.rs, custody_repository.rs}`, `xc-shift/src/domain/custody/{state.rs, repositories/custody_pages.rs}`, `api/src/application/compat/v2_json.rs`, `xc-cash/src/domain/posting.rs` (`activity_in` with its default), `xc-cash/src/infrastructure/ledger_postings.rs` (its override) | LG-4, CA-10 (re-measured after it) |
| LG-6.r6 | the `stock_movements` readers (`admin_cash_movements`, `ShiftRepository`'s movement methods and their callers) list `cash_transfers` (ruling A, §4.2.1) | `views/admin_cash_movements.rs`, `xc-shift` shift repository's movement methods, `compat/snapshot.rs`, `v3/actions/shift_valuation.rs`, `compat/abandon_request.rs` | LG-4, CA-10, CA-5 |
| LG-6.m | migration 3 (§9); its precheck is tied to LG-7's `QUERY` constants (§6.1) | the migration only | every LG-6.r child; LG-7.1–7.5; `git grep` finds no reader |
| LG-7.0 | the reconcile core (§6.1): `ReconcileCheck`, `Reconcile`, `Report` | `xc-cash/src/application/reconcile/mod.rs` | CA-3 (the crate) |
| LG-7.1 | check 1, balanced postings | `reconcile/balanced.rs` | LG-7.0, LG-1 |
| LG-7.2 | check 2, positions = movements | `reconcile/positions.rs` | LG-7.0, LG-1 |
| LG-7.3 | check 3, cash places = positions | `reconcile/places.rs` | LG-7.0, LG-2 (its transfer cases post through the port) |
| LG-7.4 | check 4, margin = sales − cost | `reconcile/margin.rs` | LG-7.0, TX-2 (`transaction_lines`) |
| LG-7.5 | check 5, valuations sum to zero | `reconcile/valuation.rs` | LG-7.0, LG-1 |

### 10.1 Up-front tests (🧪)

| Ticket | Behaviour locked |
|---|---|
| LG-0 | `run_pair(A, B)` and `run_pair(B, A)` started at once both commit (no deadlock, measured with a barrier, repeated); both versions move by one on commit and neither on rollback; `run_pair(A, A)` = `run(A)` |
| LG-1 | an unbalanced posting (per currency) is refused at commit; the same `(source, key)` twice is one posting; `WeightedAverage`: buy 100 USD at 3,200 then 100 at 3,300 ⇒ average 32.50; selling 50 costs 1,625.00; selling the last unit takes the whole remaining cost |
| LG-2 | `v2_shift_cases.json` and `v2_stock_cases.json` give the same `expected` through `LedgerCashPostings` as through `InterimCashPostings`; a BUY posts a lot and NULL margin; a SELL freezes `cost_basis_thb` and `realized_margin_thb = thb_amount − cost`; non-cash THB goes to `BANK`, not the drawer; a move inside a branch changes no position; **§4.6 after the average has moved**: buy 100 USD at 32.00, buy 100 at 34.00, then a `reverses` request on the first buy disposes 3,200.00 (not the average 3,300.00) and leaves 100 at 3,400.00; a reversed SELL acquires a lot at the original `cost_basis_thb` whatever the average is; buy 100 at 32.00, sell 50, buy 50 at 34.00, reverse the first buy ⇒ the position ends at 0 / 0.00 and `difference_thb = 100.00`; `line` and `reverses` together ⇒ `validationFailed`; the original line is unchanged |
| LG-3 | a shortfall without approval ⇒ `stockInsufficient`; with an APPROVED `NEGATIVE_STOCK` request ⇒ posted and the position goes negative; a teller cannot approve; a difference above the limit waits for FINANCE; a NULL limit sends every non-zero difference to FINANCE |
| LG-4 | migration 2 on a seeded DB: every OPEN shift's expected equals its drawer balance after the backfill; reconciliation (§6) is clean; a BRANCH transfer received through `TransferFlow::receive` moves both branches' versions, and two opposite receipts at once both complete |
| LG-7 | each of the five checks fails on a hand-broken row and passes on a clean ledger; check 3 holds for a BRANCH transfer DISPATCHED and not received (sender's `IN_TRANSIT` and position both carry it), after a short receipt before reconcile, and after reconcile (`VARIANCE` + `ADJUST`); check 5 holds for a SELL with a margin; **a receipt moves the sender's and the receiver's `branch_versions`**; two opposite receipts (A→B and B→A, both USD) run at once both complete |

---


### Tickets as opened (reconciled 2026-10-11)

> Owner's order 2026-10-11: every opened ticket must appear in its design's ticket list; a split is either listed here (this PR, lane controller claude/5edf7bb5) or folded back into its parent. Measured on `origin/main` `5a2b0d19` against all 605 issues. No new ticket may be opened outside this list.

| Code | Issue | State 2026-10-11 | Listed because |
|---|---|---|---|
| LG-3b | #926 | open | the `cash_limits` commands half of the LG-3 row, split at cutting (LG-3 #925 closed) |
| LG-6 (parent) | #930 | open | holder only for LG-6.0 … LG-6.m |
| LG-7 (parent) | #927 | open | holder only for LG-7.0 … LG-7.5 |
| COUNT_DIFFERENCE follow-up (org-wide FINANCE) | #1920 | open (2026-10-15) | `may_decide` ⇔ "FINANCE (organisation-wide or of the branch)": `CountDifferencePolicy` refused an organisation-wide FINANCE. Shared with the reversal policy (`reversal.md` §8) |

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
