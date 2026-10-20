# XCHANGE user management & RBAC (V.9)

This implements the owner's SRS "Enterprise User Management & RBAC Platform" on top of V.9's existing accounts. It does not create a new system.

| Part | Where |
|---|---|
| Schema | `db/migrations/007-user-management.sql` (PostgreSQL); `src/online/drizzle/0004_user_management.sql` (SQLite/D1) |
| Matrix and guardrails | `src/permissions.mjs` |
| Accounts, passwords, login history | `src/online/lib/employees.ts`, `src/auth.mjs` |
| API | `POST /api/users` in `src/user-management.mjs` |
| Excel | `src/xlsx.mjs` (exceljs) |
| Page | `/users` (`src/devices-public/users.html`, `users.js`, `users.css`) |
| Tests | `test/user-management.mjs`, `src/online/tests/employees.test.mjs` |

## Decisions (owner, 2026-10-17)

- Extend V.9 rather than build a separate service. One sign-in covers Desk, admin, staff and `/users`.
- **V.9's roles stay the permission roles.** The SRS's Department and Position are employee fields, and a module matrix sits on top. The fine-grained cash and shift checks inside the handlers (`permit(...)`) are unchanged.
- **Username = employee code**, case-insensitive. `TK-010326-MN002` signs in as `tk-010326-mn002`.
- **Passwords are 12–128 characters with no character-type rules** (NIST 800-63B). The system also rejects:
  - common passwords;
  - single repeated characters and straight sequences;
  - passwords that contain the person's employee code or username.
- **Excel files are real `.xlsx`.**

## Roles

| V.9 role | SRS role | Notes |
|---|---|---|
| `OWNER` | Executive Director | `owners` table, created on the server with `create-owner.mjs` |
| `MONITOR` | Risk | |
| `RATES` | Rate | Exchange rates and currency stock |
| `FINANCE` | Finance | |
| `MANAGER` | Operations Manager / Branch Supervisor | V.9 has one manager level |
| `DOCUMENTS`, `DOCUMENT_REVIEW` | Document | The prepare/review split is kept |
| `SYSTEM` | IT/Admin | |
| `FRONT` | Front Office | The SRS's "Admin Team" (counter service) |
| `BACKEND` | — | Shift and cash control. Not in the SRS, but required for the cash handover |

Departments: Executive, Risk, Rate, Finance, Operations, IT/DEV.

## Permission matrix

The levels are `NONE < VIEW < CREATE < FULL`. This is the seed in `src/permissions.mjs`, and `test/user-management.mjs` fails if this table and the seed disagree.

| Role | SRS role | DASHBOARD | CUSTOMERS | DOCUMENTS | RATES_STOCK | FINANCE | RISK | REPORTS | USER_MANAGEMENT | SYSTEM_SETTINGS | AUDIT_LOGS |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `OWNER` | Executive Director | Full | Full | Full | Full | Full | Full | Full | Full | Full | View |
| `MONITOR` | Risk | Full | View | View | View | View | Full | Full | — | — | View |
| `RATES` | Rate | Full | Full | View | Full | View | View | Full | — | — | — |
| `FINANCE` | Finance | Full | View | View | View | Full | View | Full | — | — | — |
| `MANAGER` | Operations Manager / Branch Supervisor | Full | Full | Full | — | — | — | View | — | — | — |
| `DOCUMENTS` | Document (prepare) | View | View | Full | — | — | — | — | — | — | — |
| `DOCUMENT_REVIEW` | Document (review) | View | View | Full | — | — | — | — | — | — | — |
| `SYSTEM` | IT/Admin | View | — | — | — | — | — | View | Full | Full | View |
| `FRONT` | Front Office | View | Create | View | — | — | — | — | — | — | — |
| `BACKEND` | (none) shift & cash control | View | View | View | — | — | — | — | — | — | — |

**Guardrails** (`checkLevel`) apply to every UI save and every import, whatever the matrix says:

1. Audit Logs are at most **View** for everyone. Nobody can edit or delete the log.
2. User Management and System Settings go above View only for OWNER and SYSTEM.
3. SYSTEM goes above View only on User Management and System Settings.
4. OWNER's row is fixed.

Only OWNER can change the matrix, from the `/users` matrix tab or by importing `Role Permission Matrix.xlsx`. Each change is written to `employee_audit` as `PERMISSION_CHANGE` with its before and after values.

## Deviations from the SRS

| SRS | Here | Why |
|---|---|---|
| IT/Admin has Full on every module | Full only on User Management and System Settings; View on Dashboard, Reports and Audit Logs; none elsewhere | Segregation of duties: IT runs accounts, not money or customer data |
| Audit Logs "Full" for Executive and IT | View only | A log that can be edited is not an audit trail |
| Portfolio Management; Rate analyses portfolios and repayment | Rates & Stock | XCHANGE is currency exchange, not lending |
| One Document role | Prepare (`DOCUMENTS`) and review (`DOCUMENT_REVIEW`) | Keeps maker–checker |
| No shift/cash role | `BACKEND` | Needed for the cash handover |
| Password ≥ 8 characters with upper, lower, digit and symbol | 12–128 characters plus a blocklist | Stronger and easier to remember (NIST 800-63B) |
| Import Users, Roles and Permissions | Import Employee Register (creates and updates users) and the permission matrix (OWNER only, guardrails apply). Roles are fixed in code | Roles carry code-level controls; a sheet cannot invent one |

## Accounts

| Status | Meaning | Who can set it |
|---|---|---|
| `ACTIVE` | Can sign in | — |
| `SUSPENDED` | Temporary hold; can be re-enabled | OWNER, SYSTEM |
| `DISABLED` | Left the company | Set by OWNER or SYSTEM; re-enabled only by OWNER |

- **Ending sessions.** Any status change, role or branch change, or password reset signs the person out everywhere (`version` bump).
- **Cash guard.** Suspend and disable are refused while the person holds the cash or has a pending handover. This is the same check `/api/employees` already used.
- **Temporary passwords.** Every new account and every reset gets a generated password:
  - 16 characters, with no look-alike characters;
  - shown to the admin once;
  - valid for 72 hours;
  - must be changed at first sign-in.

  The server keeps only the scrypt hash.
- **Changing your own password** needs the current password, except during the forced first change. The new password can't reuse the old one.
- **SYSTEM limits.** SYSTEM can't change its own account, can't touch other SYSTEM accounts, and can't give anyone the SYSTEM role. Only OWNER can.
- **Audit and login history.**
  - Every change is written to `employee_audit` with before/after detail and IP.
  - Every sign-in attempt goes to `login_history`, for employees and owners, successful or not, with a reason: `OK`, `BAD_PASSWORD`, `UNKNOWN_USER`, `NOT_ACTIVE`, `TEMP_EXPIRED` or `RATE_LIMITED`.

## Excel

| File | Contents |
|---|---|
| `Employee Register.xlsx` | Employee code, full name, position, department, Tell Code, role, branch, status. This is also the import template |
| `User List.xlsx` | Username, code, name, role, branch, status, must-change flag, last sign-in, created and updated dates |
| `Role Permission Matrix.xlsx` | Role × module levels. Also the matrix import template |

**Importing the Employee Register:**
1. Upload the file to get a preview: rows to create, update or leave unchanged, plus any errors.
2. Apply. Rows apply in order, and a failing row stops the run and reports what was already applied. The preview validates everything first, so a stop mid-run is unexpected.
3. New users get temporary passwords, returned once as `Temporary passwords YYYY-MM-DD.xlsx`.

Columns are matched by Thai or English header (for example `รหัสพนักงาน` / `Employee Code`). Unknown columns are refused. **`Password` and `เลขบัตรประชาชน` (national ID) columns are never read**: they're dropped with a warning. National IDs don't belong in the login system; keep them in the HR record.

## Onboarding runbook

1. Apply `007-user-management.sql` and deploy the matching build.
2. The owner opens `/users` → Export **Employee Register.xlsx** to get the template.
3. Fill in one row per person: employee code, name, position, department, role and branch. Leave passwords and national IDs out.
4. Import → check the preview → Apply.
5. Hand each person their line from `Temporary passwords …xlsx` privately, then delete the file. People sign in with their employee code and set their own password.
6. Before turning on the cash handover for a branch, every required role must have an account: Front, Backend, Manager, Rates, Monitor and Finance (existing check in `/api/employees`).

## Route → module map (for the next enforcement step)

This PR enforces the matrix on `/api/users` and returns `permissions` from `/api/auth status`. Existing routes keep their current owner/user checks plus inline `permit(...)`. To move them onto the matrix later:

| Route | Module |
|---|---|
| `/api/rpc` (ledger reads and saves), `/staff/desk`, `/api/kiosk`, `/kiosk-queue.html` | CUSTOMERS, with FINANCE for shifts and money |
| `/api/upload`, `/files/:key`, `/staff/documents`, `/api/drawer-passport` | DOCUMENTS |
| `/api/reference-rates`, `/kiosk-currencies.html`, `/rate-board` (public) | RATES_STOCK |
| `/api/payment-methods`, `/api/referrals` payouts | FINANCE |
| `/executive` APIs, `/staff/report` | REPORTS |
| `/api/employees`, `/partner-accounts`, `/api/partner-accounts`, `/users` | USER_MANAGEMENT |
| `/devices`, `/api/devices` | SYSTEM_SETTINGS |
| `/api/users` `audit` and `logins` actions | AUDIT_LOGS |

`/api/employees` stays owner-only, because it also carries the branch cash-handover setup (`entity`, `enable`). SYSTEM manages accounts through `/api/users`.
