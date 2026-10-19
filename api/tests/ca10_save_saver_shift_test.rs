//! CA-10-save (#626 ruling 4, issuecomment-6237790469; Q1/Q2 ruled at 17:59Z): a save made by a signed-in person
//! moves stock on **that person's own** OPEN shift, found by `ShiftService::teller_of_actor_in(saver)` and moved by
//! `apply_delta_for_teller_in`. No such teller, or a teller with no OPEN shift, is `ShiftError::NoOpenShift`, as
//! for `save_as_teller`. Today the no-teller arm (`save_transaction.rs:522`) moves "the branch's shift" through
//! `apply_delta_in(branch)`, which the core #2955 deletes.
//!
//! Written before the implementation by claude/dell-claude-1. Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! Every case has two OPEN shifts at `MAIN`: the saver's own and another teller's. Before M3 the other teller's is
//! the branch's v2 shift (no `opened_by_count`), so today the save lands there. That is the red.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod save_common;

use std::collections::BTreeMap;

use pos_error::AppError;
use save_common::{JPEG, MAIN, Shop, at, branch, form, fund_ledger_drawer, unique};
use sqlx::PgPool;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xchange::application::workflows::save_step::{SaveActor, SaveExtras};

/// `ShiftError::NoOpenShift`'s sentence.
const NO_OPEN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";

const OPENING: &[(&str, &str)] = &[("THB", "100000"), ("USD", "1000")];

/// The `users.id` of an opener or saver key `id` (`"owner"` = the organisation-wide owner user).
async fn user_of(pool: &PgPool, id: &str) -> String {
    if let Some(u) =
        sqlx::query_scalar::<_, String>("SELECT id::text FROM users WHERE id::text = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .expect("look up user by id")
    {
        return u;
    }
    if id == "owner" {
        if let Some(u) = sqlx::query_scalar::<_, String>(
            "SELECT u.id::text FROM users u WHERE EXISTS \
             (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
             ORDER BY u.created_at, u.id LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .expect("look for the owner's user")
        {
            return u;
        }
        return sqlx::query_scalar(
            "WITH o AS (SELECT id FROM organizations WHERE code = 'XC'), \
             u AS (INSERT INTO users (organization_id, name, status, created_by) \
                   SELECT id, 'owner', 'PENDING', 'test' FROM o RETURNING id, organization_id), \
             r AS (INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                   SELECT u.id, v.code, u.organization_id, NULL FROM u, (VALUES ('EXECUTIVE'), ('ADMIN')) AS v(code)) \
             SELECT id::text FROM u",
        )
        .fetch_one(pool)
        .await
        .expect("seed the owner's user");
    }
    if let Some(u) = sqlx::query_scalar::<_, String>(
        "SELECT u.id::text FROM users u JOIN organizations o ON o.id = u.organization_id \
         WHERE o.code = 'XC' AND u.name = $1 AND u.created_by = 'test' \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .expect("look for an opener's user")
    {
        return u;
    }
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, $1, 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the user of {id}: {e}"))
}

/// An OPEN shift of `teller` at `branch_id`, written straight into the tables, its drawer funded through the
/// ledger. With `marked`, and only while the schema still has `shifts.opened_by_count` (before M3), it carries an
/// OPEN count's id there, so it is **not** the branch's v2 shift and a second OPEN shift at the branch is allowed.
async fn seed_shift(pool: &PgPool, branch_id: &str, teller: &str, marked: bool) -> String {
    let id = format!("SH-{}", uuid::Uuid::new_v4());
    let has_marker: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'shifts' AND column_name = 'opened_by_count')",
    )
    .fetch_one(pool)
    .await
    .expect("look for the v2 marker column");
    let count = if marked && has_marker {
        let count = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
             SELECT $1, b.organization_id, b.id, 'OPEN', $3::uuid, now(), $4 FROM branches b WHERE b.id = $2",
        )
        .bind(count)
        .bind(branch_id)
        .bind(teller)
        .bind(format!("idem-{count}"))
        .execute(pool)
        .await
        .expect("seed the OPEN count that marks the shift");
        Some(count)
    } else {
        None
    };
    let insert = if count.is_some() {
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id, \
         opened_by_count) VALUES ($1, $2, 'OPEN', now() - interval '1 hour', 'test', '', $3, $4::uuid, $5)"
    } else {
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now() - interval '1 hour', 'test', '', $3, $4::uuid)"
    };
    let mut q = sqlx::query(insert)
        .bind(&id)
        .bind(branch_id)
        .bind(unique("open").replace('_', "-"))
        .bind(teller);
    if let Some(c) = count {
        q = q.bind(c);
    }
    q.execute(pool).await.expect("seed an OPEN shift");
    fund_ledger_drawer(pool, &id, OPENING).await;
    id
}

/// What shift `id` should hold now, `code -> amount`, zeroes left out.
async fn expected(pool: &PgPool, id: &str) -> BTreeMap<String, String> {
    use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
    use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
    PgLedgerShiftBalances::new(pool.clone())
        .shift_balances(&id.to_owned())
        .await
        .expect("read the shift's balances")
        .into_iter()
        .filter(|b| !b.expected.is_zero())
        .map(|b| (b.currency.as_str().to_owned(), b.expected.to_string()))
        .collect()
}

fn opening() -> BTreeMap<String, String> {
    [("THB", "100000.00"), ("USD", "1000.00")]
        .into_iter()
        .map(|(c, a)| (c.to_owned(), a.to_owned()))
        .collect()
}

/// The opening after the save of `form()`: a cash purchase of USD 100 at 32.1, THB 3210 paid out.
fn after_one_purchase() -> BTreeMap<String, String> {
    [("THB", "96790.00"), ("USD", "1100.00")]
        .into_iter()
        .map(|(c, a)| (c.to_owned(), a.to_owned()))
        .collect()
}

fn saver(id: &str, standing: StandingKind) -> SaveActor {
    SaveActor {
        id: id.to_owned(),
        username: id.to_owned(),
        employee_id: (standing != StandingKind::Owner).then(|| id.to_owned()),
        standing,
    }
}

struct Desk {
    shop: Shop,
    /// The other teller's shift: the branch's v2 shift before M3.
    others: String,
}

impl Desk {
    async fn open() -> Desk {
        let shop = Shop::start().await;
        shop.set_rate(MAIN, "USD", "32.1", "33").await;
        for id in ["E-SAVER", "E-OTHER", "E-NOSHIFT", "owner"] {
            let _ = user_of(&shop.pool, id).await;
        }
        let other = user_of(&shop.pool, "E-OTHER").await;
        let others = seed_shift(&shop.pool, MAIN, &other, false).await;
        Desk { shop, others }
    }

    /// `form()` saved through `save_as` by `who`; the passport is uploaded by `who` (an attachment is its owner's).
    async fn save(&self, who: &SaveActor) -> Result<String, AppError> {
        let customer = self.shop.reserve(MAIN, save_common::person()).await;
        let passport = self.shop.upload(MAIN, &who.id, "image/jpeg", JPEG).await;
        let cmd = form(
            &unique("ca10-save-submission").replace('_', "-"),
            &customer,
            &passport,
        );
        self.shop
            .workflow
            .save_as(&branch(MAIN), cmd, &SaveExtras::default(), who, at(0))
            .await
            .map(|saved| saved.document_ref)
    }

    async fn transactions(&self) -> i64 {
        self.shop.count("transactions").await
    }
}

/// Row 1: a staff saver (a user id) with an OPEN shift of their own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_staff_savers_own_open_shift_takes_the_stock() {
    let desk = Desk::open().await;
    let user = user_of(&desk.shop.pool, "E-SAVER").await;
    let mine = seed_shift(&desk.shop.pool, MAIN, &user, true).await;

    desk.save(&saver(&user, StandingKind::Staff(AccessRole::Teller)))
        .await
        .expect("the saver's own shift takes the save");

    assert_eq!(
        expected(&desk.shop.pool, &mine).await,
        after_one_purchase(),
        "the stock moves on the saver's own shift"
    );
    assert_eq!(
        expected(&desk.shop.pool, &desk.others).await,
        opening(),
        "another teller's shift is untouched"
    );
    desk.shop.finish().await;
}

/// Row 2: a saver who has no OPEN shift is refused with `NoOpenShift`, even though another teller's shift is open
/// at the branch, and nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saver_with_no_open_shift_is_refused_and_nothing_is_written() {
    let desk = Desk::open().await;
    let before = desk.transactions().await;

    let refused = desk
        .save(&saver("E-NOSHIFT", StandingKind::Staff(AccessRole::Teller)))
        .await;

    assert!(
        matches!(&refused, Err(AppError::Domain(s)) if s == NO_OPEN_SHIFT),
        "expected Domain({NO_OPEN_SHIFT:?}), got {refused:?}"
    );
    assert_eq!(desk.transactions().await, before, "no transaction row");
    assert_eq!(
        expected(&desk.shop.pool, &desk.others).await,
        opening(),
        "another teller's shift is untouched"
    );
    desk.shop.finish().await;
}

/// Row 3: the owner (`"owner"`, the organisation-wide EXECUTIVE + ADMIN user) saves on the owner's own shift.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_saves_on_the_owners_own_shift() {
    let desk = Desk::open().await;
    let owners = seed_shift(
        &desk.shop.pool,
        MAIN,
        &user_of(&desk.shop.pool, "owner").await,
        true,
    )
    .await;

    desk.save(&saver("owner", StandingKind::Owner))
        .await
        .expect("the owner's own shift takes the save");

    assert_eq!(
        expected(&desk.shop.pool, &owners).await,
        after_one_purchase()
    );
    assert_eq!(expected(&desk.shop.pool, &desk.others).await, opening());
    desk.shop.finish().await;
}

/// Row 4: a saver named by a `users.id` (a Blueprint session) saves on that user's own shift.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saver_named_by_users_id_saves_on_that_users_shift() {
    let desk = Desk::open().await;
    let user = user_of(&desk.shop.pool, "E-SAVER").await;
    let mine = seed_shift(&desk.shop.pool, MAIN, &user, true).await;

    desk.save(&saver(&user, StandingKind::Staff(AccessRole::Teller)))
        .await
        .expect("the user's own shift takes the save");

    assert_eq!(expected(&desk.shop.pool, &mine).await, after_one_purchase());
    assert_eq!(expected(&desk.shop.pool, &desk.others).await, opening());
    desk.shop.finish().await;
}

/// Row 5: a saver no user answers to is `NoOpenShift`, never a 500 (`apply_delta_for_teller_in` binds a uuid).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saver_no_user_answers_to_is_refused_with_no_open_shift() {
    let desk = Desk::open().await;
    let before = desk.transactions().await;

    let refused = desk
        .save(&saver("E-GHOST", StandingKind::Staff(AccessRole::Teller)))
        .await;

    assert!(
        matches!(&refused, Err(AppError::Domain(s)) if s == NO_OPEN_SHIFT),
        "expected Domain({NO_OPEN_SHIFT:?}), got {refused:?}"
    );
    assert_eq!(desk.transactions().await, before, "no transaction row");
    assert_eq!(expected(&desk.shop.pool, &desk.others).await, opening());
    desk.shop.finish().await;
}
