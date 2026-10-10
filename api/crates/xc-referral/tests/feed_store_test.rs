//! FD-1 up-front tests — `ExchangeFeedWriter`, `PgExchangeFeedReader` and `BookingMemberKeys`
//! against PostgreSQL (design broker-seam.md §6.1–§6.4, §11 "FD-1"). Written before the
//! implementation by the ticket author. Do not edit, delete or weaken; add tests in a file of your own.

mod common;

use common::*;
use pos_error::AppError;
use uuid::Uuid;

async fn exchange(db: &TestDb) -> Uuid {
    let customer = unique("CU");
    db.seed_customer(&customer).await;
    db.seed_transaction(&customer).await.0
}

// ── writer ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_completed_exchange_is_one_completed_row_with_nothing_reversed() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    db.record(&completed(id, 3_261_630, CustomerKeys::default()))
        .await;
    let row = db.row(id).await.expect("a row");
    assert_eq!(row.status, "COMPLETED");
    assert_eq!(row.value_satang, 3_261_630);
    assert_eq!(row.reversed_satang, 0);
    assert_eq!(row.reversed_at, None);
    assert_eq!(
        (row.phone, row.customer_sub),
        (None, None),
        "a row with neither key is written"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_repeated_completion_changes_nothing_not_even_updated_at() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    db.record(&completed(id, 10_000, CustomerKeys::default()))
        .await;
    let first = db.row(id).await.expect("a row");
    let mut again = completed(
        id,
        99_900,
        CustomerKeys {
            customer_sub: Some("sub-x".into()),
            phone: None,
        },
    );
    again.completed_at = at(60);
    db.record(&again).await;
    assert_eq!(
        db.row(id).await.expect("a row"),
        first,
        "the value is fixed once, at completion"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_keys_are_stored_as_given() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    let keys = CustomerKeys {
        customer_sub: Some("sub-1".into()),
        phone: Some("+66812345678".into()),
    };
    db.record(&completed(id, 500, keys)).await;
    let row = db.row(id).await.expect("a row");
    assert_eq!(row.customer_sub.as_deref(), Some("sub-1"));
    assert_eq!(row.phone.as_deref(), Some("+66812345678"));
    db.finish().await;
}

#[tokio::test]
async fn a_partial_then_a_full_reversal_store_the_cumulative_amount() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    db.record(&completed(id, 1_000, CustomerKeys::default()))
        .await;
    let before = db.row(id).await.expect("a row").updated_at;

    let mut tx = db.begin().await;
    ExchangeFeedWriter::record_reversed(&mut tx, id, 400, at(10))
        .await
        .expect("partial");
    tx.commit().await.expect("commit");
    let partial = db.row(id).await.expect("a row");
    assert_eq!(
        (partial.status.as_str(), partial.reversed_satang),
        ("REVERSED", 400)
    );
    assert_eq!(partial.reversed_at, Some(at(10)));
    assert!(
        partial.updated_at > before,
        "a reversal moves updated_at, so Broker reads it again"
    );

    let mut tx = db.begin().await;
    ExchangeFeedWriter::record_reversed(&mut tx, id, 1_000, at(20))
        .await
        .expect("full");
    tx.commit().await.expect("commit");
    let full = db.row(id).await.expect("a row");
    assert_eq!(
        (full.reversed_satang, full.value_satang),
        (1_000, 1_000),
        "cumulative, not a delta"
    );
    assert_eq!(full.reversed_at, Some(at(20)));
    db.finish().await;
}

#[tokio::test]
async fn a_reversal_above_the_value_or_not_positive_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    db.record(&completed(id, 1_000, CustomerKeys::default()))
        .await;
    let before = db.row(id).await.expect("a row");
    for total in [1_001, 0, -5] {
        let mut tx = db.begin().await;
        let out = ExchangeFeedWriter::record_reversed(&mut tx, id, total, at(10)).await;
        assert!(
            matches!(out, Err(AppError::Validation(_))),
            "{total}: {out:?}"
        );
        drop(tx);
    }
    assert_eq!(db.row(id).await.expect("a row"), before);
    db.finish().await;
}

#[tokio::test]
async fn reversing_an_exchange_that_is_not_in_the_feed_is_not_found() {
    let db = TestDb::start().await;
    let mut tx = db.begin().await;
    let out = ExchangeFeedWriter::record_reversed(&mut tx, Uuid::new_v4(), 1, at(0)).await;
    assert!(matches!(out, Err(AppError::NotFound { .. })), "{out:?}");
    drop(tx);
    db.finish().await;
}

#[tokio::test]
async fn a_rollback_writes_no_row() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    let mut tx = db.begin().await;
    ExchangeFeedWriter::record_completed(&mut tx, &completed(id, 1_000, CustomerKeys::default()))
        .await
        .expect("record");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        db.row(id).await,
        None,
        "the feed row lives and dies with the save"
    );
    db.finish().await;
}

// ── reader ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pages_follow_the_keyset_and_an_empty_page_has_no_next_cursor() {
    let db = TestDb::start().await;
    let mut ids = Vec::new();
    for value in [100, 200, 300] {
        let id = exchange(&db).await;
        db.record(&completed(id, value, CustomerKeys::default()))
            .await;
        ids.push(id);
    }
    let reader = PgExchangeFeedReader::new(db.pool.clone());

    let first = reader.page(None, 2).await.expect("page 1");
    assert_eq!(
        first.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        ids[..2].to_vec()
    );
    let last = &first.items[1];
    assert_eq!(
        first.next_cursor,
        Some(FeedCursor {
            updated_at: last.updated_at,
            id: last.id
        }),
        "nextCursor is the last item's"
    );

    let second = reader.page(first.next_cursor, 2).await.expect("page 2");
    assert_eq!(
        second.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![ids[2]]
    );
    assert_eq!(second.items[0].status, FeedStatus::Completed);
    assert_eq!(second.items[0].value_satang, 300);

    let empty = reader.page(second.next_cursor, 2).await.expect("page 3");
    assert!(empty.items.is_empty());
    assert_eq!(
        empty.next_cursor, None,
        "never echo `since` on an empty page (Broker §7)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_lookback_cursor_with_the_nil_uuid_reads_every_row_after_its_time() {
    let db = TestDb::start().await;
    let id = exchange(&db).await;
    db.record(&completed(id, 100, CustomerKeys::default()))
        .await;
    let stored = db.row(id).await.expect("a row").updated_at;
    let lookback = FeedCursor {
        updated_at: stored - chrono::Duration::minutes(10),
        id: Uuid::nil(),
    };
    let page = PgExchangeFeedReader::new(db.pool.clone())
        .page(Some(lookback), 200)
        .await
        .expect("page");
    assert_eq!(
        page.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![id]
    );
    db.finish().await;
}

#[tokio::test]
async fn a_limit_outside_one_to_two_hundred_is_refused() {
    let db = TestDb::start().await;
    let reader = PgExchangeFeedReader::new(db.pool.clone());
    for limit in [0, MAX_LIMIT + 1] {
        assert!(
            matches!(reader.page(None, limit).await, Err(AppError::Validation(_))),
            "{limit}"
        );
    }
    assert!(reader.page(None, MAX_LIMIT).await.is_ok());
    assert!(reader.page(None, 1).await.is_ok());
    db.finish().await;
}

// ── customer keys ───────────────────────────────────────────────────────────

#[tokio::test]
async fn an_exchange_from_a_members_booking_carries_the_members_verified_phone() {
    let db = TestDb::start().await;
    let customer = unique("CU");
    db.seed_customer(&customer).await;
    let member = unique("MB");
    db.seed_member(&member, &customer).await;
    let (id, document_ref) = db.seed_transaction(&customer).await;
    db.seed_used_booking(Some(&member), &document_ref).await;

    let mut tx = db.begin().await;
    let keys = BookingMemberKeys.keys_in(&mut tx, id).await.expect("keys");
    assert_eq!(
        keys,
        CustomerKeys {
            customer_sub: None,
            phone: Some(MEMBER_PHONE.into())
        }
    );
    drop(tx);
    db.finish().await;
}

#[tokio::test]
async fn a_booking_without_a_member_or_no_booking_gives_no_keys() {
    let db = TestDb::start().await;
    let customer = unique("CU");
    db.seed_customer(&customer).await;
    // The booking's own typed phone (`bookings.phone`) is never sent (D-6).
    let (booked, document_ref) = db.seed_transaction(&customer).await;
    db.seed_used_booking(None, &document_ref).await;
    let (walk_in, _) = db.seed_transaction(&customer).await;

    let mut tx = db.begin().await;
    assert_eq!(
        BookingMemberKeys
            .keys_in(&mut tx, booked)
            .await
            .expect("keys"),
        CustomerKeys::default()
    );
    assert_eq!(
        BookingMemberKeys
            .keys_in(&mut tx, walk_in)
            .await
            .expect("keys"),
        CustomerKeys::default()
    );
    drop(tx);
    db.finish().await;
}
