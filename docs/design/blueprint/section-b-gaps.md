# Section B back-end gaps: class, file and contract rows (S1–S7)

> Written by `claude/studio-claude-4` on the xchange BE lane controller's order (`claude/a90ab16c`, 2026-10-19),
> from the FE gap survey section B (#564 issuecomment-6262359159, `5e84220d`). Every fact is re-measured on
> `origin/main` **`7e99ed0a`**. The owner's rule stands: xchange follows the Blueprint fully
> (`docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`, "the Blueprint" below).

This document gives the seven rows the class, file and contract rows the owning designs do not have yet, so their
tickets can be cut against a merged diagram (📐). It **amends** the owning design of each row. Where it says
"amends", the owning design's text is read together with this file until the controller folds the row in.

| Row | Blueprint | Owning design (amended here) | Ticket class |
|---|---|---|---|
| S1 | §03 F01 *"ตั้ง exception"* | `executive-finance.md` §5.2 (`:221-222`); `cash-shift-transfer.md` §4.1 "not built here" (`:230-233`) | logic |
| S2 | §05 negative stock policy; desk "waiting for approval" | `ledger-inventory.md` §5.1 (`:418-425`); `quote-transaction.md` TX-5 rows (`:358`, `:393`) | guarded |
| S3 | §03 A04 *"รหัส ที่อยู่ เวลาเปิด ช่องทางติดต่อ สถานะ"* | `org-roles-branches.md` §6 (`:426-427`) | guarded (migration) |
| S4 | §15 C12 *"ช่วยกรอก planner และนำทาง; preview ก่อนยืนยัน"* | `customer-app.md` P11 (`:71`), APP-6 | logic |
| S5 | §15 C09 *"Pass list + … + queue"* | `queue-tickets.md` §4.4 (`:164`); `customer-app.md` (`:237`, `:373`) | logic |
| S6 | §15 C14 *"terms version"* | `customer-identity-consent.md` (`:163`) | logic |
| S7 | §15 C08 *"ค้นสาขา + service status + เส้นทาง + จองหรือรับคิว"* | `queue-tickets.md` Q4 (`:43`), Q10 (`:49`) | guarded (migration) |

## Class diagram

```mermaid
classDiagram
  direction LR
  class CashExceptionReview {
    <<xc-shift infrastructure/custody/exception_review.rs  NEW>>
    +review_in(tx, branch, exception_id, reviewer, review_note, now) AppResult~ReviewOutcome~
  }
  class append_cash_exception_in {
    <<xc-shift custody_repository.rs  EXISTS, called only>>
  }
  class CashExceptionCommands {
    <<api http/v3/commands/cash_exception_{review,raise}.rs  NEW>>
  }
  class NegativeStockGate {
    <<api application/workflows/exchange/negative_stock_gate.rs  NEW>>
    +approved_for_in(tx, transaction_id) Option~ApprovalRequestId~
    +open_after_refusal(pool, transaction_id, shortfall, actor, key) ApprovalRequestId
  }
  class CompleteExchange {
    <<api application/workflows/exchange/complete.rs  EXISTS>>
  }
  class ApprovalService {
    <<xc-approval  EXISTS: request(), find_by_type_and_key>>
  }
  class Branch {
    <<xc-rate domain/branch/entities/branch.rs  EXISTS, gains 2 fields>>
    +contact_channels Vec~ContactChannel~
    +status BranchStatus
  }
  class ContactChannel {
    <<xc-rate domain/branch/contact_channel.rs  NEW>>
    +kind ContactKind
    +value String
  }
  class AssistantAnswer {
    <<xc-app application/assistant/assistant_service.rs  EXISTS, gains intent>>
    +intent Intent
    +preview Option~PlannerEstimate~
  }
  class MyQueueTicketView {
    <<api http/v3/views/my_queue_ticket.rs  EXISTS>>
  }
  class PublicConsentTexts {
    <<api http/public/consent_texts.rs  NEW>>
  }
  class QueueTicketTake {
    <<api http/v3/commands/queue_ticket_take.rs  NEW>>
  }
  class IssueSource {
    <<xc-queue domain/ticket/issue_source.rs  EXISTS, gains App>>
  }
  CashExceptionCommands --> CashExceptionReview : review
  CashExceptionCommands --> append_cash_exception_in : raise
  CompleteExchange --> NegativeStockGate
  NegativeStockGate --> ApprovalService : request / find
  Branch *-- ContactChannel
  QueueTicketTake --> IssueSource : App
```

**No two rows share a file**, so the seven tickets run at once. Registration lines (`commands/mod.rs`, the
`views` registry, `http/public/mod.rs`, `custody/mod.rs`, `areas/mod.rs`) are not locked by any of them.

**Feature branches.** None of the files below is touched by `feat/lg6`, `feat/rbac-groups` or `feat/or7m`
(`git diff --name-only origin/main...origin/feat/<x>` at `7e99ed0a`). In particular S1 **must not edit**
`xc-shift/src/infrastructure/custody/custody_repository.rs` or `custody_pages.rs`, which `feat/lg6` holds: it calls
`append_cash_exception_in` and writes its update in its own new file.

---

## S1. F01: FINANCE reviews an open cash exception, or raises one by hand

**Gap** (`7e99ed0a`): no command reviews or raises an exception. `grep -rn "exceptionReview\|review_exception\|resolveException\|cashException" api/src/presentation/http/v3/commands` finds nothing. Rows written by a count, a
hand-over or a receipt stay `OPEN` for ever (`cash-shift-transfer.md:230-233`). `CashException` already has the
review fields (`xc-shift/src/domain/custody/entities/cash_exception.rs:7-24`: `status`, `reviewed_by`, `reviewed_at`).

| Command | Route | Permission | Body |
|---|---|---|---|
| `cashExceptionReview` | `POST /api/v3/commands/cash_exception_review` | `FinanceReconcile` (`executive-finance.md:287`, exists) | `{branch, exceptionId, note}` |
| `cashExceptionRaise` | `POST /api/v3/commands/cash_exception_raise` | `FinanceReconcile` | `{branch, reason, place: "DRAWER"|"SAFE", differences: [{currency, amount}]}` |

- `[required]` Both are `CommandAction`s, with `Idempotency-Key`, `Locks::One(branch)`, and one `audit_events` row in
  the same transaction: `CASH_EXCEPTION_REVIEW` / `CASH_EXCEPTION_RAISE`, `object_ref = 'cash_exception:<id>'`.
- `[required]` **Review** changes only an `OPEN` row of that branch:
  - `status` → `REVIEWED`, `reviewedBy` = the actor's user id, `reviewedAt` = now;
  - the reviewer's text goes into a new key `reviewNote`. `note` keeps the source (`COUNT <id>` …).
  - Already `REVIEWED` ⇒ 409 `alreadyReviewed`. Unknown id at that branch ⇒ 404 `notFound`. Blank note ⇒ 400 `validationFailed`.
  - `[proposed]` The update is one `UPDATE cash_exceptions SET data = data || jsonb_build_object(…) WHERE branch_id = $1 AND id = $2 AND data->>'status' = 'OPEN'` (the exception's id is the row's `id` column, as `finance_reconciliation_test.rs:504-518` seeds it), in `exception_review.rs`. Zero rows ⇒ re-read to tell 404 from 409.
- `[required]` **Raise** builds a `CashException` and calls `append_cash_exception_in` (unchanged):
  - `id` = `EX-<uuid>`, `request_id` = `MANUAL-<uuid>`, `status` OPEN, `note` = `MANUAL <reason>`;
  - `cash_variance` or `safe_variance` by `place`, the other `None`.
  - Empty `differences`, a zero amount, or a repeated currency ⇒ 400 `validationFailed`.
- **Readers keep working:** `exception_from_row` (`decode.rs:310`) needs `requestId` and `status`, and both are present.
  The ticket must prove `reviewNote` is ignored by the decoder, or decoded, with a test.

## S2. The exchange completion opens a `NEGATIVE_STOCK` request instead of only refusing

**Gap** (`7e99ed0a`): `complete.rs:247` posts with `allow_negative = None`, so a sale beyond stock is refused
`stockInsufficient` and nothing asks the BRANCH_MANAGER. `NegativeStockSubject`/`Policy` exist
(`xc-cash/src/application/ledger_approval/negative_stock.rs:14-65`, LG-3 #925). `LedgerCashPostings` already honours
an APPROVED request (`ledger_postings.rs:851`, `:863-889`). The lane controller recorded this as a known gap on TX-5
(issuecomment-6107354244 Q7: *"it stays a recorded gap"*), under the pause on new tickets that the owner's
follow-the-Blueprint rule ends.

- **Amends** `quote-transaction.md` `:358` and `:393`: on `stockInsufficient` the completion still writes nothing of
  the exchange; in addition it opens the request of `ledger-inventory.md` §5.1.
- `[required]` Flow:
  1. Before posting, `NegativeStockGate::approved_for_in(tx, transaction)` looks for an APPROVED `NEGATIVE_STOCK`
     request whose `object_id` is the transaction id, and passes it as `allow_negative`.
  2. `post_in` answers `stockInsufficient` ⇒ the completion's transaction is dropped (as today).
  3. The command (`TransactionCompleteCommand::run`, `commands/complete.rs:141-163`) then opens **one new transaction** and calls
     `ApprovalService::request(tx2, "NEGATIVE_STOCK", transaction_id, {shortfall: [{place, currency, quantity}]}, actor, key)`,
     with `key` = the completion's `Idempotency-Key`, and commits it.
  4. It answers **409 `approvalRequired`** (the code already in the TX-5 list, `quote-transaction.md:311`) with
     `{approvalRequestId}`. `AlreadyPending` ⇒ the same answer with the pending request's id.
- `[required]` A REJECTED request is not reused: the next completion attempt with a **new** key opens a new request.
- **Open, must ask the owner — do not guess:** whether a completion may open a request at all for a **DEMO** mode
  exchange (DEMO posts nothing today, `complete.rs:228`). Proposed: no, DEMO never asks.
- **Open, for the controller:** the shortfall payload. The source's figures come from `post_in`'s refusal. Today
  `stockInsufficient` carries no figures; the ticket may have to widen that error inside `xc-cash`, which would add
  `ledger_postings.rs` to its scope.

## S3. A04: a branch's contact channels and status

**Gap** (`7e99ed0a`): the Blueprint's A04 shows *"ช่องทางติดต่อ"* and *"สถานะ"*. Its §07 `Branch` row lists neither, and
neither does `org-roles-branches.md` §6 (`:426-427`) or the code: `AdminBranchProfileData`
(`web/src/shared/api/generated/admin_branches_profile.ts`) has `branch, name, code, address, timezone, openingHours,
bookingEnabled` only.

- **Amends** `org-roles-branches.md` §6: `Branch` also gains `contact_channels` and `status`.
- `[required]` `contact_channels jsonb NOT NULL DEFAULT '[]'`: an array of `{kind, value}`, where `kind` ∈ `PHONE`,
  `LINE`, `EMAIL`, `FACEBOOK`, `WEBSITE`. Up to 10 entries; `value` 1–200 characters, trimmed.
- `[required]` `status text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE','INACTIVE'))`.
- `branchSave` takes both; `admin_branches_profile` answers both (`contactChannels`, `status`).
- **Open, must ask the owner — do not guess:** what `INACTIVE` does beyond being shown. Proposed for this ticket:
  nothing else; refusing service at an INACTIVE branch, and hiding it from the public list, are later tickets.

**Data migration** (owner approval before merge):
1. Existing rows get `contact_channels = '[]'` and `status = 'ACTIVE'` (the defaults). No row violates the CHECK.
2. Backfill: none; the defaults are the truth for every branch today.
3. Deploy: the migration runs before the binary. An old binary ignores both columns.
   The ticket checks that no query reads `branches` with `SELECT *` into a fixed struct.
4. `down` drops both columns and loses every contact channel and status.
- Version: the next free migration version at merge time, after `20261219110000` (`feat/rbac-groups`).

## S4. C12: the assistant answers its intent and preview

**Gap** (`7e99ed0a`): `AssistantService` computes a preview (`assistant_service.rs:149-173`) and the parsed intent
(`:57`), but `AssistantAnswer` (`:22-27`) carries no intent and `AssistantAskAnswerDto`
(`dto/v3/assistant.rs:29-41`) drops the preview, so C12 can show neither *"preview ก่อนยืนยัน"* nor the link of P11.

- `[required]` `AssistantAnswer` gains `intent: Intent` (`xc-app/src/domain/assistant/intent.rs:19-26`).
- `[required]` `AssistantAskAnswerDto` gains:
  - `intent: {kind: "QUOTE"|"QUEUE"|"ALERT"|"BRANCH"|"UNKNOWN", currency?, customerSide?, quantity?, budgetThb?, branch?}`;
  - `preview: PlannerPreviewAnswer | null`, reusing `dto/v3/planner.rs:217` (no second preview shape, ♻️).
- The link of P11 is the screen's: it is derived from `intent.kind` on the client. The back end sends no URL.
- Nothing new is stored. `my_assistant_conversation` is unchanged.

## S5. C09: the customer finds their own queue ticket

**Gap** (`7e99ed0a`): `my_queue_ticket` refuses without `ticketId` (`views/my_queue_ticket.rs:68-70`). No route gives a
customer that id, and the "called" notification is written with no object (`xc-queue/src/application/desk/queue_desk.rs:253`:
`NotificationRequest::with_params(…, None, params)`).

- **Amends** `queue-tickets.md` §4.4 (`:164`):
  - `[required]` `ticketId` becomes optional. Without it, the view answers the session customer's newest ticket at
    that branch with status `WAITING`, `CALLED` or `SERVING` on today's service date.
  - None ⇒ 404 `noTicket`. The answer's shape is unchanged (`MyQueueTicketData`), so no client breaks.
  - With `ticketId` the rules are unchanged (another customer's ticket ⇒ 403 `outOfScope`).
- `[required]` The `queue_called` notification carries
  `NotificationObject { object_type: "QUEUE_TICKET", object_id: <ticket id> }` (`xc-kernel/src/notifications.rs:31-34`).

## S6. C14: a public read of the published consent texts

**Gap** (`7e99ed0a`): `customer-identity-consent.md:163` says the `consent_texts` view needs no session ("published
texts are public"). The view on `main` inherits the staff permission (`views/consent_texts.rs:33`,
`INHERITS = (Rpc, "consentPolicy")`, i.e. `ConsentPolicyManage`), so a visitor on sign-up cannot read `TERMS`.

- **Amends** `customer-identity-consent.md:163`: the public read is a public route, because a public request carries
  no organisation:
  - `[required]` `GET /api/public/orgs/{org}/consent_texts?lang=th|en|zh` answers, for that language, the current
    published version of each purpose (`TERMS`, `MARKETING`): `[{purpose, version, body}]`. Texts have no channel
    column (`20261019100000_customers_org.sql:74-83`); channels belong to consent records.
  - Unknown org ⇒ 404. A bad `lang` ⇒ 400. No text ⇒ an empty array.
  - It sends an `ETag` of the max version; an unchanged `If-None-Match` ⇒ 304 (P15 rule).
  - The org is a path code, as `GET /api/public/orgs/{org}/branches` (`http/public/branches.rs:16`).
- The staff view `consent_texts` stays as it is (EXECUTIVE's screen).
- The reader is `xc-customer`'s consent service (`consent_service.rs:122` `current_version_in`) or a list form of it
  in the same file.

## S7. C08: a customer takes a queue number from the app

**Gap** (`7e99ed0a`): the Blueprint's C08 is *"ค้นสาขา + service status + เส้นทาง + จองหรือรับคิว"*. The design's issuers
(Q4, `queue-tickets.md:43`) are the kiosk, the desk and a booking's check-in. `IssueSource` has `Kiosk`, `Booking`,
`Desk` (`xc-queue/src/domain/ticket/issue_source.rs:9-16`). The only issuing route is the teller's walk-in
(`commands/queue.rs:226-236`).

- **Amends** `queue-tickets.md` Q4: a signed-in customer may also take a walk-in ticket from the app. Q10 still
  holds: the customer only **takes** a ticket; they never move one.
- `[required]` `queueTicketTake`: `POST /api/app/queue/tickets` `{branch}`, a `CustomerSelf` session, with an `Idempotency-Key`.
  - It issues a `WALK_IN` ticket with `customer_id` = the session's, through `TicketIssuer` with a new
    `IssueSource::App { customer_id }`.
  - Refusals: the queue is closed ⇒ 409 `queueClosed`; the customer already holds an active ticket
    (`WAITING|CALLED|SERVING`) at that branch today ⇒ 409 `alreadyQueued`, with that ticket's id.
  - The answer is `MyQueueTicketData`.
- `[required]` One active app ticket per customer per branch and day is a partial unique index, so two taps never
  make two tickets.
- **Open, must ask the owner — do not guess:** whether a remote ticket needs anything on arrival (a check-in), or
  any distance or rate limit. Proposed: none in this ticket, as the Blueprint states none.

**Data migration** (owner approval before merge):
1. `CREATE UNIQUE INDEX queue_tickets_one_active_app_ticket ON queue_tickets (branch_id, service_date, customer_id)
   WHERE customer_id IS NOT NULL AND service_type = 'WALK_IN' AND status IN ('WAITING','CALLED','SERVING')`.
   Existing rows: none changes.
2. Backfill: none. Before deploy, count what would break the index:
   `SELECT branch_id, service_date, customer_id, count(*) FROM queue_tickets WHERE customer_id IS NOT NULL AND service_type = 'WALK_IN' AND status IN ('WAITING','CALLED','SERVING') GROUP BY 1,2,3 HAVING count(*) > 1;`
   It must print no row.
3. Deploy: the migration runs before the binary. An old binary never writes an app ticket, so it can't hit the index.
   A desk ticket with a customer id can, if the same customer is already queued; the ticket names this case.
4. `down` drops the index; nothing is lost.
- Version: the next free version at merge time, after `20261219110000`.

---

## File table (each ticket's locked files)

| Row | Files |
|---|---|
| S1 | `api/crates/xc-shift/src/infrastructure/custody/exception_review.rs` (new), `api/src/presentation/http/v3/commands/cash_exception_review.rs` (new), `api/src/presentation/http/v3/commands/cash_exception_raise.rs` (new), `api/src/presentation/dto/v3/cash_exception.rs` (new), the test file |
| S2 | `api/src/application/workflows/exchange/complete.rs`, `api/src/application/workflows/exchange/negative_stock_gate.rs` (new), `api/src/presentation/http/v3/commands/complete.rs`, the test file |
| S3 | the new migration, `api/crates/xc-rate/src/domain/branch/entities/branch.rs`, `api/crates/xc-rate/src/domain/branch/contact_channel.rs` (new), `api/crates/xc-rate/src/infrastructure/branch/branch_repository.rs`, `api/crates/xc-rate/src/application/branch/service.rs`, `api/src/presentation/http/v3/commands/branch_save.rs`, `api/src/presentation/http/v3/views/admin_branches_profile.rs`, `api/src/presentation/dto/v3/admin_branches_profile.rs`, the test file |
| S4 | `api/crates/xc-app/src/application/assistant/assistant_service.rs`, `api/src/presentation/dto/v3/assistant.rs`, the test file |
| S5 | `api/src/presentation/http/v3/views/my_queue_ticket.rs`, `api/crates/xc-queue/src/application/desk/queue_desk.rs`, the test file (the DTO is unchanged) |
| S6 | `api/src/presentation/http/public/consent_texts.rs` (new), `api/crates/xc-customer/src/application/consent_service.rs`, the test file |
| S7 | the new migration, `api/crates/xc-queue/src/domain/ticket/issue_source.rs`, `api/crates/xc-queue/src/application/issue/ticket_issuer.rs`, `api/src/presentation/http/v3/commands/queue_ticket_take.rs` (new), `api/crates/xc-auth/src/domain/policy/areas/queue.rs`, the test file |

Generated TypeScript under `web/src/shared/api/generated/` follows each DTO and is regenerated by the ticket that
changes the DTO. It is not locked.
