# Staff corrections: a manager closes a forgotten clock-in; a verifier rejects a waiting count

> claude/studio-claude-4 — design for line XC-STAFF-CORRECTIONS (BE lane, claude/553782), feature branch `feat/staff-corrections`.
> Read at `origin/main` `72f86df9`. Every `file:line` below is at that sha.

## 0. Source and decisions

The owner's two decisions (recorded by the FE lane, xchange #564 issuecomment-6452993293, 2026-10-28 03:5x ICT), verbatim:

- **Forgotten clock-in:** *"ผู้จัดการปิดให้ กรอกเวลาเอง + เหตุผล (แนะนำ)"*. The manager closes the clock-in for the employee, typing the time and a reason.
- **Count that can never be verified:** *"ผู้จัดการปฏิเสธได้ + เหตุผล (แนะนำ)"*. The manager can reject it, with a reason.

These are business rules and are decided. **The one migration (§6, cash counts) still needs the owner's approval** before anyone writes it. Part A needs no SQL migration.

## 1. Root cause

| Symptom (MAIN, measured by the FE lane) | Where it is created |
|---|---|
| 21 clock-ins stay open from earlier days (#564 issuecomment-6439987825) | `/api/attendance` clocks **only the caller**: `AttendanceClockCommand` (`api/src/presentation/http/v3/commands/branch_day.rs:285-327`) calls `BranchDayService::clock(tx, who, …)` (`api/crates/xc-shift/src/application/branch_day/service.rs:325`). Clock-out looks only for `a.employee_id == who.user_id` (`:358-364`). Nothing can close someone else's row, so an open row from a past day can only be fixed with SQL. |
| 12 counts wait forever (#564 issuecomment-6452574856) | Cash counts have only submit and verify (`api/src/presentation/http/v3/commands/cash.rs:288-327`, `:348-385`). A count that verify refuses (`stockInsufficient`, `notNewestCount`, …) has no other exit. "Waiting" means `verified_by IS NULL` (`api/src/presentation/http/v3/views/cash_counts.rs:76-79`); there is no status column (`api/migrations/20261012100000_cash.sql:120-137`). |

## 2. Decisions

| # | Decision | Why |
|---|---|---|
| SC-D1 | **A** is a new command `attendanceCloseFor`: `POST /api/attendance/{id}/close` with `{clockOut, reason}`. It sits beside `clock` in `branch_day.rs` and `BranchDayService`. | The row is identified by its attendance id, so a manager closes one exact row. That cannot be confused with the caller's own clock. |
| SC-D2 | A's permission is a **new permission `AttendanceCloseFor`**, checked on the row's branch (`Authorizer::check`, `Target{org, Some(branch)}`). It is seeded on BRANCH_MANAGER, EXECUTIVE and ADMIN through `RoleGrants::SEED_ADDITIONS` at **the next free seed version when SC-0 is written** (`api/crates/xc-auth/src/domain/access/role_grants.rs:111-127`; main is at 5, `feat/binding-quote` takes 6, and ledger adjustment LA-1 takes the one after binding quote). | Groups are data (RBAC §11). "The branch manager tier" is a permission any group can be given, never a role test. The refusal is the existing `roleNotAllowed` / `outOfScope`, so there is no `notManager` code (see §5). |
| SC-D3 | A refuses closing **your own** row (`selfClose`). | The brief: "Never yourself". Your own row is closed with `clock` OUT. |
| SC-D4 | A's time rules: `clockOut ≥ clockIn`, `clockOut ≤ now`, and the date of `clockOut` in the **branch timezone** (`branches.timezone`, `api/migrations/20261011100000_organizations.sql:16`) is **before today** in that timezone. | The owner's rule plus the brief. Rows from today are still the employee's to close. Reading the branch timezone follows `check_in.rs:157-166` (`now.with_timezone(&tz).date_naive()`). The xc-shift code has no timezone logic today. |
| SC-D5 | A's audit is an `ops_events` row of a new kind `CLOCK_OUT_FOR` (`EventKind::ClockOutFor`). `actor` = the manager and `at` = the recorded time. `details` = `{attendanceId, employeeId, clockIn, clockOut, reason}`. `clockOut` is the entered time. | `ops_events.kind` has no CHECK and `details` is jsonb (`20261007140000_custody.sql:69-78`), so no migration is needed. It is the same journal the employee's own `CLOCK_OUT` writes (`service.rs:146-165`). The attendance row itself only gets its `clockOut` (jsonb `data`, `custody_repository.rs:120-126`). |
| SC-D6 | **B** is a new command `countReject`: `POST /api/cash-counts/{id}/reject` with `{reason}`. It sits beside `countVerify`. Its permission is `ShiftCloseCertify` (the same as verify: `areas/cash.rs:27-31`, `count_service.rs:1055-1061`), and it uses the same "the verifier is not the counter" rule (`person_rules.rs:10`). | Same people, same scope as verify. |
| SC-D7 | B stores the rejection **on the count row**: `rejected_by`, `rejected_at` and `reject_reason`. A count's status is **derived**: `VERIFIED` if `verified_by` is set, `REJECTED` if `rejected_by` is set, `WAITING` otherwise. A CHECK forbids both. | This mirrors `verified_by`/`verified_at` (the existing audit of verify). Every row today stays as it reads, and no backfill is needed (§6). |
| SC-D8 | B can reject **any** waiting count, newest or not. Verify refuses a rejected count with `notWaiting`. | MAIN's 12 waiting counts include ones that `notNewestCount` already blocks. |
| SC-D9 | "The counter can count again": submit has no guard on waiting counts today (`count_service.rs:829-900`), so a new count already works. A rejected count drops out of `WAITING`, and a new submit becomes the newest. **B adds no code to submit.** | Provided test SB6 pins this. |
| SC-D10 | Reasons: trimmed and required (`reasonRequired`), at most 500 UTF-16 units (`validationFailed`, field `reason`). | The owner's "+ เหตุผล" in both decisions. |

## 3. Class diagram

```mermaid
classDiagram
    class AttendanceCloseForCommand {
        <<api/src/presentation/http/v3/commands/branch_day.rs>>
        NAME = "attendanceCloseFor"
        ROUTE = "/api/attendance/{id}/close"  POST
        run(tx, branch, who, CloseForInput) AttendanceClosed
    }
    class CloseForInput {
        <<api/src/presentation/dto/v3/staff_corrections.rs>>
        clockOut: String  RFC 3339
        reason: String
    }
    class AttendanceClosed {
        <<api/src/presentation/dto/v3/staff_corrections.rs>>
        id: String
        employeeId: String
        clockIn: String
        clockOut: String
    }
    class BranchDayService {
        <<api/crates/xc-shift/src/application/branch_day/service.rs>>
        clock(tx, who, branch, ClockDirection, now) AppResult~()~
        +close_for(tx, who, branch, CloseFor, now) Result~Attendance, CloseForError~
    }
    class CloseFor {
        <<api/crates/xc-shift/src/domain/custody/value_objects/close_for.rs>>
        attendance_id: String
        clock_out: DateTime~Utc~
        reason: String
        +check(&self, row: &Attendance, who: &Actor, now, today: NaiveDate, tz: Tz) Result~(), CloseForRefusal~
    }
    class CloseForRefusal {
        <<close_for.rs>>
        SelfClose
        NotOpen
        BeforeClockIn
        InFuture
        NotEarlierDay
        ReasonRequired
        ReasonTooLong
        +code() &'static str
    }
    class CloseForError {
        <<close_for.rs>>
        Refused(CloseForRefusal)
        NotFound
        Other(AppError)
    }
    class CustodyRepository {
        <<api/crates/xc-shift/src/infrastructure/custody/custody_repository.rs>>
        load_state_in / store_state_in / append_events_in
        +branch_timezone_in(tx, branch) AppResult~Tz~
    }
    class OpenAttendanceView {
        <<api/src/presentation/http/v3/views/open_attendance.rs>>
        NAME = "open_attendance"
        rows: OpenAttendanceRow
    }
    class OpenAttendanceRow {
        <<api/src/presentation/dto/v3/staff_corrections.rs>>
        id: String
        employeeId: String
        name: String
        clockIn: String
    }
    class CountRejectCommand {
        <<api/src/presentation/http/v3/commands/cash.rs>>
        NAME = "countReject"
        ROUTE = "/api/cash-counts/{id}/reject"  POST
        run(tx, branch, who, CountRejectInput) CountRejected
    }
    class CountRejectInput {
        <<api/src/presentation/dto/v3/staff_corrections.rs>>
        reason: String
    }
    class CountRejected {
        <<api/src/presentation/dto/v3/staff_corrections.rs>>
        id: String
        status = "REJECTED"
        rejectedBy: String
        rejectedAt: String
        reason: String
    }
    class CashCountService {
        <<api/crates/xc-cash/src/application/count/count_service.rs>>
        submit_in(…) AppResult~CashCount~
        verify_in(…) AppResult~CountVerified~  +refuses a rejected count: NotWaiting
        +reject_in(tx, actor, count_id, reason, now) Result~CashCount, RejectError~
    }
    class CashCount {
        <<api/crates/xc-cash/src/domain/count/cash_count.rs>>
        verified_by / verified_at
        +rejection: Option~CountRejection~  by, at, reason
        +status() CountStatus
    }
    class CountStatus {
        <<cash_count.rs>>
        Waiting
        Verified
        Rejected
    }
    class CashCountRepository {
        <<api/crates/xc-cash/src/domain/repository.rs + infrastructure/repository.rs>>
        find_count_by_id / is_newest_count
        +reject_count(tx, id, by, at, reason) AppResult~()~
    }
    class CashCountsView {
        <<api/src/presentation/http/v3/views/cash_counts.rs>>
        filter status WAITING | VERIFIED | +REJECTED
        row +status +rejectedBy +rejectedAt +rejectReason
    }

    AttendanceCloseForCommand --> BranchDayService : close_for
    AttendanceCloseForCommand ..> CloseForInput
    AttendanceCloseForCommand ..> AttendanceClosed
    BranchDayService --> CloseFor : check
    CloseFor ..> CloseForRefusal
    BranchDayService ..> CloseForError
    BranchDayService --> CustodyRepository
    OpenAttendanceView ..> OpenAttendanceRow
    CountRejectCommand --> CashCountService : reject_in
    CountRejectCommand ..> CountRejectInput
    CountRejectCommand ..> CountRejected
    CashCountService --> CashCountRepository
    CashCountService ..> CashCount
    CashCount ..> CountStatus
    CashCountsView ..> CountStatus
```

**Who provides, who uses**

| Provider | Used by |
|---|---|
| `Permission::AttendanceCloseFor` + the next free seed version + `AreaRow{Rpc,"attendanceCloseFor"}` and `AreaRow{Rpc,"countReject"}` (skeleton SC-0) | the two commands, `me.permissions` (FE shows the form/button only when held) |
| `CloseFor::check` (SC-A1) | `BranchDayService::close_for` (SC-A2) |
| `CustodyRepository::branch_timezone_in` (SC-0 signature, SC-A2 body) | `close_for` |
| `BranchDayService::close_for` (SC-A2) | `AttendanceCloseForCommand` (SC-A3) |
| migration M-SC1 + `CashCount` fields + `CountStatus` (SC-0) | `reject_in` (SC-B1), `CashCountsView` (SC-B3) |
| `CashCountService::reject_in` + `reject_count` (SC-B1) | `CountRejectCommand` (SC-B2) |
| DTOs in `api/src/presentation/dto/v3/staff_corrections.rs` (SC-0) | SC-A3, SC-A4, SC-B2, and the generated TS types for the FE |

A consumer whose provider is not merged yet compiles against SC-0's `todo!()` signature. No ticket edits another ticket's file.

## 4. Behaviour tables

### A. `attendanceCloseFor` (`POST /api/attendance/{id}/close`)

Checked in this order. The first fault wins.

| # | Case | Answer |
|---|---|---|
| A1 | Caller lacks `AttendanceCloseFor` | 400 `roleNotAllowedHere` from the seam (`Policy::authorize`, as every refused command route answers, `command.rs:132-139`). The service's own `Authorizer::check` on the request's branch is the second guard: 403 `roleNotAllowed` / `outOfScope`. |
| A2 | No attendance row `{id}` at this branch | 404 `notFound` |
| A3 | The row is the caller's own | 409 `selfClose` |
| A4 | The row already has a clock-out | 409 `notOpen` |
| A5 | `reason` blank after trim | 400 `reasonRequired`, field `reason` |
| A6 | `reason` > 500 UTF-16 units | 400 `validationFailed`, field `reason` |
| A7 | `clockOut` missing or not RFC 3339 | 400 `validationFailed`, field `clockOut` |
| A8 | `clockOut` < the row's clock-in | 400 `beforeClockIn`, field `clockOut` |
| A9 | `clockOut` > now | 400 `inFuture`, field `clockOut` |
| A10 | The date of `clockOut` in the branch timezone ≥ today's date there | 400 `notEarlierDay`, field `clockOut` |
| A11 | Otherwise | 200 `AttendanceClosed`. The row's `clockOut` = the entered time. One `ops_events` `CLOCK_OUT_FOR` row (SC-D5). The employee's next `clock` IN opens a new row. |
| A12 | Unchanged | `clock` IN/OUT behave exactly as today (`service.rs:325-374`), including for the employee whose old row was closed. |

### B. `countReject` (`POST /api/cash-counts/{id}/reject`)

| # | Case | Answer |
|---|---|---|
| B1 | Caller lacks `ShiftCloseCertify` | 400 `roleNotAllowedHere` from the seam, as for `countVerify`. A caller holding it only on another branch passes the seam on that branch and gets 403 `outOfScope` from the service, which checks the **count's** branch. The service maps the `Authorizer` refusal to its own code, not to `validationFailed` (see §8, gap 1). |
| B2 | No count `{id}` | 404 `notFound` |
| B3 | `HANDOVER_OUT` count | 400 `onlyThroughHandover` (as verify, `count_service.rs:1049-1053`) |
| B4 | Caller counted it | 409 `sameCounter` (`verifier_is_not_counter`) |
| B5 | Count already verified or rejected | 409 `notWaiting` |
| B6 | `reason` blank / > 500 | 400 `reasonRequired` / `validationFailed`, field `reason` |
| B7 | Otherwise | 200 `CountRejected`. The row is written with `rejected_by`, `rejected_at` = now, and `reject_reason`. It leaves the `WAITING` list and shows under `REJECTED`. No ledger posting, no shift change. |
| B8 | `countVerify` on a rejected count | 409 `notWaiting` (new `CashError::NotWaiting`). `countVerify` on a verified count keeps today's answer: `AppError::Conflict` → 409 `staleVersion` (`count_service.rs:1045-1047`, `cash.rs:201-203`). |
| B9 | The counter submits again after a rejection | accepted as today. The new count is the newest and can be verified. |
| B10 | Unchanged | submit, verify and handover of counts that were never rejected, byte for byte |

## 5. Refusal codes (the FE maps these)

| Code | Status | Field | Command |
|---|---|---|---|
| `roleNotAllowedHere` | 400 | — | both, from the seam when the caller lacks the permission. **This and the two below replace the brief's `notManager`**: the permission is configurable (SC-D2), so the screen shows the existing "not allowed" text. |
| `roleNotAllowed` / `outOfScope` | 403 | — | both, from the service's own check (another branch's row or count) |
| `notFound` | 404 | — | both |
| `selfClose` | 409 | — | A |
| `notOpen` | 409 | — | A |
| `beforeClockIn` | 400 | `clockOut` | A |
| `inFuture` | 400 | `clockOut` | A |
| `notEarlierDay` | 400 | `clockOut` | A |
| `reasonRequired` | 400 | `reason` | both |
| `validationFailed` | 400 | `clockOut` / `reason` | both |
| `onlyThroughHandover` | 400 | — | B (new code; verify answers the same case as `validationFailed` today) |
| `sameCounter` | 409 | — | B (`CashError::code()` already names it; verify answers it as `validationFailed` today, gap 1) |
| `notWaiting` | 409 | — | B, and `countVerify` on a rejected count |

The wire shape is the v3 error: `{"error":{"code","field"?,"message"}}` (`api/src/presentation/http/v3/error.rs:9-95`).

## 6. Data migration — approved by the owner (claude/553782's session, 2026-10-28 04:0x ICT: *"อนุมัติทั้งสอง"*: M-SC1 and the `AttendanceCloseFor` seed; write + merge only, deploy is a separate order)

**Part A: no SQL migration.**
- The attendance row's `clockOut` lives in `attendance.data` jsonb.
- The audit is an `ops_events` row; its `kind` has no CHECK and its `details` is jsonb.
- The permission reaches existing databases through the existing boot-time seed sync (`RoleGrants::SEED_ADDITIONS`, `role_grant_store.rs:56-60`), as `MarketRatesRefresh` and the approval permissions did. When the new binary boots, BRANCH_MANAGER, EXECUTIVE and ADMIN gain `AttendanceCloseFor`, and so does every custom group based on them. This is a seed-version write, not a schema change. It is listed here so the owner sees it.

**Part B: migration M-SC1 (additive)**, slot from `scripts/migration-slot.sh next --name cash_count_reject`:

```sql
ALTER TABLE cash_counts
    ADD COLUMN rejected_by uuid REFERENCES users (id),
    ADD COLUMN rejected_at timestamptz,
    ADD COLUMN reject_reason text,
    ADD CONSTRAINT cash_counts_rejected_complete
        CHECK ((rejected_by IS NULL) = (rejected_at IS NULL) AND (rejected_by IS NULL) = (reject_reason IS NULL)),
    ADD CONSTRAINT cash_counts_verified_or_rejected
        CHECK (verified_by IS NULL OR rejected_by IS NULL),
    ADD CONSTRAINT cash_counts_rejecter_not_counter
        CHECK (rejected_by IS NULL OR rejected_by <> counted_by);

-- down:
-- ALTER TABLE cash_counts
--     DROP CONSTRAINT cash_counts_rejecter_not_counter,
--     DROP CONSTRAINT cash_counts_verified_or_rejected,
--     DROP CONSTRAINT cash_counts_rejected_complete,
--     DROP COLUMN reject_reason, DROP COLUMN rejected_at, DROP COLUMN rejected_by;
```

1. **Existing rows after it:** all three columns are NULL. Every existing count reads exactly as before: `WAITING` if `verified_by IS NULL`, `VERIFIED` otherwise. All three CHECKs hold on every existing row, because the NULLs satisfy each of them.
2. **Backfill:** none. Nothing is rejected until someone rejects it. MAIN's 12 waiting counts are rejected **on screen** once this is live (§7), not by SQL.
3. **Deploy order:** migration, then binary. The old binary still runs on the new schema, because it never reads or writes the new columns. One edge: an old binary verifying a count that a new binary already rejected fails on `cash_counts_verified_or_rejected` with a database error, not a wrong write. That window lasts only as long as the rollout.
4. **What `down` loses:** every rejection (who, when, why). Rejected counts become `WAITING` again.

Count before deploy (expect 0 rows that would violate anything, and see how many are waiting):
`SELECT count(*) FILTER (WHERE verified_by IS NULL) AS waiting, count(*) AS total FROM cash_counts;`

## 7. What this does not do

- **No SQL clean-up** of MAIN's 21 stale clock-ins and 12 waiting counts. Once this is live, a manager clears them on screen: A for each clock-in, B for each count.
- No change to how an employee clocks themselves, to submit, or to the shift/ledger side of verify.
- No auto-close of old clock-ins and no expiry of waiting counts.

## 8. Notes for the ticket author (facts, not tickets)

1. **Gap:** `CashError::Refused(r)` prints `{0:?}` (e.g. `RoleNotAllowed`), and `from_cash_error` (`cash.rs:164-210`) matches on the message, so a refusal from inside `verify_in` reaches the client as `validationFailed`. B's `reject_in` must not repeat this: SC-B2 maps the refusal to its own code (B1). Fixing verify is outside this line.
2. `CountVerify` ignores the seam's branch (`cash.rs:348-385`). `countReject` checks the count's own branch, as verify's service does.
3. `branch_day.rs` and `cash.rs` each hold several commands. SC-A3 and SC-B2 each add one struct and one `register` line; no other ticket touches those files.
4. A's view: the FE needs the open rows of **other** employees from earlier days, with their attendance id. `staff_attendance` (`views/staff_attendance.rs`) is on the staff route (`EntryPath::StaffRpc`) and has no filter for open rows from earlier days. SC-A4 adds `open_attendance` (Rpc path, `AttendanceCloseFor`), listing open rows whose clock-in date is before today in the branch timezone.

## 9. Tickets

> **Written with SC-0 (2026-10-28).** SC-0 settled three internal shapes that §3 had only sketched. These are [proposed]-level and are pinned by the up-front tests:
> - `reject_in` answers `Result<CashCount, RejectError>` (`xc-cash/src/domain/count/reject_error.rs`), with one variant per §4 B row and `code()`, so SC-B2 maps by type, not by message.
> - `CashCount` carries one `rejection: Option<CountRejection>` instead of three loose fields.
> - `close_for` answers `Result<Attendance, CloseForError>` (`NotAllowed`, `NotFound`, `Refused(CloseForRefusal)`, `Other`).
>
> The `CLOCK_OUT_FOR` details carry `clockIn`/`clockOut` as RFC 3339 with milliseconds in UTC, and `reason` trimmed.

| Ticket | Class | Does | File scope | Needs first |
|---|---|---|---|---|
| **SC-0** (sub-controller, not dispatched) | — | `Permission::AttendanceCloseFor` (+ `ALL`, module/level, the next free seed version, `SEED_ADDITIONS`); the two `AreaRow`s; `EventKind::ClockOutFor` ("CLOCK_OUT_FOR"); `close_for.rs` with types and `todo!()` `check`; `close_for` and `branch_timezone_in` signatures (`todo!()`); M-SC1; the `CashCount` fields, `CountStatus` and `status()`; the `reject_in` / `reject_count` signatures (`todo!()`); the DTO file; the empty command/view structs; and every up-front test below. | the files of the tickets below, plus `permission.rs`, `permission_module.rs`, `role_grants.rs`, `areas/branch_day.rs`, `areas/cash.rs`, `custody_event.rs`, `cash_count.rs`, `xc-cash/src/domain/error.rs` (`NotWaiting`, `RejectReasonRequired`), `dto/v3/staff_corrections.rs`, `dto/v3/mod.rs`, the migration | this doc merged; owner approved M-SC1 (#564, 2026-10-28); **`feat/binding-quote` merged into main first**, because SC-0 edits the same `permission.rs` / `role_grants.rs` / `permission_module.rs` and the seed version must follow binding quote's 6 |
| **SC-A1** | logic → Argon | `CloseFor::check`: A3–A10, in order | `api/crates/xc-shift/src/domain/custody/value_objects/close_for.rs` | SC-0 |
| **SC-A2** | logic → Argon | `BranchDayService::close_for` (load, permission, find row, check, set `clockOut`, `CLOCK_OUT_FOR`, store); `branch_timezone_in` | `api/crates/xc-shift/src/application/branch_day/service.rs`<br>`api/crates/xc-shift/src/infrastructure/custody/custody_repository.rs` | SC-0 (A1 for green) |
| **SC-A3** | mechanical → Flash | `attendanceCloseFor` command: parse input, call `close_for`, map `CloseForError` to §5 | `api/src/presentation/http/v3/commands/branch_day.rs` | SC-0 (A1+A2 for green) |
| **SC-A4** | mechanical → Flash | `open_attendance` view | `api/src/presentation/http/v3/views/open_attendance.rs` | SC-0 |
| **SC-B1** | logic → Argon | `reject_in` (B1–B7), verify refuses rejected (B8), `reject_count` | `api/crates/xc-cash/src/application/count/count_service.rs`<br>`api/crates/xc-cash/src/infrastructure/repository.rs` | SC-0 |
| **SC-B2** | mechanical → Flash | `countReject` command + its error mapping | `api/src/presentation/http/v3/commands/cash.rs` | SC-0 (B1 for green) |
| **SC-B3** | mechanical → Flash | `cash_counts` view: `REJECTED` filter, `status` and the three fields on the row (`#[ts(optional)]` on each omitted key, #5303); regenerate the committed TS type and its fixture | `api/src/presentation/http/v3/views/cash_counts.rs`<br>`api/src/presentation/dto/v3/cash.rs`<br>`web/src/shared/api/generated/cash_counts.ts`<br>`web/src/shared/testing/fixtures/cash_counts.ts` | SC-0 |

Count: **7 dispatched tickets** (3 Argon, 4 Flash), all released together once SC-0 is on the branch.

### Up-front tests (SC-0 writes them; nobody edits them)

- `api/crates/xc-shift/tests/close_for_check_test.rs` (pure, no DB): one test per row A3–A10, plus "first fault wins" and the timezone edge: a clock-out at 23:30 ICT yesterday passes, and 00:10 ICT today is `notEarlierDay`, with the branch on `Asia/Bangkok` while UTC is still yesterday.
- `api/crates/xc-shift/tests/close_for_service_test.rs` (DB): A1, A2, A11 (row closed, one `CLOCK_OUT_FOR` with the five detail keys, nothing else changed), A12 (the employee clocks in again, and their own `clock` still works).
- `api/tests/attendance_close_for_test.rs` (HTTP through `bootstrap::configure`): every §5 code for A with its status and field; BRANCH_MANAGER of another branch gets `outOfScope`; TELLER gets `roleNotAllowed`; `open_attendance` lists only other people's open rows from earlier days.
- `api/crates/xc-cash/tests/count_reject_test.rs` (DB): B1–B9, the three CHECKs refuse a bad direct write, and SB6 = B9.
- `api/tests/count_reject_http_test.rs` (HTTP): every §5 code for B; `cash_counts?status=REJECTED` lists the rejected count and `WAITING` no longer does; the row's `status`.
- `api/crates/xc-auth/tests/…` (existing seed tests): `AttendanceCloseFor` is in the seed of BRANCH_MANAGER, EXECUTIVE and ADMIN only, and `SEED_VERSION` is the version SC-0 chose (one above main's at that time).

## 10. FE contract (for the FE lane)

- **Manager close form:** list from view `open_attendance` (`id`, `employeeId`, `name`, `clockIn`). Submit with `POST /api/attendance/{id}/close` `{clockOut: RFC 3339 with offset, reason}`, which answers `AttendanceClosed`. Show the form only when `me.permissions` holds `AttendanceCloseFor`. The field errors are on `clockOut` and `reason` (§5). The time picker should default to the clock-in day in the branch timezone.
- **Reject button** beside verify on a `WAITING` count: `POST /api/cash-counts/{id}/reject` `{reason}`, which answers `CountRejected`. It needs `ShiftCloseCertify`, like verify. The `cash_counts` row gains `status` (`WAITING` | `VERIFIED` | `REJECTED`) and, when rejected, `rejectedBy`, `rejectedAt` and `rejectReason`. These keys are **omitted** when absent (not `null`, cf. #5303).
- New i18n: the permission's label for the RBAC screen, and the texts of `selfClose`, `notOpen`, `beforeClockIn`, `inFuture`, `notEarlierDay`, `reasonRequired` and `notWaiting`.
