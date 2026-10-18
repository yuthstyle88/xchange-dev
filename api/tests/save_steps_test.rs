//! Specification tests for T8c-1: the steps other contexts add around a save,
//! and custody's step.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `SaveTransactionWorkflow::save_as` is the save as a signed-in person makes
//!   it: the form's `staff` becomes the saver's username, and every registered
//!   `SaveStep` runs before and after the save **inside its transaction**.
//! - `CustodySaveStep` is the first such step: it asks `CustodyService` whether
//!   a live exchange may be saved now by this person, and completes the draft
//!   the exchange was begun under.
//! - `save` (ticket T8b) stays the save with nothing around it.
//!
//! The custody rule itself is `xc-shift`'s and has its own tests; here it is
//! reached through a save, with the state put straight into the custody tables.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

// the shop of the save tests: its helpers prepare a branch and read it back
#[allow(unused_imports)]
mod save_common;
mod shift_holder_common;

use std::sync::{Arc, Mutex};

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xchange::application::workflows::save_step::{
    SaveActor, SaveContext, SaveExtras, SaveStep, StepFuture,
};

fn owner() -> SaveActor {
    SaveActor {
        id: OWNER.to_owned(),
        username: "boss".to_owned(),
        employee_id: None,
        standing: StandingKind::Owner,
    }
}

/// A staff member: the id is the employee's id, the username something else.
fn staff(id: &str, standing: StandingKind) -> SaveActor {
    SaveActor {
        id: id.to_owned(),
        username: format!("user-{id}"),
        employee_id: Some(id.to_owned()),
        standing,
    }
}

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(Transactor::new(shop.pool.clone()))
}

fn workflow(shop: &Shop, steps: Vec<Arc<dyn SaveStep>>) -> SaveTransactionWorkflow {
    SaveTransactionWorkflow::new(branch_tx(shop), shop.shifts.clone(), shop.files.clone())
        .with_steps(steps)
}

// ------------------------------------------------------------ a step that tells

/// What a step saw, in order: `before <name> …` / `after <name> …`.
type Seen = Arc<Mutex<Vec<String>>>;

/// A step that writes down each call, leaves a row behind through the
/// transaction it is handed, and can be told to refuse or to fail.
struct Telling {
    name: &'static str,
    seen: Seen,
    refuse_before: Option<&'static str>,
    fail_after: bool,
}

impl Telling {
    fn new(name: &'static str, seen: &Seen) -> Telling {
        Telling {
            name,
            seen: seen.clone(),
            refuse_before: None,
            fail_after: false,
        }
    }

    /// A row only this step writes, in a table nothing in a save touches.
    async fn mark(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &SaveContext<'_>,
        when: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(ctx.branch.as_str())
        .bind(format!("{when}-{}-{}", self.name, ctx.form.submission_id))
        .bind(ctx.now)
        .bind(&ctx.actor.id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}

impl SaveStep for Telling {
    fn before_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(format!(
                "before {} staff={} actor={} draft={:?} counted={} kiosk={:?} prior={:?} amount={} at={}",
                self.name,
                ctx.form.staff,
                ctx.actor.id,
                ctx.extras.staff_draft_id,
                ctx.extras.count_confirmed,
                ctx.extras.kiosk_request_id,
                ctx.prior.map(|t| t.document_ref.clone()),
                ctx.form.amount,
                ctx.now.timestamp_subsec_micros(),
            ));
            self.mark(tx, ctx, "before").await?;
            match self.refuse_before {
                Some(sentence) => Err(AppError::Domain(sentence.to_owned())),
                None => Ok(()),
            }
        })
    }

    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(format!(
                "after {} saved={} staff={} prior={:?}",
                self.name,
                saved.document_ref,
                saved.staff,
                ctx.prior.map(|t| t.document_ref.clone()),
            ));
            self.mark(tx, ctx, "after").await?;
            if self.fail_after {
                return Err(AppError::Internal("the step failed".to_owned()));
            }
            Ok(())
        })
    }
}

fn told(seen: &Seen) -> Vec<String> {
    std::mem::take(&mut *seen.lock().unwrap())
}

async fn marks(shop: &Shop) -> Vec<String> {
    sqlx::query_scalar("SELECT request_id FROM abandoned_requests ORDER BY request_id")
        .fetch_all(&shop.pool)
        .await
        .expect("read the steps' rows")
}

// ------------------------------------------------------------------ save_as

#[tokio::test]
async fn the_forms_staff_is_the_savers_username_whatever_was_sent() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let flow = workflow(&shop, vec![]);

    // sent with another name
    let mut cmd = form("staff-0001-aaaaaaaaaa", &customer, &passport);
    cmd.staff = "somebody else".to_owned();
    let saved = flow
        .save_as(&branch(MAIN), cmd, &SaveExtras::default(), &owner(), at(0))
        .await
        .expect("a save");
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.staff, "boss", "the saver's username");
    assert_eq!(stored.recorded_by, OWNER, "the saver's id");
    let receipt = shop
        .files
        .open_owned(saved.receipt_url.strip_prefix("/files/").unwrap(), OWNER)
        .await
        .unwrap()
        .expect("the receipt belongs to the saver's id");
    let html = String::from_utf8(receipt.bytes).unwrap();
    assert!(
        html.contains(">boss<") && !html.contains("somebody else"),
        "{html}"
    );

    // sent with no name at all: the form is not refused for it
    let mut cmd = form("staff-0002-aaaaaaaaaa", &customer, &passport);
    cmd.staff = String::new();
    cmd.amount = "1".to_owned();
    let saved = flow
        .save_as(&branch(MAIN), cmd, &SaveExtras::default(), &owner(), at(1))
        .await
        .expect("an empty staff field is filled in");
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.staff, "boss");

    // the same submission again with another name sent is the same exchange
    let mut again = form("staff-0002-aaaaaaaaaa", &customer, &passport);
    again.staff = "a third name".to_owned();
    again.amount = "1".to_owned();
    let answered = flow
        .save_as(
            &branch(MAIN),
            again,
            &SaveExtras::default(),
            &owner(),
            at(2),
        )
        .await
        .expect("the staff sent is not part of what makes it the same");
    assert_eq!(answered, saved);

    // `save` leaves the field as it was sent
    let mut cmd = form("staff-0003-aaaaaaaaaa", &customer, &passport);
    cmd.staff = "EMP-009".to_owned();
    cmd.amount = "2".to_owned();
    let saved = flow.save(&branch(MAIN), cmd, OWNER, at(3)).await.unwrap();
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.staff, "EMP-009");
    shop.finish().await;
}

#[tokio::test]
async fn the_steps_run_in_order_around_the_save_and_are_told_about_it() {
    let shop = Shop::start().await;
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let me = staff("employee-7", StandingKind::Staff(AccessRole::Teller));
    let shift = shift_holder_common::open_shift_for(
        &shop.pool,
        MAIN,
        &me.id,
        &[("THB", "100000"), ("USD", "1000")],
    )
    .await;
    fund_ledger_drawer(&shop.pool, &shift, &[("THB", "100000"), ("USD", "1000")]).await;
    let passport = shop.upload(MAIN, &me.id, "image/jpeg", JPEG).await;
    let seen: Seen = Seen::default();
    let flow = workflow(
        &shop,
        vec![
            Arc::new(Telling::new("A", &seen)),
            Arc::new(Telling::new("B", &seen)),
        ],
    );
    // A3 (kiosk) (TX-6 #674, lane controller's ruling R2, design quote-transaction.md §4.6.1): the request the
    // save names exists and carries the kiosk's own quote, which the save completes; a request with no quote is
    // refused, and one nobody made asks for none. What the steps are told is unchanged.
    checked_kiosk_request_with_quote(
        &shop.pool,
        MAIN,
        "kiosk-9",
        &me.id,
        Side::Buy,
        "USD",
        "100",
        "32.1",
        "3210.00",
        at(-1),
    )
    .await;
    let extras = SaveExtras {
        staff_draft_id: Some("draft-1".to_owned()),
        count_confirmed: true,
        kiosk_request_id: Some("kiosk-9".to_owned()),
    };

    let saved = flow
        .save_as(
            &branch(MAIN),
            form("order-0001-aaaaaaaaaa", &customer, &passport),
            &extras,
            &me,
            at(0),
        )
        .await
        .expect("a save");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    let before = |name: &str, prior: &str| {
        format!(
            "before {name} staff=user-employee-7 actor=employee-7 draft=Some(\"draft-1\") counted=true \
             kiosk=Some(\"kiosk-9\") prior={prior} amount=100.00 at=123000"
        )
    };
    assert_eq!(
        told(&seen),
        vec![
            before("A", "None"),
            before("B", "None"),
            "after A saved=XC-20261006-000001 staff=user-employee-7 prior=None".to_owned(),
            "after B saved=XC-20261006-000001 staff=user-employee-7 prior=None".to_owned(),
        ],
        "before in order, the save, after in order; the time is the save's, cut to milliseconds"
    );
    // what the steps wrote through the transaction is there with the exchange
    assert_eq!(
        marks(&shop).await,
        vec![
            "after-A-order-0001-aaaaaaaaaa",
            "after-B-order-0001-aaaaaaaaaa",
            "before-A-order-0001-aaaaaaaaaa",
            "before-B-order-0001-aaaaaaaaaa",
        ]
    );

    // the same submission again: both are told it is a repeat, and `after`
    // runs again with the stored exchange
    let again = flow
        .save_as(
            &branch(MAIN),
            form("order-0001-aaaaaaaaaa", &customer, &passport),
            &extras,
            &me,
            at(5),
        )
        .await
        .expect("the same exchange again");
    assert_eq!(again, saved);
    let prior = "Some(\"XC-20261006-000001\")";
    assert_eq!(
        told(&seen),
        vec![
            before("A", prior),
            before("B", prior),
            format!("after A saved=XC-20261006-000001 staff=user-employee-7 prior={prior}"),
            format!("after B saved=XC-20261006-000001 staff=user-employee-7 prior={prior}"),
        ]
    );
    assert_eq!(shop.count("transactions").await, 1);

    // again with something changed: `before` runs, the save is refused, no `after`
    let mut changed = form("order-0001-aaaaaaaaaa", &customer, &passport);
    changed.amount = "5".to_owned();
    assert_eq!(
        refusal(
            flow.save_as(&branch(MAIN), changed, &extras, &me, at(6))
                .await
        ),
        CHANGED
    );
    let calls = told(&seen);
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls.iter().all(|c| c.starts_with("before ")), "{calls:?}");

    // `save` runs no step at all
    let mut plain = form("order-0002-aaaaaaaaaa", &customer, &passport);
    plain.amount = "1".to_owned();
    flow.save(&branch(MAIN), plain, &me.id, at(7))
        .await
        .expect("the save with nothing around it");
    assert_eq!(told(&seen), Vec::<String>::new());
    shop.finish().await;
}

#[tokio::test]
async fn a_step_that_refuses_ends_the_save_and_nothing_is_kept() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let seen: Seen = Seen::default();
    let mut second = Telling::new("B", &seen);
    second.refuse_before = Some("ขั้นที่สองไม่ให้ผ่าน");
    let flow = workflow(
        &shop,
        vec![
            Arc::new(Telling::new("A", &seen)),
            Arc::new(second),
            Arc::new(Telling::new("C", &seen)),
        ],
    );
    let before = shop.everything().await;

    let got = flow
        .save_as(
            &branch(MAIN),
            form("refuse-0001-aaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), "ขั้นที่สองไม่ให้ผ่าน", "the step's own sentence");
    let calls = told(&seen);
    assert_eq!(
        calls.len(),
        2,
        "A and B before; C never, and no after: {calls:?}"
    );
    assert!(
        calls[0].starts_with("before A ") && calls[1].starts_with("before B "),
        "{calls:?}"
    );
    assert_eq!(shop.everything().await, before);
    assert_eq!(
        marks(&shop).await,
        Vec::<String>::new(),
        "what A wrote before B refused is undone"
    );

    // a step is asked after the files, the branch and the form are judged …
    let mut bad_form = form("refuse-0002-aaaaaaaaa", &customer, &passport);
    bad_form.amount = "0".to_owned();
    let got = flow
        .save_as(
            &branch(MAIN),
            bad_form,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_ne!(refusal(got), "ขั้นที่สองไม่ให้ผ่าน");
    let got = flow
        .save_as(
            &branch("XC-B99"),
            form("refuse-0003-aaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), ATTACHMENT_NOT_FOUND);
    assert_eq!(
        told(&seen),
        Vec::<String>::new(),
        "no step was asked about a save that was not well-formed"
    );
    // … after the quote's rate check (re-pinned for TX-6 by the test-writer, decision A3, design
    // quote-transaction.md §4.6.1, #674; lane controller's ruling, Fact 3 option (i)) …
    let mut wrong_rate = form("refuse-0004-aaaaaaaaa", &customer, &passport);
    wrong_rate.rate = "1".to_owned();
    let got = flow
        .save_as(
            &branch(MAIN),
            wrong_rate,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(
        refusal(got),
        RATE_MISMATCH,
        "decision A3: the quote comes first"
    );
    assert_eq!(
        told(&seen),
        Vec::<String>::new(),
        "no step was asked about a save at a rate the shop did not publish"
    );
    // … and before the customer, the shift and the stock
    let mut wrong = form("refuse-0005-aaaaaaaaa", "CX-99999999", &passport);
    wrong.amount = "99999999".to_owned();
    let got = flow
        .save_as(
            &branch(MAIN),
            wrong,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), "ขั้นที่สองไม่ให้ผ่าน");
    shop.finish().await;
}

#[tokio::test]
async fn a_step_that_fails_after_the_save_undoes_the_save() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let seen: Seen = Seen::default();
    let mut second = Telling::new("B", &seen);
    second.fail_after = true;
    let failing = workflow(
        &shop,
        vec![Arc::new(Telling::new("A", &seen)), Arc::new(second)],
    );
    let before = shop.everything().await;

    let got = failing
        .save_as(
            &branch(MAIN),
            form("undo-0001-aaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert!(
        matches!(&got, Err(e) if !matches!(e, AppError::Domain(_))),
        "a failure, not a refusal: {got:?}"
    );
    assert_eq!(told(&seen).len(), 4, "both before, the save, both after");
    assert_eq!(
        shop.everything().await,
        before,
        "the exchange, its number, the stock and the customer are undone"
    );
    assert_eq!(
        marks(&shop).await,
        Vec::<String>::new(),
        "and so is every row a step wrote"
    );

    // a save refused after the steps were asked undoes what they wrote, and runs no `after`
    // (re-pinned for TX-6 by the test-writer, decision A3, design quote-transaction.md §4.6.1, #674;
    // lane controller's ruling, Fact 3 option (i): a wrong rate is now refused at the quote, before
    // any step, so the refusal after the steps is the stock's)
    let ok = workflow(&shop, vec![Arc::new(Telling::new("A", &seen))]);
    let mut wrong_rate = form("undo-0002-aaaaaaaaaaa", &customer, &passport);
    wrong_rate.rate = "32.2".to_owned();
    let got = ok
        .save_as(
            &branch(MAIN),
            wrong_rate,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(
        refusal(got),
        RATE_MISMATCH,
        "decision A3: the quote comes first"
    );
    assert_eq!(told(&seen), Vec::<String>::new(), "no step was asked");
    let mut too_much = form("undo-0003-aaaaaaaaaaa", &customer, &passport);
    too_much.amount = "99999999".to_owned();
    let got = ok
        .save_as(
            &branch(MAIN),
            too_much,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), NOT_ENOUGH_THB);
    let calls = told(&seen);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].starts_with("before A "));
    assert_eq!(marks(&shop).await, Vec::<String>::new());
    assert_eq!(shop.everything().await, before);

    // and with nothing failing the same submission is saved as number 1
    let saved = ok
        .save_as(
            &branch(MAIN),
            form("undo-0001-aaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(1),
        )
        .await
        .expect("a save");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    shop.finish().await;
}

// -------------------------------------------------------------- custody's step
//
// Retired by CA-10 #626 (cash-shift-transfer.md §6, ruling Q4; ticket author claude/c4d72c): the five tests that
// drove `CustodySaveStep` through `save_as` with a v2 actor (custody off, the drawer's custodian, a hold and the
// hours of service, the cut-off draft, a draft failing to complete) judged the v2-actor save path, which goes.
// Their surviving rules are restated on the teller's path in `branch_day_save_test.rs`: a customer on hold, the
// hours of service, the cut-off draft and a draft failing to complete; the custodian and the Enable switch go.
