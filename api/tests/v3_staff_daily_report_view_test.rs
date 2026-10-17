//! Ticket #1350 (V3-staff-report): the `staff_daily_report` view behind `/staff/report`, design
//! `docs/design/xchange-v3-wave3-staff-report.md` §B (B.1 rows 1–19, §B.6), as revised by PR 1358 and with the
//! owner's answers to Q-SR1…Q-SR4 (2026-10-12, "as recommended", recorded by doc PR 1368).
//!
//! Written before the implementation by the test-writer claude/14cc24bd, on the FE lane controller
//! claude/5edf7bb5's order. Stacked on PR 1332's head `606b3d2c` (#1030: the seam hands views `access::Actor`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **How the data is written.** Through the merged services, never custody requests (B.6):
//! - exchanges: `QuoteService::quote_in`, the TX-2 store, `ConfirmTransaction`, `CompleteExchange` (through
//!   `tx4_common`/`tx5_common`; the posting port is `tx5_common::Ledger`, whose postings the report never reads);
//! - counts: `CashCountService::{submit, verify}`; hand-overs: `HandoverService::start`; transfers:
//!   `TransferFlow::{create, submit, decide, dispatch, receive}`;
//! - holds and the entity (company, site): the kept custody service (`pg_custody_service`, `opsHold`,
//!   `opsSetEntity`), until CA-8b #1274 (ruling N3 on #1350);
//! - a POSTED reversal: `RequestReversal::run_in` → AP-1's decide (FINANCE) → `ReversalRepository::post_in` with a
//!   receipt row this file writes, because RV-3 #684 is not merged (ruling N2 (a) on #1350);
//! - **the one exception:** a `reconciliation_certifications` row is INSERTed by SQL, because its writer FN-1
//!   #1068 is not merged (ruling N1 (a) on #1350). The report only reads it.
//!
//! **Choices of this file** (the design leaves the spelling open; the controller may object on #1350):
//! - the wire is camelCase; `status` is `"reconciled" | "closedAwaitingReconciliation" | "preliminary"`, a signer's
//!   `capacity` is `"counted" | "verified"`;
//! - money and quantities are decimal strings; a figure that is a sum is compared **as a number** (its value, not
//!   its spelling), a figure that is one stored value (a single count's totals) is compared as that value too;
//! - `currencies` holds v2's seven codes in v2's order (`THB USD EUR GBP JPY CNY AUD`, B.1 row 13), every
//!   figure `null` where no count gives it;
//! - the refusal of a session holding none of the three permissions is pinned to what `606b3d2c` answers:
//!   `400 roleNotAllowed` (B.1 row 1); another branch is `403 outOfScope` (controller's addition to B.6).
//!
//! Needs `TEST_DATABASE_URL`; nothing skips. Each test works in a schema of its own.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor as User, Permission, Role, RolePermissions, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_cash::application::count::count_service::{CashCountService, SubmitCount};
use xc_cash::application::handover::handover_service::{HandoverService, StartHandover};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::count::cash_count::CountKind;
use xc_cash::domain::count::count_line::CountLine;
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::repository::ReversalRepository;
use xc_exchange::domain::reversal::reversal::PostedReversalRow;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_exchange::infrastructure::reversal::repository::PgReversalRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::custody::custody_service::pg_custody_service;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const VIEW: &str = "staff_daily_report";
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const MAIN: &str = "MAIN";

/// LG-4 (#929): every register currency, 1,000,000,000 each, into SAFE(`branch`) through the ledger (`OPENING`
/// from BANK, `tx5_common::lg4_fund`), for each branch. Cash moves read the ledger since the cutover.
async fn fund_safes(pool: &sqlx::PgPool, branches: &[&str]) {
    let codes: Vec<String> = sqlx::query_scalar("SELECT trim(code) FROM currencies ORDER BY 1")
        .fetch_all(pool)
        .await
        .expect("the currency register");
    for branch in branches {
        for code in &codes {
            lg4_fund(
                pool,
                xc_cash::domain::posting::CashPlace::Bank,
                xc_cash::domain::posting::CashPlace::Safe(BranchId::new(*branch)),
                code,
                "1000000000",
            )
            .await;
        }
    }
}
const COMPANY: &str = "Exchange Co., Ltd.";
const SITE: &str = "Main Road";
/// v2's seven codes, in v2's order (B.1 row 13).
const CODES: [&str; 7] = ["THB", "USD", "EUR", "GBP", "JPY", "CNY", "AUD"];
/// The three permissions of the report's policy row (D-B2).
const READERS: [Permission; 3] = [
    Permission::ShiftCloseCertify,
    Permission::FinanceReconcile,
    Permission::OverviewRead,
];
/// The Blueprint staff roles, used only to *make* actors.
const STAFF_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
];

static PII: Once = Once::new();
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

// ------------------------------------------------------------ sessions

/// A signed-in person: their `users.id`, name and the spec that signs them in.
#[derive(Clone)]
struct Who {
    id: String,
    name: String,
    spec: String,
}

/// `<user id>|<ROLE>|<branch or *>`, or `OWNER` for the owner's cookie alone.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG_1,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

fn sessions_of(spec: &str) -> Sessions {
    if spec == "OWNER" {
        return Sessions {
            owner: Some(Actor::owner("owner")),
            staff: None,
        };
    }
    let mut parts = spec.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(id), Some(_), Some(branch)) if branch != "*" => Sessions {
            owner: None,
            staff: Actor::staff(id, OldRole::Manager, BranchId::new(branch)),
        },
        (Some(id), Some(_), Some(_)) => Sessions {
            owner: None,
            staff: Actor::staff(id, OldRole::Manager, BranchId::new(MAIN)),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { sessions_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let s = sessions_of(&spec);
            let user = user_of(&spec);
            Callers {
                owner: s.owner,
                staff: s.staff,
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ the branch

/// `tx4_common::Desk` (MAIN publishing USD at 32.10 / 32.45, the customer `CUSTOMER`) with MAIN's entity set,
/// custody on, a safe, and MAIN's v2-opened shift (which the exchanges post to, cash-shift §4.3; the status
/// rule never counts it, B.1 row 8).
struct Day {
    c: Counter,
    counts: CashCountService,
    transfers: TransferFlow,
    handovers: HandoverService,
}

impl Day {
    async fn start() -> Self {
        Self::start_in(None).await
    }

    /// `timezone`: MAIN's `branches.timezone`, when not the default `Asia/Bangkok`.
    async fn start_in(timezone: Option<&str>) -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            let _ = xc_customer::install_pii_keys(keys);
        });
        let desk = Desk::start().await;
        let pool = desk.db.pool.clone();
        if let Some(tz) = timezone {
            desk.db
                .exec(&format!(
                    "UPDATE branches SET timezone = '{tz}' WHERE id = '{MAIN}'"
                ))
                .await;
        }
        desk.db
            .exec("CREATE TABLE tx5_posted (source_id text NOT NULL)")
            .await;
        desk.db
            .exec("CREATE TABLE tx5_effects (name text NOT NULL, transaction_id uuid NOT NULL)")
            .await;
        sqlx::query(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             SELECT id, 'test', 'PENDING', 'test', 'test' FROM organizations WHERE code = 'XC' \
             ON CONFLICT (legacy_employee_id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed the opener");
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(pool.clone())),
            branch_tx.clone(),
        );
        // the entity is set while custody is off and no shift is open (custody_machine.rs:112-121)
        pg_custody_service(&pool, &shifts, &branch_tx)
            .command(
                &BranchId::new(MAIN),
                "opsSetEntity",
                &json!({ "company": COMPANY, "site": SITE, "confirmSeparated": true }),
                &CustodyActor {
                    id: "owner".to_owned(),
                    employee_id: None,
                    role: CustodyRole::Owner,
                },
                Utc::now(),
            )
            .await
            .unwrap_or_else(|e| panic!("set MAIN's entity: {e}"));
        desk.db
            .exec(&format!(
                "UPDATE ops_state SET enabled = true, phase = 'OPEN', \
                 safe = jsonb_build_object('USD', 500000::bigint, 'THB', 10000000::bigint) \
                 WHERE branch_id = '{MAIN}'"
            ))
            .await;
        // LG-4 (#929; #1350 issuecomment-6120642136): since the cutover an OPEN count's cash leaves the ledger's
        // SAFE and a transfer's dispatch leaves the sender's place, and the backfill turns `ops_state.safe` into an
        // OPENING posting only at migration time, before this seed. So the safes are funded through the ledger, as
        // main's own LG-4 harness does (`xc-cash` `count_service` tests): 1,000,000,000 of every register currency
        // in SAFE(MAIN) and SAFE(BR2), one OPENING posting from BANK each. No figure here reads the ledger.
        fund_safes(&pool, &[MAIN, BR2]).await;
        // a count difference up to 100,000 THB is cleared without FINANCE (`cash_limits`, a setting row written
        // by SQL as `tx4_common::Desk::limit` writes its own)
        desk.db
            .exec(&format!(
                "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, \
                 updated_at) VALUES ('{ORG_1}', '{MAIN}', 100000, 'test', now())"
            ))
            .await;
        desk.db
            .exec(&format!(
                "INSERT INTO ops_state (branch_id, enabled, phase, safe) \
                 VALUES ('{BR2}', true, 'OPEN', jsonb_build_object('USD', 500000::bigint)) \
                 ON CONFLICT (branch_id) DO NOTHING"
            ))
            .await;
        let shift = shifts
            .open(
                &BranchId::new(MAIN),
                OpenShift {
                    request_id: unique("open-shift").replace('_', "-"),
                    opening: Balances::of(&[
                        (CurrencyCode::THB, money("100000.00")),
                        (CurrencyCode::parse("USD").unwrap(), money("1000.00")),
                    ]),
                },
                "test",
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("open MAIN's v2 shift: {e}"))
            .id;
        let scratch = std::env::temp_dir().join(unique("xc-1350"));
        std::fs::create_dir_all(&scratch).unwrap();
        let c = Counter {
            files: xchange::infrastructure::files::file_service::FileService::new(
                pool.clone(),
                scratch.join("uploads"),
            ),
            desk,
            shift,
            scratch,
        };
        Self {
            counts: CashCountService::pg(pool.clone()),
            transfers: TransferFlow::pg(pool.clone()),
            handovers: HandoverService::pg(pool),
            c,
        }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn db(&self) -> &TestDb {
        &self.c.desk.db
    }

    /// A user with `role` on `branch` (`None` = organisation-wide).
    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let name = unique(&format!("{} person", role.as_str()));
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG_1)
        .bind(&name)
        .fetch_one(&self.db().pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG_1)
        .bind(branch)
        .execute(&self.db().pool)
        .await
        .expect("give the user a role");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
            name,
        }
    }

    /// A reader of MAIN: a branch manager (holds `ShiftCloseCertify`).
    async fn reader(&self) -> Who {
        self.person(Role::BranchManager, Some(MAIN)).await
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.db().pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    // -------------------------------------------------------- exchanges

    /// A LIVE exchange of `quantity` USD on `side`, completed at `at` (created and confirmed 2 s before).
    async fn completed(
        &self,
        side: Side,
        quantity: &str,
        fee: Option<&str>,
        at: DateTime<Utc>,
    ) -> TransactionId {
        let start = at - Duration::seconds(2);
        let q = self.c.desk.quote(MAIN, side, quantity, fee, start).await;
        let t = self.c.live_of(&q, PaymentMethod::Cash, true, start).await;
        self.c
            .complete(&Ledger::new("0.00", None), t.id, at)
            .await
            .unwrap_or_else(|e| panic!("complete: {e}"));
        t.id
    }

    /// A DEMO exchange of 100 USD bought, completed at `at`.
    async fn completed_demo(&self, at: DateTime<Utc>) -> TransactionId {
        let start = at - Duration::seconds(2);
        let q = self.c.desk.quote(MAIN, Side::Buy, "100", None, start).await;
        let t = self.c.desk.draft(&q, start).await;
        assert_eq!(
            self.c
                .desk
                .confirm(t.id, later(start, 1))
                .await
                .expect("confirm"),
            ConfirmOutcome::Confirmed
        );
        self.c
            .complete(&Ledger::new("0.00", None), t.id, at)
            .await
            .unwrap_or_else(|e| panic!("complete a DEMO exchange: {e}"));
        t.id
    }

    async fn status_of(&self, id: TransactionId) -> String {
        self.c.status(id).await
    }

    // -------------------------------------------------------- counts

    fn actor(who: &Who) -> User {
        user_of(&who.spec).expect("an access actor")
    }

    /// A count of `kind` by `teller`, submitted at `at` with `lines` (currency, face value, qty).
    async fn submit(
        &self,
        teller: &Who,
        kind: CountKind,
        shift: Option<&str>,
        lines: &[(&str, &str, i32)],
        note: Option<&str>,
        at: DateTime<Utc>,
    ) -> String {
        let mut count_lines = Vec::new();
        for (currency, face, qty) in lines {
            count_lines.push(CountLine::new(
                currency,
                self.denomination(currency, face).await,
                *qty,
            ));
        }
        self.counts
            .submit(
                &Self::actor(teller),
                SubmitCount {
                    branch: BranchId::new(MAIN),
                    shift_id: shift.map(str::to_owned),
                    kind,
                    lines: count_lines,
                    note: note.map(str::to_owned),
                    idempotency_key: unique("count"),
                },
                at,
            )
            .await
            .unwrap_or_else(|e| panic!("submit a {kind:?} count: {e}"))
            .id
    }

    async fn verify(&self, manager: &Who, count: &str, at: DateTime<Utc>) {
        self.counts
            .verify(&Self::actor(manager), count, at)
            .await
            .unwrap_or_else(|e| panic!("verify count {count}: {e}"));
    }

    /// A counted shift (opened by a verified OPEN count, cash-shift §4.2) of a new teller, opened at `at`.
    async fn counted_shift(
        &self,
        manager: &Who,
        lines: &[(&str, &str, i32)],
        at: DateTime<Utc>,
    ) -> (Who, String) {
        let teller = self.person(Role::Teller, Some(MAIN)).await;
        let count = self
            .submit(&teller, CountKind::Open, None, lines, None, at)
            .await;
        self.verify(manager, &count, at + Duration::seconds(1))
            .await;
        let shift = self
            .db()
            .text(&format!(
                "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
                teller.id
            ))
            .await
            .expect("the verified OPEN count opened the teller's shift");
        (teller, shift)
    }

    /// The shift's CLOSE count, submitted at `at` and verified 1 s later when `verified`. Returns its id.
    #[allow(clippy::too_many_arguments)]
    async fn close(
        &self,
        teller: &Who,
        manager: &Who,
        shift: &str,
        lines: &[(&str, &str, i32)],
        note: Option<&str>,
        at: DateTime<Utc>,
        verified: bool,
    ) -> String {
        let count = self
            .submit(teller, CountKind::Close, Some(shift), lines, note, at)
            .await;
        if verified {
            self.verify(manager, &count, at + Duration::seconds(1))
                .await;
        }
        count
    }

    /// Σ of a `cash_count_totals` column over `counts`, for `currency`, as a number.
    async fn summed(&self, column: &str, counts: &[&str], currency: &str) -> Option<Decimal> {
        let ids = counts
            .iter()
            .map(|c| format!("'{c}'::uuid"))
            .collect::<Vec<_>>()
            .join(", ");
        self.db()
            .text(&format!(
                "SELECT SUM({column})::text FROM cash_count_totals \
                 WHERE count_id IN ({ids}) AND currency = '{currency}'"
            ))
            .await
            .map(|t| Decimal::from_str(&t).expect("a decimal"))
    }

    // -------------------------------------------------------- transfers and hand-overs

    async fn policy(&self, executive: &Who) {
        self.db()
            .exec(&format!(
                "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
                 VALUES ('{}', '{ORG_1}'::uuid, jsonb_build_object('USD', '100000.00', 'THB', '1000000.00'), \
                 '{}'::uuid)",
                unique("v"),
                executive.id
            ))
            .await;
    }

    /// A transfer created at `at` and walked to `to` (`DRAFT`, `APPROVED`, `DISPATCHED`, `RECEIVED`).
    #[allow(clippy::too_many_arguments)]
    async fn transfer(
        &self,
        creator: &Who,
        approver: &Who,
        receiver: Option<&Who>,
        kind: TransferKind,
        from: &str,
        to: Option<&str>,
        lines: &[(&str, &str, i32)],
        note: Option<&str>,
        at: DateTime<Utc>,
        stage: &str,
    ) -> String {
        let mut new_lines = Vec::new();
        for (currency, face, qty) in lines {
            new_lines.push(NewTransferLine {
                currency: CurrencyCode::parse(currency).unwrap(),
                denomination_id: self.denomination(currency, face).await,
                qty: *qty,
            });
        }
        let id = self
            .transfers
            .create(
                &Self::actor(creator),
                NewTransfer {
                    kind,
                    from_branch: BranchId::new(from),
                    to_branch: to.map(BranchId::new),
                    from_shift: None,
                    to_shift: None,
                    lines: new_lines.clone(),
                    note: note.map(str::to_owned),
                    idempotency_key: unique("transfer"),
                },
                at,
            )
            .await
            .unwrap_or_else(|e| panic!("create a {kind:?} transfer: {e}"))
            .id;
        if stage == "DRAFT" {
            return id;
        }
        self.transfers
            .submit(&Self::actor(creator), &id, at)
            .await
            .unwrap_or_else(|e| panic!("submit transfer {id}: {e}"));
        self.transfers
            .decide(&Self::actor(approver), &id, Outcome::Approved, "ok", at)
            .await
            .unwrap_or_else(|e| panic!("approve transfer {id}: {e}"));
        if stage == "APPROVED" {
            return id;
        }
        self.transfers
            .dispatch(&Self::actor(creator), &id, at)
            .await
            .unwrap_or_else(|e| panic!("dispatch transfer {id}: {e}"));
        if stage == "DISPATCHED" {
            return id;
        }
        let receiver = receiver.expect("a receiver for RECEIVED");
        self.transfers
            .receive(&Self::actor(receiver), &id, new_lines, at)
            .await
            .unwrap_or_else(|e| panic!("receive transfer {id}: {e}"));
        id
    }

    async fn transfer_status(&self, id: &str) -> String {
        self.db()
            .text(&format!(
                "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
            ))
            .await
            .expect("a transfer")
    }

    // -------------------------------------------------------- holds, certification, reversal

    /// A LIVE customer of MAIN's organisation, reserved through the customer service (a hold names one).
    async fn live_customer(&self) -> String {
        xc_customer::application::customer::service::CustomerService::new(BranchTx::new(
            Transactor::new(self.db().pool.clone()),
        ))
        .reserve(
            &BranchId::new(MAIN),
            xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer {
                name: "Held Customer".to_owned(),
                passport_no: unique("P").replace('_', "").chars().take(9).collect(),
                nationality: "THA".to_owned(),
                birth_date: String::new(),
                passport_expiry: String::new(),
                mode: "LIVE".to_owned(),
            },
            whole_second_now(),
        )
        .await
        .unwrap_or_else(|e| panic!("reserve a LIVE customer: {e}"))
        .customer_id
    }

    async fn hold(&self, at: DateTime<Utc>) {
        let customer = self.live_customer().await;
        let branch_tx = BranchTx::new(Transactor::new(self.db().pool.clone()));
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(self.db().pool.clone())),
            branch_tx.clone(),
        );
        let custody = pg_custody_service(&self.db().pool, &shifts, &branch_tx);
        // v2's MONITOR, an `employees` row (the kept custody path reads it, `custody_ports.rs:151-165`)
        self.db()
            .exec(&format!(
                "INSERT INTO employees (id, username, name, role, branch_id, password_hash, created_at) \
                 VALUES ('employee-monitor', 'monitor', 'Monitor', 'MONITOR', '{MAIN}', 'x', now()) \
                 ON CONFLICT (id) DO NOTHING"
            ))
            .await;
        let monitor = CustodyActor {
            id: "employee-monitor".to_owned(),
            employee_id: Some("employee-monitor".to_owned()),
            role: CustodyRole::Monitor,
        };
        // a hold is made by someone clocked in (custody_machine.rs, `Clock` before any other command)
        custody
            .command(
                &BranchId::new(MAIN),
                "opsClock",
                &json!({ "direction": "IN" }),
                &monitor,
                at - Duration::seconds(1),
            )
            .await
            .unwrap_or_else(|e| panic!("clock the monitor in: {e}"));
        custody
            .command(
                &BranchId::new(MAIN),
                "opsHold",
                &json!({ "customerId": customer, "reason": unique("check the passport") }),
                &monitor,
                at,
            )
            .await
            .unwrap_or_else(|e| panic!("hold the customer: {e}"));
    }

    /// FN-1 #1068 is not merged, so nothing writes `reconciliation_certifications` yet: this row is INSERTed
    /// by SQL (ruling N1 (a) on #1350, the only exception to "through the merged services").
    async fn certify(&self, day: NaiveDate, note: Option<&str>, at: DateTime<Utc>) {
        let finance = self.person(Role::Finance, Some(MAIN)).await;
        sqlx::query(
            "INSERT INTO reconciliation_certifications \
             (id, organization_id, branch_id, business_date, figures_hash, figures, certified_by, \
              certified_at, note) \
             VALUES (gen_random_uuid(), $1::uuid, $2, $3, 'hash', '{}'::jsonb, $4::uuid, $5, $6)",
        )
        .bind(ORG_1)
        .bind(MAIN)
        .bind(day)
        .bind(&finance.id)
        .bind(at)
        .bind(note)
        .execute(&self.db().pool)
        .await
        .expect("a certification row (FN-1 #1068's, written by SQL until it merges)");
    }

    /// A reversal of `original`, requested and approved through the merged workflow and AP-1, then POSTED through
    /// the merged repository with a receipt row this file writes (RV-3 #684 is not merged; ruling N2 (a)).
    async fn posted_reversal(&self, original: TransactionId, at: DateTime<Utc>) -> Decimal {
        let pool = self.db().pool.clone();
        let requester = SaveActor {
            id: "employee-manager".to_owned(),
            username: "manager".to_owned(),
            employee_id: Some("employee-manager".to_owned()),
            role: OldRole::Manager,
        };
        let mut tx = pool.begin().await.unwrap();
        let reversal = RequestReversal::new(pool.clone(), self.c.desk.approvals.clone())
            .run_in(
                &mut tx,
                RequestReversalInput {
                    branch: BranchId::new(MAIN),
                    original,
                    reason: "customer returned the notes".to_owned(),
                    key: unique("reversal"),
                },
                &requester,
                at,
            )
            .await
            .unwrap_or_else(|e| panic!("request a reversal: {e}"));
        tx.commit().await.unwrap();
        let request =
            xc_approval::domain::request::RequestId::new(reversal.approval_request_id.to_string());
        self.c
            .desk
            .decide(
                &request,
                &staff("employee-finance", OldRole::Finance, MAIN),
                Outcome::Approved,
                "ok",
            )
            .await
            .expect("FINANCE of the branch approves");
        let receipt = uuid::Uuid::new_v4();
        let number = unique("R");
        sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2, 'employee-manager', 'text/html', 'sha-reversal', now())",
        )
        .bind(&number)
        .bind(MAIN)
        .execute(&pool)
        .await
        .expect("the reversal receipt's file row");
        sqlx::query(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
             document_ref, kind, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 2, $5, $4, 'REVERSAL', $5, $5)",
        )
        .bind(receipt)
        .bind(ORG_1)
        .bind(original.0)
        .bind(&number)
        .bind(at)
        .execute(&pool)
        .await
        .expect("the reversal's receipt row");
        let posting_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
             VALUES ($1, $2::uuid, $3, 'REVERSAL', $4, 'reversal', 'POSTED', $5, 'employee-manager')",
        )
        .bind(posting_id)
        .bind(ORG_1)
        .bind(MAIN)
        .bind(original.0.to_string())
        .bind(at)
        .execute(&pool)
        .await
        .expect("ledger posting");
        let mut tx = pool.begin().await.unwrap();
        PgReversalRepository::new(pool.clone())
            .post_in(
                &mut tx,
                reversal.id,
                &PostedReversalRow {
                    posting_id,
                    receipt_id: receipt,
                    shift_id: None,
                    posted_by: "employee-manager".to_owned(),
                    posted_at: at,
                    realized_margin_reversed_thb: None,
                    cost_difference_thb: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("post the reversal: {e}"));
        tx.commit().await.unwrap();
        let amount = self
            .db()
            .text(&format!(
                "SELECT amount_thb::text FROM reversals WHERE original_transaction_id = '{}'",
                original.0
            ))
            .await
            .expect("the reversal's amount");
        Decimal::from_str(&amount).unwrap()
    }

    // -------------------------------------------------------- the view

    async fn get(&self, who: Option<&Who>, query: &str) -> (StatusCode, Value) {
        self.get_as(who.map(|w| w.spec.as_str()), query).await
    }

    async fn get_as(&self, spec: Option<&str>, query: &str) -> (StatusCode, Value) {
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                self.c.scratch.join("uploads").to_str().expect("UTF-8"),
            )])),
            self.db().pool.clone(),
        );
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let mut req = atest::TestRequest::get()
            .uri(&format!("/api/v3/views/{VIEW}?{query}"))
            .insert_header((header::HOST, HOST));
        if let Some(spec) = spec {
            req = req.insert_header((AS, spec.to_owned()));
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }

    /// The report of `day` (or of today when `None`) as `who`, which must be answered 200.
    async fn report(&self, who: &Who, day: Option<NaiveDate>) -> Value {
        let query = match day {
            Some(d) => format!("branch={MAIN}&day={d}"),
            None => format!("branch={MAIN}"),
        };
        let (status, body) = self.get(Some(who), &query).await;
        assert_eq!(status, StatusCode::OK, "the report of {day:?}: {body}");
        let data = body["data"].clone();
        assert!(data.is_object(), "a report: {body}");
        data
    }
}

// ------------------------------------------------------------ helpers

/// The instant `h:m:s` of `day` in `tz`.
fn local(tz: Tz, day: NaiveDate, h: u32, m: u32, s: u32) -> DateTime<Utc> {
    tz.from_local_datetime(&day.and_hms_opt(h, m, s).unwrap())
        .single()
        .expect("an unambiguous local time")
        .with_timezone(&Utc)
}

fn bangkok() -> Tz {
    "Asia/Bangkok".parse().unwrap()
}

/// 2026-10-05, a day in the past of every test run.
fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
}

/// Noon of `day()` in Bangkok, plus `s` seconds.
fn noon(s: i64) -> DateTime<Utc> {
    local(bangkok(), day(), 12, 0, 0) + Duration::seconds(s)
}

fn number(v: &Value, what: &str) -> Decimal {
    let text = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a decimal string: {v}"));
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{what} = {text:?}: {e}"))
}

fn count_of(v: &Value, what: &str) -> u64 {
    v.as_u64()
        .unwrap_or_else(|| panic!("{what} is a JSON number: {v}"))
}

/// The `currencies` row of `code`.
fn row<'a>(report: &'a Value, code: &str) -> &'a Value {
    report["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("currencies is a list: {report}"))
        .iter()
        .find(|r| r["currency"] == code)
        .unwrap_or_else(|| panic!("a {code} row: {report}"))
}

fn holds_any(role: Role) -> bool {
    READERS
        .iter()
        .any(|p| RolePermissions::of(role).contains(p))
}

/// The branch a role is given in these tests: organisation-wide for the roles §02 makes so (EXECUTIVE), MAIN
/// otherwise.
fn scope_of(role: Role) -> Option<&'static str> {
    if role == Role::Executive {
        None
    } else {
        Some(MAIN)
    }
}

/// Every figure key of a report (B.3 `StaffDailyReportData`).
const FIGURES: [&str; 20] = [
    "day",
    "timezone",
    "generatedAt",
    "company",
    "site",
    "branchName",
    "status",
    "completed",
    "buyVolume",
    "sellVolume",
    "reversedVolume",
    "pendingTransactions",
    "currencies",
    "pendingRequests",
    "openExceptions",
    "notes",
    "openHolds",
    "inTransit",
    "signers",
    "financeNote",
];

fn has_no_figure(body: &Value) {
    assert!(
        body.get("data").is_none_or(Value::is_null),
        "a refusal carries no data: {body}"
    );
    let text = body.to_string();
    for key in FIGURES {
        assert!(
            !text.contains(&format!("\"{key}\"")),
            "a refusal carries no figure ({key}): {body}"
        );
    }
}

// ------------------------------------------------------------ who may read it (B.1 rows 1, 2; D-B2)

/// B.1 row 1, D-B2: exactly the holders of `ShiftCloseCertify`, `FinanceReconcile` or `OverviewRead` read the
/// report; every other staff role is refused `roleNotAllowed` (400 on `606b3d2c`) with no figure. Each of the
/// three permissions alone admits its holder.
#[actix_web::test]
async fn exactly_the_holders_of_the_three_permissions_read_the_report() {
    let d = Day::start().await;
    let mut admitted = Vec::new();
    for role in STAFF_ROLES {
        let who = d.person(role, scope_of(role)).await;
        let (status, body) = d
            .get(Some(&who), &format!("branch={MAIN}&day={}", day()))
            .await;
        if holds_any(role) {
            assert_eq!(
                status,
                StatusCode::OK,
                "{} holds a reader permission: {body}",
                role.as_str()
            );
            assert!(body["data"].is_object(), "{}: {body}", role.as_str());
            admitted.push(role);
        } else {
            assert_eq!(
                (status, body["error"]["code"].as_str()),
                (StatusCode::BAD_REQUEST, Some("roleNotAllowed")),
                "{} holds none of the three: {body}",
                role.as_str()
            );
            has_no_figure(&body);
        }
    }
    // each of the three permissions is the reason of at least one admitted role
    for p in READERS {
        assert!(
            admitted
                .iter()
                .any(|r| RolePermissions::of(*r).contains(&p)),
            "a holder of {p:?} alone is admitted: {admitted:?}"
        );
    }
    d.finish().await;
}

/// The controller's addition to B.6: a holder of a reader permission naming a branch outside their scope is
/// refused `403 outOfScope`, with no figure.
#[actix_web::test]
async fn a_reader_of_another_branch_is_out_of_scope() {
    let d = Day::start().await;
    let other = d.person(Role::BranchManager, Some(BR2)).await;
    let (status, body) = d
        .get(Some(&other), &format!("branch={MAIN}&day={}", day()))
        .await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("outOfScope")),
        "BR2's manager asks for MAIN: {body}"
    );
    has_no_figure(&body);
    d.finish().await;
}

/// B.1 row 2 and D-B2: the view inherits the staff path, so nobody signed in, and the owner's cookie alone, are
/// refused as the staff path refuses a visitor with no staff session: `401 employeeAuthRequired` on `606b3d2c`
/// (`Policy::authorize(None, StaffRpc, …)`, policy.rs: `EntryPath::Staff* => EmployeeAuthRequired`); no figure.
#[actix_web::test]
async fn nobody_and_the_owners_cookie_alone_are_refused() {
    let d = Day::start().await;
    for spec in [None, Some("OWNER")] {
        let (status, body) = d
            .get_as(spec, &format!("branch={MAIN}&day={}", day()))
            .await;
        assert_eq!(
            (status, body["error"]["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("employeeAuthRequired")),
            "{spec:?}: {body}"
        );
        has_no_figure(&body);
    }
    d.finish().await;
}

/// D-B1: a malformed `day` is refused `400 invalidRequest`, field `day`.
#[actix_web::test]
async fn a_malformed_day_is_an_invalid_request_on_day() {
    let d = Day::start().await;
    let reader = d.reader().await;
    for bad in ["2026-13-01", "05-10-2026", "today"] {
        let (status, body) = d
            .get(Some(&reader), &format!("branch={MAIN}&day={bad}"))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert_eq!(body["error"]["code"], "invalidRequest", "{bad}: {body}");
        assert_eq!(body["error"]["field"], "day", "{bad}: {body}");
    }
    d.finish().await;
}

// ------------------------------------------------------------ the day (B.1 rows 4, 5, 20; D-B3)

/// B.1 row 5 and B.6: an exchange completed at 23:59:59 and one at 00:00:00 of the next day **in the branch's
/// timezone** land on different days, with a branch whose timezone is not `Asia/Bangkok`.
#[actix_web::test]
async fn completions_split_at_midnight_in_the_branchs_timezone() {
    let tz: Tz = "America/Los_Angeles".parse().unwrap();
    let d = Day::start_in(Some("America/Los_Angeles")).await;
    let reader = d.reader().await;
    let first = day();
    let next = first.succ_opt().unwrap();
    d.completed(Side::Buy, "100", None, local(tz, first, 23, 59, 59))
        .await;
    d.completed(Side::Buy, "100", None, local(tz, next, 0, 0, 0))
        .await;
    d.completed(Side::Buy, "100", None, local(tz, next, 0, 0, 1))
        .await;

    let a = d.report(&reader, Some(first)).await;
    let b = d.report(&reader, Some(next)).await;
    assert_eq!(a["day"], json!(first.to_string()), "{a}");
    assert_eq!(a["timezone"], json!("America/Los_Angeles"), "{a}");
    assert_eq!(count_of(&a["completed"], "completed"), 1, "{first}: {a}");
    assert_eq!(count_of(&b["completed"], "completed"), 2, "{next}: {b}");
    d.finish().await;
}

/// D-B3, B.1 rows 4 and 20: with no `day`, the server answers its own business date in the branch's timezone,
/// the branch's timezone, and `generatedAt` (RFC 3339) of the read.
#[actix_web::test]
async fn no_day_is_todays_business_date_in_the_branchs_timezone() {
    let d = Day::start_in(Some("Pacific/Kiritimati")).await;
    let reader = d.reader().await;
    let before = Utc::now();
    let report = d.report(&reader, None).await;
    let after = Utc::now();
    let tz: Tz = "Pacific/Kiritimati".parse().unwrap();
    let today = [before, after].map(|t| t.with_timezone(&tz).date_naive().to_string());
    assert!(
        today.contains(&report["day"].as_str().unwrap_or("").to_owned()),
        "today in Pacific/Kiritimati is one of {today:?}: {report}"
    );
    assert_eq!(report["timezone"], json!("Pacific/Kiritimati"), "{report}");
    let generated: DateTime<Utc> = report["generatedAt"]
        .as_str()
        .unwrap_or_else(|| panic!("generatedAt is a string: {report}"))
        .parse()
        .unwrap_or_else(|e| panic!("generatedAt is RFC 3339: {e}: {report}"));
    assert!(
        generated >= before - Duration::seconds(1) && generated <= after + Duration::seconds(1),
        "generatedAt is the read's instant: {generated} not in [{before}, {after}]"
    );
    d.finish().await;
}

/// B.1 row 9: the entity's company and site, and the branch's name.
#[actix_web::test]
async fn the_heading_is_the_entity_and_the_branch_name() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let name = d
        .db()
        .text(&format!("SELECT name FROM branches WHERE id = '{MAIN}'"))
        .await
        .expect("MAIN's name");
    let report = d.report(&reader, Some(day())).await;
    assert_eq!(report["company"], json!(COMPANY), "{report}");
    assert_eq!(report["site"], json!(SITE), "{report}");
    assert_eq!(report["branchName"], json!(name), "{report}");
    d.finish().await;
}

// ------------------------------------------------------------ exchanges (B.1 rows 5, 10, 11, 12)

/// B.1 rows 10, 11 and B.6: `buyVolume` / `sellVolume` = Σ `transaction_lines.thb_amount` of the day's COMPLETED
/// LIVE exchanges per `shop_side`, gross (a fee does not change them); a POSTED reversal is in `reversedVolume` and
/// is not subtracted; a DEMO exchange is in none of them, nor in `completed`.
#[actix_web::test]
async fn volumes_are_gross_thb_per_shop_side_with_reversals_apart_and_live_only() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let now = Utc::now();
    let today = now.with_timezone(&bangkok()).date_naive();
    // a fee makes settle_thb differ from thb_amount
    let buy = d
        .completed(Side::Buy, "100", Some("25.00"), now - Duration::seconds(30))
        .await;
    let sell = d
        .completed(Side::Sell, "40", Some("15.00"), now - Duration::seconds(20))
        .await;
    d.completed_demo(now - Duration::seconds(10)).await;
    let reversed = d.posted_reversal(sell, now).await;

    let thb = |id: TransactionId| {
        let db = d.db();
        async move {
            Decimal::from_str(
                &db.text(&format!(
                    "SELECT SUM(thb_amount)::text FROM transaction_lines WHERE transaction_id = '{}'",
                    id.0
                ))
                .await
                .expect("the lines"),
            )
            .unwrap()
        }
    };
    let (buy_thb, sell_thb) = (thb(buy).await, thb(sell).await);
    let settle: Decimal = Decimal::from_str(
        &d.db()
            .text(&format!(
                "SELECT SUM(settle_thb)::text FROM transaction_lines WHERE transaction_id = '{}'",
                buy.0
            ))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_ne!(settle, buy_thb, "the fixture: a fee makes settle differ");

    let report = d.report(&reader, Some(today)).await;
    assert_eq!(
        count_of(&report["completed"], "completed"),
        2,
        "LIVE only: {report}"
    );
    assert_eq!(
        number(&report["buyVolume"], "buyVolume"),
        buy_thb,
        "{report}"
    );
    assert_eq!(
        number(&report["sellVolume"], "sellVolume"),
        sell_thb,
        "gross: the reversal is not subtracted: {report}"
    );
    assert_eq!(
        number(&report["reversedVolume"], "reversedVolume"),
        reversed,
        "{report}"
    );
    d.finish().await;
}

/// B.1 row 12 and B.6: `pendingTransactions` counts the day's DRAFT, PENDING_APPROVAL and CONFIRMED exchanges,
/// and no COMPLETED one; a DEMO DRAFT is not counted (LIVE only).
#[actix_web::test]
async fn pending_transactions_are_the_days_unfinished_live_exchanges() {
    let d = Day::start().await;
    let reader = d.reader().await;
    // CONFIRMED
    let q = d.c.desk.quote(MAIN, Side::Buy, "100", None, noon(0)).await;
    let confirmed = d.c.live_of(&q, PaymentMethod::Cash, true, noon(0)).await;
    // DRAFT
    let q = d.c.desk.quote(MAIN, Side::Buy, "100", None, noon(10)).await;
    let draft =
        d.c.live_draft(&q, PaymentMethod::Cash, true, noon(10))
            .await;
    // COMPLETED
    d.completed(Side::Buy, "100", None, noon(20)).await;
    // a DEMO DRAFT
    let q = d.c.desk.quote(MAIN, Side::Buy, "100", None, noon(30)).await;
    d.c.desk.draft(&q, noon(30)).await;
    // PENDING_APPROVAL: above a limit set now
    d.c.desk.limit(MAIN, "1000.00").await;
    let q = d.c.desk.quote(MAIN, Side::Buy, "100", None, noon(40)).await;
    let pending =
        d.c.live_draft(&q, PaymentMethod::Cash, true, noon(40))
            .await;
    assert!(matches!(
        d.c.desk.confirm(pending.id, noon(41)).await,
        Ok(ConfirmOutcome::PendingApproval(_))
    ));
    // one of the next day: not counted on this day
    let q =
        d.c.desk
            .quote(MAIN, Side::Buy, "100", None, noon(86_400))
            .await;
    d.c.live_draft(&q, PaymentMethod::Cash, true, noon(86_400))
        .await;
    for (id, status) in [
        (confirmed.id, "CONFIRMED"),
        (draft.id, "DRAFT"),
        (pending.id, "PENDING_APPROVAL"),
    ] {
        assert_eq!(d.status_of(id).await, status, "the fixture");
    }

    let report = d.report(&reader, Some(day())).await;
    assert_eq!(
        count_of(&report["pendingTransactions"], "pendingTransactions"),
        3,
        "{report}"
    );
    assert_eq!(count_of(&report["completed"], "completed"), 1, "{report}");
    d.finish().await;
}

// ------------------------------------------------------------ counts (B.1 rows 7, 8, 13, 18; Q-SR1)

/// B.1 rows 7 and 13 (Q-SR1): per currency, the drawer figures are the sum over **every** verified `CLOSE` count of
/// the day's counted shifts of the stored `counted`, `expected` and `difference`; an unverified CLOSE count and a
/// CLOSE count of another day are not in them. The seven codes come in v2's order, `null` where no count gives a
/// figure.
#[actix_web::test]
async fn the_drawer_is_the_sum_of_every_verified_close_count_of_the_day() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let (a, a_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 5), ("USD", "100", 3)], noon(0))
        .await;
    let (b, b_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 2)], noon(10))
        .await;
    let (c, c_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 7)], noon(20))
        .await;
    // a counted shift of the day before, closed that day
    let (y, y_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 9)], noon(-86_400))
        .await;
    let a_close = d
        .close(
            &a,
            &manager,
            &a_shift,
            &[("THB", "1000", 5), ("USD", "100", 3)],
            None,
            noon(3_600),
            true,
        )
        .await;
    // B counts one note fewer than expected: a difference
    let b_close = d
        .close(
            &b,
            &manager,
            &b_shift,
            &[("THB", "1000", 1)],
            None,
            noon(3_610),
            true,
        )
        .await;
    // C's CLOSE is submitted, not verified
    d.close(
        &c,
        &manager,
        &c_shift,
        &[("THB", "1000", 7)],
        None,
        noon(3_620),
        false,
    )
    .await;
    d.close(
        &y,
        &manager,
        &y_shift,
        &[("THB", "1000", 9)],
        None,
        noon(-80_000),
        true,
    )
    .await;

    let report = d.report(&reader, Some(day())).await;
    let codes: Vec<&str> = report["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("currencies: {report}"))
        .iter()
        .map(|r| r["currency"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(codes, CODES, "v2's seven codes in v2's order: {report}");
    let verified = [a_close.as_str(), b_close.as_str()];
    for (key, column) in [
        ("drawerCounted", "counted"),
        ("drawerExpected", "expected"),
        ("drawerDifference", "difference"),
    ] {
        for code in ["THB", "USD"] {
            let want = d.summed(column, &verified, code).await;
            let got = &row(&report, code)[key];
            match want {
                Some(want) => assert_eq!(
                    number(got, key),
                    want,
                    "{code} {key} = Σ {column} of A's and B's verified CLOSE counts: {report}"
                ),
                None => assert!(got.is_null(), "{code} {key}: {report}"),
            }
        }
    }
    assert_ne!(
        d.summed("difference", &[b_close.as_str()], "THB").await,
        Some(Decimal::ZERO),
        "the fixture: B's count has a difference"
    );
    for code in ["EUR", "GBP", "JPY", "CNY", "AUD"] {
        for key in ["drawerCounted", "drawerExpected", "drawerDifference"] {
            assert!(row(&report, code)[key].is_null(), "{code} {key}: {report}");
        }
    }
    d.finish().await;
}

/// B.1 row 13: the safe columns are the day's **last** verified `SAFE_OPEN`/`SAFE` count (`shift_id NULL`), last
/// by `verified_at`, as stored; `null` when the day has none.
#[actix_web::test]
async fn the_safe_is_the_days_last_verified_safe_count() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let finance = d.person(Role::Finance, Some(MAIN)).await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let first = d
        .submit(
            &finance,
            CountKind::SafeOpen,
            None,
            &[("USD", "100", 50)],
            None,
            noon(0),
        )
        .await;
    d.verify(&manager, &first, noon(1)).await;
    let last = d
        .submit(
            &finance,
            CountKind::Safe,
            None,
            &[("USD", "100", 48)],
            None,
            noon(600),
        )
        .await;
    d.verify(&manager, &last, noon(601)).await;
    // a later SAFE count that is not verified
    d.submit(
        &finance,
        CountKind::Safe,
        None,
        &[("USD", "100", 1)],
        None,
        noon(1_200),
    )
    .await;

    let report = d.report(&reader, Some(day())).await;
    for (key, column) in [
        ("safeCounted", "counted"),
        ("safeExpected", "expected"),
        ("safeDifference", "difference"),
    ] {
        let want = d.summed(column, &[last.as_str()], "USD").await;
        let got = &row(&report, "USD")[key];
        match want {
            Some(want) => assert_eq!(number(got, key), want, "USD {key}: {report}"),
            None => assert!(got.is_null(), "USD {key}: {report}"),
        }
    }
    for key in ["safeCounted", "safeExpected", "safeDifference"] {
        assert!(row(&report, "EUR")[key].is_null(), "EUR {key}: {report}");
    }
    let other = d.report(&reader, Some(day().pred_opt().unwrap())).await;
    for key in ["safeCounted", "safeExpected", "safeDifference"] {
        assert!(
            row(&other, "USD")[key].is_null(),
            "no safe count the day before: {other}"
        );
    }
    d.finish().await;
}

/// B.1 row 8 and B.6: `preliminary` while one counted shift of the day lacks a verified `CLOSE` count (or the day
/// has no counted shift), then `closedAwaitingReconciliation`, then `reconciled` once a certification exists for
/// (branch, day). The v2-opened shift of the fixture, never closed by a count, does not hold the status back.
#[actix_web::test]
async fn the_status_moves_from_preliminary_to_closed_to_reconciled() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    assert_eq!(
        d.report(&reader, Some(day())).await["status"],
        "preliminary",
        "no counted shift"
    );
    let (a, a_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 1)], noon(0))
        .await;
    let (b, b_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 1)], noon(10))
        .await;
    d.close(
        &a,
        &manager,
        &a_shift,
        &[("THB", "1000", 1)],
        None,
        noon(3_600),
        true,
    )
    .await;
    assert_eq!(
        d.report(&reader, Some(day())).await["status"],
        "preliminary",
        "B's shift has no verified CLOSE count yet"
    );
    d.close(
        &b,
        &manager,
        &b_shift,
        &[("THB", "1000", 1)],
        None,
        noon(3_700),
        false,
    )
    .await;
    assert_eq!(
        d.report(&reader, Some(day())).await["status"],
        "preliminary",
        "B's CLOSE count is not verified yet"
    );
    let b_count = d
        .db()
        .text(&format!(
            "SELECT id::text FROM cash_counts WHERE shift_id = '{b_shift}' AND kind = 'CLOSE'"
        ))
        .await
        .unwrap();
    d.verify(&manager, &b_count, noon(3_701)).await;
    assert_eq!(
        d.report(&reader, Some(day())).await["status"],
        "closedAwaitingReconciliation",
        "every counted shift of the day has a verified CLOSE count; the v2-opened shift is left out"
    );
    d.certify(day().succ_opt().unwrap(), Some("another day"), noon(86_400))
        .await;
    assert_eq!(
        d.report(&reader, Some(day())).await["status"],
        "closedAwaitingReconciliation",
        "a certification of another day does not reconcile this one"
    );
    d.certify(day(), Some("checked"), noon(7_200)).await;
    assert_eq!(d.report(&reader, Some(day())).await["status"], "reconciled");
    d.finish().await;
}

/// B.1 row 8 and B.6: a v2-opened shift of the day (`opened_by_count IS NULL`; the fixture opens MAIN's at the
/// start of the test, today) is left out of the status rule: today's one counted shift, closed and verified, makes
/// the day `closedAwaitingReconciliation` although the v2 shift has no CLOSE count.
#[actix_web::test]
async fn a_v2_opened_shift_of_the_day_does_not_hold_the_status_back() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let now = Utc::now();
    let today = now.with_timezone(&bangkok()).date_naive();
    let v2_today = d
        .db()
        .count(&format!(
            "SELECT count(*) FROM shifts WHERE branch_id = '{MAIN}' AND opened_by_count IS NULL \
             AND status = 'OPEN' AND opened_at >= now() - interval '1 hour'"
        ))
        .await;
    assert_eq!(
        v2_today, 1,
        "the fixture: MAIN's v2-opened shift, opened today"
    );
    let (a, a_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 1)], now - Duration::seconds(30))
        .await;
    d.close(
        &a,
        &manager,
        &a_shift,
        &[("THB", "1000", 1)],
        None,
        now - Duration::seconds(10),
        true,
    )
    .await;
    assert_eq!(
        d.report(&reader, Some(today)).await["status"],
        "closedAwaitingReconciliation"
    );
    d.finish().await;
}

/// B.1 row 18 (Q-SR1): the signers are every verified CLOSE count's `counted_by` and `verified_by`, named from
/// `users`, with the capacity, not a role; an unverified CLOSE count signs nothing; none → an empty list.
#[actix_web::test]
async fn the_signers_are_every_verified_close_counts_counter_and_verifier() {
    let d = Day::start().await;
    let reader = d.reader().await;
    assert_eq!(
        d.report(&reader, Some(day())).await["signers"],
        json!([]),
        "no CLOSE count: no signer"
    );
    let m1 = d.person(Role::BranchManager, Some(MAIN)).await;
    let m2 = d.person(Role::BranchManager, Some(MAIN)).await;
    let (a, a_shift) = d.counted_shift(&m1, &[("THB", "1000", 1)], noon(0)).await;
    let (b, b_shift) = d.counted_shift(&m1, &[("THB", "1000", 1)], noon(10)).await;
    let (c, c_shift) = d.counted_shift(&m1, &[("THB", "1000", 1)], noon(20)).await;
    d.close(
        &a,
        &m1,
        &a_shift,
        &[("THB", "1000", 1)],
        None,
        noon(3_600),
        true,
    )
    .await;
    d.close(
        &b,
        &m2,
        &b_shift,
        &[("THB", "1000", 1)],
        None,
        noon(3_700),
        true,
    )
    .await;
    d.close(
        &c,
        &m2,
        &c_shift,
        &[("THB", "1000", 1)],
        None,
        noon(3_800),
        false,
    )
    .await;

    let report = d.report(&reader, Some(day())).await;
    let mut got: Vec<(String, String)> = report["signers"]
        .as_array()
        .unwrap_or_else(|| panic!("signers: {report}"))
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap_or("").to_owned(),
                s["capacity"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    got.sort();
    let mut want = vec![
        (a.name.clone(), "counted".to_owned()),
        (m1.name.clone(), "verified".to_owned()),
        (b.name.clone(), "counted".to_owned()),
        (m2.name.clone(), "verified".to_owned()),
    ];
    want.sort();
    assert_eq!(got, want, "{report}");
    d.finish().await;
}

// ------------------------------------------------------------ hand-overs, transfers, exceptions (rows 14, 17)

/// B.1 row 14 (Q-SR2): `pendingRequests` = PENDING hand-overs of the branch, plus transfers in DRAFT, APPROVED or
/// DISPATCHED with the branch at either end, every one still open, not by day. RECEIVED transfers, other
/// branches' transfers and decided hand-overs are not counted.
#[actix_web::test]
async fn pending_requests_are_every_open_handover_and_transfer_at_either_end() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let approver = d.person(Role::BranchManager, Some(MAIN)).await;
    let br2 = d.person(Role::BranchManager, Some(BR2)).await;
    let br2_approver = d.person(Role::BranchManager, Some(BR2)).await;
    let executive = d.person(Role::Executive, None).await;
    d.policy(&executive).await;
    let usd = [("USD", "100", 1)];
    let yesterday = noon(-86_400);
    // counted: a DRAFT of yesterday, an APPROVED, a DISPATCHED out, a DISPATCHED in (from BR2)
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        None,
        yesterday,
        "DRAFT",
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        None,
        noon(0),
        "APPROVED",
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        None,
        noon(10),
        "DISPATCHED",
    )
    .await;
    d.transfer(
        &br2,
        &br2_approver,
        None,
        TransferKind::Branch,
        BR2,
        Some(MAIN),
        &usd,
        None,
        noon(20),
        "DISPATCHED",
    )
    .await;
    // not counted: RECEIVED, and BR2's transfer that does not touch MAIN
    let received = d
        .transfer(
            &manager,
            &approver,
            Some(&br2),
            TransferKind::Branch,
            MAIN,
            Some(BR2),
            &usd,
            None,
            noon(30),
            "RECEIVED",
        )
        .await;
    assert_eq!(
        d.transfer_status(&received).await,
        "RECEIVED",
        "the fixture"
    );
    d.transfer(
        &br2,
        &br2_approver,
        None,
        TransferKind::ExternalOut,
        BR2,
        None,
        &usd,
        None,
        noon(40),
        "DISPATCHED",
    )
    .await;
    // a PENDING hand-over, started yesterday
    let (teller, shift) = d
        .counted_shift(&manager, &[("THB", "1000", 1)], yesterday)
        .await;
    let receiver = d.person(Role::Teller, Some(MAIN)).await;
    let lines = vec![CountLine::new(
        "THB",
        d.denomination("THB", "1000").await,
        1,
    )];
    d.handovers
        .start(
            &Day::actor(&teller),
            StartHandover {
                branch: BranchId::new(MAIN),
                from_shift_id: shift,
                receiver_id: receiver.id.clone(),
                lines,
                note: None,
                idempotency_key: unique("handover"),
            },
            yesterday + Duration::seconds(60),
        )
        .await
        .unwrap_or_else(|e| panic!("start a hand-over: {e}"));

    let report = d.report(&reader, Some(day())).await;
    assert_eq!(
        count_of(&report["pendingRequests"], "pendingRequests"),
        5,
        "1 hand-over + DRAFT + APPROVED + DISPATCHED out + DISPATCHED in: {report}"
    );
    let later_day = d.report(&reader, Some(day().succ_opt().unwrap())).await;
    assert_eq!(
        count_of(&later_day["pendingRequests"], "pendingRequests"),
        5,
        "not by day: {later_day}"
    );
    d.finish().await;
}

/// B.1 row 17 (Q-SR4): in transit = every transfer of the branch in DISPATCHED now, of every kind, with the branch
/// at either end, whenever it was dispatched; each as `{kind, amount, currency}` with `amount` = Σ `qty_sent ×
/// face_value` per currency. DRAFT, APPROVED and RECEIVED transfers are not in transit. None → an empty list.
#[actix_web::test]
async fn in_transit_is_every_dispatched_transfer_of_every_kind_at_either_end() {
    let d = Day::start().await;
    let reader = d.reader().await;
    assert_eq!(
        d.report(&reader, Some(day())).await["inTransit"],
        json!([]),
        "nothing dispatched"
    );
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let approver = d.person(Role::BranchManager, Some(MAIN)).await;
    let br2 = d.person(Role::BranchManager, Some(BR2)).await;
    let br2_approver = d.person(Role::BranchManager, Some(BR2)).await;
    let executive = d.person(Role::Executive, None).await;
    d.policy(&executive).await;
    // out of MAIN, dispatched the day before
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &[("USD", "100", 2), ("USD", "50", 1)],
        None,
        noon(-86_400),
        "DISPATCHED",
    )
    .await;
    // into MAIN from BR2
    d.transfer(
        &br2,
        &br2_approver,
        None,
        TransferKind::Branch,
        BR2,
        Some(MAIN),
        &[("USD", "100", 3)],
        None,
        noon(0),
        "DISPATCHED",
    )
    .await;
    // another kind: MAIN's safe to outside
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::ExternalOut,
        MAIN,
        None,
        &[("USD", "20", 4)],
        None,
        noon(10),
        "DISPATCHED",
    )
    .await;
    // not in transit
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &[("USD", "100", 9)],
        None,
        noon(20),
        "APPROVED",
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        Some(&br2),
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &[("USD", "100", 8)],
        None,
        noon(30),
        "RECEIVED",
    )
    .await;

    let report = d.report(&reader, Some(day())).await;
    let mut got: Vec<(String, Decimal, String)> = report["inTransit"]
        .as_array()
        .unwrap_or_else(|| panic!("inTransit: {report}"))
        .iter()
        .map(|t| {
            (
                t["kind"].as_str().unwrap_or("").to_owned(),
                number(&t["amount"], "amount"),
                t["currency"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    got.sort();
    let mut want = vec![
        ("BRANCH".to_owned(), Decimal::from(250), "USD".to_owned()),
        ("BRANCH".to_owned(), Decimal::from(300), "USD".to_owned()),
        (
            "EXTERNAL_OUT".to_owned(),
            Decimal::from(80),
            "USD".to_owned(),
        ),
    ];
    want.sort();
    assert_eq!(got, want, "{report}");
    d.finish().await;
}

/// B.1 row 14: `openExceptions` = the day's OPEN `cash_exceptions`, read through `CustodyPages::exceptions_between`
/// (`CashException.at` in the day bounds). A verified count with a difference writes one (cash-shift §4.1).
///
/// **Expected red until the count path writes the kept table's shape** (ruling N4 (a) on #1350): the v3 count path
/// writes `cash_exceptions.data = {countId, currency, difference}` with no `at` and no `status`
/// (`xc-cash/src/application/count/count_service.rs:1094`), which `exceptions_between` cannot see. The fix is BE's
/// ticket at that root; this test is not to be edited to pass before it.
#[actix_web::test]
async fn open_exceptions_are_the_days_open_cash_exceptions() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    assert_eq!(
        count_of(
            &d.report(&reader, Some(day())).await["openExceptions"],
            "openExceptions"
        ),
        0
    );
    let (a, a_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 3)], noon(0))
        .await;
    d.close(
        &a,
        &manager,
        &a_shift,
        &[("THB", "1000", 2)],
        None,
        noon(3_600),
        true,
    )
    .await;
    let written = d
        .db()
        .count(&format!(
            "SELECT count(*) FROM cash_exceptions WHERE branch_id = '{MAIN}'"
        ))
        .await;
    assert!(
        written >= 1,
        "the fixture: a verified difference writes an exception"
    );

    assert_eq!(
        count_of(
            &d.report(&reader, Some(day())).await["openExceptions"],
            "openExceptions"
        ),
        written as u64,
        "the day's OPEN exceptions"
    );
    assert_eq!(
        count_of(
            &d.report(&reader, Some(day().succ_opt().unwrap())).await["openExceptions"],
            "openExceptions"
        ),
        0,
        "by day"
    );
    d.finish().await;
}

// ------------------------------------------------------------ notes, holds, finance (rows 15, 16, 19)

/// B.1 row 15 (Q-SR3): the notes are the day's non-empty `cash_counts.note` and `cash_transfers.note`, at most 5,
/// in creation order, each `{kind, note}` with the record's own kind code.
#[actix_web::test]
async fn notes_are_the_first_five_count_and_transfer_notes_of_the_day() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let approver = d.person(Role::BranchManager, Some(MAIN)).await;
    let finance = d.person(Role::Finance, Some(MAIN)).await;
    let executive = d.person(Role::Executive, None).await;
    d.policy(&executive).await;
    let usd = [("USD", "100", 1)];
    // the day before: not on this day
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        Some("yesterday's note"),
        noon(-86_400),
        "DRAFT",
    )
    .await;
    let (a, a_shift) = d
        .counted_shift(&manager, &[("THB", "1000", 1)], noon(0))
        .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        Some("note 1"),
        noon(100),
        "DRAFT",
    )
    .await;
    d.submit(
        &finance,
        CountKind::Safe,
        None,
        &[("USD", "100", 1)],
        Some("note 2"),
        noon(200),
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        None,
        noon(300),
        "DRAFT",
    )
    .await;
    d.close(
        &a,
        &manager,
        &a_shift,
        &[("THB", "1000", 1)],
        Some("note 3"),
        noon(400),
        true,
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::ExternalOut,
        MAIN,
        None,
        &usd,
        Some("note 4"),
        noon(500),
        "DRAFT",
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        Some("note 5"),
        noon(600),
        "DRAFT",
    )
    .await;
    d.transfer(
        &manager,
        &approver,
        None,
        TransferKind::Branch,
        MAIN,
        Some(BR2),
        &usd,
        Some("note 6"),
        noon(700),
        "DRAFT",
    )
    .await;

    let report = d.report(&reader, Some(day())).await;
    assert_eq!(
        report["notes"],
        json!([
            { "kind": "BRANCH", "note": "note 1" },
            { "kind": "SAFE", "note": "note 2" },
            { "kind": "CLOSE", "note": "note 3" },
            { "kind": "EXTERNAL_OUT", "note": "note 4" },
            { "kind": "BRANCH", "note": "note 5" },
        ]),
        "{report}"
    );
    d.finish().await;
}

/// B.1 row 16: `openHolds` = every OPEN `customer_holds` row of the branch, not by day.
#[actix_web::test]
async fn open_holds_are_every_open_hold_not_by_day() {
    let d = Day::start().await;
    let reader = d.reader().await;
    d.hold(noon(-86_400 * 3)).await;
    d.hold(noon(0)).await;
    let report = d.report(&reader, Some(day().succ_opt().unwrap())).await;
    assert_eq!(count_of(&report["openHolds"], "openHolds"), 2, "{report}");
    d.finish().await;
}

/// B.1 row 19: `financeNote` = the `note` of the newest certification of (branch, day); `null` when none.
#[actix_web::test]
async fn the_finance_note_is_the_newest_certifications_note() {
    let d = Day::start().await;
    let reader = d.reader().await;
    assert!(
        d.report(&reader, Some(day())).await["financeNote"].is_null(),
        "no certification"
    );
    d.certify(day(), Some("first look"), noon(3_600)).await;
    d.certify(day(), Some("all matched"), noon(7_200)).await;
    d.certify(
        day().succ_opt().unwrap(),
        Some("the next day"),
        noon(90_000),
    )
    .await;
    assert_eq!(
        d.report(&reader, Some(day())).await["financeNote"],
        json!("all matched")
    );
    d.finish().await;
}

// ------------------------------------------------------------ the answer's keys

/// B.3: the answer carries every figure key and nothing that names a customer.
#[actix_web::test]
async fn the_report_carries_every_figure_and_no_customer() {
    let d = Day::start().await;
    let reader = d.reader().await;
    d.completed(Side::Buy, "100", None, noon(0)).await;
    let report = d.report(&reader, Some(day())).await;
    let keys: Vec<&str> = report
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut want: Vec<&str> = FIGURES.to_vec();
    want.sort();
    let mut got = keys.clone();
    got.sort();
    assert_eq!(got, want, "{report}");
    let text = report.to_string();
    assert!(!text.contains(CUSTOMER), "no customer id: {report}");
    assert!(
        !text.contains("Test Customer"),
        "no customer name: {report}"
    );
    let _ = BTreeMap::<u8, u8>::new();
    d.finish().await;
}

// ------------------------------------------------------------ issue #1521: overnight shifts and close counts

/// Issue #1521, row 1: shift opened D 22:00 local, CLOSE counted and verified D+1 01:00.
/// Day D: drawer includes it, signers include its two users, status closed-awaiting.
/// Day D+1: drawer and signers exclude it.
#[actix_web::test]
async fn overnight_shift_opened_d_evening_close_counted_and_verified_d_plus_1_morning() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;

    let open_time = local(bangkok(), day(), 22, 0, 0);
    let close_time = local(bangkok(), day().succ_opt().unwrap(), 1, 0, 0);

    let (teller, shift) = d
        .counted_shift(
            &manager,
            &[("THB", "1000", 5), ("USD", "100", 3)],
            open_time,
        )
        .await;

    let close_count = d
        .close(
            &teller,
            &manager,
            &shift,
            &[("THB", "1000", 5), ("USD", "100", 3)],
            None,
            close_time,
            true,
        )
        .await;

    // Day D's report: drawer includes it, signers include teller and manager, status closedAwaitingReconciliation
    let report_d = d.report(&reader, Some(day())).await;
    assert_eq!(
        report_d["status"], "closedAwaitingReconciliation",
        "Day D status is closedAwaitingReconciliation: {report_d}"
    );
    for (key, column) in [
        ("drawerCounted", "counted"),
        ("drawerExpected", "expected"),
        ("drawerDifference", "difference"),
    ] {
        for code in ["THB", "USD"] {
            let want = d.summed(column, &[&close_count], code).await;
            let got = &row(&report_d, code)[key];
            match want {
                Some(w) => assert_eq!(
                    number(got, key),
                    w,
                    "Day D {code} {key} includes the overnight shift: {report_d}"
                ),
                None => assert!(got.is_null(), "{code} {key}: {report_d}"),
            }
        }
    }
    let mut got_signers_d: Vec<(String, String)> = report_d["signers"]
        .as_array()
        .unwrap_or_else(|| panic!("signers: {report_d}"))
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap_or("").to_owned(),
                s["capacity"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    got_signers_d.sort();
    let mut want_signers_d = vec![
        (teller.name.clone(), "counted".to_owned()),
        (manager.name.clone(), "verified".to_owned()),
    ];
    want_signers_d.sort();
    assert_eq!(
        got_signers_d, want_signers_d,
        "Day D signers include both users: {report_d}"
    );

    // Day D+1's report: drawer and signers exclude it
    let report_d1 = d.report(&reader, Some(day().succ_opt().unwrap())).await;
    for code in ["THB", "USD", "EUR", "GBP", "JPY", "CNY", "AUD"] {
        for key in ["drawerCounted", "drawerExpected", "drawerDifference"] {
            assert!(
                row(&report_d1, code)[key].is_null(),
                "Day D+1 {code} {key} must exclude the overnight shift: {report_d1}"
            );
        }
    }
    assert_eq!(
        report_d1["signers"],
        json!([]),
        "Day D+1 signers exclude the overnight shift: {report_d1}"
    );

    d.finish().await;
}

/// Issue #1521, row 2: shift opened and closed on D (unchanged).
/// Day D: included.
/// Day D+1: excluded.
#[actix_web::test]
async fn shift_opened_and_closed_on_d_is_included_in_d_and_excluded_in_d_plus_1() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;

    let open_time = local(bangkok(), day(), 9, 0, 0);
    let close_time = local(bangkok(), day(), 17, 0, 0);

    let (teller, shift) = d
        .counted_shift(&manager, &[("THB", "1000", 5)], open_time)
        .await;

    let close_count = d
        .close(
            &teller,
            &manager,
            &shift,
            &[("THB", "1000", 5)],
            None,
            close_time,
            true,
        )
        .await;

    // Day D: included
    let report_d = d.report(&reader, Some(day())).await;
    assert_eq!(
        report_d["status"], "closedAwaitingReconciliation",
        "Day D status: {report_d}"
    );
    let want_thb = d.summed("counted", &[&close_count], "THB").await.unwrap();
    assert_eq!(
        number(&row(&report_d, "THB")["drawerCounted"], "drawerCounted"),
        want_thb,
        "Day D drawer includes the shift: {report_d}"
    );
    let mut got_signers: Vec<(String, String)> = report_d["signers"]
        .as_array()
        .unwrap_or_else(|| panic!("signers: {report_d}"))
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap_or("").to_owned(),
                s["capacity"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    got_signers.sort();
    let mut want_signers = vec![
        (teller.name.clone(), "counted".to_owned()),
        (manager.name.clone(), "verified".to_owned()),
    ];
    want_signers.sort();
    assert_eq!(got_signers, want_signers, "Day D signers: {report_d}");

    // Day D+1: excluded
    let report_d1 = d.report(&reader, Some(day().succ_opt().unwrap())).await;
    assert!(
        row(&report_d1, "THB")["drawerCounted"].is_null(),
        "Day D+1 drawer excludes Day D shift: {report_d1}"
    );
    assert_eq!(
        report_d1["signers"],
        json!([]),
        "Day D+1 signers exclude Day D shift: {report_d1}"
    );

    d.finish().await;
}

/// Issue #1521, row 3: a verified CLOSE whose shift is v2-opened (`opened_by_count IS NULL`).
/// Day D: excluded.
/// Day D+1: excluded.
#[actix_web::test]
async fn verified_close_whose_shift_is_v2_opened_is_excluded_from_both_days() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;
    let teller = d.person(Role::Teller, Some(MAIN)).await;

    // Create a v2-opened shift directly in DB with opened_by_count NULL on Day D
    let shift_id = unique("v2-shift");
    let open_req = unique("open-req");
    d.db()
        .exec(&format!(
            "UPDATE shifts SET status = 'CLOSED' WHERE id = '{}'",
            d.c.shift
        ))
        .await;
    d.db()
        .exec(&format!(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id, opened_by_count) \
             VALUES ('{shift_id}', '{MAIN}', 'OPEN', '{}', 'owner', '{open_req}', '{}'::uuid, NULL)",
            local(bangkok(), day(), 10, 0, 0),
            teller.id
        ))
        .await;

    // Verified CLOSE count for that v2-opened shift on Day D
    d.close(
        &teller,
        &manager,
        &shift_id,
        &[("THB", "1000", 5)],
        None,
        noon(3_600),
        true,
    )
    .await;

    // Day D: excluded
    let report_d = d.report(&reader, Some(day())).await;
    assert!(
        row(&report_d, "THB")["drawerCounted"].is_null(),
        "Day D drawer must exclude v2-opened shift's CLOSE count: {report_d}"
    );
    assert_eq!(
        report_d["signers"],
        json!([]),
        "Day D signers must exclude v2-opened shift's CLOSE count: {report_d}"
    );

    // Day D+1: excluded
    let report_d1 = d.report(&reader, Some(day().succ_opt().unwrap())).await;
    assert!(
        row(&report_d1, "THB")["drawerCounted"].is_null(),
        "Day D+1 drawer must exclude v2-opened shift's CLOSE count: {report_d1}"
    );
    assert_eq!(
        report_d1["signers"],
        json!([]),
        "Day D+1 signers must exclude v2-opened shift's CLOSE count: {report_d1}"
    );

    d.finish().await;
}

/// Issue #1521, row 4: an unverified CLOSE of a D shift.
/// Day D: excluded.
/// Day D+1: excluded.
#[actix_web::test]
async fn unverified_close_of_d_shift_is_excluded_from_both_days() {
    let d = Day::start().await;
    let reader = d.reader().await;
    let manager = d.person(Role::BranchManager, Some(MAIN)).await;

    let (teller, shift) = d
        .counted_shift(&manager, &[("THB", "1000", 5)], noon(0))
        .await;

    // Unverified CLOSE count on Day D
    d.close(
        &teller,
        &manager,
        &shift,
        &[("THB", "1000", 5)],
        None,
        noon(3_600),
        false,
    )
    .await;

    // Day D: excluded (and status is preliminary because the shift is not verified-closed)
    let report_d = d.report(&reader, Some(day())).await;
    assert_eq!(
        report_d["status"], "preliminary",
        "Day D status with unverified close is preliminary: {report_d}"
    );
    assert!(
        row(&report_d, "THB")["drawerCounted"].is_null(),
        "Day D drawer excludes unverified CLOSE count: {report_d}"
    );
    assert_eq!(
        report_d["signers"],
        json!([]),
        "Day D signers exclude unverified CLOSE count: {report_d}"
    );

    // Day D+1: excluded
    let report_d1 = d.report(&reader, Some(day().succ_opt().unwrap())).await;
    assert!(
        row(&report_d1, "THB")["drawerCounted"].is_null(),
        "Day D+1 drawer excludes unverified CLOSE count: {report_d1}"
    );
    assert_eq!(
        report_d1["signers"],
        json!([]),
        "Day D+1 signers exclude unverified CLOSE count: {report_d1}"
    );

    d.finish().await;
}
