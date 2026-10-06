//! Written before the implementation (ticket T12b). Do not edit, delete or weaken.
//!
//! `KioskService`: the kiosk's rules (`KioskMachine`, T12a) put on a database.
//!
//! The first group replays `fixtures/v2_kiosk_service_cases.json`: 11 stories that
//! were played against the kiosk module of the v2 server. After every step the
//! answer, the audit lines, the stored requests and the stored currency switches
//! must be what v2 had — or what the step's `v3` entry says, in the three places
//! where v3 differs on purpose (the sweep is per branch; a save cannot complete a
//! request whose passport was never checked).
//!
//! The second group is written by hand: what v2 does not have (the branch must
//! exist, the per-address limit), and what a single-threaded replay cannot see
//! (everything a write does happens in one transaction, under the branch lock).
//!
//! These tests need PostgreSQL: see `kiosk_service_common/mod.rs`.

mod kiosk_service_common;

use std::sync::Arc;

use kiosk_service_common::*;
use pos_error::AppError;
use serde_json::{Value, json};
use xc_kernel::money::Side;

async fn replay(name: &str) {
    let wrong = play(name).await;
    assert!(
        wrong.is_empty(),
        "{} difference(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// ── the stories recorded from v2 ────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn v2_a_request_goes_from_quote_to_completed() {
    replay("a request goes from quote to completed").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_a_request_is_only_found_in_its_own_branch() {
    replay("a request is only found in its own branch").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_an_old_quote_is_deleted_when_the_next_quote_arrives() {
    replay("an old quote is deleted when the next quote arrives").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_the_sweep_stays_in_its_branch() {
    replay("the sweep and the other branch").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_the_queue_lists_what_was_confirmed_newest_first() {
    replay("the queue lists what was confirmed, newest first").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_a_published_receipt_is_shown_for_just_under_ten_minutes() {
    replay("a published receipt is shown for just under ten minutes").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_what_counts_as_a_receipt_image() {
    replay("what counts as a receipt image").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_the_kiosk_offers_six_currencies_until_the_branch_says_otherwise() {
    replay("the kiosk offers six currencies until the branch says otherwise").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_the_quote_and_the_confirm_look_at_the_counter_as_it_is_now() {
    replay("the quote and the confirm look at the counter as it is now").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_a_transaction_is_saved_for_the_request_its_employee_holds() {
    replay("a transaction is saved for the request its employee holds").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_the_save_must_match_the_queue() {
    replay("the save must match the queue").await;
}

/// A story added to the fixture without a test above would never be played.
#[test]
fn every_story_of_the_fixture_is_played_above() {
    let names: Vec<&str> = fixture()["scenarios"]
        .as_array()
        .expect("scenarios")
        .iter()
        .map(|s| s["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        [
            "a request goes from quote to completed",
            "a request is only found in its own branch",
            "an old quote is deleted when the next quote arrives",
            "the sweep and the other branch",
            "the queue lists what was confirmed, newest first",
            "a published receipt is shown for just under ten minutes",
            "what counts as a receipt image",
            "the kiosk offers six currencies until the branch says otherwise",
            "the quote and the confirm look at the counter as it is now",
            "a transaction is saved for the request its employee holds",
            "the save must match the queue",
        ]
    );
    assert_eq!(fixture()["counts"]["steps"], 236);
    assert_eq!(fixture()["t0"], T0);
}

// ── the figures v2 was measured to use ──────────────────────────────────────

#[test]
fn the_limits_are_the_ones_measured_on_v2() {
    let limits = &fixture()["limits"];
    assert_eq!(limits["listLimit"], QUEUE_LIMIT);
    assert_eq!(limits["receiptImageMaxLength"], ReceiptImage::MAX_LENGTH);
    let mut defaults: Vec<&str> = DEFAULT_CODES.to_vec();
    defaults.sort_unstable();
    assert_eq!(limits["defaults"], json!(defaults));
    assert_eq!(DEFAULT_CODES.len(), 6);
}

#[test]
fn a_receipt_image_is_a_png_data_url_of_at_most_1_800_000_characters() {
    let of = |n: usize| image_of(&json!({ "png": n }));
    assert!(ReceiptImage::parse(&of(1_800_000)).is_some());
    assert!(ReceiptImage::parse(&of(1_800_001)).is_none());
    assert_eq!(ReceiptImage::parse(PNG).expect("a PNG").as_str(), PNG);
    for bad in [
        "",
        "data:image/png;base64,",
        "data:image/jpeg;base64,AAAA",
        "data:image/png;base64,AAAA\n",
        "data:image/png;base64,AA AA",
        "data:image/png;base64,AAAA-",
        "data:image/png;base64,AAAA_",
        "data:image/png;base64,ก",
        "x",
    ] {
        assert!(ReceiptImage::parse(bad).is_none(), "accepted {bad:?}");
    }
}

#[test]
fn the_sentences_of_the_service_are_these() {
    assert_eq!(KioskServiceError::NoSuchBranch.to_string(), NO_SUCH_BRANCH);
    assert_eq!(fixture()["noSuchBranch"], NO_SUCH_BRANCH);
    assert_eq!(
        KioskServiceError::InvalidReceiptImage.to_string(),
        "Invalid receipt image."
    );
    assert_eq!(
        KioskServiceError::InvalidCurrencySettings.to_string(),
        "Invalid currency settings."
    );
    assert_eq!(
        KioskServiceError::RatesFirst.to_string(),
        "Set valid counter buy and sell rates first."
    );
    assert_eq!(
        KioskServiceError::TooManyQuotes.to_string(),
        TOO_MANY_QUOTES
    );
    assert_eq!(CUSTOMER_KIOSK, "CUSTOMER-KIOSK");
    assert_eq!(
        KioskAudit::CurrencyConfigured {
            enabled: true,
            buy: "1".into(),
            sell: "1".into()
        }
        .kind(),
        "KIOSK_CURRENCY_CONFIGURED"
    );
    assert_eq!(KioskAudit::Accepted.kind(), "KIOSK_ACCEPTED");
    assert_eq!(KioskAudit::IdChecked.kind(), "KIOSK_ID_CHECKED");
    assert_eq!(
        KioskAudit::Completed {
            document_ref: "x".into()
        }
        .kind(),
        "KIOSK_COMPLETED"
    );
}

/// 103 confirmed requests: the queue shows the newest 100, the bell counts all.
#[tokio::test(flavor = "multi_thread")]
async fn the_queue_shows_the_newest_hundred_and_the_bell_counts_them_all() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    let id = |n: i64| format!("{n:08x}-0000-4000-8000-000000000000");
    for n in 1..=103 {
        // each from an address of its own: this test is not about the per-address limit
        let quote = from_ip(buy100(&id(n)), &format!("198.51.100.{n}"));
        service
            .quote(&main, quote, at(n * 10))
            .await
            .expect("quote");
    }
    for n in 1..=103 {
        service
            .confirm(&main, &id(n), at(5000 + n))
            .await
            .expect("confirm");
    }
    let listed = service
        .list(&main, &front1(), at(9000))
        .await
        .expect("list");
    let limits = &fixture()["limits"];
    assert_eq!(json!(listed.len()), limits["listLimit"]);
    assert_eq!(listed.len(), 100);
    assert_eq!(listed[0].request.id, id(103), "newest first");
    assert_eq!(
        json!(listed[99].request.id),
        json!(id(limits["listedOldestOf103"].as_i64().expect("n"))),
        "the three oldest are cut"
    );
    let bell = service.notifications(&main, &front1()).await.expect("bell");
    assert_eq!(json!(bell.count), limits["waitingCountOf103"]);
    assert_eq!(bell.latest, Some(id(103)));
    db.finish().await;
}

// ── the branch ──────────────────────────────────────────────────────────────

/// v3's rule, which v2 does not have: a call on a branch that does not exist is
/// refused with the shop's sentence for it, whatever the action — after the
/// question of who may call it, and before anything is read or written.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_does_not_exist_is_refused_whatever_is_asked() {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    bring(&service, MAIN, "q1", "checked").await;
    let before = (db.rows().await, db.count("branch_versions").await);
    let nope = branch("NOPE");
    let (f1, o) = (front1(), owner());

    assert_eq!(refusal(service.public_rates(&nope).await), NO_SUCH_BRANCH);
    assert_eq!(
        refusal(service.quote(&nope, buy100("q9"), at(7000)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(service.confirm(&nope, "q1", at(7000)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(service.status(&nope, "q1", at(7000)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(service.customer_receipt(&nope, "q1", at(7000)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(service.notifications(&nope, &f1).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(service.list(&nope, &f1, at(7000)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(service.accept(&nope, &f1, "q1", at(7000)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(
            service
                .identity_checked(&nope, &f1, "q1", true, at(7000))
                .await
        ),
        NO_SUCH_BRANCH
    );
    assert_eq!(refusal(service.get(&nope, &f1, "q1").await), NO_SUCH_BRANCH);
    assert_eq!(
        refusal(
            service
                .publish_receipt(&nope, &f1, "q1", PNG, at(7000))
                .await
        ),
        NO_SUCH_BRANCH
    );
    assert_eq!(refusal(service.manage(&nope, &o).await), NO_SUCH_BRANCH);
    assert_eq!(
        refusal(
            service
                .configure(
                    &nope,
                    &o,
                    configure_of("CHF", true, "36.4", "37.1"),
                    at(7000)
                )
                .await
        ),
        NO_SUCH_BRANCH
    );
    assert!(
        ports.rate_saves().is_empty(),
        "no rate is saved for a branch that does not exist"
    );

    // who may call is answered first, as v2 answers it for any branch name
    let customer = KioskActor::Customer;
    assert_eq!(
        refusal(service.list(&nope, &customer, at(7000)).await),
        "Staff access required."
    );
    assert_eq!(
        refusal(service.notifications(&nope, &customer).await),
        "Staff access required."
    );
    assert_eq!(
        refusal(service.accept(&nope, &rates1(), "q1", at(7000)).await),
        "Staff access required."
    );
    assert_eq!(
        refusal(service.manage(&nope, &f1).await),
        "Owner or Rates access required."
    );
    assert_eq!(
        refusal(
            service
                .configure(&nope, &f1, configure_of("XXX", true, "0", "0"), at(7000))
                .await
        ),
        "Owner or Rates access required."
    );

    assert_eq!(
        (db.rows().await, db.count("branch_versions").await),
        before,
        "nothing was written, not even the lock row of the branch that does not exist"
    );
    db.finish().await;
}

// ── the per-address limit ───────────────────────────────────────────────────

/// v3's rule (design §6; v2 has none): one address may make 20 quotes in one
/// branch within 60,000 ms. Counted are the requests stored, so a refused quote
/// does not count.
#[tokio::test(flavor = "multi_thread")]
async fn the_twenty_first_quote_from_one_address_within_a_minute_is_refused() {
    assert_eq!((QuoteLimit::MAX, QuoteLimit::WINDOW_MS), (20, 60_000));
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let (main, b2) = (branch(MAIN), branch(B2));
    let a = |id: &str| from_ip(buy100(id), "192.0.2.1");

    // refused quotes do not count
    for n in 0..30 {
        let bad = from_ip(
            quote_of(&format!("bad{n}"), Side::Buy, "USD", "0"),
            "192.0.2.1",
        );
        assert_eq!(
            refusal(service.quote(&main, bad, at(0)).await),
            "Please enter a valid amount and currency."
        );
    }
    service.quote(&main, a("a0"), at(0)).await.expect("1st");
    for n in 1..20 {
        service
            .quote(&main, a(&format!("a{n}")), at(1000 + n))
            .await
            .unwrap_or_else(|e| panic!("quote {} of 20: {e:?}", n + 1));
    }
    assert_eq!(
        refusal(service.quote(&main, a("a20"), at(59_999)).await),
        TOO_MANY_QUOTES
    );
    assert_eq!(
        db.count("kiosk_requests").await,
        20,
        "the refused quote is not stored"
    );

    // another address, and the same address in another branch, are not held back
    service
        .quote(&main, from_ip(buy100("b0"), "192.0.2.2"), at(59_999))
        .await
        .expect("another address");
    service
        .quote(&b2, a("a20"), at(59_999))
        .await
        .expect("the same address in another branch");
    // confirming does not free a place: the request is still one made from that address
    service
        .confirm(&main, "a0", at(4000))
        .await
        .expect("confirm");
    assert_eq!(
        refusal(service.quote(&main, a("a20"), at(59_999)).await),
        TOO_MANY_QUOTES
    );

    // 60,000 ms after the first quote it no longer counts
    service
        .quote(&main, a("a20"), at(60_000))
        .await
        .expect("the first quote is a minute old");
    assert_eq!(
        refusal(service.quote(&main, a("a21"), at(60_000)).await),
        TOO_MANY_QUOTES
    );
    db.finish().await;
}

// ── keys ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn an_id_is_used_once_per_branch() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let (main, b2) = (branch(MAIN), branch(B2));
    let f1 = front1();
    bring(&service, MAIN, "same", "checked").await;

    let again = service.quote(&main, buy100("same"), at(7000)).await;
    assert!(
        matches!(again, Err(ref e) if !matches!(e, AppError::Domain(_))),
        "a second request with the id of a stored one is an error, not a refusal: {again:?}"
    );
    assert_eq!(
        db.status(MAIN, "same").await.as_deref(),
        Some("ID_CHECKED"),
        "and the stored one is not overwritten"
    );

    // the key is (branch, id): the other branch may hold the same id, and each row is its own
    service
        .quote(&b2, buy100("same"), at(7000))
        .await
        .expect("the other branch");
    service
        .publish_receipt(&main, &f1, "same", PNG, at(8000))
        .await
        .expect("publish in MAIN");
    let other = service
        .customer_receipt(&b2, "same", at(8001))
        .await
        .expect("the other branch's request");
    assert_eq!(
        other.image, None,
        "a receipt belongs to its branch's request"
    );
    assert_eq!(other.identity.status, KioskStatus::Quoted);
    let mine = service
        .customer_receipt(&main, "same", at(8001))
        .await
        .expect("receipt");
    assert_eq!(mine.image.as_deref(), Some(PNG));

    service
        .confirm(&b2, "same", at(11_000))
        .await
        .expect("confirm in the other branch");
    assert_eq!(db.status(B2, "same").await.as_deref(), Some("WAITING"));
    assert_eq!(db.status(MAIN, "same").await.as_deref(), Some("ID_CHECKED"));

    // the sweep deletes this branch's old quote and not the other branch's request of the same id
    service
        .quote(&main, buy100("twin"), at(20_000))
        .await
        .expect("quote");
    service
        .quote(&b2, buy100("twin"), at(20_000))
        .await
        .expect("quote");
    service
        .confirm(&b2, "twin", at(24_000))
        .await
        .expect("confirm");
    service
        .quote(&main, buy100("next"), at(700_000))
        .await
        .expect("the quote that sweeps");
    assert_eq!(db.status(MAIN, "twin").await, None);
    assert_eq!(db.status(B2, "twin").await.as_deref(), Some("WAITING"));
    db.finish().await;
}

/// A status that is only ever shown must not be storable.
#[tokio::test(flavor = "multi_thread")]
async fn expired_is_never_a_stored_status() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    bring(&service, MAIN, "q1", "accepted").await;
    let set = |status: &'static str| {
        sqlx::query("UPDATE kiosk_requests SET status = $1 WHERE id = 'q1'")
            .bind(status)
            .execute(&db.pool)
    };
    // the row can be given any status the kiosk stores …
    set("ID_CHECKED").await.expect("a stored status");
    // … and not the one it only shows
    assert!(
        set("EXPIRED").await.is_err(),
        "the table accepted the status EXPIRED"
    );
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("ID_CHECKED"));
    db.finish().await;
}

// ── one transaction, under the branch lock ──────────────────────────────────

/// With a single connection, a statement that does not run on the transaction of
/// its command cannot get a connection: the command fails instead of finishing.
#[tokio::test(flavor = "multi_thread")]
async fn inside_a_write_every_statement_runs_on_its_transaction() {
    let db = TestDb::start_with(1).await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    let main = branch(MAIN);
    let f1 = front1();
    within(MUST_FINISH, "the whole story on one connection", async {
        service
            .quote(&main, buy100("q0"), at(0))
            .await
            .expect("quote");
        // the next quote sweeps the first one: a delete and an insert in one transaction
        service
            .quote(&main, buy100("q1"), at(700_000))
            .await
            .expect("quote with a sweep");
        service
            .confirm(&main, "q1", at(704_000))
            .await
            .expect("confirm");
        service
            .accept(&main, &f1, "q1", at(705_000))
            .await
            .expect("accept");
        service
            .identity_checked(&main, &f1, "q1", true, at(706_000))
            .await
            .expect("identity checked");
        service
            .publish_receipt(&main, &f1, "q1", PNG, at(707_000))
            .await
            .expect("publish");
        service
            .configure(
                &main,
                &owner(),
                configure_of("CHF", true, "36.4", "37.1"),
                at(708_000),
            )
            .await
            .expect("configure");
        // the save's transaction is the only connection there is
        let mut tx = db.begin().await;
        service
            .check_save_in(&mut tx, &main, "q1", &save100("SUB-1"), &f1)
            .await
            .expect("check_save_in");
        service
            .complete_in(
                &mut tx,
                &main,
                "q1",
                &link("SUB-1", "XC-20261006-000001"),
                &f1,
                at(709_000),
            )
            .await
            .expect("complete_in");
        tx.commit().await.expect("commit");
    })
    .await;
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("COMPLETED"));
    assert_eq!(db.status(MAIN, "q0").await, None);
    assert_eq!(db.count("test_audit").await, 5);
    assert_eq!(
        ports.counter_reads(),
        (0, 3),
        "two quotes and a confirm read the counter, each inside its own transaction"
    );
    service.public_rates(&main).await.expect("rates");
    assert_eq!(
        ports.counter_reads(),
        (1, 3),
        "a read asks for committed data"
    );
    db.finish().await;
}

/// The audit line is the last thing a command writes. When it cannot be written,
/// what the command wrote before it must be gone as well.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_whose_audit_line_fails_stores_nothing() {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    let main = branch(MAIN);
    let f1 = front1();
    bring(&service, MAIN, "q1", "quoted").await;

    ports.audit_fails(true);
    assert!(service.confirm(&main, "q1", at(4000)).await.is_err());
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("QUOTED"));
    ports.audit_fails(false);
    service
        .confirm(&main, "q1", at(4000))
        .await
        .expect("confirm");

    ports.audit_fails(true);
    assert!(service.accept(&main, &f1, "q1", at(5000)).await.is_err());
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("WAITING"));
    assert_eq!(db.rows().await[0]["staff"], Value::Null);
    ports.audit_fails(false);
    service
        .accept(&main, &f1, "q1", at(5000))
        .await
        .expect("accept");

    ports.audit_fails(true);
    assert!(
        service
            .identity_checked(&main, &f1, "q1", true, at(6000))
            .await
            .is_err()
    );
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("ACCEPTED"));
    assert!(
        service
            .configure(
                &main,
                &owner(),
                configure_of("CHF", true, "36.4", "37.1"),
                at(6000)
            )
            .await
            .is_err()
    );
    assert_eq!(
        db.count("kiosk_currencies").await,
        0,
        "the switch is not stored"
    );
    assert_eq!(
        ports.rate_saves().len(),
        1,
        "the rate was saved before, in the rate context's own transaction; that is not undone"
    );
    ports.audit_fails(false);

    assert_eq!(
        db.count("test_audit").await,
        2,
        "confirm and accept, once each"
    );
    db.finish().await;
}

/// A refused quote leaves the old quotes it would have swept.
#[tokio::test(flavor = "multi_thread")]
async fn a_quote_refused_for_the_limit_sweeps_nothing() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    for n in 0..20 {
        let quote = from_ip(buy100(&format!("a{n}")), "192.0.2.1");
        service
            .quote(&main, quote, at(700_000 + n))
            .await
            .expect("quote");
    }
    // an old quote that arrives in the table behind the service's back: purgeable at once
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, \
         foreign_amount, thb, input_amount, payment_method) \
         VALUES ('MAIN', 'old', 'QUOTED', $1, 'BUY', 'USD', 32.1, 100, 3210, 100, 'Cash')",
    )
    .bind(exact(0))
    .execute(&db.pool)
    .await
    .expect("insert an old quote");

    let refused = from_ip(buy100("a20"), "192.0.2.1");
    assert_eq!(
        refusal(service.quote(&main, refused, at(700_100)).await),
        TOO_MANY_QUOTES
    );
    assert_eq!(
        db.status(MAIN, "old").await.as_deref(),
        Some("QUOTED"),
        "the sweep of a refused quote is undone with it"
    );
    // a quote that is made does sweep it
    service
        .quote(&main, from_ip(buy100("b0"), "192.0.2.2"), at(700_100))
        .await
        .expect("another address");
    assert_eq!(db.status(MAIN, "old").await, None);
    db.finish().await;
}

/// The save hook writes on the transaction it is handed and nowhere else: when
/// the save rolls back, the request is as it was.
#[tokio::test(flavor = "multi_thread")]
async fn the_save_hook_lives_and_dies_with_the_saves_transaction() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    let f1 = front1();
    bring(&service, MAIN, "q1", "checked").await;
    let l = link("SUB-1", "XC-20261006-000001");

    let mut tx = db.begin().await;
    service
        .check_save_in(&mut tx, &main, "q1", &save100("SUB-1"), &f1)
        .await
        .expect("ready to save");
    service
        .complete_in(&mut tx, &main, "q1", &l, &f1, at(7000))
        .await
        .expect("complete");
    assert_eq!(
        db.status(MAIN, "q1").await.as_deref(),
        Some("ID_CHECKED"),
        "not visible before the save commits"
    );
    tx.rollback().await.expect("the save fails after the hook");
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("ID_CHECKED"));
    assert_eq!(db.count("test_audit").await, 3, "no KIOSK_COMPLETED line");
    assert_eq!(db.rows().await[0]["documentRef"], Value::Null);

    // a request that does not exist, or lives in another branch
    let mut tx = db.begin().await;
    assert_eq!(
        refusal(
            service
                .complete_in(&mut tx, &main, "nope", &l, &f1, at(7000))
                .await
        ),
        NOT_READY
    );
    assert_eq!(
        refusal(
            service
                .complete_in(&mut tx, &branch(B2), "q1", &l, &f1, at(7000))
                .await
        ),
        NOT_READY
    );
    tx.rollback().await.expect("rollback");

    let mut tx = db.begin().await;
    service
        .complete_in(&mut tx, &main, "q1", &l, &f1, at(7000))
        .await
        .expect("complete");
    tx.commit().await.expect("the save commits");
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("COMPLETED"));
    assert_eq!(db.rows().await[0]["documentRef"], "XC-20261006-000001");
    let (lines, _, times) = db.audit_since(3).await;
    assert_eq!(
        lines,
        json!([{ "branch": "MAIN", "actor": "front1", "kind": "KIOSK_COMPLETED", "subject": "q1",
                 "documentRef": "XC-20261006-000001" }])
    );
    assert_eq!(times, [exact(7000)], "the time is cut to the millisecond");
    db.finish().await;
}

/// Every writing command waits for the branch's lock, and a command of another
/// branch does not. Whether the command waits is read from PostgreSQL.
#[tokio::test(flavor = "multi_thread")]
async fn every_write_waits_for_the_branch_lock() {
    let db = Arc::new(TestDb::start().await);
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    let (main, b2) = (branch(MAIN), branch(B2));
    let f1 = front1();
    bring(&service, MAIN, "q1", "quoted").await;
    bring(&service, MAIN, "q2", "waiting").await;
    bring(&service, MAIN, "q3", "accepted").await;
    bring(&service, MAIN, "q4", "checked").await;

    type Job = tokio::task::JoinHandle<Result<(), AppError>>;
    type Start = Box<dyn Fn() -> Job>;
    let commands: Vec<(&str, Start)> = vec![
        ("quote", {
            let (s, b) = (service.clone(), main.clone());
            Box::new(move || {
                let (s, b) = (s.clone(), b.clone());
                tokio::spawn(async move { s.quote(&b, buy100("q5"), at(7000)).await.map(|_| ()) })
            })
        }),
        ("confirm", {
            let (s, b) = (service.clone(), main.clone());
            Box::new(move || {
                let (s, b) = (s.clone(), b.clone());
                tokio::spawn(async move { s.confirm(&b, "q1", at(7000)).await.map(|_| ()) })
            })
        }),
        ("accept", {
            let (s, b, f) = (service.clone(), main.clone(), f1.clone());
            Box::new(move || {
                let (s, b, f) = (s.clone(), b.clone(), f.clone());
                tokio::spawn(async move { s.accept(&b, &f, "q2", at(7000)).await })
            })
        }),
        ("identityChecked", {
            let (s, b, f) = (service.clone(), main.clone(), f1.clone());
            Box::new(move || {
                let (s, b, f) = (s.clone(), b.clone(), f.clone());
                tokio::spawn(async move {
                    s.identity_checked(&b, &f, "q3", true, at(7000))
                        .await
                        .map(|_| ())
                })
            })
        }),
        ("publishReceipt", {
            let (s, b, f) = (service.clone(), main.clone(), f1.clone());
            Box::new(move || {
                let (s, b, f) = (s.clone(), b.clone(), f.clone());
                tokio::spawn(async move { s.publish_receipt(&b, &f, "q4", PNG, at(7000)).await })
            })
        }),
        ("configure", {
            let (s, b) = (service.clone(), main.clone());
            Box::new(move || {
                let (s, b) = (s.clone(), b.clone());
                tokio::spawn(async move {
                    s.configure(
                        &b,
                        &owner(),
                        configure_of("CHF", true, "36.4", "37.1"),
                        at(7000),
                    )
                    .await
                })
            })
        }),
    ];

    for (n, (name, start)) in commands.iter().enumerate() {
        let before = (
            db.rows().await,
            db.switches().await,
            db.count("kiosk_receipts").await,
            db.count("test_audit").await,
        );
        let held = db.hold_branch(MAIN).await;
        let job = start();
        db.until_blocked_by(&held).await;
        assert!(
            !job.is_finished(),
            "{name} finished while the branch was locked"
        );
        assert_eq!(
            (
                db.rows().await,
                db.switches().await,
                db.count("kiosk_receipts").await,
                db.count("test_audit").await,
            ),
            before,
            "{name} wrote before it had the lock"
        );
        // another branch is not held up
        within(MUST_FINISH, "a quote in the other branch", async {
            service
                .quote(&b2, buy100(&format!("other{n}")), at(7000))
                .await
                .expect("the other branch is free")
        })
        .await;
        // the other branch's quote is stored; from here on only the held command can change things
        let before = (
            db.rows().await,
            db.switches().await,
            db.count("kiosk_receipts").await,
            db.count("test_audit").await,
        );
        assert!(
            !job.is_finished(),
            "{name} finished while the branch was locked"
        );
        held.release().await;
        within(MUST_FINISH, name, job)
            .await
            .expect("the task did not panic")
            .unwrap_or_else(|e| panic!("{name} failed after the lock was released: {e:?}"));
        assert_ne!(
            (
                db.rows().await,
                db.switches().await,
                db.count("kiosk_receipts").await,
                db.count("test_audit").await,
            ),
            before,
            "{name} stored nothing"
        );
    }
    drop(commands);
    drop(service);
    Arc::try_unwrap(db)
        .ok()
        .expect("no task holds the database")
        .finish()
        .await;
}

/// Two employees accept the same request at the same moment: one gets it.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_accepted_twice_at_once_goes_to_one_employee() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    for round in 0..8 {
        let id = format!("q{round}");
        service
            .quote(&main, buy100(&id), at(0))
            .await
            .expect("quote");
        // two customers' screens confirm the same request at once: one audit line
        let (c1, c2) = tokio::join!(
            service.confirm(&main, &id, at(4000)),
            service.confirm(&main, &id, at(4000))
        );
        assert_eq!(c1.expect("confirm").status, KioskStatus::Waiting);
        assert_eq!(c2.expect("confirm").status, KioskStatus::Waiting);

        let (f1, f2) = (front1(), front2());
        let (a, b) = tokio::join!(
            service.accept(&main, &f1, &id, at(5000)),
            service.accept(&main, &f2, &id, at(5000))
        );
        let refused = match (a, b) {
            (Ok(()), other) | (other, Ok(())) => refusal(other),
            (a, b) => panic!("round {round}: nobody got the request: {a:?} / {b:?}"),
        };
        assert_eq!(refused, "This request is already assigned or unavailable.");
        assert_eq!(db.status(MAIN, &id).await.as_deref(), Some("ACCEPTED"));
    }
    assert_eq!(
        db.count("test_audit").await,
        16,
        "one KIOSK_REQUEST and one KIOSK_ACCEPTED line per request"
    );
    db.finish().await;
}

// ── what the answers carry ──────────────────────────────────────────────────

/// The time handed in is cut to the millisecond, never rounded, and never read
/// from the machine's clock.
#[tokio::test(flavor = "multi_thread")]
async fn time_is_what_the_caller_says_cut_to_the_millisecond() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    // 0.000999 s after T0 is still millisecond 0
    let made = service
        .quote(&main, buy100("q1"), at(0))
        .await
        .expect("quote");
    assert_eq!(made.request.created, kiosk_time(exact(0)));
    let stored: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT created_at FROM kiosk_requests WHERE id = 'q1'")
            .fetch_one(&db.pool)
            .await
            .expect("created_at");
    assert_eq!(stored, exact(0));
    // 2,999.999 ms after the quote was made is too early; rounded, it would be 3,000
    assert_eq!(
        refusal(service.confirm(&main, "q1", at(2999)).await),
        "Please review your quote for 3 seconds."
    );
    // 300,000.999 ms is still in time; rounded up it would have expired
    service
        .confirm(&main, "q1", at(300_000))
        .await
        .expect("in time");

    bring(&service, MAIN, "q2", "checked").await;
    service
        .publish_receipt(&main, &front1(), "q2", PNG, at(10_000))
        .await
        .expect("publish");
    let published: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM kiosk_receipts WHERE request_id = 'q2'")
            .fetch_one(&db.pool)
            .await
            .expect("updated_at");
    assert_eq!(published, exact(10_000));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quote_answers_the_request_it_stored() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    let mut cmd = quote_of(
        "ab54c0de-0000-4000-8000-000000000001",
        Side::Sell,
        "USD",
        "1000",
    );
    cmd.input.payment_method = Some("PromptPay".to_owned());
    let made = service.quote(&main, cmd, at(0)).await.expect("quote");
    assert_eq!(made.identity.queue, "K-AB54C0DE");
    assert_eq!(made.identity.status, KioskStatus::Quoted);
    assert_eq!(made.request.quote.foreign.to_string(), "30.67");
    assert_eq!(made.request.quote.thb.to_string(), "999.84");
    assert_eq!(made.request.quote.rate, rate("32.6"));

    // what was stored is what was answered, to the last field
    service
        .confirm(&main, &made.request.id, at(4000))
        .await
        .expect("confirm");
    let listed = service
        .list(&main, &front1(), at(5000))
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    let mut expected = made.request.clone();
    expected.status = KioskStatus::Waiting;
    assert_eq!(listed[0].request, expected);

    service
        .accept(&main, &front1(), &made.request.id, at(6000))
        .await
        .expect("accept");
    let got = service
        .get(&main, &front1(), &made.request.id)
        .await
        .expect("get");
    expected.status = KioskStatus::Accepted;
    expected.staff = Some("front1".to_owned());
    assert_eq!(got, expected);
    db.finish().await;
}

/// A figure above the entry cap that the machine computes must come back from
/// the table as it went in.
#[tokio::test(flavor = "multi_thread")]
async fn a_computed_figure_far_above_the_entry_cap_is_stored_whole() {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    ports.set_rate(MAIN, counter_rate("JPY", "0.0001", "0.0001"));
    let service = db.service(ports);
    let main = branch(MAIN);
    let made = service
        .quote(&main, quote_of("q1", Side::Sell, "JPY", "10000000"), at(0))
        .await
        .expect("quote");
    assert_eq!(made.request.quote.foreign.to_string(), "100000000000.00");
    service
        .confirm(&main, "q1", at(4000))
        .await
        .expect("confirm");
    let listed = service
        .list(&main, &front1(), at(5000))
        .await
        .expect("list");
    assert_eq!(listed[0].request.quote, made.request.quote);
    db.finish().await;
}
