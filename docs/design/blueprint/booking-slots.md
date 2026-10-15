# Blueprint bookings: time slots with atomic capacity, a QR token, check-in, no-show (#555)

> designed-by: claude/5edf7bb5 (subagent) · 2026-10-10 · measured on `main` at `978d94b`
> Source of the rules: *XC Desk — Blueprint & Data Model v2* (`docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`)
> §06 Booking states, §07 `BookingSlot` / `Booking` / `BookingEvent`, §08 *"Customer → Booking → Transaction 1:N →
> 0..1 completed"*, §09 *"Booking slot"*, §10 `POST /api/bookings`, `POST /api/bookings/:id/check-in`, §12 Booking,
> §13 phase 4, §16 *"Booking → QueueTicket 1:0..N"*, §17 *"ปุ่มลูกค้าแค่ check-in หรือแสดง QR"* and *"ต้อง verified
> ก่อนเริ่ม booking"*, §18 *"Pass → Booking → Transaction"*, screens A04, B02, C02, C03, C08, R03.
> Built on: PR 589 (`org-roles-branches.md`: `organizations`, `users`, `BookingsDesk`, `BranchesManage`, `CustomerSelf`,
> `Branch.booking_enabled`, `BranchHours`), PR 639 (`customer-identity-consent.md`: `xc-customer`, one customer per
> company, `customer_identities`, `PiiKeys`), PR 570 (`rates.md`: `CommandAction`, `idempotency_records`), PR 608
> (`quote-transaction.md`: `quotes` with channel `BOOKING`, `transactions.booking_id`, `BookingSaveStep` kept, §6.4),
> PR 609 (`queue-tickets.md`: `QueueTickets` port, Q-A2 *a ticket at check-in, called first*), PR 606 (`broker-seam.md`:
> `ReferralCaptureService::capture_in`, `CaptureTarget::Booking`).
> Umbrella: #564. Standing owner rules: *"follow the Blueprint fully"* and *"logic ห้ามแก้ไข — ปรับได้แค่หน้าตา"*.

## 0. The owner's answers

The owner was asked in the lane controller's session (`claude/5edf7bb5`) on **2026-10-10**:
*"คำถามธุรกิจของเอกสารออกแบบ 6 ใบ (การจอง ตรวจเอกสาร แอปลูกค้า แต้ม แคมเปญ แจ้งเตือนเรท) จะให้ใช้คำแนะนำของผมตามรายการข้างบนไหม?"*
and answered *"ใช้คำแนะนำทั้งหมด"* (use every recommendation as listed). The record is on #555.

| Question | Decided |
|---|---|
| Q-B1 slot length, capacity, horizon | per-branch data ADMIN sets in A04; a branch with no rule offers no slots (option A) |
| Q-B2 cancellation cutoff | 60 minutes before the slot, a company setting ADMIN can edit; staff can always cancel with a reason (option A) |
| Q-B3 no-show grace | slot start + 15 minutes, editable (option A) |
| Q-B4 partner confirmation expiry | 60 minutes after the booking is made, never later than the cutoff, editable (option A) |
| Q-B5 teller-made booking | starts `CONFIRMED` (option A) |

The numbers of Q-B2–Q-B4 are the seeded values of `booking_policies` (§8 M1 item 2), not constants in code; Q-B1 has no
seeded value.

Already decided elsewhere, not asked again:

| Decision | Source |
|---|---|
| D5 is reopened: customers book themselves (C02/C03) | mapping Q4, O-7 (`xc-desk-blueprint-v2-mapping.md:289`) |
| A booking is a queue place; the rate is set at completion (mapping K5/Q5) | `xc-desk-blueprint-v2-mapping.md:290`; quote-transaction §6.4 |
| A booking gets a queue ticket at check-in, called before waiting walk-ins | queue-tickets §0 Q-A2 |
| A called customer who did not come: NO_SHOW on the ticket, final; the customer takes a new number | queue-tickets §0 Q-A3 |
| Brokers, referrals, commission and payout are Broker-Platform's; v3 holds only the attribution | #564, `broker-seam.md` |
| Branch hours are per branch data; a branch with no hours does not open | `org-roles-branches.md` §0 B4, B5 |

## 1. v3 today (measured at `978d94b`)

| Fact | Evidence |
|---|---|
| `bookings` is v2's staff-made rate lock: `PENDING / CONFIRMED / USED / CANCELLED`, plain `name` and `phone`, `rate`, `expires_at`, no slot, no customer column (a member's customer via `member_id`) | `api/migrations/20261008130000_bookings.sql:13-37` (`:25` statuses) |
| A CONFIRMED booking locks the counter rate until `expires_at`; the save skips the rate check for it | `xc-exchange/src/domain/booking/entities/booking_machine.rs:27-67`; `api/src/application/workflows/booking_save_step.rs:28-30` (`vouches_for_rate`) |
| Confirm compares the line stored at create with the current line (RE-4d, rates R6) | `booking_machine.rs:99-116`; `rates.md` §4.9 R6 |
| `members` is a staff-verified phone per (branch, customer), with a member-code hash nothing reads | `20261008130000_bookings.sql:1-11`; `xchange-v3.md` §5.6.2 |
| Readers of `members`: the admin customers view's per-branch member flag, the bookings settings count, the compat snapshot's `member` fields | `api/src/presentation/http/v3/views/admin_customers.rs:96`; `views/admin_bookings.rs:280`; `api/src/application/compat/snapshot.rs:98`, `:113` |
| Per-branch `bookingRules {enabled, minutes}` in `branch_settings` (minutes = rate-lock life, 5–1440, default 30) | `api/src/application/bookings/booking_rules_service.rs:12`; `xc-exchange/src/domain/booking/value_objects/booking_rules.rs:13-29` |
| `branches.booking_enabled` exists (OR-1) but nothing reads it | `api/migrations/20261011100000_organizations.sql:18` |
| Routes: `POST /api/bookings` (owner, action envelope) and `POST /api/staff/bookings` (counter roles) | `api/src/presentation/http/bookings.rs:113-124`; the desk page rewrites one into the other, `api/src/presentation/pages.rs:453` |
| The admin screen posts action envelopes to `/api/bookings` | `web/src/screens/admin/bookings/BookingsScreen.tsx:189`, `:248`, `:305`, `:416`; `web/src/screens/admin/customers/Customers.tsx:68`, `:469` |
| Quotes already allow channel `BOOKING`, type `INDICATIVE` only | `api/migrations/20261014100000_quotes.sql:9-10` |
| `idempotency_records` serves every `CommandAction` | `api/migrations/20261010115000_idempotency.sql:1-11`; `rates.md` §5.6 |
| SQL that names `bookings`: 7 queries in `xc-exchange/src/infrastructure/booking/booking_repository.rs:33-80`, 2 in `views/admin_bookings.rs:66`, `:72` | `git grep -n -E "FROM bookings\|INTO bookings\|UPDATE bookings" 978d94b -- api/src api/crates` |
| No slot, no capacity, no QR, no check-in, no no-show, no booking event | `git grep -n -i -E "booking_slot\|check_in\|no_show" 978d94b -- api` is empty |

## 2. Decisions

| # | Decision | Source |
|---|---|---|
| B1 | **A booking is a place in a time slot, not a rate.** It holds what the customer intends (`shop_side`, `requested_currency`, `requested_quantity`) and, optionally, the indicative quote they were shown (`quote_id`). It stores no rate, no rate line and no lock. The rate is the transaction's own quote at completion. RE-4d's R6/R7 apply to v2 rows only and leave with them (§8 M2). | §06 Quote *"indicative สำหรับจองคิว; binding … ระยะ 6"*, C02 *"เรทสุดท้ายยืนยันเมื่อทำรายการ"*, K5/Q5 |
| B2 | **Slots are rows.** `booking_slots(start_at, end_at, capacity, reserved, status)` per branch. A generator makes them from the branch's slot rule (Q-B1) inside `BranchHours::window` for each day ahead; closed days (`branch_closures`) and days with hours `Closed`/`NotSet` get none. | §07 BookingSlot *"วันหยุด/เวลาปิดไม่เปิด slot"*, A04, B4 |
| B3 | **Capacity is checked atomically.** Reserving is one `UPDATE booking_slots SET reserved = reserved + 1 WHERE id = $1 AND status = 'OPEN' AND reserved < capacity RETURNING …` in the booking's transaction. No row back = `CAPACITY_FULL`. `reserved` counts the bookings of the slot in a holding state (§4.2). | §09, §10 `CAPACITY_FULL`, §13 phase 4 |
| B4 | **Rescheduling moves one place atomically.** Lock both slot rows in ascending `id` order, reserve the new one (B3), release the old one, in one transaction. The same slot is `unchanged`. | §09 *"เปลี่ยนเวลา release slot เดิม + reserve ใหม่พร้อมกัน"*, §13 |
| B5 | **States** (§06): `PENDING_CUSTOMER_CONFIRMATION → CONFIRMED → CHECKED_IN → IN_SERVICE → COMPLETED`; `CANCELLED`, `NO_SHOW`, `EXPIRED`. Start state by channel: `APP` → `CONFIRMED`; `PARTNER` → `PENDING_CUSTOMER_CONFIRMATION`; `DESK` → `CONFIRMED` (Q-B5). | §06 *"ลูกค้าจองเองเริ่ม CONFIRMED; partner จองต้องยืนยันลูกค้า; COMPLETED ได้เมื่อ transaction สำเร็จ"* |
| B6 | **Every booking has a customer** (`customer_id NOT NULL`, one customer per company, PR 639 K1). APP: the signed-in customer (`Actor.customer_id`). DESK: a customer the teller picks, or one created from a display name and a phone (`ContactReserve`, §5). PARTNER: created from the contact Broker sends. No plain name or phone on the booking. | §07 Booking `customer_id FK`; PR 639 K2 (PII only in `customer_identities`) |
| B7 | **The QR is an opaque token, never stored.** `token = base64url(HMAC-SHA256(qr_key, booking_id ‖ qr_version))`, `qr_key` derived from the PII key file with HKDF (`info = "xc-booking-qr-v1"`). The row stores `qr_token_hash = SHA-256(token)` (unique) and `qr_version`. The owner's view recomputes the token; nothing stores it, so an idempotency replay never carries it. Re-issuing bumps `qr_version` and the old QR stops working. | §07 *"QR ใช้ opaque token hash ไม่ใส่ PII"*, §18 |
| B8 | **Check-in issues the BOOKING ticket in the same transaction** (`QueueTickets::issue_in`, `IssueSource::Booking`). The teller proves the owner by the QR or by ticking that the identity was checked; the kiosk by the QR. A refused issue (`queueClosed`) leaves the booking `CONFIRMED`. A `CHECKED_IN` booking whose ticket ended `NO_SHOW` may check in again and gets a new ticket (Q-A3; §16 *1:0..N ticket history*). | §10 *"check-in ตรวจ QR/เจ้าของ"*, Q-A2, Q-A3 |
| B9 | **The queue moves the booking forward.** The booking's ticket becoming `SERVING` makes the booking `IN_SERVICE`; its ticket cancelled with `dayClosed` makes it `CANCELLED` (`dayClosed`). The queue calls the `BookingQueueEvents` port (xc-kernel), so `xc-queue` does not depend on `xc-booking`. | queue-tickets Q5, Q8 |
| B10 | **Completion completes the booking.** The transaction's completion runs `BookingSaveStep` (kept, quote-transaction D-7): `before_in` requires the booking `CHECKED_IN` or `IN_SERVICE`, the same branch and the same customer; `after_in` makes it `COMPLETED` with `transaction_id`. Side, currency and quantity may differ from the request (a queue place, B1). One booking → at most one completed transaction (unique `transaction_id`). | §06, §08 *"1:N → 0..1 completed"*, quote-transaction §6.4 |
| B11 | **Time rules are data.** Cancellation cutoff (Q-B2, seeded 60 min), no-show grace (Q-B3, seeded 15 min) and confirmation expiry (Q-B4, seeded 60 min) live in `booking_policies` per organisation, editable by ADMIN; slot length, capacity and horizon (Q-B1) in `booking_slot_rules` per branch, entered by ADMIN with no default. No number is in code. | §12 Booking *"slot length/capacity; cutoff ยกเลิก; no-show grace; confirmation expiry"* |
| B12 | **Expiry and no-show are written by a sweeper**, every minute, compare-and-set, actor `system`: `PENDING_CUSTOMER_CONFIRMATION` past `confirm_expires_at` → `EXPIRED`; `CONFIRMED` past `slot.start_at + grace` → `NO_SHOW`; `CHECKED_IN` with no active ticket after the slot's service date → `CANCELLED` (`dayEnded`). Each releases its place. Readers also treat an overdue row as its final state, so a late sweep never shows a wrong state. | §06, §12 |
| B13 | **Partner bookings need an attribution reference.** `source_channel = 'PARTNER'` requires `referralCode`; the booking's create captures it (`ReferralCaptureService::capture_in`, `CaptureTarget::Booking`) in the same transaction and stores `partner_attribution_id`. The Partner Portal (R03) is Broker-Platform's; v3 serves one machine route for it (BK-12). **For DESK and APP `referralCode` is optional**: when given, `BookingService::create` captures it the same way, in the create's transaction (BK-3), so a code given while booking is never dropped; without it nothing is captured. The capture moves to the exchange with `link_transaction_in` (FD-2 #973 / RA-1 #979). | #555 §3, `broker-seam.md` §4.2 |
| B14 | **Verification is not required to book.** The booking shows the customer's verification readiness (#557). Requiring it would be a booking policy, not code. | §17 *"Blueprint เสนอจองคิวได้และแสดง verification readiness … ให้ตั้ง booking policy ชัดแทน hard-code"*; #557 Q-V3 |
| B15 | **Idempotency.** Every command is a `CommandAction` with `Idempotency-Key` (§09: booking named). A replay returns the stored answer, which never holds the QR token (B7). | §09, §10, `rates.md` §5.6 |

## 3. Who does what (PR 589 permissions)

| Step | Permission | Notes |
|---|---|---|
| Book for myself (C02), cancel, reschedule, confirm a partner booking, see my bookings and QR (C03) | `CustomerSelf` | own customer only (#550 owner check); cancel and reschedule before the cutoff (Q-B2: 60 min, editable) |
| Book for a customer at the desk or on the phone (B02 *"รับจอง"*) | `BookingsDesk` (TELLER) | PR 589 C9: BRANCH_MANAGER is refused, as today |
| Check in, mark a hand-off, cancel with a reason, reschedule, re-issue a QR, look up by QR (B02) | `BookingsDesk` | no cutoff for staff; a reason is required to cancel |
| Check in by QR at the kiosk | the kiosk device's path (`EntryPath::Kiosk`) | QR only |
| Set the slot rule, closures, a slot's capacity, close/open a slot, the booking policy, the branch's booking switch (A04 *"เพิ่ม/หยุดรับจอง; ตั้งเวลาและ capacity"*) | `BranchesManage` (ADMIN) | |
| Create a partner booking (R03, via Broker) | Broker's machine route (BK-12) | `referralCode` required |
| Read free places of a branch (C02, C08) | public | counts only |

## 4. Model

### 4.1 Tables (migration M1, §8)

Every table has `organization_id uuid NOT NULL REFERENCES organizations(id)` and `created_at`; mutable tables also
`updated_at`. Branch-scoped tables use the composite key `(organization_id, branch_id) → branches(organization_id, id)`
(PR 589 D-1). Person columns are `uuid REFERENCES users(id)`, except `actor_id` text where the system or a machine acts.

```
booking_policies(organization_id uuid PK,
                 cancel_cutoff_minutes  integer NOT NULL CHECK (cancel_cutoff_minutes  >= 0),   -- Q-B2: seeded 60
                 no_show_grace_minutes  integer NOT NULL CHECK (no_show_grace_minutes  >= 0),   -- Q-B3: seeded 15
                 confirm_expiry_minutes integer NOT NULL CHECK (confirm_expiry_minutes > 0),    -- Q-B4: seeded 60
                 updated_by uuid NULL, updated_at, created_at)
booking_slot_rules(organization_id, branch_id text PK,
                   slot_minutes integer NOT NULL CHECK (slot_minutes BETWEEN 5 AND 240),         -- Q-B1: ADMIN, no default
                   capacity     integer NOT NULL CHECK (capacity > 0),                           -- Q-B1: ADMIN, no default
                   horizon_days integer NOT NULL CHECK (horizon_days BETWEEN 1 AND 90),          -- Q-B1: ADMIN, no default
                   updated_by uuid NOT NULL, updated_at, created_at)
branch_closures(organization_id, branch_id text, closed_on date, reason text NOT NULL,
                created_by uuid NOT NULL, created_at, PRIMARY KEY (branch_id, closed_on))
booking_slots(id uuid PK, organization_id, branch_id text NOT NULL,
              service_date date NOT NULL,                                   -- the branch's local date of start_at
              start_at timestamptz NOT NULL, end_at timestamptz NOT NULL,
              capacity integer NOT NULL CHECK (capacity >= 0),
              reserved integer NOT NULL DEFAULT 0 CHECK (reserved >= 0 AND reserved <= capacity),
              status text NOT NULL CHECK (status IN ('OPEN','CLOSED')),
              updated_at, created_at,
              CHECK (end_at > start_at),
              UNIQUE (branch_id, start_at),
              UNIQUE (organization_id, branch_id, id))                     -- target of the bookings FK
booking_sequences(organization_id, issued_on date, last_sequence integer NOT NULL,
                  PRIMARY KEY (organization_id, issued_on))
bookings(id text PK,                                                        -- BK-<uuid>, the form v2 ids use
         organization_id, branch_id text NOT NULL,
         booking_no text NOT NULL,                                          -- B<yyMMdd>-<0001>, [proposed] look
         customer_id text NOT NULL REFERENCES customers(id),
         slot_id uuid NOT NULL,
         quote_id uuid NULL REFERENCES quotes(id),
         shop_side text NOT NULL CHECK (shop_side IN ('BUY','SELL')),
         requested_currency char(3) NOT NULL REFERENCES currencies(code),
         requested_quantity numeric(20,4) NOT NULL CHECK (requested_quantity > 0),
         source_channel text NOT NULL CHECK (source_channel IN ('APP','DESK','PARTNER')),
         status text NOT NULL CHECK (status IN ('PENDING_CUSTOMER_CONFIRMATION','CONFIRMED','CHECKED_IN',
                                                'IN_SERVICE','COMPLETED','CANCELLED','NO_SHOW','EXPIRED')),
         qr_token_hash bytea NOT NULL UNIQUE, qr_version integer NOT NULL DEFAULT 1,
         confirm_expires_at timestamptz NULL, checked_in_at timestamptz NULL,
         transaction_id uuid NULL REFERENCES transactions(id),
         partner_attribution_id uuid NULL,                                  -- FK added by BK-12 (§9)
         ended_reason text NULL,
         created_by text NOT NULL,                                          -- user id, or "system:<route>"
         created_at, updated_at,
         UNIQUE (organization_id, booking_no),
         FOREIGN KEY (organization_id, branch_id, slot_id) REFERENCES booking_slots (organization_id, branch_id, id),
         CHECK ((status = 'COMPLETED') = (transaction_id IS NOT NULL)),
         CHECK ((status = 'PENDING_CUSTOMER_CONFIRMATION') <= (confirm_expires_at IS NOT NULL)),
         CHECK ((source_channel = 'PARTNER') <= (partner_attribution_id IS NOT NULL)))
CREATE UNIQUE INDEX bookings_one_per_transaction ON bookings (transaction_id) WHERE transaction_id IS NOT NULL
CREATE INDEX bookings_slot_holding ON bookings (slot_id)
       WHERE status IN ('PENDING_CUSTOMER_CONFIRMATION','CONFIRMED','CHECKED_IN','IN_SERVICE','COMPLETED')
CREATE INDEX bookings_branch_day ON bookings (branch_id, slot_id, status)
booking_events(id uuid PK, organization_id, booking_id text NOT NULL REFERENCES bookings(id),
               kind text NOT NULL CHECK (kind IN ('CREATE','CONFIRM','RESCHEDULE','CHECK_IN','START','COMPLETE',
                                                  'CANCEL','NO_SHOW','EXPIRE','QR_REISSUE')),
               from_status text NULL, to_status text NOT NULL, actor_id text NOT NULL, at timestamptz NOT NULL,
               reason text NULL, detail jsonb NOT NULL DEFAULT '{}',          -- RESCHEDULE: {fromSlot, toSlot}
               created_at)                                                    -- append only (trigger, as consents)
```

- **Holding states** are `PENDING_CUSTOMER_CONFIRMATION`, `CONFIRMED`, `CHECKED_IN`, `IN_SERVICE` and `COMPLETED`.
  `booking_slots.reserved` always equals the number of the slot's bookings in a holding state. Every transition into
  `CANCELLED`, `NO_SHOW` or `EXPIRED` decrements it in the same transaction.
- **A slot's capacity can be lowered only to `reserved` or more** (`capacityBelowReserved`). A slot with places taken
  cannot be closed (`slotHasBookings`); staff cancel its bookings first. Re-running the generator after a rule change
  adds missing future slots and closes future slots with `reserved = 0` that no longer fit. It never deletes a slot.
- **Requested quantity** is `numeric(20,4)`, the quote lines' scale (`20261014100000_quotes.sql:31`). It replaces
  v2's `amount numeric(20,2)` (`rates.md` §1 hands `bookings.amount` to this design).
- **`booking_no`** comes from `booking_sequences` with one `INSERT … ON CONFLICT … DO UPDATE SET last_sequence =
  last_sequence + 1 RETURNING` (the queue's Q11 pattern). `issued_on` is the organisation's local date (`organizations.timezone`).

### 4.2 States

```
                 create(PARTNER)                       create(APP); create(DESK) (Q-B5)
                       │                                         │
                       ▼                                         ▼
  PENDING_CUSTOMER_CONFIRMATION ──customer confirms──▶ CONFIRMED ──check-in (QR / identity)──▶ CHECKED_IN
        │                                               │   ▲                                   │     ▲
        │ confirm_expires_at passes (sweeper)           │   └── reschedule (same state)          │     └─ ticket NO_SHOW: check in again,
        ▼                                               │                                       │        new ticket (B8)
     EXPIRED                                            │ start + grace passes (sweeper)        │ ticket → SERVING
                                                        ▼                                       ▼
                                                     NO_SHOW                               IN_SERVICE ──transaction COMPLETED──▶ COMPLETED
  cancel:  PENDING | CONFIRMED  ──customer (before cutoff) or teller (reason)──▶ CANCELLED
           CHECKED_IN | IN_SERVICE ──teller (reason)──▶ CANCELLED   (CHECKED_IN also cancels its active ticket)
           CHECKED_IN | IN_SERVICE ──ticket cancelled dayClosed──▶ CANCELLED (dayClosed)
           CHECKED_IN, no active ticket, after its service date (sweeper) ──▶ CANCELLED (dayEnded)
  final:   COMPLETED, CANCELLED, NO_SHOW, EXPIRED
```

### 4.3 Rules the machine enforces

| Rule | Where | Refusal |
|---|---|---|
| bookings are open: `branches.booking_enabled`, a slot rule exists, a `booking_policies` row exists | `BookingService::create` | `bookingsClosed` (`VALIDATION_FAILED`) |
| the slot is the branch's, `OPEN`, and starts after now | `BookingService::create`, `reschedule` | `slotUnavailable` |
| capacity reserved atomically (B3); two people booking the last place: exactly one succeeds | `SlotRepository::reserve_in` | `CAPACITY_FULL` |
| reschedule locks both slots in id order, reserves, releases (B4) | `SlotRepository::move_in` | `CAPACITY_FULL`, `STALE_VERSION` |
| a given `quoteId` is the branch's, channel `BOOKING`, same side, currency and quantity, not `CANCELLED` | `BookingService::create` | `VALIDATION_FAILED` (field `quoteId`) |
| PARTNER needs `referralCode`; the capture runs in the same transaction | `BookingService::create` | `VALIDATION_FAILED` (field `referralCode`), the capture's own errors |
| DESK/APP: an optional `referralCode` is captured on the booking in the same transaction (`capture_in`, `CaptureTarget::Booking`); a bad code refuses the whole create and writes nothing | `BookingService::create` | `VALIDATION_FAILED` (field `referralCode`), the capture's own errors |
| customer cancel/reschedule only before `slot.start_at − cutoff` (Q-B2) | `BookingMachine::cancel`, `reschedule` | `cancelCutoffPassed` |
| confirm only from `PENDING_CUSTOMER_CONFIRMATION`, by the owner, before `confirm_expires_at` (Q-B4) | `BookingMachine::confirm` | `STALE_VERSION`, `bookingExpired` |
| check-in: `CONFIRMED` (or `CHECKED_IN` with no active ticket), the slot's service date is today at the branch, now ≤ `start_at + grace` (Q-B3), the owner proven (QR or the teller's identity tick) | `CheckInService::check_in` | `checkInWindow`, `qrMismatch`, `STALE_VERSION` |
| check-in and the ticket issue commit together; `queueClosed` rolls both back | `CheckInService::check_in` | the queue's `queueClosed` |
| completion: booking `CHECKED_IN`/`IN_SERVICE`, same branch and customer; repeat with the same transaction is a no-op | `BookingLifecycle::complete_in` via `BookingSaveStep` | `bookingNotUsable`, `bookingNotThisCustomer` |
| every transition is compare-and-set on the expected status, writes one `booking_events` row and raises the branch version | `BookingMachine`, `PgBookingRepository::transition_in` | `STALE_VERSION` |
| a customer reads and changes only their own bookings; no view shows another customer's data or a phone in full | `CustomerSelf` + owner check; views | `outOfScope` |

### 4.4 Commands and views

All commands are `CommandAction`s (`rates.md` §6) with an `Idempotency-Key`. The two Blueprint §10 routes are kept as
written; the others are [proposed] in the same style and may be renamed in review without changing logic.

| Command | Route | Permission |
|---|---|---|
| `bookingCreate` (body: `branchId, slotId, shopSide, currency, quantity, quoteId?, referralCode?`; DESK adds `customerId` or `{displayName, phone}`) | `POST /api/bookings` (§10) | `CustomerSelf` (APP) or `BookingsDesk` (DESK) |
| `bookingCheckIn` (body: `qrToken?`, `identityChecked?`; one of them) | `POST /api/bookings/{id}/check-in` (§10) | `BookingsDesk` |
| `kioskBookingCheckIn` (body: `qrToken`) | `POST /api/kiosk/bookings/check-in` | kiosk path |
| `bookingConfirm` | `POST /api/bookings/{id}/confirm` | `CustomerSelf` |
| `bookingCancel` (body: `reason`, required for staff) | `POST /api/bookings/{id}/cancel` | `CustomerSelf` or `BookingsDesk` |
| `bookingReschedule` (body: `slotId`) | `POST /api/bookings/{id}/reschedule` | `CustomerSelf` or `BookingsDesk` |
| `bookingQrReissue` | `POST /api/bookings/{id}/qr` | `CustomerSelf` or `BookingsDesk` |
| `slotRuleSet` / `closureAdd` / `closureRemove` | `PUT /api/branches/{id}/booking-slot-rule` · `POST /api/branches/{id}/closures` · `POST /api/branches/{id}/closures/{date}/remove` | `BranchesManage` |
| `slotCapacitySet` / `slotClose` / `slotOpen` | `PUT /api/booking-slots/{id}/capacity` · `POST /api/booking-slots/{id}/close` · `/open` | `BranchesManage` |
| `bookingPolicySet` | `PUT /api/booking-policy` | `BranchesManage` |
| `partnerBookingCreate` | Broker's machine route, BK-12 | its own token (`broker-seam.md` §7: a new machine route needs its own design and token) |

`POST /api/bookings` is today the owner's action envelope (`bookings.rs:113-118`). BK-6 registers the command there and
moves the envelope to `POST /api/v2/bookings` in the same commit, with the admin screen's six URL constants
(`BookingsScreen.tsx`, `Customers.tsx`) and the desk page rewrite (`pages.rs:453`). BK-11 deletes the envelope.

| View | Who | Content |
|---|---|---|
| `booking_slots` (C02, C08) | public | per branch and date: `start`, `end`, `available` (= capacity − reserved, never the bookings), `asOf`, `since` |
| `desk_bookings` (B02) | `BookingsDesk` | today's bookings by slot: number, status, customer display name, phone last 4, readiness (#557), ticket number; `qr` filter for a scanned token; `since` → `{changed:false}` |
| `my_bookings`, `my_booking` (C03) | `CustomerSelf` | status, branch, slot, the QR token (recomputed, B7), the indicative quote's summary (*"เรทสุดท้ายยืนยันเมื่อทำรายการ"*), cancel/reschedule deadline, ticket number and estimate (queue's `my_queue_ticket`), readiness |
| `admin_booking_slots` (A04) | `BranchesManage` | the rule, closures, slots with capacity and reserved per day, the policy |

### 4.5 One command runner: booking commands move onto `CommandEndpoint` (#1679, BK-6 follow-up)

Measured on main `1eba5943`. Rule ♻️: one shared core, never a copy per variant.

**Today.** Each booking command implements `CommandAction`, but its `run` is a stub that answers `serverError`
(e.g. `commands/bookings/create.rs:76-86`). The route is served instead by a copy of the runner,
`handle_booking_command` (`api/src/presentation/http/v3/commands/bookings.rs:248-460`). `kioskBookingCheckIn`
(`commands/bookings/kiosk_check_in.rs:20`) has no `CommandAction` at all.

**Q1. Can `CommandEndpoint` run a command with no signed-in user? Yes, already.**
- `authenticate_caller` (`api/src/presentation/http/v3/caller.rs:52-57`) lets an `EntryPath::Rpc` action whose area row
  is `Need::Public` through with `user = None`.
- `Policy::authorize_with` grants a public action the **named** branch (`api/crates/xc-auth/src/domain/policy/entities/policy.rs:674-681`).
  It falls back to `MAIN` when none is named, so a public command must name its branch.
- The command then gets the whole `Caller` through `CommandAction::run_as` (`command.rs:89-106`).
  APP-18 (#1373) added `run_as` for exactly this.
- The row already exists: `kioskBookingCheckIn` is `Need::Public` (`api/crates/xc-auth/src/domain/policy/areas/bookings.rs:24-27`).
- The organisation comes from the granted branch for every caller (`command.rs`, "Resolve organisation from caller's
  branch"), which replaces the copy's `SELECT organization_id FROM branches` for a user-less call (`bookings.rs:368-380`).

⇒ **The kiosk check-in becomes an ordinary `CommandAction`** with `ROUTE = "/api/kiosk/bookings/check-in"`.
- It overrides `run_as`, because there is no user.
- Its input names the branch as `branch`, the field `CommandEndpoint` reads (`command.rs`, Step 5), instead of `branchId`.
- No web screen calls this route yet (`git grep -n "kiosk/bookings/check-in" -- web` prints nothing), so the field
  rename has no other caller to move.
- No exception and no second runner.

**Q2. What does the copy do differently?** This was measured by reading both paths, not run. Each row is a behaviour
the move changes, and the code ticket must pin it with a test.

| # | Shared `CommandEndpoint` (`command.rs`) | Booking copy (`bookings.rs`) | After the move |
|---|---|---|---|
| 1 | refuses without `x-xchange-request: 1` and a same-origin `Origin` (`:238`, `presentation/extractors/origin.rs:33`) | **no origin check**; the bk6 suites send neither header | booking commands get the same CSRF guard; the bk6 suites add the two headers |
| 2 | a refused command **releases** its claim (`:554`, `store.release`) | the claim stays `PENDING`; a retry with the same key within 60 s answers `idempotencyConflict` (`infrastructure/idempotency/pg_store.rs:104-105`) instead of the real refusal | same key after a refusal → the refusal again, not `409` |
| 3 | runs inside `BranchTx` with the command's `Locks` (`:490-531`) | `services.pool.begin()`, no branch lock | booking commands take the branch lock like every other command |
| 4 | authorizes with the named branch, then `require_branch` | authorizes with no branch, then reads `branchId` or `branch` from the body after the gate | the branch is checked at the gate |
| 5 | parses the typed `Input` **before** claiming the key (Step 6, `:363`) | hashes the raw body, path params merged, and parses inside the closure | a malformed body never takes a key |
| 6 | replay answers `CommandAction::stored_output`. Its default stores the whole output (`command.rs:75`); only `coupon_issue.rs:61` overrides it | answers the closure's `replay_val`. `create.rs:266-267` and `qr.rs:184-185` set `replay["qrToken"] = Null` inside the closure. No booking command overrides `stored_output` (`git grep -n "fn stored_output" -- api/src` finds only those two) | **must be carried over by hand.** Once the body moves into `run`/`run_as`, the trait default would store and replay the live QR token. So `BookingCreate` and `BookingQrReissue` each override `stored_output` to set `qrToken` to `null` |

Rows 1 to 5 are behaviour the copy lacks, not features to keep. Row 6 is the one thing the copy does that the shared path does not do by default, and it is kept through the existing `stored_output` hook. Nothing needs a new hook in the shared core.

**What the code ticket keeps (tests that must stay green unchanged, apart from adding the two headers of row 1).**
These are the BK-6 suites:
- `api/tests/bk6_booking_create_test.rs` (8), including `booking_create_idempotency_replay_returns_stored_answer_without_qr_token` and `…_conflict_refuses_same_key_different_body`;
- `api/tests/bk6_booking_lifecycle_test.rs` (13), including `lifecycle_commands_idempotency_replay_and_conflict`;
- `api/tests/bk6_check_in_test.rs` (10), including `kiosk_check_in_by_valid_qr_token_issues_ticket` and `kiosk_check_in_idempotency_replay_and_conflict`;
- `api/tests/bk6_slot_admin_test.rs` (8).

The code ticket adds up-front tests for rows 1, 2 and 5. For row 6 it adds a QR-reissue replay test: a replayed `bookingQrReissue` answers without `qrToken`, the same guard `bk6_booking_create_test.rs:313` gives `bookingCreate`. No test covers QR reissue today.

**Files the code ticket touches.**
- Remove: `bookings.rs` `handle_booking_command`, and `BookingCommandResult` once unused.
- `create.rs` and `qr.rs` each override `stored_output` to drop `qrToken` (row 6).
- Each `commands/bookings/*.rs`: its `register` goes, and its `run` / `run_as` holds the closure's body.
- `kiosk_check_in.rs` gains `impl CommandAction`.
- `command.rs` is **not** edited.
- `closureRemove` (`bookings.rs:84-160`) moves with the others.

**Decided (BE controller claude/7ca80c, 2026-10-15, on #1679): commands addressed by booking id name the branch.**
- The client sends `branch` in the body of `/api/bookings/{id}/confirm`, `cancel`, `reschedule` and `qr`, as every other v3 command does.
- The command checks that the booking belongs to that branch. A mismatch answers `outOfScope`.
- `locks()` stays synchronous and the shared core is not changed. Resolving the branch from the booking inside `locks()` would make the shared gate async for one family of commands.
- This is a wire change on the customer and desk screens that call these routes, so it ships in the same code ticket.
- The code ticket's up-front tests include the mismatch: a booking of branch A, sent with `branch` = B, gets `outOfScope` and changes nothing.

## 5. Class diagram

```mermaid
classDiagram
  class Booking { <<xc-booking domain/booking/booking.rs>> id; booking_no; customer_id; slot_id; status; qr_version }
  class BookingStatus { <<xc-booking domain/booking/booking_status.rs>> 8 states; +holds_place() }
  class BookingMachine { <<xc-booking domain/booking/booking_machine.rs  pure transitions>> +confirm() +cancel() +reschedule() +check_in() +start() +complete() +expire() +no_show() }
  class SourceChannel { <<xc-booking domain/booking/source_channel.rs>> APP; DESK; PARTNER; +start_status() }
  class BookingNo { <<xc-booking domain/booking/booking_no.rs>> +format(date, seq) }
  class QrToken { <<xc-booking domain/booking/qr_token.rs  HMAC, never stored>> +derive(key, id, version) +hash() }
  class BookingEvent { <<xc-booking domain/booking/booking_event.rs>> }
  class BookingPolicy { <<xc-booking domain/policy/booking_policy.rs  booking_policies row, Q-B2..B4>> +cutoff_at(slot) +grace_until(slot) +confirm_expires_at(now, slot) }
  class BookingRepository { <<trait xc-booking domain/booking/repository.rs>> +insert_in() +transition_in() +by_qr_hash() }
  class PgBookingRepository { <<xc-booking infrastructure/booking_repository.rs>> }
  class BookingSlot { <<xc-booking domain/slot/booking_slot.rs>> start_at; end_at; capacity; reserved; status }
  class SlotRule { <<xc-booking domain/slot/slot_rule.rs  Q-B1>> }
  class BranchClosure { <<xc-booking domain/slot/branch_closure.rs>> }
  class SlotGenerator { <<xc-booking domain/slot/slot_generator.rs  pure>> +slots_for(rule, hours, closures, from, days) }
  class SlotRepository { <<trait xc-booking domain/slot/repository.rs>> +reserve_in() +release_in() +move_in() }
  class PgSlotRepository { <<xc-booking infrastructure/slot_repository.rs>> }
  class SlotService { <<xc-booking application/slot_service.rs>> +set_rule() +add_closure() +set_capacity() +close() +generate() }
  class BookingService { <<xc-booking application/booking_service.rs>> +create() +confirm() +cancel() +reschedule() +reissue_qr() }
  class CheckInService { <<xc-booking application/check_in.rs>> +check_in(by) }
  class BookingLifecycle { <<xc-booking application/lifecycle.rs>> +complete_in() }
  class LifecycleFactory { <<xc-booking infrastructure/lifecycle_factory.rs  #1641>> +pg_booking_lifecycle(pool) Arc~BookingLifecycle~ }
  class BookingSweeper { <<xc-booking application/sweeper.rs>> +run(now) }
  class BookingQueueEvents { <<trait xc-kernel booking_queue_events.rs>> +on_ticket_serving_in() +on_ticket_ended_in() }
  class QueueTickets { <<trait xc-kernel queue_tickets.rs  PR 609 QU-2>> +issue_in() +cancel_active_for_booking_in() }
  class ContactReserve { <<xc-customer application/customer/contact_reserve.rs>> +reserve_in(display_name, phone) }
  class CustomerService { <<xc-customer  PR 639>> }
  class PiiKeys { <<xc-kernel pii/key_file.rs  PR 639>> }
  class ReferralCaptureService { <<xc-referral  PR 606 RF-1 = RA-1 #979>> +capture_in() }
  class BranchHours { <<xc-rate domain/branch/branch_hours.rs  PR 589>> }
  class BookingSaveStep { <<api application/workflows/booking_save_step.rs  kept seam>> }
  class QueueDesk { <<xc-queue  PR 609>> }
  class QueueDayHook { <<xc-queue  PR 609>> }
  class BookingCommands { <<api presentation/http/v3/commands/bookings/*.rs  one file per command; each a CommandAction served by CommandEndpoint, kiosk check-in via run_as (§4.5)>> }
  class BookingViews { <<api presentation/http/v3/views/{booking_slots,desk_bookings,my_bookings,admin_booking_slots}.rs>> }

  Booking --> BookingStatus
  Booking --> SourceChannel
  BookingMachine --> Booking
  BookingMachine --> BookingPolicy
  BookingMachine --> BookingEvent
  BookingService --> BookingMachine
  BookingService --> BookingRepository
  BookingService --> SlotRepository
  BookingService --> BookingNo
  BookingService --> QrToken
  BookingService --> ContactReserve
  BookingService --> ReferralCaptureService : referralCode (optional DESK/APP, required PARTNER)
  CheckInService --> BookingMachine
  CheckInService --> QrToken
  CheckInService --> QueueTickets : issue BOOKING ticket
  BookingLifecycle ..|> BookingQueueEvents
  BookingLifecycle --> BookingMachine
  LifecycleFactory --> BookingLifecycle : builds with PgBookingRepository + PgSlotRepository
  LifecycleFactory --> PgSlotRepository
  BookingSweeper --> BookingMachine
  BookingSweeper --> SlotRepository
  QrToken --> PiiKeys
  ContactReserve --> CustomerService
  SlotService --> SlotGenerator
  SlotService --> SlotRepository
  SlotGenerator --> SlotRule
  SlotGenerator --> BranchClosure
  SlotGenerator --> BranchHours
  SlotRepository --> BookingSlot
  BookingRepository <|.. PgBookingRepository
  SlotRepository <|.. PgSlotRepository
  QueueDesk --> BookingQueueEvents : start
  QueueDayHook --> BookingQueueEvents : dayClosed
  BookingSaveStep --> BookingLifecycle
  BookingCommands --> BookingService
  BookingCommands --> CheckInService
  BookingCommands --> SlotService
  CommandEndpoint ..> BookingCommands : registers and runs (§4.5)
  BookingViews --> BookingRepository
```

- **New crate `xc-booking`.** It depends on `xc-kernel`, `xc-customer`, `xc-rate` (for `BranchHours`) and `xc-referral`.
  `xc-exchange` and `xc-queue` never depend on it: the queue calls `BookingQueueEvents`, the completion calls it through
  `BookingSaveStep` in `api/src`. The v2 module `xc-exchange/src/{domain,application,infrastructure}/booking/**` stays
  untouched until BK-11 deletes it.
- **`ContactReserve`** is a new file in `xc-customer`: find a customer by the phone's lookup hash, else create one with
  an unverified `PHONE` identity, through `CustomerService`'s existing lock and counter. It encrypts and hashes only
  through `xc-kernel::pii` (PR 639 K3).
- **`QueueTickets::cancel_active_for_booking_in`** is a second method on the queue's port. If QU-2 (PR 886) merges
  without it, BK-4 adds it to `xc-kernel/src/queue_tickets.rs` and `TicketIssuer` after QU-2 merges.

### 5.1 Who provides, who uses

| Provider | Used by | Fallback if not merged |
|---|---|---|
| PR 589 `organizations`, `users`, `branches.booking_enabled`, `BranchHours`, `BookingsDesk`, `BranchesManage`, `CustomerSelf`, `Authorizer` | every table and command | merged (OR-1, OR-2); `BranchHours` is OR-1's |
| PR 639 `xc-customer`, `customer_identities`, `PiiKeys`, `Actor.customer_id` (CI-7) | B6, B7, `ContactReserve`, APP bookings | CI-4 merged; APP create waits for CI-7 (BK-6 ships DESK first) |
| PR 570 `CommandAction`, `idempotency_records` (CM-1) | every command | merged |
| PR 608 `quotes` (TX-1), `transactions.booking_id` (TX-2), completion running `SaveStep`s (TX-5/TX-6) | `quote_id`; B10 | TX-1 merged; BK-7 starts after TX-6 |
| PR 609 `QueueTickets` (QU-2), `QueueDesk::start` (QU-3), `QueueDayHook` (QU-5) | B8, B9 | BK-4 starts after QU-2; its two hook lines after QU-3 and QU-5 |
| PR 606 `ReferralCaptureService` (RF-1, cut as RA-1 #979) | `referralCode` on DESK/APP create (BK-3) and PARTNER create (BK-12), B13 | none: BK-3 starts after RA-1 #979 merges |
| #557 verification readiness | `desk_bookings`, `my_booking` | the field reads `unknown` until #557's view exists |
| this design `BookingQueueEvents` | `xc-queue` | — |
| #1641 `pg_booking_lifecycle(pool)` (the one place a production `BookingLifecycle` is built, always with slots) | `rpc.rs`, v3 `transactions.rs`, `kiosk.rs`, v3 `queue.rs`, v3 `bookings.rs` | none: until #1641 merges, those sites build it by hand |
| this design `bookings` (COMPLETED, slot date) | executive EX-8 booking conversion (`executive-finance.md` D-2) | — |

## 6. Kept, changed, retired

| Today | After |
|---|---|
| `bookings` (v2 shape) | renamed `v2_bookings` by M1, read only by the v2 code until BK-11 drops it (M2) |
| a booking locks the counter rate (`rate`, `expires_at`, `published_rate_line_id`, R6, R7) | no rate on a booking (B1); the transaction's quote sets it |
| `PENDING` = a member's booking waiting for the counter's expiry | `PENDING_CUSTOMER_CONFIRMATION` = a partner booking waiting for the customer |
| `USED` with `submission_id`, `transaction_ref` | `COMPLETED` with `transaction_id` |
| plain `name`, `phone` on a booking | the booking's customer; phone only as an encrypted `PHONE` identity |
| `members` (staff-verified phone per branch, member code) | **[proposed] mapping:** the staff member dialog becomes "verify the customer's phone": a `PHONE` identity with `verified_at`, written by `customerContactVerify` (BK-10). The member code is dropped (only v2's public site read it, `xchange-v3.md` §5.6.2; the customer account replaces it, CI-7). The member flag becomes "has a verified `PHONE` identity", company-wide (PR 639 K1). Its readers move in BK-10: `admin_customers.rs:96`, the snapshot's `member` fields (`snapshot.rs:98`), `broker-seam.md`'s `BookingMemberKeys` (reads the booking's customer's verified phone) |
| `bookingRules {enabled, minutes}` | `enabled` → `branches.booking_enabled` (A04 switch); `minutes` (rate-lock life) retired with the lock |
| owner action envelope `POST /api/bookings`, `BookingsAdmin` | moved to `/api/v2/bookings` (BK-6), then deleted with `BookingsAdmin` (BK-11) |
| admin Bookings screen | B02 desk screen (BK-8) and A04 slot settings (BK-9); the old screen goes in BK-11 |

## 7. Known differences from v2

| v2 / v3 today | After |
|---|---|
| staff and the owner create bookings; no customer booking | customers book (C02), staff book (B02), partners book through Broker (R03) |
| a booking is a locked rate for N minutes | a place in a time slot; the rate is set at the counter |
| no time, no capacity | slots with capacity; the last place goes to exactly one |
| the desk opens a booking by id | check-in by QR or identity issues a queue ticket, called first |
| no no-show | `NO_SHOW` after the grace period |

## 8. Data migration

v3 has not been installed for a shop and starts empty (`xchange-v3.md` D6). The test instance holds test data only.
The owner approved v3's migrations until its first install (2026-10-06). **Versions are not fixed here.** Each
ticket chooses its slot above main's newest migration at push time, because the `migration-order` gate refuses any
version at or below main's highest. Fixed numbers collided: `20261103100000` with OB-1 #899 and `20261104100000` with
LG-1 #923. The order is fixed: **BK-1's M1 comes before BK-11's M2** (M2 drops what M1 renamed). Below, `<M1>` and
`<M2>` stand for the versions chosen at push time.

**M1. `api/migrations/<M1>_booking_slots.sql` (slot chosen above main's newest at push time) (BK-1): rename and create.**
1. *Existing rows.* `ALTER TABLE bookings RENAME TO v2_bookings`: every v2 row stays, with its constraints and FKs
   (`members`, `transactions`, `published_rate_lines`; PostgreSQL FKs follow a renamed table). `members` and
   `branch_settings` are untouched. The new tables start empty. No branch offers a slot until ADMIN sets its rule
   (Q-B1).
   **The queue's FK is re-pointed, not carried.** `queue_tickets.booking_id text NULL REFERENCES bookings (id)`
   (`api/migrations/20261101100000_queue.sql:16`, PR 886, head `12060ea`) would follow the rename to `v2_bookings`.
   Then every BK-4 check-in, which names a new booking, would break the FK, and M2's `DROP TABLE v2_bookings` would be
   refused. So M1, after creating the new `bookings`, runs
   `ALTER TABLE queue_tickets DROP CONSTRAINT queue_tickets_booking_id_fkey, ADD CONSTRAINT queue_tickets_booking_id_fkey
   FOREIGN KEY (booking_id) REFERENCES bookings (id)` (PostgreSQL's default name for that unnamed FK). No row can be
   affected: no BOOKING ticket exists before BK-4, the only issuer. M1 checks this first and `RAISE`s if
   `SELECT count(*) FROM queue_tickets WHERE booking_id IS NOT NULL` is not 0.
   **The referral capture's FK is re-pointed too, the same way** (BK-1 #1377, lane controller's ruling 2, decided
   here). `referral_attributions.booking_id text NULL REFERENCES bookings(id)`
   (`api/migrations/20261110130000_referral_attributions.sql:16`, RA-1) would also follow the rename. Then BK-3's
   DESK/APP `referralCode` capture on a new booking would break it. So M1 also runs
   `ALTER TABLE referral_attributions DROP CONSTRAINT referral_attributions_booking_id_fkey, ADD CONSTRAINT
   referral_attributions_booking_id_fkey FOREIGN KEY (booking_id) REFERENCES bookings (id)`.
   - **The guard:** it first `RAISE`s if `SELECT count(*) FROM referral_attributions WHERE booking_id IS NOT NULL` is
     not 0.
   - **Why no row can be affected:** measured on `main` `6b35749a`, nothing in production builds a booking capture. The
     only capture route, `attach_referral.rs:76`, captures on a transaction, and `CaptureService::link_transaction_in`
     has no production caller.
   - **v2's `CaptureTarget::Booking` from M1 on.** The variant names a **Blueprint** booking. Its target lookup
     (`xc-referral/src/infrastructure/capture_repository.rs:50`, today `FROM bookings … LEFT JOIN members … member_id`)
     reads the new `bookings` and takes `customer_id`, `organization_id` and `branch_id` from that row. The new table's
     `customer_id` is `NOT NULL`, so `NoCustomer` cannot occur for it.
   - **A v2 booking can no longer be captured.** Its id is not in `bookings`, so the lookup answers `NoSuchTarget`
     before the FK is reached. No v2 caller exists, so nothing changes for v2.
   - **Who changes it:** this one query is BK-1's. BK-3 then calls `CaptureTarget::Booking` unchanged and touches no
     `xc-referral` file (§9).
2. *Backfill.* One `booking_policies` row per organisation with the owner's 2026-10-10 values (§0):
   `cancel_cutoff_minutes = 60`, `no_show_grace_minutes = 15`, `confirm_expiry_minutes = 60`, `updated_by = NULL`
   (seeded by the migration). An organisation created later has no row, so its bookings stay closed (§4.3 row 1) until
   ADMIN saves one. Count first: `SELECT count(*) FROM organizations;` (one row each) and, for the record,
   `SELECT status, count(*) FROM bookings GROUP BY status;` (the v2 rows that the rename keeps) and
   `SELECT count(*) FROM queue_tickets WHERE booking_id IS NOT NULL;` and
   `SELECT count(*) FROM referral_attributions WHERE booking_id IS NOT NULL;` (both must be 0; item 1).
3. *Order.* After PR 589 OR-1/OR-2, PR 639's `20261019100000_customers_org.sql` and TX-1's quotes (all on `main`),
   and after the queue's M1 (`20261101100000_queue.sql`, PR 886), whose FK item 1 re-points. A push-time `<M1>` always
   sorts after it.
   The deploy is M1 with BK-1's binary, which reads `v2_bookings` in the two v2 SQL files. The previous binary fails
   after M1 (its `bookings` queries meet the new columns), so the two go together.
   **TX-2 interaction:** if TX-2 (`transactions.booking_id REFERENCES bookings(id)`, `quote-transaction.md:148`)
   merges first, the rename carries that FK to `v2_bookings` and M2 re-points it. If TX-2 merges after M1, its FK lands on
   the new `bookings` and its backfill (`quote-transaction.md:605`) must read `v2_bookings.transaction_ref`.
4. *`down`* first re-points `queue_tickets_booking_id_fkey` and `referral_attributions_booking_id_fkey` at the table
   that becomes `bookings` again. It refuses (`RAISE`) while any `queue_tickets.booking_id` or
   `referral_attributions.booking_id` is not NULL, since such a row names a new booking. It then drops the new tables
   and renames `v2_bookings` back to `bookings`. It loses the new rows.

**M2. `api/migrations/<M2>_retire_v2_bookings.sql` (slot chosen above main's newest at push time, after `<M1>`) (BK-11): drop the v2 booking.**
1. *Existing rows.* `v2_bookings` and `members` are dropped with their rows (test data). The `bookingRules` rows of
   `branch_settings` and `branch_setting_history` are deleted. `transactions.booking_id` values that point at a v2
   booking are set to `NULL`, then the FK is re-pointed at `bookings(id)`. `queue_tickets.booking_id` already points at
   the new `bookings` (M1 item 1), so the drop is not blocked by it.
2. *Backfill.* None. The v2 rows cannot become Blueprint rows: they have no slot and, for non-members, no customer.
   Count before the deploy: `SELECT count(*) FROM v2_bookings; SELECT count(*) FROM members;
   SELECT count(*) FROM branch_settings WHERE name = 'bookingRules';
   SELECT count(*) FROM transactions t WHERE t.booking_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM bookings b WHERE b.id = t.booking_id);`
3. *Order.* With BK-11's binary, which no longer reads them. The previous binary fails against M2.
4. *`down`* re-creates `v2_bookings` and `members` empty, and drops the FK change. It does not restore the rows.

M2 is a destructive operation. The deploy that runs it is ordered by the owner.

## 9. Tickets

Each owns disjoint files. Up-front tests are written by the ticket author on each ticket's `feat/…` branch once its
start dependencies are merged (fleet rule 🧪).

| # | Ticket | Owns | Starts after |
|---|---|---|---|
| BK-1 | schema M1; the v2 SQL renamed to `v2_bookings`; the queue and referral FKs re-pointed; the booking capture's target lookup on the new `bookings` | `api/migrations/20261113110000_booking_slots.sql` (lane controller's slot, after VF-1's `20261113100000`, #1377); `xc-exchange/src/infrastructure/booking/booking_repository.rs` (table name only, `:33-80`); `api/src/presentation/http/v3/views/admin_bookings.rs` (table name only); `xc-referral/src/infrastructure/capture_repository.rs` (the `CaptureTarget::Booking` lookup reads the new `bookings`, §8 M1 item 1); table name only in `xc-referral/src/infrastructure/booking_member_keys.rs`, `api/src/application/executive/channel.rs` and `api/src/application/workflows/save_transaction.rs` (lane controller's ruling 1 on #1377: they read v2's shape); the tests whose SQL names `bookings`, measured on `main` at `6b35749a` with `git grep -l -E "(FROM\|INTO\|UPDATE\|JOIN\|TABLE\|TRUNCATE) bookings" -- api web` (table name only, except the booking captures of `xc-referral/tests/capture_test.rs`, which the ticket author re-seeds on a Blueprint booking in the up-front commit): `api/crates/xc-exchange/tests/{booking_published_line_test,booking_repository_db_test,booking_service_test}.rs`, `api/crates/xc-queue/tests/queue_common/mod.rs`, `api/crates/xc-referral/tests/{capture_test.rs,common/mod.rs}`, `api/tests/{booking_save_step_test,booking_tiers_test,bookings_http_test,broker_feed_save_step_test,ex11a_transactions_drilldown_test,executive_overview_test,queue_commands_views_test,rates_re4_bookings_test,rates_re4_desk_save_test,rates_re4_migration_test,rates_re4c_unit3_booking_test,stale_policy_bookings_test,tx_states_migration_test,v3_admin_bookings_view_test}.rs`; a new `api/tests/booking_slots_migration_test.rs` | this design (Q-B2–Q-B4 seed values in §0) |
| BK-2 | crate skeleton; slots: rule, closures, generator, capacity reserve/release/move, `SlotService`, the policy value | `api/crates/xc-booking/{Cargo.toml,src/lib.rs}`, workspace `Cargo.toml` (one line), `xc-booking/src/domain/slot/**`, `xc-booking/src/domain/policy/**`, `xc-booking/src/application/slot_service.rs`, `xc-booking/src/infrastructure/slot_repository.rs` | BK-1 |
| BK-3 | the booking: entity, machine, status, channel, number, QR, events, repository, `BookingService` (create DESK/APP, confirm, cancel, reschedule, QR re-issue); `ContactReserve` | `xc-booking/src/domain/booking/**`, `xc-booking/src/application/booking_service.rs`, `xc-booking/src/infrastructure/booking_repository.rs`, `xc-customer/src/application/customer/contact_reserve.rs` (+ one `mod` line); the optional DESK/APP `referralCode` capture is inside `booking_service.rs` (no `xc-referral` file is touched) | BK-2, RA-1 #979 |
| BK-4 | check-in and the queue's hooks | `xc-booking/src/application/check_in.rs`, `xc-booking/src/application/lifecycle.rs`, `xc-kernel/src/booking_queue_events.rs` (+ one `pub mod` line); one call in `xc-queue/src/application/desk/queue_desk.rs` (start) and one in `xc-queue/src/application/day/queue_day_hook.rs`; `cancel_active_for_booking_in` on the port if QU-2 lacks it | BK-3, QU-2; the two calls after QU-3 and QU-5 |
| BK-5 | the sweeper (B12) and its schedule | `xc-booking/src/application/sweeper.rs`; the one spawn line in `api/src/bootstrap/mod.rs` (LG-4 and CI-4 also hold one line each there: different lines, either order) | BK-3 |
| BK-6 | commands and views (§4.4); the v2 envelope moved to `/api/v2/bookings` | `api/src/presentation/http/v3/commands/bookings/**` (own `permissions.rs`), `views/{booking_slots,desk_bookings,my_bookings,admin_booking_slots}.rs`, their DTOs and generated TS; `api/src/presentation/http/bookings.rs` (the path string), `pages.rs:453`, the six URL constants in `BookingsScreen.tsx` and `Customers.tsx` | BK-4, BK-5; APP create also CI-7 |
| BK-7 | completion: `BookingSaveStep` on `BookingLifecycle` (B10) | `api/src/application/workflows/booking_save_step.rs`, `api/tests/booking_save_step_test.rs` (re-pointed by the ticket author first) | BK-4, TX-6, RE-4c (PR 902) merged |
| BK-8 | B02 desk bookings screen: day by slot, QR scan, check-in, cancel, reschedule, new booking with an optional referral-code field (sent as `referralCode`) | `web/src/screens/desk/bookings/**` | BK-6 |
| BK-9 | A04 slot settings: rule, closures, capacity, policy, booking switch | `web/src/screens/admin/booking-slots/**` | BK-6 |
| BK-10 | `members` mapping: `customerContactVerify`; the member flag's readers | `xc-customer/src/application/customer/contact_verify.rs` (+ `mod` line), `http/v3/commands/customer_contact_verify.rs`; `admin_customers.rs:96` (after TX-8's ticket for that file); `snapshot_sources` members source; `xc-referral/.../booking_member_keys.rs` (after FD-1) | BK-3 |
| BK-11 | retire v2 (M2) | `api/migrations/<M2>_retire_v2_bookings.sql` (slot chosen above main's newest at push time, after `<M1>`); delete `xc-exchange/src/{domain,application,infrastructure}/booking/**` and its 6 tests; `api/src/presentation/http/bookings.rs`, `dto/booking.rs`, `views/admin_bookings.rs`, `dto/v3/admin_bookings.rs`, generated `admin_bookings.ts`, `api/src/application/bookings/**`, `api/src/infrastructure/bookings/booking_ports.rs`; `web/src/screens/admin/bookings/**` and its menu entry; `Permission::BookingsAdmin` and its policy rows; the retired tests listed by `git grep -l -E "booking::\|admin_bookings\|BookingsAdmin" -- api web` at that time | BK-7, BK-8, BK-9, BK-10 |
| BK-12 | partner bookings (R03) | the machine route and its token (own design per `broker-seam.md` §7), `http/v3/commands/bookings/partner_create.rs`, the `partner_attribution_id` FK migration | BK-6, Broker PL-0 and RA-1 #979, #558's rule for how a customer confirms a booking made from a phone |
| BK-8b | (#1891) B02 writes go through `ApiClient` (`view`/`command`), so every write carries `x-xchange-request` and errors go through `wordedError`; the local fetch, key and error helpers are removed. FE follow-up (owner's choice A, 2026-10-15) | `web/src/screens/desk/bookings/DeskBookingsScreen.tsx`, `DeskBookings.client.test.tsx` | BK-8 |
| BK-6-FE | (#1915) every booking command the web sends names its `branch` (FE twin of #1913); merges with or before #1913 | customer `book/Book.tsx`, `bookings/Bookings.tsx`, the desk screen, admin `booking-slots/BookingSlots.tsx` and their tests | BK-8b |

C02/C03/C08 customer screens are #558's and read BK-6's views. EX-8 (booking conversion) reads `bookings` after BK-3.

### 9.1 Tests written before implementation

| Ticket | Tests (red until implemented) |
|---|---|
| BK-1 | every migration applies in version order on a fresh database; each existing organisation has one `booking_policies` row with 60 / 15 / 60; v2 rows survive the rename and the v2 list/create/redeem tests still pass; after M1 a `queue_tickets` row can name a new `bookings` id and cannot name a `v2_bookings` id; M1 refuses when a `queue_tickets.booking_id` is set; the new tables' CHECKs refuse `reserved > capacity`, a `COMPLETED` booking without `transaction_id`, a second booking with the same `transaction_id`, a booking whose slot is another branch's; a `PARTNER` booking without `partner_attribution_id` is refused (lane controller's ruling 3 on #1377); after M1 a `referral_attributions` row can name a new `bookings` id and cannot name a `v2_bookings` id; M1 refuses when a `referral_attributions.booking_id` is set; `down` refuses while either FK names a new booking; a booking capture on a new booking takes its organisation, branch and customer, and one on a v2 booking id is `NoSuchTarget` |
| BK-2 | the generator: slots of `slot_minutes` from open to close in the branch timezone, none on a closure date, none on a `Closed`/`NotSet` day, none past `horizon_days`; re-running adds nothing; a rule change closes empty future slots and leaves slots with bookings; **50 concurrent reserves on a slot of capacity 1 → exactly 1 succeeds, 49 `CAPACITY_FULL`, `reserved = 1`**; move between two slots under concurrency never leaves `reserved` off by one; capacity below `reserved` refused |
| BK-3 | APP create → `CONFIRMED`; DESK create → `CONFIRMED` (Q-B5); DESK with a new phone creates one customer with an unverified PHONE identity, and the same phone again finds it; a replay with the same key returns the same booking and writes once; the same key with another body → `idempotencyConflict`; the stored replay answer holds no QR token; customer cancel before the cutoff → `CANCELLED` and the place released; after the cutoff → `cancelCutoffPassed`; teller cancel without a reason refused; reschedule to a full slot refused and nothing moves; confirm after `confirm_expires_at` → `bookingExpired`; a quote of another branch or side refused; QR re-issue: the old token finds nothing; a DESK booking with a `referralCode` gives one ACTIVE capture on the booking, one without gives none; an APP booking with a code gives one; a malformed code refuses the create and writes neither booking nor capture |
| BK-4 | check-in by the right QR issues one BOOKING ticket and makes the booking `CHECKED_IN` in one transaction; a wrong QR → `qrMismatch`; a closed queue → refused and the booking stays `CONFIRMED` with no ticket; check-in on another day or after start + grace → `checkInWindow`; the ticket `SERVING` → booking `IN_SERVICE`; ticket `NO_SHOW` → booking stays `CHECKED_IN` and a second check-in issues a second ticket; day close → booking `CANCELLED` (`dayClosed`) |
| BK-5 | an overdue `PENDING_CUSTOMER_CONFIRMATION` → `EXPIRED`; `CONFIRMED` after start + grace → `NO_SHOW`; each releases its place; running twice changes nothing; a booking checked in a moment before the sweep is not touched (compare-and-set) |
| BK-6 | every route, its permission refusals, `Idempotency-Key` replay and conflict; `booking_slots` carries counts only; `my_booking` of another customer → `outOfScope`; `desk_bookings` shows phone last 4 only; `since` → `{changed:false}`; the v2 envelope answers on `/api/v2/bookings` as before |
| BK-7 | a completion with a `CHECKED_IN` or `IN_SERVICE` booking of the same customer makes it `COMPLETED` with the transaction id; another customer → `bookingNotThisCustomer` and nothing written; a `CONFIRMED` (not checked in) booking → `bookingNotUsable`; a repeat completion is a no-op; no rate check depends on the booking |
| BK-8, BK-9 | vitest: the screens send the routes of §4.4 with an `Idempotency-Key`; TELLER sees B02, BRANCH_MANAGER does not; ADMIN sees A04; BK-8's new-booking form sends the referral field's value as `referralCode`, and omits the key when the field is empty |
| BK-10 | verifying a phone writes one PHONE identity with `verified_at` and one audit row without the value; the member flag is true for that customer at every branch |
| BK-11 | the workspace builds and passes with the v2 module, routes, tables and screen gone; `git grep -n -E "v2_bookings\|FROM members" -- api` is empty |
| BK-12 | a partner booking without `referralCode` refused; with it, one ACTIVE capture on the booking; it starts `PENDING_CUSTOMER_CONFIRMATION` |

## 10. Owner decisions (2026-10-10)

The owner was asked in the lane controller's session (`claude/5edf7bb5`) on **2026-10-10**:
*"คำถามธุรกิจของเอกสารออกแบบ 6 ใบ (การจอง ตรวจเอกสาร แอปลูกค้า แต้ม แคมเปญ แจ้งเตือนเรท) จะให้ใช้คำแนะนำของผมตามรายการข้างบนไหม?"*
and answered *"ใช้คำแนะนำทั้งหมด"* (use every recommendation as listed). The record is on #555.
Every question below is decided with the proposed option; the options are kept as the record of what was weighed.

**Q-B1. Slot length, capacity and how far ahead customers can book** (§12; used by B2, B11, `booking_slot_rules`). **Decided: A.**
- A. Data per branch, set by ADMIN in A04; a branch with no rule offers no slots. Cost: someone enters each branch; nothing goes live by accident. Reversible: yes.
- B. One company-wide value fixed in code. Cost: a code change to adjust; every branch the same whatever its staff.
- C. Data per branch with a system default. Cost: a branch opens slots with numbers nobody chose for it.

**Decided A** (as proposed), because A04 sets *"เวลาและ capacity"* per branch, capacity depends on each branch's counters, and the
owner already chose this pattern for hours (B4: *a branch with no hours does not open*). Each branch's numbers are
entered by ADMIN in A04 when that branch opens bookings; no number is seeded.

**Q-B2. Cancellation cutoff** (§12; used by §4.3 and `booking_policies.cancel_cutoff_minutes`). Staff can always cancel with a reason. **Decided: A, 60 minutes, editable.**
- A. Minutes before the slot starts, a company setting with a suggested default ADMIN can change (for example 60). Cost: none beyond one setting.
- B. No cutoff: the customer can cancel or move until the slot starts. Cost: places freed too late to refill.
- C. Per-branch cutoff. Cost: one more A04 field, and different rules per branch for customers.

**Decided A with 60 minutes** (as proposed), the same shape as the quote expiry the owner chose (T1: *"5 นาที (แนะนำ) แต่ให้ ตั้งค่าเองได้"*).

**Q-B3. No-show grace and when check-in opens** (§12; used by B12 and §4.3). **Decided: A, start + 15 minutes, editable.**
- A. Check-in is open on the slot's day until slot start + grace; after that the booking is `NO_SHOW`. Grace is a company setting with a suggested default (for example 15 minutes). Cost: a customer more than 15 minutes late loses the place.
- B. Grace until the slot ends. Cost: a late customer still gets called first ahead of walk-ins who waited.
- C. No grace: no-show at the slot's start. Cost: strict; queue delays at the counter count against the customer.

**Decided A with 15 minutes** (as proposed), because a booked customer is called ahead of walk-ins (Q-A2), so a short grace keeps
that priority fair, and the value stays editable.

**Q-B4. When an unconfirmed (partner) booking expires** (§12; used by B12, `confirm_expires_at`). **Decided: A, 60 minutes, capped by the cutoff, editable.**
- A. Minutes after it is made, a company setting (for example 60), but never later than the cancellation cutoff before the slot. Cost: one setting.
- B. At the slot's start. Cost: an unconfirmed booking holds a place until the last moment.
- C. A fixed time of day before the booked day. Cost: needs a time-of-day rule per branch.

**Decided A with 60 minutes** (as proposed), capped by the cutoff, so an unconfirmed place is freed while it can still be rebooked.

**Q-B5. Does a booking a teller makes for a customer (B02, on the phone) start `CONFIRMED`?** (B5). **Decided: A, yes.**
- A. Yes, `CONFIRMED`: the teller is talking to the customer. Cost: none; this is how v3 works today (`booking_machine.rs:57-65`).
- B. No, `PENDING_CUSTOMER_CONFIRMATION` like a partner booking. Cost: the customer must confirm in the app, so a phone customer without the app cannot book.

**Decided A** (as proposed), because the Blueprint asks for customer confirmation only for partner bookings (§06), and B would
lock out customers who phone without the app.

Nothing waits on the owner any more: BK-1 seeds the Q-B2–Q-B4 row, BK-3 starts DESK bookings `CONFIRMED`, and no branch
opens slots until ADMIN enters its Q-B1 rule.

## 11. Blueprint conformance

| Blueprint item | This design |
|---|---|
| §06 Booking states, *"ลูกค้าจองเองเริ่ม CONFIRMED; partner จองต้องยืนยันลูกค้า; COMPLETED ได้เมื่อ transaction สำเร็จ"* | B5, B10, §4.2 |
| §06 Quote *"indicative สำหรับจองคิว; binding … ระยะ 6"* | B1 (`quote_id` indicative; no lock) |
| §07 BookingSlot(id, branch_id, start_at, end_at, capacity, status); *"capacity check atomic; วันหยุด/เวลาปิดไม่เปิด slot"* | §4.1 (`reserved`, `service_date` added), B2, B3 |
| §07 Booking(id, booking_no UQ/org, customer_id, branch_id, slot_id, quote_id nullable, requested_currency, shop_side, requested_quantity, source_channel, status) | §4.1, names kept |
| §07 BookingEvent(id, booking_id, from/to_status, actor_id, at, reason); *"QR ใช้ opaque token hash ไม่ใส่ PII"* | `booking_events` (+ `kind`, `detail`), B7 |
| §08 Customer → Booking → Transaction 1:N → 0..1 completed; walk-in without booking | B6, B10, unique `transaction_id` |
| §09 *"reserve capacity และสร้าง booking ใน transaction; เปลี่ยนเวลา release slot เดิม + reserve ใหม่พร้อมกัน"*; Idempotency-Key for booking | B3, B4, B15 |
| §10 `POST /api/bookings`, `POST /api/bookings/:id/check-in`; *"idempotency + slot capacity + scope; check-in ตรวจ QR/เจ้าของ"*; `CAPACITY_FULL`, `STALE_VERSION` | §4.4, §4.3 |
| §12 Booking: slot length/capacity, cutoff, no-show grace, confirmation expiry | B11, Q-B1–Q-B4 (decided, §0) |
| §13 phase 4: last slot to one; partner confirmation; booking linked to transaction; QR without PII; reschedule does not take two places | §9.1 BK-2, BK-3, BK-7, BK-12; B7; B4 |
| §16 Booking → QueueTicket 1:0..N, one active | B8 (ticket history; the queue's partial unique index) |
| §17 customer button only checks in or shows a QR; verification readiness, not required | §4.4 (`kioskBookingCheckIn`, `my_booking` QR), B14 |
| §18 QR is a token, not PII; customer cannot complete through the API | B7; completion only through the teller's transaction (B10) |
| A04 *"เพิ่ม/หยุดรับจอง; ตั้งเวลาและ capacity"*; B02 *"คิวตามเวลา; QR lookup; สถานะ; check-in; ส่งต่อไปทำรายการ; no-show"*; C02, C03, C08 | §4.4 commands and views; BK-8, BK-9; customer screens are #558's |
| R03 partner booking through Broker with an attribution | B13, BK-12 |
