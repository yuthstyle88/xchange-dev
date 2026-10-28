# Desk walk-in enters the queue (XC-WALKIN-QUEUE)

> Design by claude/e2926a49 (sub-controller, xchange BE lane under claude/553782), 2026-10-28 ICT.
> Measured on `origin/main` `a6ad18474`; rework re-checked on `1f364a09`. Brief: #564 issuecomment-6456885111.

## 0. The owner's rule

*"G1 ก็ต้องเข้าคิว เสมือน จองเหมือกกัน"* (recorded by the FE lane on #5055, issuecomment-6456873918): a customer served
straight at the desk enters the queue, the same way a booking does.

Rules already in force that this design keeps:
- A kiosk request or a booking reserves a queue place, never a rate. The desk always prices at the current rate (G7, `create.rs:252-285`).
- A ticket is a place in a line, not an order. A ticket's completion is not a transaction's completion (`queue-tickets.md` Q1).
- Cancels need no approval and no limit.
- Server texts are Thai only in feature work. Language work is separate.

## 1. The gap (measured)

- The desk's booking step only moves on. `web/src/screens/desk/steps/booking/step.tsx:15-26` checks the name, the staff and the value date, then calls `store.go(3)`. It writes nothing.
- The save sends `bookingId` / `kioskRequestId` only when the sale came from an entry (`web/src/screens/desk/save.ts:180-181`, read from `extras`).
- So a sale with neither has no `queue_tickets` row. Nothing in the queue, the estimate or the calling display knows the customer was served.

## 2. Existing entry paths (all through one writer)

Every ticket is written by **`TicketIssuer::issue_in`** (`api/crates/xc-queue/src/application/issue/ticket_issuer.rs:160-308`). It takes an `IssueSource` (`xc-queue/src/domain/ticket/issue_source.rs:9-18`), and in order:
1. returns the ticket already stored under the idempotency key, or refuses `idempotencyConflict` when that key belongs to another source (`:170-180`);
2. returns the source's existing ticket, for a kiosk request or a booking (`:183-206`);
3. refuses `queueClosed` when the queue is closed, using no number (`:209-211`);
4. takes the next number of (branch, local date, line), inserts a WAITING ticket and its `ISSUE` event, and refreshes the estimate (`:213-306`).

| Path | Who | Source | Where it calls the core | Line | Starts as |
|---|---|---|---|---|---|
| Kiosk/web request confirm (customer submits on the tablet or the web) | the kiosk / member | `Kiosk { request_id }`, key `kiosk-confirm:<id>` | `xc-kiosk/src/application/kiosk/customer.rs:281` (`issue_by_token_with_sessions`, `:243`; route `issueByToken`, `api/src/presentation/http/kiosk.rs:769-782`) | WALK_IN (`W`) | WAITING |
| Kiosk request accepted at the desk | TELLER | (existing ticket) | `xc-kiosk/src/application/kiosk/desk.rs:197` → `TicketIssuer::start_for_kiosk_in` (`ticket_issuer.rs:89-128`): `QueueDesk::call_in` then `start_in` | — | → SERVING for that teller |
| Online booking check-in | TELLER (`bookingCheckIn`) / kiosk (`kioskBookingCheckIn`) | `Booking { booking_id }` | `xc-booking/src/application/check_in.rs:188` | BOOKING (`B`), called first (`call_order.rs:13-18`) | WAITING |
| App "take a number" | the signed-in customer (`queueTicketTake`, `POST /api/app/queue/tickets`) | `App { customer_id }`, one active per branch and day (`ticket_issuer.rs:225-241`; index `migrations/20261219140000_queue_one_active_app_ticket.sql`) | `api/src/presentation/http/v3/commands/queue_ticket_take.rs:70-76` | WALK_IN | WAITING |
| Desk board "issue a walk-in number" | TELLER/BRANCH_MANAGER (`ticketIssueWalkIn`, `POST /api/queue/tickets`) | `Desk` | `api/src/presentation/http/v3/commands/queue.rs:240-265` | WALK_IN | WAITING |
| Flutter queue machine (printer) | the queue machine | (prints the ticket `issueByToken` answered) | `app/lib/features/queue_machine/infrastructure/queue_machine_api.dart:35` (`ticketEscpos`) | — | — |

Nothing finishes a ticket when a sale completes. A ticket reaches COMPLETED only through the board's `ticketFinish`
(`QueueDesk::finish_in`, `queue_desk.rs:729`). The day close cancels what is left (`queue_day_hook.rs:110`).

**Conclusion: reuse the core.** A desk walk-in is `IssueSource::Desk`, which already exists for exactly this customer
("a teller at the desk, for a customer without the tablet", `issue_source.rs:15-16`). It is issued through `issue_in`
and started through `QueueDesk::call_in` + `start_in`, the same composition `start_for_kiosk_in` already uses. No new
source, no new line, no second writer of `queue_tickets`.

## 3. Decisions

| # | Decision | Why |
|---|---|---|
| W1 | **One new command, `ticketServeWalkIn`** (`POST /api/queue/tickets/serve`), called by the desk's booking step when the sale has no `bookingId` and no `kioskRequestId`. It answers the ticket the customer at the counter is served under. `ticketIssueWalkIn` (a WAITING number from the board) stays as it is. | The board's command issues a number for someone who will wait. The sale's command is for someone who is already at the counter, so its ticket must start at once (W3). |
| W2 | **The save sends the queue ticket id** in a new field `queueTicketId`, not `bookingId`. | A `bookings` row needs a slot and a booking policy (`booking-slots.md` B11). It would rank in the BOOKING line and be called before people already waiting (`call_order.rs:13-18`). Inventing one would be a second, parallel queue entry. The ticket is the queue entry the owner asked for. |
| W3 | **The ticket is SERVING for the caller when the command answers.** It is issued WAITING, then called and started in the same transaction (`call_in`, `start_in`). | The customer is already being served. It is never WAITING outside the transaction, so **call order for others does not change**: call-next reads WAITING tickets only. It does take the next `W` number, so later walk-ins get the number after it. Earlier numbers keep theirs. |
| W4 | **What the command answers, in order:** (1) the ticket stored under this sale's key (a replay); (2) the caller's held CALLED/SERVING **walk-in** ticket that has no booking and no kiosk request and was not issued by another desk sale: started if CALLED, then returned (the customer at the counter is the one the teller called from the board); (3) the customer's active `App` ticket today in this branch, called and started for the caller; (4) a new `Desk` ticket, called and started. A held **booking or kiosk** ticket, or a held ticket another desk sale issued, refuses `alreadyServing`. | The teller serves one ticket at a time (`queue_desk.rs:95-108`). (2) and (3) avoid two tickets for one person. A held booking or kiosk ticket means the desk should open that entry instead. |
| W5 | **Idempotency: one entry per desk sale.** The web adds one key per desk sale and keeps it until `reset()` (§7.1). The desk store has no per-sale key today: `newKey()` (`desk-store.ts:299`) makes a fresh key for each quote call. The ticket stores it as `desk-sale:<key>`. The same key answers the same ticket. The same key on a different source refuses `idempotencyConflict` (`ticket_issuer.rs:175-177`). | The prefix lets W4 (2) tell a ticket issued by a desk sale from one called off the board. |
| W6 | **The 10-minute cutoff applies.** A new ticket (W4 (3) and (4)) needs `ServiceWindow::allows_begin`, i.e. inside `[open, close − 10 min)` (`xc-shift/src/domain/custody/value_objects/service_window.rs:18-27`). It is read through `CustodyFactsPort::service_window_in` (`api/src/infrastructure/custody/custody_ports.rs:245`). A replay (W4 (1)) and an already-held ticket (W4 (2)) are answered at any time. | Issuing the entry begins the service, and beginning a transaction has the same cutoff (`branch_day/service.rs:503-505`). Answering the stored ticket keeps a retry from failing at 22:51. |
| W7 | **A closed queue refuses `queueClosed`**, as booking check-in and the kiosk confirm do (`ticket_issuer.rs:209-211`). | "Like a booking": the check-in of a booking refuses on a closed queue. **This blocks the sale while the queue is closed; the owner kept it (O-2).** |
| W8 | **Completing the sale finishes the ticket** (SERVING → COMPLETED, with its `FINISH` event and the estimate refresh), inside `CompleteExchange`'s transaction, through `WalkInDesk::finish_for_sale_in`. If the ticket is no longer SERVING (finished from the board, or ended at day close), the sale completes and the ticket is left alone. A draft that is never completed leaves the ticket SERVING; the teller finishes it from the board. | Without this, every walk-in ticket would stay SERVING until the day close, and W4's `alreadyServing` would stop the teller's next sale. Q1 still holds: the ticket never blocks the sale. |
| W9 | **The calling display (KS-5)** shows the number while it is CALLED/SERVING, and it enters "last 5 called" through its `CALL` event (`kiosk-shared-phone.md` D10). It shows numbers only. | Nothing new: a walk-in is a `W` number like any other. |

## 4. Class diagram

```mermaid
classDiagram
  class WalkInDesk {
    <<xc-queue application/issue/walk_in_desk.rs  NEW>>
    +new(issuer: TicketIssuer, desk: QueueDesk) WalkInDesk
    +serve_in(tx, actor: &Actor, branch: &BranchId, customer_id: Option~&str~, sale_key: &str, may_issue: bool, now) AppResult~QueueTicket~
    +finish_for_sale_in(tx, branch: &BranchId, ticket_id: &str, now) AppResult~bool~
  }
  class QueueError {
    <<xc-queue domain/ticket/queue_error.rs  CHANGED (skeleton)>>
    +OutsideServiceHours  code "outsideServiceHours"
  }
  class TicketIssuer {
    <<xc-queue application/issue/ticket_issuer.rs  unchanged>>
    +issue_in(tx, branch, source, key, now) AppResult~QueueTicket~
  }
  class QueueDesk {
    <<xc-queue application/desk/queue_desk.rs  unchanged>>
    +call_in(tx, actor, branch, id, now)
    +start_in(tx, actor, branch, id, now)
  }
  class ServeWalkIn {
    <<api presentation/http/v3/commands/queue_walk_in_serve.rs  NEW, CommandAction>>
    NAME = "ticketServeWalkIn"
    ROUTE = "/api/queue/tickets/serve"
    Input = QueueServeWalkInInput
    Output = QueueTicketDto
  }
  class QueueServeWalkInInput {
    <<same file, ts export web/src/shared/api/generated/queue.ts>>
    +customer_id: Option~String~
  }
  class TransactionCreateInput {
    <<api presentation/http/v3/commands/transactions.rs:101  CHANGED>>
    +queue_ticket_id: Option~String~
  }
  class CreateTransaction {
    <<api application/workflows/exchange/create.rs  CHANGED>>
    checks queueTicketId (§5)
  }
  class CompleteExchange {
    <<api application/workflows/exchange/complete.rs  CHANGED>>
    finishes t.queue_ticket_id
  }
  class Transaction {
    <<xc-exchange domain/transaction/transaction.rs:74  CHANGED>>
    +queue_ticket_id: Option~String~
  }
  ServeWalkIn --> WalkInDesk : serve_in
  ServeWalkIn --> CustodyFactsPort : service_window_in (W6)
  WalkInDesk --> TicketIssuer : issue_in(Desk, "desk-sale:key")
  WalkInDesk --> QueueDesk : call_in, start_in
  WalkInDesk --> QueueError : OutsideServiceHours (W6)
  CreateTransaction --> Transaction : queue_ticket_id
  CompleteExchange --> WalkInDesk : finish_for_sale_in (W8)
```

| Provides | Uses it |
|---|---|
| `WalkInDesk::serve_in` | `ServeWalkIn` |
| `WalkInDesk::finish_for_sale_in` | `CompleteExchange` |
| `TransactionCreateInput.queue_ticket_id`, `Transaction.queue_ticket_id`, column `transactions.queue_ticket_id` | `CreateTransaction`, `PgTransactionStore` (`xc-exchange/src/infrastructure/transaction/store.rs:95,122,217,411`), `CompleteExchange` |
| `ticketServeWalkIn` + `QueueTicketDto` | the desk web (booking step) |

`serve_in`'s `may_issue` is the command's W6 answer: `false` lets W4 (1) and (2) answer and refuses (3) and (4) with
`QueueError::OutsideServiceHours`. That variant is new: `QueueError` (`api/crates/xc-queue/src/domain/ticket/queue_error.rs:7-31`)
has no such refusal today. The skeleton adds the variant and its `code()` arm (`"outsideServiceHours"`), so no function ticket edits
that file. `From<QueueError> for AppError` already sends it to `AppError::Validation(code)`; WQ-2 adds the code to `queue.rs::refusal`,
which otherwise maps an unknown code to `unexpected`.
That keeps `xc-queue` free of `xc-shift`.

`finish_for_sale_in` writes the same transition and event as `QueueDesk::finish_in`, using `QueueTicket::apply(Finish)`. It skips the
on-duty and owner checks: the sale's completion is the authority, and the completer may be a manager. It answers `Ok(false)` when the
ticket is not SERVING.

## 5. Refusals

`ticketServeWalkIn` (status as `queue.rs:107-125` maps it; new rows marked):

| Case | Code | HTTP |
|---|---|---|
| no `QueueOperate` in the branch | out of scope (`queue.rs:109`) | 403 |
| same key, another source | `idempotencyConflict` | 409 |
| a new ticket at or after `close − 10 min`, or outside hours, or no hours set | **`outsideServiceHours`** (new `QueueError` variant, new code in `refusal`; text `ปิดรับรายการใหม่แล้ว`, as `CustodyError::OutsideServiceHours`) | 409 |
| the queue is closed | `queueClosed` | 409 |
| the caller has no OPEN shift in the branch | `noOpenShift` | 409 |
| the caller's counter is PAUSED | `counterPaused` | 409 |
| the caller holds a booking or kiosk ticket, or another desk sale's ticket | `alreadyServing` | 409 |
| lost race on the ticket | `staleVersion` | 409 |
| `customerId` is not a customer of the organisation | `validationFailed`, field `customerId` | 400 |

`POST /api/transactions` with `queueTicketId` (new `validationFailed` rows, field `queueTicketId`):

| Case | Answer |
|---|---|
| `queueTicketId` together with `bookingId` or `kioskRequestId` | `validationFailed:queueTicketId` |
| no such ticket in the branch | `validationFailed:queueTicketId` |
| the ticket is not SERVING, or not held by the caller | `validationFailed:queueTicketId` |
| the ticket has a booking or a kiosk request | `validationFailed:queueTicketId` |
| another non-CANCELLED transaction already names it | `staleVersion` (the unique index of §6) |

`queueTicketId` joins `payload_hash` (`create.rs:432-450`), so the same key with another ticket is `idempotencyConflict`, as for every other field.

## 6. Data migration (owner approved 2026-10-28, write and merge only; see §10)

One nullable column and one index. No new status, and nothing in `queue_tickets` changes.

```sql
ALTER TABLE transactions ADD COLUMN queue_ticket_id uuid NULL REFERENCES queue_tickets (id);
CREATE UNIQUE INDEX transactions_one_per_queue_ticket
    ON transactions (queue_ticket_id)
    WHERE queue_ticket_id IS NOT NULL AND status <> 'CANCELLED';
```

1. **Existing rows:** `queue_ticket_id` is NULL on every existing transaction. No sale before this change had a desk ticket, and a booking or kiosk sale keeps finding its ticket through `booking_id` / `kiosk_request_id`. No row breaks the index, because they are all NULL.
2. **Backfill:** none. Nothing links a past sale to a ticket, so there is nothing to copy.
3. **Deploy order:** migration first, then the binary. The old binary never names the column, so it keeps working on the new schema.
4. **Reversal:** `api/migrations/` has no down files. A later migration that drops the index and the column would lose only the sale-to-ticket link. Tickets and transactions stay.
5. The migration slot is taken with `scripts/migration-slot.sh next` right before `pr_open`.

The owner approved writing and merging this migration (O-1). Deploying it is a separate owner order.

## 7. FE contract (for the FE lane's ticket)

1. **Booking step** (`web/src/screens/desk/steps/booking/step.tsx`): on submit, when `store.entry()?.payloadExtras()` has no `bookingId`, `onlineBookingId` or `kioskRequestId`:
   - call `ticketServeWalkIn` (`POST /api/queue/tickets/serve`, body `{ customerId: store.customerId() || null }`) with one Idempotency-Key per desk sale, kept until `reset()`;
   - on success store the ticket (`id`, `displayNumber`), show the number, then `store.go(3)`;
   - on a refusal show the worded refusal and stay on step 2.
   A sale that has an entry calls nothing new.
2. **Save** (`web/src/screens/desk/save.ts:173-196`): `createBody` adds `queueTicketId: <the stored id> | null`. **This part ships only after WQ-3 merges.** Before that the field does not exist on the server.
3. **Types:** `QueueServeWalkInInput` and `QueueTicketDto` come from `web/src/shared/api/generated/queue.ts`. `TransactionCreateInput.queueTicketId` comes from `…/generated/transactions.ts`. Both are generated by ts-rs, and their fixtures by `npm run gen:fixtures`, in the BE ticket that changes the Rust type (§9); never hand-written.
4. **Refusal texts:** `outsideServiceHours`, `queueClosed`, `noOpenShift`, `counterPaused`, `alreadyServing`, in Thai only.

## 8. Up-front tests (written by the sub-controller before release)

| Test file | Rows |
|---|---|
| `api/crates/xc-queue/tests/walk_in_desk_test.rs` (DB) | new Desk ticket is SERVING for the caller, held by them, with `ISSUE`, `CALL`, `START` events; the waiting tickets' call order is unchanged (call-next still takes the earlier `B`, then `W`); the same key twice gives one ticket; the caller's held CALLED walk-in from the board is started and returned, with no new number; the customer's WAITING `App` ticket is served instead of a new one; a held booking ticket, a held kiosk ticket, or a held ticket from another desk sale refuses `alreadyServing`; `may_issue = false` refuses a new ticket with `outsideServiceHours` but answers a replay; closed queue refuses `queueClosed` and uses no number; `finish_for_sale_in` moves SERVING → COMPLETED with a `FINISH` event, and answers `false` and changes nothing on a COMPLETED / CANCELLED ticket |
| `api/tests/walkin_serve_command_test.rs` (DB, through the route) | 200 with `status: "SERVING"` and a `W` number; replay with the same Idempotency-Key gives the same body; at `close − 10 min` → 409 `outsideServiceHours`; queue closed → 409 `queueClosed`; a caller without `QueueOperate` → 403; the policy row `ticketServeWalkIn` exists with `QueueOperate` |
| `api/tests/walkin_sale_link_test.rs` (DB, WQ-3, after the migration is approved) | create with `queueTicketId` stores it; each `validationFailed:queueTicketId` row of §5; a second create naming the same ticket → `staleVersion`; complete moves the ticket to COMPLETED in the same transaction; complete still succeeds when the ticket was finished from the board first; a sale with `bookingId` is unchanged |

## 9. Tickets

| Ticket | Class | Test (provided, §8) | Waits for |
|---|---|---|---|
| Skeleton (sub-controller, not a ticket) | — | writes all three test files | design merged |
| WQ-1 `WalkInDesk` | logic, Argon | `api/crates/xc-queue/tests/walk_in_desk_test.rs` | the skeleton on `feat/walkin-queue` |
| WQ-2 `ticketServeWalkIn` command + policy row + `outsideServiceHours` mapping | logic, Argon | `api/tests/walkin_serve_command_test.rs` | the skeleton |
| WQ-3 sale link + finish on complete | guarded, Argon | `api/tests/walkin_sale_link_test.rs` | the PR that changes `complete.rs` for #5315 on main |
| FE part 1 (booking step) / part 2 (`queueTicketId` in the save) | FE lane | the FE lane's own | WQ-2 / WQ-3 on main |

File scopes, one path per line as the queue reads them:

- **Skeleton** (signatures with `todo!()`, the `walk_in_desk` `mod` line (the command file is picked up by `build.rs`, `v3/mod.rs:31`), the `QueueError` variant, the regenerated TS files for the new input type, and the three test files):
  ```
  api/crates/xc-queue/src/domain/ticket/queue_error.rs
  api/crates/xc-queue/src/application/issue/mod.rs
  api/crates/xc-queue/src/application/issue/walk_in_desk.rs
  api/src/presentation/http/v3/commands/queue_walk_in_serve.rs
  web/src/shared/api/generated/queue.ts
  web/src/shared/testing/fixtures/queue.ts
  api/crates/xc-queue/tests/walk_in_desk_test.rs
  api/tests/walkin_serve_command_test.rs
  api/tests/walkin_sale_link_test.rs
  ```
- **WQ-1:**
  ```
  api/crates/xc-queue/src/application/issue/walk_in_desk.rs
  ```
- **WQ-2** (the generated pair is listed because ts-rs rewrites it whenever the input type's shape changes):
  ```
  api/src/presentation/http/v3/commands/queue_walk_in_serve.rs
  api/src/presentation/http/v3/commands/queue.rs
  api/crates/xc-auth/src/domain/policy/areas/queue.rs
  web/src/shared/api/generated/queue.ts
  web/src/shared/testing/fixtures/queue.ts
  ```
- **WQ-3** (`<slot>` is taken with `scripts/migration-slot.sh next` when the ticket is released, and written into the ticket as a full path):
  ```
  api/migrations/<slot>_transactions_queue_ticket_id.sql
  api/src/presentation/http/v3/commands/transactions.rs
  api/src/application/workflows/exchange/create.rs
  api/src/application/workflows/exchange/complete.rs
  api/crates/xc-exchange/src/domain/transaction/transaction.rs
  api/crates/xc-exchange/src/infrastructure/transaction/store.rs
  web/src/shared/api/generated/transactions.ts
  web/src/shared/testing/fixtures/transactions.ts
  ```
- **FE parts:** `web/src/screens/desk/steps/booking/step.tsx` and `web/src/screens/desk/save.ts`, in the FE lane's tickets.

WQ-1 and WQ-2 have disjoint files and are released together. WQ-3 touches neither's files.

## 10. Owner decisions

Answered by the owner on 2026-10-28 at 10:0x ICT, through the BE lane controller claude/553782 (PR #5326 issuecomment-6460747991).

| # | Question | Decision |
|---|---|---|
| O-1 | Approve the migration of §6 (`transactions.queue_ticket_id` + unique index)? | **Approved: write and merge only.** Deploy is a separate owner order. WQ-3 no longer waits on approval. |
| O-2 | A closed queue refuses the walk-in entry (W7), so the desk cannot sell while the queue is closed and the branch is open. Keep that, or issue the entry anyway? | **Keep the refusal** (`queueClosed`), as booking check-in does. The desk opens the queue (`queueOpen`) at the start of the day. |

Out of scope: finishing a booking's or a kiosk request's ticket when its sale completes. Today the teller finishes it from the board. W8's step could serve them too later, through the same `finish_for_sale_in`.
