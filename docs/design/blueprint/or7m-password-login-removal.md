# OR-7.m: v3's password login and the old identity types go

Child design of `org-roles-branches.md` §10 row OR-7 and §9 migration 3 (ticket #1027, parent #947), after
`or7-standing.md`. Branch: `feat/or7m`. Sub-controller: claude/ba2128ed ("AG controller OR-7.m"), under the BE
controller claude/a90ab16c.

## 1. What is left

Measured on `main` `5cdac176` (2026-10-19):

- **Old types still imported outside tests:** `access/v3_roles.rs`, `policy/entities/policy.rs`,
  `presentation/extractors/actor.rs`.
- **Password-cookie path:** `pages.rs` (`Sessions`, `SessionReader::sessions`, `old_cookie_callers`), `extractors/actor.rs`
  (`password_session`, `staff_password_session`, `actor_of`, `staff_actor`, `AccountSessionReader::sessions`) and its
  callers `http/{auth,employees,kiosk,rpc}.rs`.
- **Readers of the dropped tables:** `custody_ports.rs` (`employees.role`), `kiosk_ports.rs` and `http/kiosk.rs`
  (`employees.username`, `owners.username`), `compat/rpc.rs` and `compat/snapshot_sources.rs` (`employees`),
  `xc-shift` `shift_repository.rs` (`resolve_teller` forms 2–4), `xc-cash` `handover_service.rs` (`legacy_employee_id`),
  `User.legacy_employee_id` (`xc-auth` user, session and user services), the purge timer in `bootstrap/mod.rs`.
- **Missed by earlier lists:** the device agent's rate limits write `auth_attempts` through `PgAccountRepository`
  (`infrastructure/device_agents/agent_service.rs:117,134`); `userCreate` uses `domain/account/temp_password.rs`; the
  desk script `api/static/xc-desk.js:262` and `deploy/server/smoke.sh:96` call `POST /api/auth`.
- **Tests:** 262 test files use the old types, the dropped tables or a `SessionReader::sessions` override.

## 2. The seams (skeleton commit on `feat/or7m`)

```mermaid
classDiagram
  class PgAttemptCounter {
    -pool: PgPool
    +new(pool) PgAttemptCounter
    +increment(key: &str, window_start) AppResult~i32~
    +purge_before(current_window) AppResult~u64~
  }
  class directory {
    <<module>>
    +user_names(executor: PgExecutor, ids: &[String]) AppResult~HashMap~String,String~~
  }
  class TempPassword {
    +generate() TempPassword
  }
  class test_callers {
    <<module, doc(hidden)>>
    +owner(id) access::Actor
    +staff(id, roles: &[access::Role], branch) access::Actor
    +callers(owner, staff) Callers
    +nobody() Callers
  }
  class SessionReader {
    <<trait>>
    +sessions(req) Sessions  %% default: no session; deleted by T-last
    +callers(req) Callers
  }
  AgentService ..> PgAttemptCounter : rate limits (T2)
  bootstrap ..> PgAttemptCounter : purge timer (T7)
  kiosk ..> directory : names by users.id (T5)
  kiosk_ports ..> directory : same rule, inside its transaction (T5)
  test_callers ..> Callers : builds
```

| Type / function | File | Provided by | Used by |
|---|---|---|---|
| `PgAttemptCounter` | `api/crates/xc-auth/src/infrastructure/attempts.rs` | skeleton (`todo!()`), bodies T2 | T2 (agent), T7 (purge) |
| `directory::user_names` | `api/crates/xc-auth/src/application/user/directory.rs` | skeleton (`todo!()`), body T1 | T5 |
| `TempPassword` | `api/crates/xc-auth/src/domain/user/temp_password.rs` (re-exported from `domain::account`) | skeleton (move) | `userCreate` |
| `test_callers::{owner, staff, callers, nobody}` | `api/src/presentation/test_callers.rs` | skeleton (bodies) | the test sweep |
| `SessionReader::sessions` default | `api/src/presentation/pages.rs` | skeleton | the test sweep, then deleted |

**Provided tests:** `api/crates/xc-auth/tests/or7m_seams_test.rs` (T1, T2) fail at `todo!()` on the skeleton.
`test_callers.rs` `mod tests` pins `callers(..)` to what `old_cookie_callers` built for every v2 role.

## 3. Rulings (sub-controller, 2026-10-19)

1. **`auth_attempts` stays.** It is the device agent's rate-limit table, written through `PgAttemptCounter`. Migration 3
   drops the other five tables and the column, which is narrower than design §9 (it drops less).
2. **A name comes from `users.name` by `users.id`.** Where nobody has the id (an old employee id, `"owner"`), the id itself
   is shown, as `username_or_id` does today. v3 holds test data only, so old rows printing raw ids is accepted.
3. **`create_owner`'s `u.legacy_employee_id IS NULL` filter goes.** No v2 employee maps to EXECUTIVE (`v3_roles.rs:13`: only
   OWNER does), so no migrated employee can count as a second owner.
4. **Logout stops clearing `__Host-xc_owner` / `__Host-xc_staff`** when the password routes go; they lapse within
   `SESSION_SECONDS` (12 h).
5. **The purge timer purges attempt windows only.** It never purged `user_sessions`; that gap is not this ticket's.
6. **The desk script reads `POST /api/auth/session`** and shows `user.name`; `smoke.sh` checks that route's 200.
7. **`resolve_teller` keeps form 1 only** (#1027 §5, unchanged): `"owner"` and usernames stop resolving (`TellerUnknown`).

## 4. Tickets

All are cut against `feat/or7m`, PRs into `feat/or7m`. File scopes are disjoint within a wave.

| Id | Area | Model | After |
|---|---|---|---|
| T1 | `directory::user_names` body | Flash | skeleton |
| T2 | `PgAttemptCounter` bodies; `AgentService` and `device_agent.rs` use it | Flash | skeleton |
| T3 | `User.legacy_employee_id` out of `xc-auth` and its callers | Flash | skeleton |
| T4 | custody: `employee_role` port and `custody_role.rs` go | Flash | skeleton |
| T5 | kiosk: names by `users.id`, no password `who` | Argon | skeleton (T1 for its tests to pass) |
| T6 | the old employees screen, its routes, view, DTO, workflows and tests go | Flash | skeleton |
| T7 | purge timer on `PgAttemptCounter`; `create-owner --username` goes | Flash | skeleton |
| T8 | desk script and `smoke.sh` on `/api/auth/session` | Flash | skeleton |
| T9 | `resolve_teller` forms 2–4; `holds_cash_in` | Flash | `feat/lg6` on `main` |
| T10 | compat RPC and snapshot people from `users` + `user_roles` | Argon | `feat/lg6` on `main`; its own seam commit |
| S1–Sn | test sweep: `Sessions {..}` → `test_callers`, dropped-table seeds → `users` | Flash | T1–T10; #3262–#3267 merged |
| T-last | password login, the old types, `policy.rs` old half, `v3_roles.rs`, `domain/account` go | Argon | all above |
| M3 | migration 3 | Argon, guarded | T-last; owner confirms OR-3 is live |

## 5. Migration 3 (data migration)

1. **Existing rows:** `owners`, `owner_sessions`, `employees`, `employee_sessions` and their rows go; `users` loses
   `legacy_employee_id`. `employee_audit` and `auth_attempts` stay unchanged.
2. **Backfill:** none. Precheck, inside the migration: it `RAISE`s unless
   `SELECT count(*) FROM employees e WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.legacy_employee_id = e.id)` is 0.
3. **Deploy order:** this binary with the migration. The old binary must not run after it (it reads the dropped tables).
4. **What `down` loses:** every password hash and old session, and the employee-to-user link, for good.

**Gate:** the owner's standing v3 migration approval covers writing and merging it into `feat/or7m`. The `feat/or7m` → `main`
PR waits until the owner confirms OR-3 is live on the test server (the owner signs in through Identity). No deploy.
