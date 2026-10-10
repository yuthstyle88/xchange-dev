# Blueprint: organisation, roles, branch scope and users (#550)

- **Source:** `docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html` (#565). "§N" below is the Blueprint's own section.
- **Gap list:** `docs/design/xc-desk-blueprint-v2-mapping.md` §3.2 (roles), K12, K13.
- **v3 measured** at `origin/main` `3690ed5`.
- **Rules:**
  - Blueprint logic is fixed; only the look may change (owner, #564).
  - Brokers, partners and commission belong to Broker-Platform (§1 of #564). PARTNER is therefore not a v3 role.
- **Waiting on this design:** #582 (cash certifiers), #549's approval policy (PR 570 §5.3), the staff report (PR 533
  B.0), the desk (PR 536) and the staff page's branch hours (PR 532 S12).

---

## 0. The owner's answers (2026-10-09, in session claude/e5ab6c)

| # | Question (#550 §8 and this design) | Answer, verbatim | What it fixes |
|---|---|---|---|
| B1 | What becomes of v3 roles with no Blueprint equivalent | "Map by duty" | §3.3: FRONT→TELLER; BACKEND, MANAGER→BRANCH_MANAGER; RATES→RATE_MANAGER; FINANCE→FINANCE; SYSTEM→ADMIN; MONITOR, DOCUMENTS, DOCUMENT_REVIEW→VERIFICATION_REVIEWER. The migration converts the rows and the old roles go |
| B2 | Is a user authenticated here or by Identity | "Everyone logs in via Identity", then, asked again because it reverses the 10-06 order *"ให้ xc-auth เก็บบัญชีพนักงานเอง"*: **"Follow the Blueprint: Identity for everyone"** | §5: OWNER, staff and CUSTOMER sign in through Identity Platform. v3 keeps `auth_subject`, roles and status only, and stores no password or OTP (§07 `User(auth_subject)`, §17). Who creates the Identity account is B6 |
| B3 | The owner account | "OWNER = EXECUTIVE + ADMIN" | §3.3: one user with two organisation-wide `UserRole`s |
| B4 | The service window | "Yes, per branch" | §6: every rule that uses 11:00–22:50/23:00 (`xchange-v3.md` D7) reads the branch's `opening_hours`. A branch with no hours set does not open service |
| B5 | Branch hours (asked by claude/89458e53 on #550) | *"เอาตาม blueprint ที่ผมส่งให้แล้ว"* | per-branch `opening_hours` as A04 sets them (§6) |
| B6 | Who creates a staff member's account (2026-10-10, #1097 and #1157) | *"ให้เจ้าของร้านเป็นคนสร้างบัญชีให้พนักงาน"* · *"ให้เอาตามของเดิม"* · *"หน้าพนักงานไม่ควรสมัครบัญชีตรงนี้"* · *"ลบ register ที่ server ออกด้วย"* · then, on finding an invite step: *"ของเดิมมันก็ต้องอยู่แล้ว มีใครไปเอาระบบเชิญมาใส่"* | §5.3: the OWNER/ADMIN creates the account with a password; the server registers it in `xc-identity` and binds `auth_subject`. No invite code, no self-registration, no public register route. The Blueprint has no invite step (`docs/blueprint/XC-Desk-Blueprint-Data-Model-v2.html`: `User(id, auth_subject UQ, status)` and C14 only); the invite design of 2026-10-09 was this document's own addition and is withdrawn |

---

## 1. v3 today (measured)

- **No organisation.** `git grep -i 'tenant|organi[sz]ation|org_id'` over `api` and `web` finds nothing.
  - **One owner** is assumed: `create_owner` "the one owner" (`api/crates/xc-auth/src/application/account/service.rs:562`) and `LOCK TABLE owners … count(*)>0` (`api/crates/xc-auth/src/infrastructure/account/account_repository.rs:444-457`).
  - The owner is the constant `OWNER_ID="owner"` (`api/src/presentation/extractors/actor.rs:26`).
- **Accounts.** `owners` and `employees`, both with a password hash (`api/migrations/20261007150000_accounts.sql:1-29`).
  - An employee has exactly **one** `role` (text, no CHECK) and **one** `branch_id` (no FK).
  - Sessions are two cookies, `__Host-xc_owner` and `__Host-xc_staff` (`actor.rs:23-24`), and both can be present at once (`api/src/presentation/pages.rs:19-24`).
- **Roles.** `Role { Owner, Front, Backend, Manager, Rates, Monitor, Finance, System, Documents, DocumentReview }`
  (`api/crates/xc-auth/src/domain/policy/value_objects/role.rs:5-45`) has no validity period and no multi-branch scope.
- **Policy.**
  - `Policy::allow(actor, EntryPath, action)` (`api/crates/xc-auth/src/domain/policy/entities/policy.rs:17-40`): 16 entry paths, each with a role × action-string table.
  - It is called from 19 sites (e.g. `compat/rpc.rs:213`, `http/v3/view.rs:349`).
  - `Policy::branch` (`:43-53`) silently **overrides** a staff member's named branch with their own. It does not refuse it.
- **Branches.** `branches(id, name, created_at, created_by, request_id)` (`20261007100000_branches_rates.sql:1-8`). There is no code, address, timezone, hours or booking switch.
- **Fixed hours.** The service window is fixed at `660..1370` / `660..1380` minutes of the day **in Bangkok**
  (`api/crates/xc-shift/src/domain/custody/entities/custody_machine.rs:746`, `:804`, `:1195`, via
  `custody_time.rs:18`), i.e. 11:00, 22:50 and 23:00 (D7).

---

## 2. Decisions

- **D-1. One organisation table, and every entity carries `organization_id`** (§07: *"ทุก entity ในองค์กรมี organization_id
  … ใช้ composite FK หรือ server constraint ป้องกันข้อมูลข้ามองค์กร"*).
  - Tables that already have `branch_id` get `organization_id` with a **composite FK** `(organization_id, branch_id)`
    → `branches(organization_id, id)`, so a row can never name another organisation's branch.
  - Tables without a branch get `organization_id` → `organizations(id)`.
  - The column's `DEFAULT` is the organisation the install migration creates. Until a second organisation exists, no
    insert has to change. Reads are scoped by the actor's organisation (D-4).
- **D-2. Users and scoped roles replace owners and employees** (§07 `User`, `Role`, `UserRole`).
  - A user holds **any number** of `UserRole(role, organization_id, branch_id NULL, valid_from, valid_to)`.
  - `branch_id NULL` means the whole organisation, *"เมื่อมีสิทธิ"* (only for roles whose §02 scope allows it, §3.2).
- **D-3. Nine roles: the Blueprint's nine minus PARTNER, plus VERIFICATION_REVIEWER** (§02, §15). Their scopes and
  duties are §02's, word for word (§3.1). v3-only features keep the role rows they have today, translated through B1
  (§4.2).
- **D-4. Permissions replace action strings, and the server checks organisation, branch scope and owner on every
  request** (§02: *"Server ตรวจทั้ง organization_id, branch scope และ owner ของข้อมูลทุก request; UI ซ่อนปุ่มอย่างเดียวไม่เพียงพอ"*).
  - A request naming a branch outside the actor's scope is **refused** (`outOfScope`, the §10 name). It is no longer
    silently replaced. That replacement was v3's own rule (`policy.rs:43-53`), and the Blueprint rule supersedes it.
- **D-5. Identity authenticates, v3 authorises** (B2).
  - v3 keeps no password and no OTP.
  - Its session is its own cookie, issued after it verifies Identity's token.
  - Disabling a user is v3's `users.status`; Identity is not asked to do anything new.
- **D-6. Branch hours are data** (§07 `Branch.opening_hours JSON`, A04). The rules of D7 read them (§6). A branch with
  no hours is "not set", never a default (owner via claude/89458e53).

---

## 3. Roles, scopes and permissions

### 3.1 The roles (§02, §15), verbatim

| Role | Scope | Does | Control | v3 wire |
|---|---|---|---|---|
| EXECUTIVE | every branch of the organisation | overview; approves rate frames, budgets and exceptions | by the approval policy, per type and amount | `EXECUTIVE` |
| ADMIN | organisation or branches, by grant | system settings, users, roles, branches | **never approves money automatically** | `ADMIN` |
| RATE_MANAGER | assigned branches | drafts rates, adjusts %, proposes publication | an out-of-frame change needs approval | `RATE_MANAGER` |
| BRANCH_MANAGER | own branch | queue, cash, hand-over and shift close | exceptions per the approval policy | `BRANCH_MANAGER` |
| TELLER | branch and the **open** shift | checks customers, takes bookings, makes exchanges | never edits a completed exchange directly | `TELLER` |
| FINANCE | branches, by grant | reconciliation, cost, (commission/payout: Broker-Platform) | maker, approver and payer kept apart | `FINANCE` |
| MARKETING | the data marketing needs | promotion drafts, segments, measurement | **never opens identity documents** | `MARKETING` |
| CUSTOMER | own data | booking, history, benefits | the account is verified before personal data | `CUSTOMER` |
| VERIFICATION_REVIEWER | organisation or branch | document queue, evidence, request changes, approve/reject | not for Marketing/Partner | `VERIFICATION_REVIEWER` |
| *(PARTNER)* | — | — | Broker-Platform, not v3 | — |

### 3.2 Scope rules (what `branch_id` may be)

| Role | `branch_id NULL` (whole organisation) allowed | Several branches |
|---|---|---|
| EXECUTIVE | **required** (*"ทุกสาขาตามองค์กร"*) | — |
| ADMIN, VERIFICATION_REVIEWER | allowed (§02 ADMIN *"องค์กร/สาขาตามสิทธิ"*; §15 VERIFICATION_REVIEWER *"scope ตามองค์กร/สาขา"*) | yes, one row per branch |
| FINANCE | **no** (§02 *"สาขาตามสิทธิ"*: branches, by grant) | yes, one row per branch |
| MARKETING | allowed (§02 *"ข้อมูลที่จำเป็นต่อการตลาด"*: data, not a branch) | — |
| RATE_MANAGER | no | yes (*"สาขาที่ได้รับมอบหมาย"*), one row per branch |
| BRANCH_MANAGER | no | no: one branch (*"สาขาตนเอง"*) |
| TELLER | no | no: one branch. The "open shift" part of its scope is checked by the shift rules (#582) |
| CUSTOMER | not branch-scoped: own data only (owner check, §3.5) | — |

These are enforced by `ScopeRule::check(role, branch_id, existing_rows)`
(`api/crates/xc-auth/src/domain/access/scope_rule.rs`) when a role is granted. A role row outside
`[valid_from, valid_to)` grants nothing.

### 3.3 v3 roles → Blueprint roles (B1, B3)

| v3 | Blueprint | Why (by duty) |
|---|---|---|
| OWNER | EXECUTIVE **and** ADMIN, both organisation-wide (B3) | approves (EXECUTIVE) and manages users and branches (ADMIN) |
| FRONT | TELLER | serves customers at the desk |
| BACKEND | BRANCH_MANAGER | opens and closes service, dispatches and commits transfers (`policy.rs` Custody rows) = §02's *"เงินสด ส่งมอบและปิดกะ"* |
| MANAGER | BRANCH_MANAGER | branch head |
| RATES | RATE_MANAGER | rates |
| FINANCE | FINANCE | reconciliation |
| SYSTEM | ADMIN, scoped to the employee's branch | system settings |
| MONITOR, DOCUMENTS, DOCUMENT_REVIEW | VERIFICATION_REVIEWER, scoped to the employee's branch | reviews customers and documents |

Each v3 employee's single `branch_id` becomes the `branch_id` of the mapped role row. The OWNER's rows are
organisation-wide.

### 3.4 Permissions

`Permission` (`api/crates/xc-auth/src/domain/access/permission.rs`) is one enum. There is one variant per
Blueprint screen or capability (§03), plus the v3-only capabilities (§4.2). `RolePermissions`
(`api/crates/xc-auth/src/domain/access/role_permissions.rs`) is the **one** table role → permissions, written
from §02/§03 and §4.2 below. No other file decides access.

| Permission | Blueprint | Roles |
|---|---|---|
| `OverviewRead` | A01 | EXECUTIVE |
| `RatesDraft` (draft, simulate, submit, publish) | A02, §05; EXECUTIVE per PR 570 A5 (owner, 2026-10-09: *"OWNER publishes directly"*, which presupposes OWNER drafting) | RATE_MANAGER, EXECUTIVE |
| `RatesHistoryRead` | A03 | RATE_MANAGER, EXECUTIVE |
| `RatePolicySave` | §02 EXECUTIVE approves rate frames | EXECUTIVE (PR 570 A3) |
| `BranchesManage` | A04 | ADMIN |
| `UsersManage` (users, roles, scopes) | A05 | ADMIN |
| `ApprovalsDecide` | A06 | per the approval policy of each type (PR 570 §5.3); a role only qualifies, the type's policy decides |
| `ShiftOperate` (open, count, hand over) | B01 | TELLER, BRANCH_MANAGER |
| `ShiftCloseCertify` | B01 *"ปิดและรับรอง"* | BRANCH_MANAGER (#582) |
| `BookingsDesk` | B02 | TELLER |
| `ExchangeTransact` | B03 | TELLER |
| `TransactionRead`, `ReceiptReprint`, `ReversalRequest` | B04 | TELLER, BRANCH_MANAGER |
| `CashManage` (stock, transfers, counts) | B05 | BRANCH_MANAGER, FINANCE |
| `QueueOperate` | B06 | TELLER, BRANCH_MANAGER |
| `SystemSettings` (devices, printers, payment settings) | §02 ADMIN *"ตั้งค่าระบบ"*; EXECUTIVE by B1 | ADMIN, EXECUTIVE |
| `CompatRead`, `RequestAbandon`, `DocumentsList`, `FileRead`, `ProofUpload`, `DeviceQueue`, `DeviceProcess`, `DevicePrinterOnline`, `DrawerPassport` | v3-only features; roles = the B1 translation of today's rows (§4.2) | per §4.2 |
| `FinanceReconcile` | F01 | FINANCE |
| `VerificationReview` | F02 | VERIFICATION_REVIEWER |
| `IdentityDocumentRead` | B03 *"ตรวจลูกค้า"*, F02; never MARKETING (§02) | TELLER, VERIFICATION_REVIEWER |
| `MarketingCampaigns`, `MarketingMembers` | M01–M03 | MARKETING |
| `MarketingPerformance` | M04 | MARKETING, EXECUTIVE |
| `CustomerSelf` (own bookings, history, receipts, profile, consent) | C01–C14 | CUSTOMER, with the owner check |
| `AttendanceClock` (clock in and out) | v3 custody feature kept by the owner (#582 A4, *"ใช้ตามนี้"*) | every staff role: all roles except CUSTOMER (#582 decides the exact set) |
| `CustomerHoldPlace` (place a hold; releasing one is `VerificationReview`) | v3 custody feature kept (#582 A4) | TELLER, BRANCH_MANAGER |
| `CashTargetSet` | v3 custody feature kept (#582 A4) | FINANCE |
| `BranchDayOperate` (start service, cut-off) | v3 custody feature kept (#582 A4); §02 BRANCH_MANAGER *"เงินสด ส่งมอบและปิดกะ"* | BRANCH_MANAGER |

The last four rows are named by the cash design (#582, PR 590) and live in this one table, so that
`role_permissions.rs` keeps one owner (OR-2). #582's rules check them only through `Authorizer::check`.

### 3.5 The check (D-4)

`Authorizer::check(actor, permission, target) -> Result<Grant, Refusal>`
(`api/crates/xc-auth/src/domain/access/authorizer.rs`), pure. In order:

1. **No actor** → `signIn`.
2. **Organisation.** The target's organisation must be the actor's, else `outOfScope`.
3. **Permission and branch.** The actor needs a role row that is valid now, carries the permission, and has a scope
   covering the target's branch (`branch_id` equal, or `NULL` where §3.2 allows). Otherwise `outOfScope` if the role
   exists elsewhere, else `roleNotAllowed`.
4. **Owner.** For `CustomerSelf`, the target's customer must be the actor's customer, else `outOfScope`.

`Grant { branch: BranchId, roles_used }` is what handlers use. The branch is the **named** branch when it is in
scope. A user who names none gets the **session branch** (§14): the branch the user chose in the account bar, kept on
the session. Before §14 lands, the old rule applies: a user whose roles cover exactly one branch gets that branch, and a
user with an organisation-wide role gets `MAIN` (`policy.rs:43-53`). §14 replaces the `MAIN` fallback (owner, 2026-10-10).

---

## 4. How `xc-auth` and its callers change

### 4.1 Types

| Today | Becomes | File |
|---|---|---|
| `Role` (10 v3 variants) | `access::Role` (9 Blueprint variants, §3.1), **in a new file beside the old one** | `xc-auth/src/domain/access/role.rs` (new). The old `policy/value_objects/role.rs` stays until OR-7 |
| `Actor { id, kind, role, branch_id }` | `access::Actor { user_id, organization_id, roles: Vec<ScopedRole>, customer_id: Option }`, also new | `xc-auth/src/domain/access/actor.rs` (new). The old `actor.rs` and `actor_kind.rs` stay until OR-7 |
| `ActorKind {Owner, Staff}` | none in the new model; deleted with the old types in OR-7 | — |
| `ScopedRole` | new: `{ role, branch: Option<BranchId>, valid_from, valid_to }` | `xc-auth/src/domain/access/scoped_role.rs` |
| `Policy::allow(actor, EntryPath, action)` | `Authorizer::check(actor, Permission, Target)` (§3.5) | `xc-auth/src/domain/access/authorizer.rs` |
| `Policy::branch` | `Grant.branch` (§3.5) | — |
| `EntryPath` (16) | kept **only** as the route-to-permission table of §4.2 while compat routes live; each route names its `Permission` | `xc-auth/.../entry_path.rs` |
| v3 view / action / command `INHERITS: (EntryPath, &str)` | **kept** (amended 2026-10-10, lane controller's ruling (b) on #767): each file still declares `INHERITS`, and the route table of §4.2 resolves it to a `Permission` (`Policy::need(INHERITS)`). A file that answers anyone declares an explicit public marker instead | `api/src/presentation/http/v3/{view,action,command}.rs`; the per-file declarations are unchanged |

**Why `INHERITS` stays rather than a per-file `REQUIRES: Permission`.** The answer is the same permission either way, because
the route table is the single place a path·action becomes a `Permission`. Renaming the declaration in about 50 view and command
files changes no behaviour and would collide with the tickets that own those files (#759, #651, #603).
- **Guard (OR-4b, #767):** a test fails if any view, action or command file declares neither `INHERITS` nor the explicit public
  marker, so a new route cannot skip the table.

**The seam lands green.** OR-2 adds `domain/access/**` **beside** the old types and touches no file that uses them.
- OR-3 switches the extractor (`extractors/actor.rs`) to `access::Actor`.
- OR-4 switches `policy.rs` (reduced to the route → permission table) and the 19 call sites.
- OR-7 deletes the old `role.rs`, `actor.rs` and `actor_kind.rs`.

Every ticket compiles on its own.

### 4.2 Every v3 route and action → its permission (the controller's ruling (b))

**The rule** (lane controller's ruling on §4.3, issuecomment on PR 589):
- each row's roles are the **B1 translation of today's roles**: OWNER→EXECUTIVE, FRONT→TELLER, BACKEND/MANAGER→BRANCH_MANAGER, RATES→RATE_MANAGER,
  FINANCE→FINANCE, SYSTEM→ADMIN, MONITOR/DOCUMENTS/DOCUMENT_REVIEW→VERIFICATION_REVIEWER;
- a row changes **only** where the Blueprint's role table says otherwise, and the change is listed in the last column
  with its citation.

The owner's user holds EXECUTIVE + ADMIN (B3), so wherever OWNER was allowed, the owner still is: OWNER's rows go to
EXECUTIVE, SYSTEM's to ADMIN. Each compat path keeps its own row while compat routes live (`EntryPath`, §4.1).

| Path · action (today, `policy.rs`) | Today | B1 translation | Permission | Roles after | Change (Blueprint) |
|---|---|---|---|---|---|
| Rpc `adminSnapshot`, `adminDeskConfig`, `getConnectionStatus` | OWN + 9 staff | E, T, BM, RM, VR, F, A | `CompatRead` (v3-only; retires with V4) | same | none |
| StaffRpc `adminSnapshot`, `getConnectionStatus` | 9 staff | T, BM, RM, VR, F, A | `CompatRead` | same | **C1** below |
| StaffRpc `adminDeskConfig`; StaffDesk; StaffRpc `reserveCustomer`, `saveTransaction` | FR | T | `ExchangeTransact` | T | none |
| Rpc `reserveCustomer`, `saveTransaction` | OWN, FR | E, T | `ExchangeTransact` | T | **C2**: E refused (§03 B03 user TELLER) |
| Rpc / StaffRpc `adminSaveRate`; Kiosk `manage`, `configure` | OWN, RT | E, RM | `RatesDraft` | RM, E | none. E drafts per PR 570 A5 (owner, 2026-10-09: OWNER drafts and publishes) |
| Rpc `adminOpenShift`, `adminCloseShift` | OWN, BE, MG | E, BM | `ShiftOperate` / `ShiftCloseCertify` | **#582** | #582's owner-confirmed table (PR 590) |
| Rpc `adminMoveStock` | OWN, FI | E, F | `CashManage` | BM, F | **C3**: E refused, BM allowed (§03 B05 users MANAGER/FINANCE) |
| Rpc `adminCreateBranch`; OwnerRoute (branches, employees) | OWN | E | `BranchesManage` / `UsersManage` | A | **C4**: E refused, A allowed within its scope (§03 A04/A05 user ADMIN; §02 ADMIN *"ผู้ใช้ บทบาท สาขา"*). The owner keeps it through ADMIN |
| DeviceSetup; Devices `setPrinter`, `setDrawer`; PaymentMethods `save` | OWN | E | `SystemSettings` | E, A | **C5**: A allowed (§02 ADMIN *"ตั้งค่าระบบ"*) |
| Rpc `adminAbandonRequest` | OWN | E | `RequestAbandon` (v3-only) | E | none |
| StaffRpc `documents`; StaffFile | all staff but RT, SY | T, BM, VR, F | `IdentityDocumentRead` | same | none |
| UserRoute `documents`; File | OWN + all signed in (File: but RT, SY) | as translated | `DocumentsList` / `FileRead` (v3-only) | same | **C6**: MARKETING refused (§02 *"ไม่เปิดสิทธิเอกสารยืนยันตัวตน"*). The role is new, so no v3 user loses anything |
| Upload | OWN, FR, FI | E, T, F | `ProofUpload` (v3-only) | same | none |
| StaffUpload | FR, FI | T, F | `ProofUpload` | same | none |
| UserRoute (other); Devices other names; Kiosk `notifications`, `list`; PaymentMethods `list` | anyone signed in | every role | `SignedIn` | same | none |
| Devices `queue` | OWN, FR, BE, MG | E, T, BM | `DeviceQueue` (v3-only) | same | none |
| Devices `process`, `retry`, `drawerObservation` | OWN, FR, BE | E, T, BM | `DeviceProcess` (v3-only) | same | **C7**: ex-MANAGER users gain them, because B1 merges BACKEND and MANAGER into BRANCH_MANAGER (the owner's mapping) |
| Devices `printerOnline` | OWN, BE | E, BM | `DevicePrinterOnline` (v3-only) | same | **C7** (same cause). TELLER does not gain it |
| Kiosk `identityChecked`, `publishReceipt`, `get`, `accept` | OWN, FR, BE, MG | E, T, BM | `QueueOperate` | T, BM | **C8**: E refused (§03 B06 users TELLER/MANAGER) |
| Kiosk `rates`, `quote`, `confirm`, `status`, `customerReceipt` | everyone | public | public | public | none |
| DrawerPassport | OWN, FR, BE, MG | E, T, BM | `DrawerPassport` (v3-only) | same | none |
| StaffBookings | FR, BE, MG | T, BM | `BookingsDesk` | T | **C9**: BM refused (§03 B02 user TELLER) |
| OwnerRoute `/api/bookings`; StaffBookings `member`, `settings`, `registrations`, `approveRegistration` | OWN | E | `BookingsAdmin` (v3-only) | E | none. A v3-only row: the Blueprint's role table has no such route. Owner-only today, kept as it is; revisit with #555 and the RE work (amended 2026-10-10, ruling (c) on #766) |
| OwnerRoute `/api/reference-rates` | OWN | E | `ReferenceRatesRead` (v3-only) | E | none. Same as the row above |
| Custody `ops*` | per action | — | #582 (PR 590): `ShiftOperate`, `ShiftCloseCertify`, `CashManage`, `FinanceReconcile`, `VerificationReview`, `ApprovalsDecide`, `AttendanceClock`, `CustomerHoldPlace`, `CashTargetSet`, `BranchDayOperate` | #582 | #582's |

**C1, the one change that comes from the user model rather than a table row.** Today the owner is refused on every
staff path (StaffRpc, StaffFile, StaffUpload, StaffBookings), because the owner is not an employee (`ActorKind`).
- The Blueprint has one `User` holding `UserRole`s (§07). There is no owner kind, so a staff path admits whoever holds
  one of its roles.
- The owner's user holds **EXECUTIVE + ADMIN** organisation-wide (B3), and the check is per role, not per path. So on a
  staff path it now passes every row that either role passes:
  - `CompatRead` on StaffRpc, through ADMIN (ex-SYSTEM);
  - `RatesDraft` on StaffRpc (`adminSaveRate`, `rateDraft`), through EXECUTIVE;
  - `ProofUpload` on StaffUpload, through EXECUTIVE.
- No other staff-path row allows EXECUTIVE or ADMIN, so nothing else changes.
- Measured in #766's `api/crates/xc-auth/tests/fixtures/blueprint_policy_cases.json`: C1 tags the 18 v2 cases where the owner
  asks on a staff path. Four are now allowed (StaffRpc `adminSnapshot`, `getConnectionStatus`, `adminSaveRate`; StaffUpload
  `upload`). The other 14 stay refused, now with the role's refusal instead of the not-an-employee one. `rateDraft` has no
  v2 case; RE-3's row covers it.
  (Amended 2026-10-10: the earlier text named `CompatRead` only, which missed EXECUTIVE's rows; lane controller's ruling (a)
  on #766.)

---

### 4.3 Moving the provided tests (the controller's ruling: (a) 1:1 table, (b) changes cited, (c) one commit per file)

**The fixture.** The 1,425 cases of `v2_policy_cases.json` (`policy_test.rs:54-69`) are re-expressed once by
`api/tools/policy-map.mjs` (OR-4):
- each case's actor becomes the B1-mapped user (OWNER → one user with EXECUTIVE + ADMIN);
- its expected outcome stays **unless** the case's (path, action, mapped role) is in C1–C9;
- the output is `blueprint_policy_cases.json`, with the same count.

The test `the_mapping_changes_exactly_c1_to_c9` asserts that the outcomes differing between the two files are exactly
the cases C1–C9 name, no more and no fewer. Then `every_mapped_case_is_answered_as_mapped` replays them all.

**`policy_test.rs`, one row per test:**

| Test (line) | New test | Kept / changed / dropped |
|---|---|---|
| `the_fixture_is_whole` (:52) | `the_mapped_fixture_is_whole` (1,425) | kept |
| `every_answer_v2_gave_is_given_again` (:99) | `every_mapped_case_is_answered_as_mapped` + `the_mapping_changes_exactly_c1_to_c9` | kept, changes C1–C9 only |
| `the_sentences_said_are_exactly_v2s_twenty_two` (:128) | same name | kept: the Thai refusal sentences are unchanged. `outOfScope` adds one (§10) |
| `every_call_runs_on_the_branch_v2_ran_it_on` (:147) | `every_call_runs_on_the_branch_named_in_scope` | changed: a branch outside scope is **refused** (§02 *"Server ตรวจ … branch scope"*) |
| `which_branch_an_employee_belongs_to_changes_no_answer` (:169) | same | kept |
| `roles_are_spelled_as_v2_spells_them` (:189) | `roles_are_spelled_as_the_blueprint_spells_them` (the nine of §3.1) | changed (§02, §15; B1) |
| `a_role_v2_does_not_have_is_no_role` (:214) | `a_role_the_blueprint_does_not_have_is_no_role` (`FRONT`, `OWNER`, `PARTNER` refused) | changed (§02; PARTNER is Broker-Platform's) |
| `the_owner_has_no_branch_and_an_employee_always_has_one` (:235) | `scope_rules_hold` (§3.2: EXECUTIVE organisation-wide; TELLER and BRANCH_MANAGER exactly one branch) | changed (§07 UserRole, §02 scopes) |
| `nobody_is_an_employee_with_the_owners_role` (:254) | — | **dropped**: §07 has no owner kind (one `User`). `scope_rules_hold` covers what remains |
| `an_employee_is_pinned_to_their_own_branch_whatever_they_name` (:264), `naming_another_branch_is_not_refused_it_is_ignored` (:281) | `naming_a_branch_outside_scope_is_refused` | changed (§02 server checks branch scope) |
| `the_owner_works_on_the_branch_named_and_on_main_when_none_is` (:293) | `an_organisation_wide_user_works_on_the_branch_named_and_on_main_when_none_is` | kept (MAIN is v3's rule; the Blueprint is silent) |
| `a_caller_who_is_not_signed_in_is_on_the_branch_named` (:301) | same | kept |
| `opening_and_closing_a_shift_depends_on_the_path` (:310) | moves to #582's tickets | #582 |
| `the_desk_configuration_depends_on_the_path` (:320), `reading_a_transactions_documents_depends_on_the_path` (:337), `the_two_paths_refuse_in_sentences_one_word_apart` (:371) | same names | kept (per-path rows), plus C6 |
| `what_each_role_may_write_on_both_paths` (:382) | same | kept, with C2, C3, C4 |
| `the_owner_may_do_all_eleven_actions_of_the_owner_rpc` (:397) | `the_owners_user_may_do_the_owner_rpc_actions_its_roles_allow` | changed: C2, C3 (C4 keeps it through ADMIN) |
| `the_owner_is_let_through_wherever_a_signed_in_caller_is` (:417) | same | kept |
| `the_owner_is_not_an_employee` (:437) | `a_staff_path_admits_whoever_holds_its_role` | changed: C1 (§07) |
| `only_the_owner_passes_an_owner_route` (:462) | `only_admin_passes_branch_and_user_management` | changed: C4 (A04/A05) |
| `an_unknown_action_is_refused_in_the_words_of_its_path` (:481), `the_device_route_does_not_stop_a_name_it_has_no_list_for` (:548), `paths_with_one_purpose_do_not_read_the_action` (:562) | same | kept |
| `the_staff_bridge_lets_through_whatever_starts_with_ops` (:523), `each_custody_action_has_its_roles` (:720), `signing_is_refused_in_three_different_sentences` (:744), `the_owner_starts_custody_and_does_nothing_else_in_it` (:762) | move to #582's tickets | #582 |
| `front_and_finance_upload_evidence_and_nobody_else_on_the_staff` (:590), `rates_and_system_read_no_customer_file_and_each_path_says_so_differently` (:609), `the_staff_desk_page_is_fronts` (:627) | renamed to the mapped roles | kept |
| `device_actions_have_their_own_role_lists` (:646) | same | kept, with C5, C7 |
| `the_customer_kiosk_needs_nobody_signed_in` (:668) | same | kept |
| `the_kiosk_queue_is_for_front_backend_manager_and_the_owner` (:677) | `the_kiosk_queue_is_for_teller_and_branch_manager` | changed: C8 |
| `a_refusal_carries_the_status_v2_answered_with` (:784), `a_refusal_is_an_error_whose_text_is_the_sentence` (:804), `an_action_is_its_v2_name_and_nothing_else` (:818) | same | kept |

**The other files:**

| File | Becomes |
|---|---|
| `policy_common/mod.rs` (`PATHS`, `:106`) | kept: `EntryPath` stays the route table |
| `policy_addon_paths_test.rs`, `policy_kiosk_test.rs`, `policy_staff_bookings_test.rs` | the same assertions with mapped roles, with C5, C7, C8 and C9 |
| `api/tests/rpc_common/mod.rs:598-622` `one_of_each_role` | the same 10 users in the same order, each with its B1-mapped role(s): OWNER → EXECUTIVE + ADMIN, FRONT → TELLER, BACKEND → BRANCH_MANAGER, MANAGER → BRANCH_MANAGER, … So the 38 files that index it keep their positions |
| `STAFF_ROLES` (`v3_view_test.rs:201-211`, `v3_me_version_test.rs:195-205`) | the 9 entries mapped by B1, in the same order (BRANCH_MANAGER and VERIFICATION_REVIEWER appear more than once, on purpose) |
| `compat_staff_view_test.rs:224,566` | mapped roles: RATES→RATE_MANAGER, SYSTEM→ADMIN keep their stripped views |
| `custody_role_test.rs` | #582 |
| `auth_http_test.rs:488-490`, `account_test.rs` (password login, `:456` `create_owner`) | **dropped with OR-7**: §17 *"ไม่เก็บ raw password/OTP"* and B2. Replaced by OR-3's sign-in tests and OR-5's create/activate tests |

**(c) Order of commits.** Each code ticket that moves a file above moves it in **one commit per file**, before any
behaviour change, so the reviewer diffs assertion by assertion.

---

## 5. Sign-in through Identity (B2)

### 5.1 Flow

```mermaid
sequenceDiagram
  participant B as Browser
  participant X as v3 (xc-auth BFF)
  participant I as Identity Platform
  B->>X: POST /api/auth/login {identifier, password | otp…}
  X->>I: POST /auth/login (client credentials of the "xchange" client app)
  I-->>X: access token (EdDSA JWT, aud=xchange) + refresh token
  X->>X: verify JWT (JWKS / pinned key), read sub
  X->>X: users WHERE auth_subject = sub AND status = ACTIVE → roles
  X-->>B: Set-Cookie __Host-xc_session (opaque; user_id + users.version)
```

- **The routes v3 forwards.** Identity's `POST /auth/login`, `/auth/refresh`, `/auth/logout`, `/auth/otp/request` and
  `/auth/otp/verify`
  (`108-platform-services` `crates/auth/src/api/mod.rs:84-103`). The browser never sees the client secret.
- **Errors** are mapped by Identity's error code, never by HTTP status, because Identity reuses 401/404/403 for both
  user and caller faults. Unknown codes, caller-side codes, an unreadable body and 5xx all become `503` with a log line.
- **Sessions.** v3 keeps its own session row (`user_sessions(hash, user_id, version, expires, refresh_token_enc)`). The
  refresh token is stored encrypted server-side. `users.version` moves on any role or status change, which ends that
  user's sessions.
- **One cookie** `__Host-xc_session` replaces `__Host-xc_owner` + `__Host-xc_staff`, so a browser has exactly one
  actor. The "owner wins" rule (`view.rs:339-342`) disappears with it.

### 5.2 Which Identity, and the client (done 2026-10-09)

v3 uses **its own Identity instance on the xchange host**, `xc-identity.service` on do-sgp1. It is not the Dell Identity.
The owner's order: *"Identity จะต้อง deploy ที่โฮสต์ xchange ไม่ยุ่งกับของเดิม"*. The host record is PR 593,
`deploy/HOST-do-sgp1.md` §4.

| Item | Value (from that record) |
|---|---|
| Address | `127.0.0.1:8095`; public `xc-id.spacing.cloud` once DNS and the certificate exist (I-1, the owner's) |
| Token | `iss=auth-service`, `aud=xchange`, 15-minute life, EdDSA; keys at `/.well-known/jwks.json` |
| The xchange client | already registered (`audience=xchange`). Its credentials are in `/etc/xc-v3/identity-client.env` on the host (`IDENTITY_BASE_URL`, `IDENTITY_CLIENT_ID`, `IDENTITY_CLIENT_SECRET`, `IDENTITY_AUDIENCE`, `IDENTITY_ISSUER`, …), never in the repo |
| Registration | requires a phone number (`phone_required`). The server registers each staff member with the phone and password the OWNER/ADMIN enters (§5.3); nobody registers themselves |
| OTP delivery | `noop` today, so sign-in is by password until an SMS/e-mail sender is chosen (I-3, the owner's) |

- **Token verification** (the JWKS, `aud`, `iss`) is ticket **I-2** under #554's design. OR-3 uses I-2's verifier, under
  the name #554 gives it, and does not build a second one.
- **Staff sign-in.** Everyone signs in through `xc-identity`: staff, OWNER and CUSTOMER (B2 in §0, 2026-10-09; the
  host record's I-2 row says the same, PR 593 `deploy/HOST-do-sgp1.md:88`). All of them verify through the same I-2
  verifier.

### 5.3 Creating a staff account: the OWNER/ADMIN creates it (B6)

Identity gains no feature: v3 calls the registration `xc-identity` already has (`POST /auth/register` with phone and
password, the call `IdentityClient::register` made from PR 760 until #1097 removed the public route). Only the
server calls it, and only for an OWNER/ADMIN.

- **Creating a user.** An OWNER/ADMIN enters name, phone, an initial password, roles and scopes (A05, and the existing
  employee create, *"ของเดิม"*). In one request v3:
  1. inserts the `users` row (`PENDING`, `auth_subject NULL`) and its `user_roles` rows, and commits;
  2. calls `IdentityClient::register` server-side (`POST /auth/register` behind the client's `x-api-key`;
     108-platform-services `crates/auth/src/api/mod.rs:83-84`, request `contract/mod.rs:12-36`) with the phone in
     E.164, the password and `must_change_password = false`; the answer's `identityId` is the `sub`;
  3. sets `auth_subject = sub` (`UNIQUE (auth_subject)`) and `ACTIVE`, `version+1`.

  The password is never stored or logged (§17 *"ไม่เก็บ raw password/OTP"*).
  - **Identity refuses or is down at step 2** ⇒ the user stays `PENDING` and the OWNER/ADMIN sees a typed reason;
    activating it later (below) retries step 2.
  - **A phone Identity already has** (`PhoneTaken`; a phone is unique across every 108 app, `register.rs:131-136`)
    ⇒ refused as `phoneTaken`, never silently bound: binding an existing account needs its own decision.
  - **Step 3 fails after Identity created the account** ⇒ the `identityId` is written to `user_audit` with the
    error, and the reason names it. A retry gets `phoneTaken`, so that account is removed in Identity by hand. Two
    writes in two systems cannot share one transaction; this case is expected to be rare and stays visible.
  - **Forcing a password change on first sign-in:** no (`must_change_password = false`). The OWNER hands the password
    over in person, as in v2 (*"ของเดิม"*). §13 O-4 lets the owner overturn this.
- **Signing in.** The staff member signs in at `/login` through Identity with that phone and password (§5.1), and
  changes the password in Identity.
- **A user with no account yet** (`PENDING`, `auth_subject NULL`: rows migrated from v2 `employees`, §9). The
  OWNER/ADMIN activates it by entering phone and password; the server registers it as above, binds `auth_subject`
  and sets `ACTIVE`. There is no invite code.
- **Disabling.** `users.status = DISABLED` plus `version+1`. All sessions end. Identity is untouched.
- **The first user of an install.** The `create-owner` CLI takes the owner's phone and password, registers them in
  `xc-identity` the same way, and creates the organisation's first user, `ACTIVE`, with EXECUTIVE + ADMIN (B3).
  `create-admin-invite` is withdrawn.
- **No public registration.** There is no `/api/auth/register` and no invite or claim route (B6, #1097).

**Withdrawn 2026-10-10 (#1157):** the invite design (`invite_code`, `user_invites`, `/api/auth/invite/claim`,
`create-admin-invite`, the login screen's invite step). It came from this document, not from the Blueprint or the
owner, and after #1097 removed self-registration it left new staff with no way in. Removing it is #1157; its data
migration is §9.4.

### 5.4 CUSTOMER

A customer is a `User` with role CUSTOMER and an `auth_subject`. The `Customer` record, its link to the user, its
verification and consent belong to #554 and #557. This design gives only the role and the owner check (§3.5).

---

## 6. Branches and hours (D-6, B4, B5)

- **`Branch` gains** (§07, A04): `organization_id`, `code` (unique per organisation), `address`, `timezone` (IANA),
  `opening_hours jsonb NULL` and `booking_enabled bool`.
- **`opening_hours` shape** [required], weekly, as A04 sets *"เวลาเปิด"*:
  `{"mon":[{"open":"11:00","close":"23:00"}], …, "sun":[]}`. An empty day is closed. `NULL` means "not set".
  Holiday and closure dates are booking slots' (#555), not here.
- **`BranchHours::window(date) -> HoursWindow { Closed | NotSet | Open { open, close } }`**
  (`api/crates/xc-rate/src/domain/branch/branch_hours.rs`) uses the branch timezone. It replaces the Bangkok minutes:

| v3 rule today (`custody_machine.rs`) | Becomes |
|---|---|
| `:746` / `:804`: begin a transaction or start service only within `660..1370` (11:00–22:50) | within `[open, close − 10 min)` of today's window |
| `:1195`: save only within `660..1380` (11:00–23:00) | within `[open, close)` |
| `NotSet` or `Closed` | service does not open: `OutsideServiceHours` / `ServiceNotReady`, as today outside the window |

The 10 minutes before close is v3's existing rule (D7, v2 parity). The Blueprint does not state it, so it stays.
`CustodyTime::bangkok_minute_of_day` (`custody_time.rs:18`) is replaced by the branch-timezone minute.

---

## 7. Class diagram

```mermaid
classDiagram
  direction LR
  class Organization { <<xc-auth domain/organization/organization.rs>> id; code; legal_name; timezone }
  class User { <<xc-auth domain/user/user.rs>> id; organization_id; auth_subject?; status; version }
  class UserStatus { <<enum xc-auth domain/user/user_status.rs>> PENDING; ACTIVE; DISABLED }
  class Role { <<enum xc-auth domain/access/role.rs>> 9 Blueprint roles }
  class ScopedRole { <<xc-auth domain/access/scoped_role.rs>> role; branch?; valid_from; valid_to? }
  class ScopeRule { <<xc-auth domain/access/scope_rule.rs>> +check(role, branch, rows) }
  class Permission { <<enum xc-auth domain/access/permission.rs>> }
  class RolePermissions { <<xc-auth domain/access/role_permissions.rs>> +of(role) }
  class Authorizer { <<xc-auth domain/access/authorizer.rs>> +check(actor, permission, target) Grant }
  class Actor { <<xc-auth domain/access/actor.rs>> user_id; organization_id; roles; customer_id? }
  class UserRepository { <<trait xc-auth domain/user/user_repository.rs>> }
  class PgUserRepository { <<xc-auth infrastructure/user/user_repository.rs>> }
  class UserService { <<xc-auth application/user/user_service.rs>> +create_with_account() +activate() +grant() +revoke() +disable() }
  class IdentityClient { <<trait xc-auth domain/identity/identity_client.rs>> +login() +refresh() +logout() +otp…() +register() }
  class HttpIdentityClient { <<xc-auth infrastructure/identity/http_identity_client.rs>> }
  class TokenVerifier { <<provided by I-2 (#554's design); name per that design>> +verify(jwt) Subject }
  class SessionService { <<xc-auth application/session/session_service.rs>> +sign_in() +actor_of(cookie) +sign_out() }
  class Branch { <<xc-rate domain/branch/entities/branch.rs>> +organization_id; code; address; timezone; opening_hours; booking_enabled }
  class BranchHours { <<xc-rate domain/branch/branch_hours.rs>> +window(date) HoursWindow }

  User --> UserStatus
  User "1" --> "*" ScopedRole
  ScopedRole --> Role
  ScopeRule --> ScopedRole
  RolePermissions --> Role
  RolePermissions --> Permission
  Authorizer --> RolePermissions
  Authorizer --> Actor
  Actor "1" --> "*" ScopedRole
  PgUserRepository ..|> UserRepository
  UserService --> UserRepository
  UserService --> ScopeRule
  UserService --> IdentityClient
  SessionService --> IdentityClient
  SessionService --> TokenVerifier
  SessionService --> UserRepository
  HttpIdentityClient ..|> IdentityClient
  Branch --> BranchHours
  Organization "1" --> "*" Branch
  Organization "1" --> "*" User
```

HTTP: `api/src/presentation/http/auth.rs` (rewritten: the BFF routes of §5.1; no invite, claim or register route, §5.3);
`api/src/presentation/extractors/actor.rs` (rewritten: one cookie → `Actor`). The A04/A05 commands are
`CommandAction`s (PR 570 §6) under `api/src/presentation/http/v3/commands/`; the views are under `http/v3/views/`.

### 7.1 Who provides, who uses

| Provider | Used by |
|---|---|
| `Authorizer` + `Permission` | every route and view (§4.2), the approval policies (PR 570 §5), #582's custody rules, the staff report (PR 533), the desk (PR 536) |
| `Actor` | every handler |
| `BranchHours` | custody rules (§6), the staff page heading (PR 532 S12), booking slots (#555) |
| `Organization`, `organization_id` | every table (D-1), the approval and idempotency keys (PR 570 `idempotency_records.org_id`) |
| `SessionService`, `IdentityClient` | the sign-in screen (`/login`), the customer app (#558) |
| `IdentityClient::register` (server-side only) | `UserService::create_with_account` and `activate`, the employee create mirror, the `create-owner` CLI (§5.3); no route exposes it |

---

## 8. Tables

```
organizations(id uuid PK, code text UNIQUE, legal_name text, timezone text, created_at)
branches + organization_id uuid NOT NULL FK, code text, address text NULL, timezone text NOT NULL,
           opening_hours jsonb NULL, booking_enabled bool NOT NULL DEFAULT false,
           UNIQUE (organization_id, code), UNIQUE (organization_id, id)   -- the composite FK target
users(id uuid PK, organization_id FK, auth_subject text NULL UNIQUE, name text, status text
      CHECK (status IN ('PENDING','ACTIVE','DISABLED')), version integer, created_at, created_by,
      legacy_employee_id text NULL UNIQUE)          -- the migration window only (§9); dropped by migration 3
roles(code text PK)                              -- the 10 of §3.1, seeded; Blueprint Role(id, code UQ)
user_roles(id uuid PK, user_id FK, role_code FK, organization_id FK, branch_id NULL,
           valid_from timestamptz, valid_to timestamptz NULL,
           FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id))
user_sessions(hash text PK, user_id FK, version integer, expires timestamptz, refresh_token_enc bytea)
user_audit(id, actor_id, subject_id, action, at)   -- or PR 570's audit_events once AP-1 lands
<every other table> + organization_id uuid NOT NULL DEFAULT <install org>, with the composite FK where branch_id exists,
                      except branch_versions and employees (§9 migration 1)
```

---

## 9. Data migration

v3 is not installed anywhere; its data is test data (D6), and `xc_v3` on do-sgp1 may be recreated. v3 migrations are
approved until the first install. The migrations below still run correctly on a database that has rows.

**1. `api/migrations/20261011100000_organizations.sql`** (OR-1)

- **Changes:**
  - creates `organizations` and inserts one row (`code 'XC'`, `timezone 'Asia/Bangkok'`, `legal_name` NULL until
    ADMIN sets it);
  - adds the branch columns with **defaults**, so that a binary that inserts only `(id, name, …)` still works:
    - `code` backfilled from `id`;
    - `timezone text NOT NULL DEFAULT 'Asia/Bangkok'`;
    - `opening_hours jsonb NULL`;
    - `booking_enabled bool NOT NULL DEFAULT false`.

    `code` stays nullable in this migration. OR-1's branch creation writes it (the id, unless given), and a later
    migration sets it `NOT NULL` once no writer leaves it empty (checked by `SELECT count(*) FROM branches WHERE code IS
    NULL` = 0);
  - adds `organization_id` (default: that org) to **every** table, plus the composite FKs, **except on two tables**
    that keep only the FK to `organizations(id)` (found by OR-1's up-front tests, #615, measured):
    - `branch_versions`: the kernel's lock row for any branch id, by contract (`BranchTx::run` inserts it on first use,
      `xc-kernel/src/branch_tx.rs:70`; `api/tests/v3_view_unknown_branch_test.rs:131-142`). It holds no business data;
    - `employees`: the old model OR-7 retires. Its branch check is at the writer (§9 migration 2, "Mirroring"), and
      `user_roles` carries the composite FK. With the FK here, v2-parity creates at an unregistered branch would fail
      with a raw FK error before OR-2's check exists.
- **Existing rows** all belong to the one organisation. `opening_hours NULL` means "not set", so **service will not
  open on any branch until ADMIN sets its hours** (B4). That is the rule, not a fault, and the release note must say it.
- **Count first:**

  ```sql
  SELECT count(*) FROM branches;
  SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY 1;
  ```
- **Deploy order and the old binary.**
  - It ships with the OR-1 binary, which reads `opening_hours` and writes `code`.
  - The pre-OR-1 binary still works against the new schema: every new column has a default or is nullable, and
    `organization_id` defaults to the one organisation.
  - So a rollback of the binary alone is safe. Its only cost is that new branches get no `code` until OR-1 is back.
- **`down`** drops the columns and the table. The hours and codes are lost.

**2. `api/migrations/20261011110000_users_roles.sql`** (OR-2)

- **Changes:** creates `users`, `user_invites`, `roles`, `user_roles`, `user_sessions` and `user_audit`.
- **Backfill:**
  - **each `employees` row** becomes a `users` row with status `PENDING`, `auth_subject NULL`,
    **`legacy_employee_id = e.id`** and no Identity account; an OWNER/ADMIN activates it by entering phone and password (§5.3). It gets exactly one `user_roles`
    row, mapped by §3.3, at its `branch_id`: every v3 staff role maps to exactly one Blueprint role. A disabled
    employee becomes `DISABLED`. Setting `legacy_employee_id` here is what lets migration 3's precheck pass for every
    employee that existed before migration 2;
  - **the `owners` row** becomes a `users` row with EXECUTIVE + ADMIN, organisation-wide (B3).
- **Count first:**

  ```sql
  SELECT role, count(*) FROM employees GROUP BY role;
  SELECT count(*) FROM owners;
  ```

  An employee role outside §3.3's ten v3 roles is refused by the migration (`RAISE`), not guessed.
  **An employee whose `branch_id` names no `branches` row** is refused the same way: `employees.branch_id` has no FK
  today (§1), and `user_roles` has the composite FK of §8, so the backfill would fail on it with a raw FK error. The
  migration counts them first and `RAISE`s with a clear message (they can only be test data, D6; fix or delete the
  row, then run again):

  ```sql
  SELECT count(*) FROM employees e WHERE NOT EXISTS (SELECT 1 FROM branches b WHERE b.id = e.branch_id);   -- must be 0
  ```
- **`owners` / `employees` are not dropped here.** They are dropped with the password login (migration 3).
- **The window between migration 2 and OR-7.**
  - **Who writes accounts:** until OR-5 (the A05 screen) is live, the existing `employees` route
    (`api/src/presentation/http/employees.rs:43`) is the only writer.
  - **Mirroring:** OR-2 mirrors each employee write into `users` + `user_roles` **in the same transaction**, mapped by
    §3.3, at the two repository methods every path goes through
    (`xc-auth/src/infrastructure/account/account_repository.rs`, measured at main `4b8f73b`):
    - `create_employee` (`:254`, its own tx `:260-300`): inserts the `users` row (`PENDING`, `auth_subject NULL`,
      `legacy_employee_id = <new id>`) and its one `user_roles` row before the commit; after the commit it registers
      the employee in `xc-identity` and binds `auth_subject` + `ACTIVE` (§5.3 steps 2–3). No Identity call runs inside
      a database transaction;
    - `disable_employee_in` (`:345`): sets `users.status = 'DISABLED'` where `legacy_employee_id = id`, in the
      caller's tx. Every disable reaches it:
      - the route's `DisableEmployee` (`api/src/presentation/http/employees.rs:107-144` →
        `api/src/application/workflows/disable_employee.rs:74` → `AccountService::disable_employee_in`,
        `service.rs:531-542`);
      - `employees("disable")` → `disable_employee` (`account_repository.rs:338-342`), which wraps it.
    - `create_owner` (`:428`): inserts the owner's `users` row (`PENDING`, `legacy_employee_id NULL`) with EXECUTIVE +
      ADMIN, organisation-wide, in its transaction; after the commit it registers the owner in `xc-identity` and binds
      the account (§5.3 steps 2–3). On a fresh install the migrations run before the owner is
      created, so without this the owner would have no `users` row, and #582 CA-1's `"owner"` lookup would find none
      (found while writing OR-2's tests, #616);
    - `reset_employee_password` changes nothing in `users`: passwords are not part of the new model.
    - **An unknown branch is refused before the insert.** `create_employee` checks that `branch_id` names a
      `branches` row and refuses with the route's sentence (`NO_SUCH_BRANCH`,
      `xc-rate/src/application/branch/service.rs:36`). Today only the route checks it
      (`api/src/presentation/http/employees.rs:149-155`, `BranchService::require`); `AccountService::employees("create")`
      checks only the format. With `user_roles`' composite FK, the check belongs at the writer, where every path meets
      it. `[proposed]` how: a `SELECT 1 FROM branches` in the same transaction, so the FK violation never reaches the
      caller as a database error.

    Mirroring in `AccountService::employees` would miss the route's disable, which does not pass through it.
    `users.legacy_employee_id` (nullable, dropped in migration 3) records the link.
  - **Nothing is frozen and nothing is stale.** Once OR-5 is live, the `employees` route is refused (`UsersManage`
    moves to A05), and from then on only `users` is written.
- **Deploy order:** with the OR-2 binary, which does the mirroring. The pre-OR-2 binary works against the schema,
  because nothing it reads changes. But it would not mirror, so it must not run once migration 2 is applied; v3
  deploys one binary at a time.
- **`down`** drops the new tables.

**3. `api/migrations/20261030100000_drop_password_login.sql`** (OR-7, after OR-3 is live)

- **Version: the cleanup block.** This migration retires the old model and drops a transitional column, so it is in
  the lane's cleanup block `20261030*`, ordered after every feature block (lane controller's rule, #564
  issuecomment-6074383060; its slot is `20261030100000`). This design's own block `20261011*` holds only the
  creating migrations 1 and 2. On a fresh database, every migration that reads `legacy_employee_id` (#582's cash M1
  in `20261012*`) therefore runs before the drop, with plain SQL.

- **Changes:** drops `owners`, `owner_sessions`, `employees`, `employee_sessions`, `auth_attempts` and
  `users.legacy_employee_id`. v3 then holds no password hash. `employee_audit` is **kept** as read-only history, with
  no writer from here on, because `user_audit` takes over.
- **Precheck**, which must be 0, else the migration `RAISE`s:

  ```sql
  SELECT count(*) FROM employees e WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.legacy_employee_id = e.id);
  ```
- **Readers of `legacy_employee_id` that must go first.** #582's CA-1 (PR 590 `cash-shift-transfer.md` §8, M1)
  reads the column twice: its migration M1 fills `shifts.teller_id` from it, and its binary resolves the opener's user
  through it in `xc-shift/src/infrastructure/shift/shift_repository.rs`.
  - **CA-1 merges before OR-7**, so that M1 runs while the column exists.
  - **The id that reaches the lookup changes at OR-3, not at OR-7.** Before OR-3 the actor id is an employee id or
    `"owner"`. From OR-3 on, the extractor (`api/src/presentation/extractors/actor.rs`, OR-3's file) gives
    `access::Actor`, whose id is a `users.id`. If the lookup only knew `legacy_employee_id`, every shift open would fail
    between OR-3 and OR-7 (`shifts.teller_id` is NOT NULL), the owner's included.
  - **So CA-1's lookup accepts both forms from the start** (PR 590 issuecomment of this round asks its text to agree):
    `SELECT id FROM users WHERE id::text = $1 OR legacy_employee_id = $1` (`users.id` is uuid and `$1` may be an employee id or `"owner"`, so the uuid is compared as text), and `"owner"` resolves to the one organisation-wide
    EXECUTIVE + ADMIN user with `legacy_employee_id IS NULL` (migration 2's owner row). It works on either side of
    OR-3, so no ticket has to land on one exact release.
  - **OR-7 removes the legacy half** (the `legacy_employee_id` branch and the `"owner"` case) in the PR that drops the
    column, leaving `id::text = $1` (or `$1` parsed as a uuid first). OR-7 owns that one site (§10 row).
  - Before writing migration 3, OR-7 greps for every other reader:
    `git grep -n legacy_employee_id origin/main -- api ':!api/migrations'` must list only the sites OR-7 changes.
- **Deploy:** after OR-3 has shipped and the owner's first user can sign in through Identity (§5.3). Before that there is no way
  in.
- **`down`** recreates empty tables. **All password hashes are lost for good**, which is the point.

**4. Dropping `user_invites` and renaming `INVITED`** (#1157; B6, §5.3)

- **Changes:** `DROP TABLE user_invites`, and the status `INVITED` is renamed `PENDING` (the owner: *"ไม่มีระบบเชิญสำหรับพนักงาน"*): `UPDATE users SET status = 'PENDING' WHERE status = 'INVITED'` and the `users.status` CHECK becomes `('PENDING','ACTIVE','DISABLED')`. `PENDING` means
  "no Identity account yet" (§5.3).
- **Existing rows, by kind:**
  - a user with status `INVITED`: becomes `PENDING`; nothing else about it changes.
  - an unspent invite (`used_at IS NULL`): it disappears. Its user stays `PENDING` with `auth_subject NULL`, and an
    OWNER/ADMIN activates it by entering phone and password (§5.3). Nothing could claim it anyway once #1157's
    code is gone.
  - a spent invite (`used_at` set): it disappears. Its user is already `ACTIVE` with `auth_subject` set, which
    `users` keeps. Nothing reads the spent row.
- **Backfill:** one, the `UPDATE` above that renames `INVITED` to `PENDING`, in the same migration. Nothing is copied
  out of `user_invites`: the only fact it holds that matters (who is bound to which `sub`) already lives in
  `users.auth_subject`. Rows affected, measured before deploy:
  `SELECT count(*) FILTER (WHERE used_at IS NULL) AS unspent, count(*) AS total FROM user_invites;` and
  `SELECT count(*) FROM users WHERE status = 'INVITED';`
- **Deploy order:** the migration is embedded in #1157's binary and runs at its boot, so the new schema and the code
  that writes `PENDING` arrive together. An older binary on the new schema is **not** safe: it writes `'INVITED'`
  (`account_repository.rs:305`, `:564`; `session_service.rs:763`, `:811`; `user_service.rs:131` on main `d200a3b3`),
  which the new CHECK refuses, so every staff create fails. A rollback runs `down` first, then the old binary.
- **`down`** recreates an empty `user_invites` and renames `PENDING` back to `INVITED`. The invite codes are lost, and nothing uses them.

**Creating accounts (§5.3) needs no migration.** `users.auth_subject` (`UNIQUE`, NULL-able) and `status` exist
since migration 2.

---

## 10. Tickets

Each ticket owns disjoint files.

| # | Ticket | Owns | Starts after |
|---|---|---|---|
| OR-1 | organisation and branch profile; hours | `xc-auth/src/domain/organization/**`, `xc-auth/src/domain/mod.rs`; `xc-rate/src/domain/branch/{entities/branch.rs, branch_hours.rs, mod.rs}`; `xc-rate/src/application/branch/service.rs` (the `Branch` literal, `:113`); `xc-rate/Cargo.toml` + `api/Cargo.lock` (an IANA timezone crate: `chrono-tz`, none in the workspace today); `xc-rate/src/infrastructure/branch/branch_repository.rs`; migration 1 | this design |
| OR-2 | users, scoped roles, permissions, `Authorizer`, the employees mirror | `xc-auth/src/domain/{user,access}/**` (all **new** files, beside the old types), `xc-auth/src/application/user/**`, `xc-auth/src/infrastructure/user/**`, `xc-auth/src/infrastructure/account/account_repository.rs` (the mirror only: `create_employee`, `disable_employee_in`, `create_owner`, and the unknown-branch check); migration 2 | OR-1 |
| OR-3 | Identity sign-in BFF and sessions | `xc-auth/src/domain/identity/**`, `xc-auth/src/infrastructure/identity/**`, `xc-auth/src/application/session/**`, `api/src/presentation/http/auth.rs`, `api/src/presentation/extractors/actor.rs`, `web/src/screens/login/**` | OR-2; I-2 (#554, the token verifier) |
| OR-4 | every route and view onto `Authorizer` (§4.2); the provided tests of §4.3 moved 1:1 (one commit per file, before any behaviour change); `api/tools/policy-map.mjs` + `blueprint_policy_cases.json` | `xc-auth/src/domain/policy/entities/policy.rs` (reduced to the route table), `entry_path.rs`, `api/src/presentation/http/v3/view.rs`, `action.rs`, the 19 call sites, the test files of §4.3 | OR-2, OR-3 (which supplies `access::Actor`). Split per area if the controller prefers; custody rows are #582's |
| OR-5 | A04 branches and A05 users screens and their commands | `web/src/screens/admin/{branches,users}/**`, `http/v3/commands/{branch_save,user_activate,user_grant,user_revoke,user_disable}.rs`, `api/src/presentation/http/users.rs` (`userCreate`: not a `CommandAction`, because §5.3 commits the PENDING row before Identity is called and binds in a new transaction; lane controller ruling on #1157, issuecomment-6102036476), `http/v3/views/{admin_branches_profile,admin_users}.rs` | OR-2, OR-4, PR 570 CM-1 |
| OR-6 | service window per branch (§6) | `xc-shift/.../custody_machine.rs` (the three checks), `custody_time.rs`, `custody_facts.rs`, the new `service_window.rs`, `CustodyPorts` (domain + `api/src/infrastructure/custody/custody_ports.rs`) and their fixture impls; **no kiosk file**: `xc-kiosk/.../work_times.rs` holds a request's start and finish times, not an hours rule (measured on `410524a`, #754) | OR-1. **OR-6 merges before #582's CA-7** (PR 590 §5.1), which then moves these checks into `branch_day.rs` and carries `BranchHours` along |
| OR-7 | drop password login and the old types; re-point every `legacy_employee_id` reader | migration 3; the old account code in `xc-auth`; `xc-auth/src/domain/policy/value_objects/{role.rs, actor.rs, actor_kind.rs}` (deleted); `xc-shift/src/infrastructure/shift/shift_repository.rs` (the `teller_id` lookup only: removes its legacy half, #582 CA-1); **re-point the services that still take the old `Actor` for business behaviour** and remove the `legacy_actor` bridge (OR-4b keeps it as a named stopgap off every authorization path, ruling (a) on #767): xc-rate `rate_draft_service.rs`, `rate_publication.rs`, xc-approval, `payment_settings.rs`, custody, the save steps. Those files belong to other tickets (the rates lane, #582, CI-4), so OR-7 waits for them or is split per service when cut | OR-3 live, OR-4 merged, the owner's first user signs in through Identity (§5.3), **#582 CA-1 merged** |

**Up-front tests per ticket** (🧪), written by the ticket author:

- **OR-1:**
  - `BranchHours::window` across the branch timezone's midnight;
  - `NotSet` for NULL hours; an empty day is `Closed`;
  - a row naming another organisation's branch is refused by the composite FK.
- **OR-2:**
  - `ScopeRule`: EXECUTIVE must be organisation-wide; BRANCH_MANAGER and TELLER take exactly one branch;
    RATE_MANAGER can hold several;
  - a role row outside `[valid_from, valid_to)` grants nothing;
  - `Authorizer`, in its §3.5 order:
    - another organisation → `outOfScope`;
    - a branch outside scope → `outOfScope`, not overridden;
    - MARKETING asking for `IdentityDocumentRead` → refused;
    - CUSTOMER reading another customer's data → `outOfScope`;
  - the §3.3 backfill maps each of the nine v3 roles and the owner, and sets `legacy_employee_id = e.id` on every
    backfilled employee: after migration 2, migration 3's precheck query returns 0;
  - the mirror: a create through the `employees` route leaves one `users` row (`ACTIVE`, bound to the `sub` Identity returned; `legacy_employee_id` = the
    new id) with its one role; a disable through the route (the `DisableEmployee` workflow) and one through
    `employees("disable")` each leave `users.status = 'DISABLED'`; a failed disable (holds cash) changes no `users`
    row;
  - an owner created after migration 2 (`create_owner`) has one `users` row with EXECUTIVE + ADMIN, organisation-wide;
  - a create naming a branch with no `branches` row is refused with `NO_SUCH_BRANCH` and writes no `employees`,
    `users` or `user_roles` row; migration 2 `RAISE`s on a pre-existing employee whose branch has no row.
- **OR-3:**
  - a JWT with a wrong `aud`, a wrong signature or an expired time is refused;
  - an Identity error code maps to its sentence, and an unknown code is 503;
  - a valid `sub` with no user → `signIn`; a `DISABLED` user → refused;
  - a create whose phone Identity already has → refused, and no `users` row is written;
  - a `users.version` change ends the session;
  - **a shift opens for a user signed in the new way:** after sign-in through the BFF, opening a shift writes
    `shifts.teller_id` = that user's `users.id` (the id now reaching #582 CA-1's lookup; §9 migration 3).
- **OR-4:** one row per old assertion (§4.3), old role → mapped role → outcome, with every changed outcome named.
- **OR-5:**
  - OWNER/ADMIN creates, activates, grants and revokes; a non-ADMIN is refused;
  - the password never reaches a log, a response body or a table;
  - repeating a command with the same key applies it once (`idempotency_records`, §09).
- **OR-6:**
  - for a branch whose window is 10:00–20:00 in its timezone, a service start at 19:51 is refused and a save at 19:59
    is allowed;
  - a branch with `NotSet` hours never opens.

---


### Tickets as opened (reconciled 2026-10-11)

> Owner's order 2026-10-11: every opened ticket must appear in its design's ticket list; a split is either listed here (this PR, lane controller claude/5edf7bb5) or folded back into its parent. Measured on `origin/main` `5a2b0d19` against all 605 issues. No new ticket may be opened outside this list.

| Code | Issue | State 2026-10-11 | Listed because |
|---|---|---|---|
| OR-3 web | #762 | closed | the web half of the OR-3 row (`web/src/screens/login/**`), split at cutting; done |
| OR-4a, OR-4b | #766, #767 | closed | the OR-4 row allows "Split per area if the controller prefers"; done |
| OR-5a | #945 | open | the API half of the OR-5 row (branch profile, grant, revoke, disable); no invite (B6, §5.3) |
| OR-5b | #946 | open | the web half of the OR-5 row (A04/A05 screens) |
| OR-5c | #1157 | open | §5.3 (B6): the OWNER/ADMIN creates a staff account and the server registers it in Identity; removes the withdrawn invite path |
| OR-6 follow-up | #755 | closed | lane controller ruling on #754; done |
| OR-7 (parent) | #947 | open | holder only; never `ready` (the OR-7 row: "split per service when cut") |
| OR-7.1 … OR-7.5 | #1020–#1024 | open | one service each, the services the OR-7 row names |
| OR-7.6 | #1025 | closed not planned | folded |
| OR-7.7, OR-7.e, OR-7.m | #1030, #1026, #1027 | open | seams, the extractor bridge, and migration 3 with the deletions: the rest of the OR-7 row |
| SB-3 | #842 | open | §14: kiosk-queue, kiosk-currencies and devices take the session branch; after SB-2 #892 |
| Need::AnyOf | #1149 | open | §3.4–3.5 permissions: a view open to either of two permissions (raised by S-1 #573) |

SB-3 is also recorded here because §14.7 lists only SB-1 and SB-2.

## 11. Blueprint conformance

| Blueprint rule | § | Here |
|---|---|---|
| Organization(code, legal_name, timezone); Branch(organization_id, code UQ/org, address, timezone, opening_hours JSON, booking_enabled) | 07 | §6, §8 |
| User(auth_subject UQ, status); Role(code UQ); UserRole(user, role, organization, branch nullable, valid_from/to); a NULL branch means the organisation, when permitted | 07 | §3.2, §8 |
| Every entity in the organisation has `organization_id`; a composite FK or server constraint against cross-organisation data | 07 | D-1, §8 |
| The server checks organisation, branch scope and owner on every request; hiding a button is not enough | 02 | D-4, §3.5 |
| The role table: scopes, duties and controls | 02 | §3.1, §3.4 |
| VERIFICATION_REVIEWER, organisation or branch scoped; never Marketing or Partner | 15 | §3.1, §3.4 |
| Real auth provider; store `auth_subject`; no raw password or OTP; no demo credential path | 14, 17 | §5 (B2); OR-7 removes v3's hashes |
| Times stored in UTC, shown in Asia/Bangkok; business date by branch timezone | 07, 11 | §6 `BranchHours` |
| Idempotency for mutations | 09 | A04/A05 commands through PR 570's `CommandAction` |

**Where the Blueprint is silent:**

| Silent on | v3's choice | Why |
|---|---|---|
| The service window inside opening hours | the last start at close − 10 min, the last save at close | v3's existing D7 rule (v2 parity), now relative to the branch's own hours (B4) |
| How a person is bound to a user | the OWNER/ADMIN creates the account; the server registers it in Identity | the owner's B6; Identity's existing registration, no new feature |
| The `opening_hours` JSON shape | weekly, per day | A04 sets *"เวลาเปิด"*; closure dates belong to booking slots (#555) |
| The branch a user works on when naming none | the session branch, chosen from the account bar (§14) | the owner, 2026-10-10: *"sysadmin ให้ เลือก สาขา เข้าทำงานได้"* · *"ให้ มี เมนูสลับ สาขา เอาก็ได้"* · *"ตรงloginไม่ต้องใส่แล้ว สาขา"* |
| Error codes `signIn`, `roleNotAllowed` | kept | §10 lists its error names as proposals (*"errors เสนอ"*); v3's existing codes stay beside them |

---

## 12. Deploy steps that belong to the owner

| Step | When | Why |
|---|---|---|
| ~~Register an Identity client with audience `xchange`~~ **done 2026-10-09**: xc-identity on do-sgp1 (PR 593 §4) | — | §5.2 |
| DNS `xc-id.spacing.cloud`, then the certificate (I-1) | before OR-3 is used from outside the host | §5.2 |
| Choose an SMS/e-mail sender for OTP (I-3) | before any sign-in by OTP; password sign-in works without it | §5.2 |
| Enter each branch's opening hours in A04 | after OR-1 and OR-5 | a branch with no hours does not open service (B4) |
| Create the owner's first user (`create-owner`, §5.3) | after OR-3, before OR-7 | OR-7 removes the password login |

## 13. Must ask the owner (not decided here)

| # | Question | Blocks |
|---|---|---|
| O-1 | The organisation's legal name and code | nothing: ADMIN enters them (`legal_name` NULL until then) |
| O-2 | Each branch's opening hours | service on that branch. ADMIN enters them in A04 (B4), and until then the branch does not open |
| O-3 | ~~Registering the `xchange` client in Identity~~: done (§5.2) | — |
| O-4 | Should a staff member have to change the password the OWNER set, at first sign-in (§5.3) | nothing: v3 says no (v2 parity) until the owner says otherwise |

---

## 14. Session branch and the branch switch menu (SB)

> designed-by: claude/83f682 · 2026-10-10 · measured on `origin/main` `8084ccec` · assigned by the lane controller
> claude/608dff (previously claude/b060e9's, after OR-4b #767)

### 14.0 The owner's decision and the controller's rulings

The owner, 2026-10-10, in session claude/b060e9:
- *"sysadmin ให้ เลือก สาขา เข้าทำงานได้"* (sysadmin may choose the branch they work in);
- *"ให้ มี เมนูสลับ สาขา เอาก็ได้"* (a branch switch menu is fine);
- *"ตรงloginไม่ต้องใส่แล้ว สาขา"* (no branch at login any more).

The lane controller (claude/608dff) ruled, 2026-10-10:

| # | Ruling |
|---|---|
| R1 | The switch menu is reachable from every screen. It opens from the account bar, which shows the branch **name**, not its id. |
| R2 | There is **one** session branch, on the session (`__Host-xc_session`), set by **one** server command. Admin, desk and devices all read it. |
| R3 | The server refuses a branch outside the user's grants (D-4), with a test. |
| R4 | There is no branch picker at login. |
| R5 | The initial branch is the only branch; else the user's last choice (stored per user); else the first granted branch by `branches.code`. |
| R6 | The last choice is a new column, so this section has a data migration (§14.5). |

### 14.1 v3 today (measured on `8084ccec`)

- **The session row has no branch.**
  - `user_sessions(hash, user_id, version, expires, refresh_token_enc, organization_id)` (`api/migrations/20261011110000_users_roles.sql:74-81`); no later migration alters it.
  - `access::Actor` has no branch either (`api/crates/xc-auth/src/domain/access/actor.rs:6-11`).
- **The branch is guessed per request.**
  - The extractor's stopgap `legacy_actor` takes the first role by priority and its branch, else `"MAIN"` (`api/src/presentation/extractors/actor.rs:135-188`).
  - `Policy::branch` (`policy.rs:43-53`) lets a staff user's own branch win, then the named branch, then `MAIN`.
  - `Authorizer::check` refuses a user with several branches who names none (`authorizer.rs:95-110`).
- **Each screen keeps its own branch.**
  - **Admin:** a sidebar `<select>` "branch-manager" (`web/src/screens/admin/AdminShell.tsx:422-437`, `handleBranchChange` `:261-271`). The initial value is `me.branch`, else `?branch=`, else `'MAIN'` (`:178-206`). `?branch=` is copied onto links (`:486`, `:559`, `:568`, `:579`).
  - **Devices:** `?branch=` or `'MAIN'` (`web/src/screens/devices/main.tsx:19`), plus a `<select>` over `overview().branches` (`DevicesScreen.tsx:403-415`, `handleBranchChange` `:222-230`).
  - **Desk:** no branch code at all (`web/src/screens/desk/main.tsx:16-23`); the server decides.
- **The account bar shows the branch id:** `t('shell.account.branch', {branch: u().branch})` (`web/src/shared/ui/AccountBar.tsx:48`); `'สาขา {branch}'` (`shared/i18n/th.ts:5`). `Me.branch` is an id (`api/src/presentation/dto/v3/common.rs:59`).
- **Login has no branch field** (`web/src/screens/login/auth-api.ts:23,34`; `api/src/presentation/http/auth.rs:231-241`). R4 needs no change; SB-1 keeps it that way with a test.

### 14.2 The rules

**Switchable set `S(user, now)`.** The user's role rows valid now (`valid_from <= now < valid_to`) give:
- if any row has `branch_id IS NULL` (organisation-wide, §3.2), **every** branch of the user's organisation;
- otherwise, the distinct `branch_id`s of those rows.

`S` is ordered by `branches.code`, then by `id`.

**Initial branch (R5)**, computed at sign-in (`create_session_for_user`):
1. `|S| = 1` → that branch;
2. else, if `users.last_branch_id ∈ S` → that branch;
3. else → the first branch of `S`;
4. `S` empty → `NULL`. That user is refused everything anyway (`roleNotAllowed`).

**Effective branch, on every request.** It is the stored `user_sessions.branch_id` **if it is still in `S`**; otherwise it is recomputed by the initial rule. This covers a grant revoked mid-session. Reading never writes.

**The check (§3.5), amended.** When the target names no branch, `Authorizer::check` uses the actor's effective branch. A named branch is unchanged: honoured when it is in scope, `outOfScope` when not (D-4). The screens stop naming one (§14.4).

**The switch command (R2, R3).** It sets the session's branch for **this session only**. Another browser's session keeps its own. It also records the choice as `users.last_branch_id`, in one transaction. A branch not in `S` is refused `403 outOfScope` and nothing is written. Repeating the same switch writes the same values, so the command is idempotent by value and needs no `idempotency_records` row.

### 14.3 Server shape

| Item | Where | Shape |
|---|---|---|
| `SessionBranch` [required: name, file] | `api/crates/xc-auth/src/domain/access/session_branch.rs` (**new**) | pure: `fn switchable(roles: &[ScopedRole], org_branches: &[BranchRef], now) -> Vec<BranchRef>`; `fn initial(s: &[BranchRef], last: Option<&BranchId>) -> Option<BranchId>`; `fn effective(stored: Option<&BranchId>, s: &[BranchRef], last: Option<&BranchId>) -> Option<BranchId>`. `BranchRef { id: BranchId, code: String, name: String }` in the same file |
| `access::Actor` | `xc-auth/src/domain/access/actor.rs` | gains `branch: Option<BranchId>`, the effective branch [required] |
| `Authorizer::check` | `xc-auth/src/domain/access/authorizer.rs:95-110` | the "names none" arm uses `actor.branch` when it is `Some` and in scope; the old arms stay for `None` |
| `SessionService` | `xc-auth/src/application/session/session_service.rs` | `create_session_for_user` writes `branch_id = SessionBranch::initial(..)`; `active_user_and_roles` also returns the stored branch, `last_branch_id` and the organisation's branches; new `switch_branch(cookie_hash, branch) -> Result<Me-ready data, SessionError::OutOfScope>` |
| The command route [required: path, body, codes] | `api/src/presentation/http/auth.rs` (beside `/api/auth/session`) | `POST /api/auth/session/branch`, body `{"branch": "<branch id>"}`, `X-Xchange-Request: 1` and the same-origin check as the other auth POSTs. `200` + the `Me` of §14.4; `400` malformed; `401 signIn` no session; `403 outOfScope` not in `S` |
| The extractor | `api/src/presentation/extractors/actor.rs` | builds `access::Actor.branch` from `SessionBranch::effective` |
| `Me` [required: JSON fields] | `api/src/presentation/dto/v3/common.rs` + `http/v3/views/me.rs` | `branch` = the effective branch id (unchanged name); new `branchName: string`; new `branches: {id, code, name}[]` = `S`, ordered as §14.2. A user with `|S| = 1` gets a one-element list |

**Why not a v3 `CommandAction`.** A `CommandAction` runs inside one branch's lock and needs an `INHERITS` row (`command.rs:25-60`). Choosing a branch is not business on any branch. It is session state, like sign-out, so it sits with the session routes in `auth.rs`.

### 14.4 Web shape

| Item | Where | Change |
|---|---|---|
| The account bar [required: R1] | `web/src/shared/ui/AccountBar.tsx` (+ `account-bar.css` if one is needed) | It shows `สาขา {branchName}`. When `branches.length > 1`, that text is a `button` (`aria-haspopup="menu"`, `aria-expanded`) opening a `role="menu"` list of `branches`, each a `menuitemradio` with the current one checked. Choosing one posts §14.3. On `200` it reloads the page with any `branch` query parameter removed. On an error it shows `shell.account.switchFailed` in the bar and keeps the old branch. Esc and a click outside close the menu. |
| i18n | `web/src/shared/i18n/{th,en,zh}.ts` | `shell.account.branch` keeps `'สาขา {branch}'` (the value becomes the name); new `shell.account.switchBranch` (`'สลับสาขา'` / `'Switch branch'`), `shell.account.switchFailed` (`'สลับสาขาไม่สำเร็จ'` / `'Could not switch branch'`); `zh` as the file's convention |
| The session store | `web/src/shared/session/session.ts:57-117` | `SessionUser` carries `branchName` and `branches`. Its 60-second poll ends the page with a new `EndReason` `'branchChanged'` when `me.branch` differs from the held one (switched in another tab), and the shell reloads on it |
| Admin | `web/src/screens/admin/AdminShell.tsx` | The sidebar `<select>` and `handleBranchChange` are removed. `currentBranch` is `me.branch`, with no `?branch=` and no `'MAIN'` fallback, and links stop carrying `?branch=`. Screens keep receiving `branch={currentBranch}`, so no menu file changes |
| Devices | `web/src/screens/devices/main.tsx`, `DevicesScreen.tsx` | The branch comes from `me.branch`; the `<select>` at `:403-415` and `handleBranchChange` go |
| Desk | none | It already takes the server's branch |
| Login | none | R4: no field (§14.1) |

**Out of this design:** the per-screen *filters* that list another branch's records inside admin, `Payments.tsx:50-52` and `Slips.tsx:34`. They choose what to **view**, not where the user **works**, and they stay. Kiosk and device-token routes have no user session and are unchanged.

### 14.5 Data migration (R6)

**`api/migrations/20261031100000_session_branch.sql`** (SB-1). It is ordered after the latest file on `8084ccec`, `20261030160000_idempotency_organization.sql`.

```sql
ALTER TABLE users ADD COLUMN last_branch_id text NULL;
ALTER TABLE users ADD CONSTRAINT users_last_branch_fk
  FOREIGN KEY (organization_id, last_branch_id) REFERENCES branches(organization_id, id) ON DELETE SET NULL (last_branch_id);
ALTER TABLE user_sessions ADD COLUMN branch_id text NULL;
ALTER TABLE user_sessions ADD CONSTRAINT user_sessions_branch_fk
  FOREIGN KEY (organization_id, branch_id) REFERENCES branches(organization_id, id) ON DELETE SET NULL (branch_id);
```

1. **Existing rows after the migration.**
   - Every `users` row gets `last_branch_id = NULL`: no choice made yet. Its next sign-in uses R5 steps 1 and 3.
   - Every `user_sessions` row gets `branch_id = NULL`. The read rule (§14.2 "effective") treats `NULL` as "recompute", so an open session keeps working and gets the R5 branch on its next request.
   - No row breaks the new FKs, because both columns are `NULL`.
2. **Backfill: none.** `NULL` is the correct value for both columns ("no choice yet"), and the code computes the branch lazily. Rows measured to confirm: `SELECT count(*) FROM users; SELECT count(*) FROM user_sessions;`. Both are counted only, not changed.
3. **Deploy order.** Migration first, then the new binary.
   - The **old** binary still works on the new schema: it never names the two new nullable columns, and its `INSERT INTO user_sessions` lists its columns (`session_service.rs:241`).
   - The **new** binary needs the columns, so it must not run before the migration (sqlx runs migrations at start, as for every v3 migration).
4. **What `down` loses.** Dropping the two columns loses each user's last choice and each open session's chosen branch. Sessions fall back to the pre-§14 rule (`MAIN` for organisation-wide users). Nothing else depends on them.

`ON DELETE SET NULL (col)` needs PostgreSQL 15 or later. If the install is older, the ticket uses a plain FK; branches are never deleted in v3 (no delete route exists).

### 14.6 Class diagram

```mermaid
classDiagram
  direction LR
  class SessionBranch { <<xc-auth domain/access/session_branch.rs NEW>> +switchable(roles, org_branches, now) Vec~BranchRef~ +initial(s, last) BranchId? +effective(stored, s, last) BranchId? }
  class BranchRef { <<xc-auth domain/access/session_branch.rs NEW>> id; code; name }
  class Actor { <<xc-auth domain/access/actor.rs>> user_id; organization_id; roles; customer_id?; +branch? }
  class Authorizer { <<xc-auth domain/access/authorizer.rs>> +check(actor, permission, target) Grant }
  class SessionService { <<xc-auth application/session/session_service.rs>> +sign_in() +actor_of(cookie) +switch_branch(cookie, branch) }
  class AuthRoutes { <<api presentation/http/auth.rs>> POST /api/auth/session/branch }
  class ActorExtractor { <<api presentation/extractors/actor.rs>> }
  class MeView { <<api http/v3/views/me.rs + dto/v3/common.rs>> branch; branchName; branches }
  class AccountBar { <<web shared/ui/AccountBar.tsx>> switch menu }
  class SessionStore { <<web shared/session/session.ts>> branchChanged }
  SessionBranch --> BranchRef
  SessionService --> SessionBranch
  ActorExtractor --> SessionService
  ActorExtractor --> Actor
  Authorizer --> Actor
  AuthRoutes --> SessionService
  MeView --> SessionService
  AccountBar --> AuthRoutes
  AccountBar --> SessionStore
  SessionStore --> MeView
```

| Provider | Used by |
|---|---|
| `SessionBranch` | `SessionService` (sign-in, read, switch) |
| `Actor.branch` | `Authorizer::check` (the "names none" arm), so every view and command through `Policy::authorize` |
| `POST /api/auth/session/branch` | `AccountBar` |
| `Me.branchName`, `Me.branches` | `AccountBar`, `SessionStore`, `AdminShell`, devices |

### 14.7 Tickets

| # | Ticket | Owns | Starts after |
|---|---|---|---|
| SB-1 | session branch on the server: migration, `SessionBranch`, `Actor.branch`, the check, the switch route, `Me` | the §14.5 migration; `xc-auth/src/domain/access/{session_branch.rs (new), actor.rs, authorizer.rs, mod.rs}`; `xc-auth/src/application/session/session_service.rs`; `api/src/presentation/http/auth.rs`; `api/src/presentation/extractors/actor.rs`; `api/src/presentation/http/v3/views/me.rs`; `api/src/presentation/dto/v3/common.rs` (`Me` only); the exported `common.ts`; its new test files | OR-4b #767 merged (it holds `extractors/actor.rs`, and puts every route on `Policy::authorize`, which this changes) |
| SB-2 | the switch menu and the screens reading the session branch | `web/src/shared/ui/AccountBar.tsx` (+ css), `web/src/shared/session/session.ts`, `web/src/shared/i18n/{th,en,zh}.ts` (three keys), `web/src/screens/admin/AdminShell.tsx`, `web/src/screens/devices/{main.tsx, DevicesScreen.tsx}`; the tests named below | SB-1 merged (it needs the route and `Me`'s fields) |

**Up-front tests** (🧪), written by the ticket author before `ready`:
- **SB-1:**
  - **`SessionBranch` (unit):**
    - one branch → it;
    - several, `last ∈ S` → `last`;
    - `last ∉ S` → the first by code;
    - an organisation-wide row → every branch of the organisation;
    - an expired row gives nothing;
    - `S` empty → `None`;
    - a stored branch revoked → recomputed.
  - **HTTP, real Postgres (`api/tests/*_test.rs`, `Desk::start`):**
    - sign-in stores the R5 branch;
    - `POST /api/auth/session/branch` to an in-scope branch → `200`, the session row and `users.last_branch_id` change, and the next view with no `?branch=` answers for that branch;
    - an out-of-scope branch → `403 outOfScope`, and **nothing written** (R3, D-4);
    - no session → `401`;
    - a second sign-in lands on the last choice;
    - the login body still has no branch field (R4);
    - a named in-scope `?branch=` still wins for that request.
- **SB-2 (vitest, jsdom):**
  - the bar shows the branch **name**;
  - one branch → no menu button;
  - several → the menu lists them by code with the current one checked;
  - choosing one posts `{branch}` and reloads;
  - an error shows `switchFailed` and keeps the old name;
  - the session store ends with `'branchChanged'` when `me.branch` changes;
  - the admin sidebar has no `<select>` and links carry no `?branch=`;
  - devices reads `me.branch`.
  - The existing pins that SB-2 changes are rewritten by the ticket author in the test commit, with each change named: `AccountBar.test.tsx:130,141,229,235` (id → name), `AdminShell.test.tsx:830,937` (the select), and `DevicesScreen.test.tsx:995` (D19).

**Open, for the controller:** none. R1–R6 cover every choice. The one judgement made here is that an explicit in-scope `?branch=` still wins per request (§14.2): D-4 already allows it, and removing it would touch every route again after OR-4b.
