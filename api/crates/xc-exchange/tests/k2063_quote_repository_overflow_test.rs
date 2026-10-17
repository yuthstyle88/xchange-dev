#[allow(unused_imports)]
mod common;

use chrono::Utc;
use common::TestDb;
use pos_error::AppError;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteStatus};
use xc_exchange::domain::quote::quote_line::QuoteLine;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

fn from_hundredths(minor: i128) -> Money {
    Money::try_from_minor_i128(minor).expect("fits Decimal at scale 2")
}

fn money(text: &str) -> Money {
    Money::parse(text).expect("a valid money amount")
}

async fn prepare_schema(db: &TestDb) {
    let sql = r#"
        ALTER TABLE quotes DROP CONSTRAINT IF EXISTS quotes_publication_id_fkey;
        ALTER TABLE quotes DROP CONSTRAINT IF EXISTS quotes_organization_id_fkey;
        ALTER TABLE quotes DROP CONSTRAINT IF EXISTS quotes_organization_id_branch_id_fkey;
        ALTER TABLE quote_lines DROP CONSTRAINT IF EXISTS quote_lines_organization_id_fkey;
        ALTER TABLE quote_lines DROP CONSTRAINT IF EXISTS quote_lines_published_rate_line_id_fkey;
        ALTER TABLE quote_lines DROP CONSTRAINT IF EXISTS quote_lines_currency_fkey;
        ALTER TABLE quote_lines DROP CONSTRAINT IF EXISTS quote_lines_fee_thb_check;
        ALTER TABLE quote_lines DROP CONSTRAINT IF EXISTS quote_lines_thb_amount_check;
        ALTER TABLE quote_lines DROP CONSTRAINT IF EXISTS quote_lines_settle_thb_check;
        ALTER TABLE quote_lines ALTER COLUMN thb_amount TYPE numeric;
        ALTER TABLE quote_lines ALTER COLUMN fee_thb TYPE numeric;
        ALTER TABLE quote_lines ALTER COLUMN settle_thb TYPE numeric;
    "#;
    for stmt in sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() {
            sqlx::query(stmt)
                .execute(&db.pool)
                .await
                .expect("prepare schema");
        }
    }
}

fn test_quote(id: Uuid, lines: Vec<QuoteLine>) -> Quote {
    Quote {
        id,
        organization_id: Uuid::new_v4(),
        branch_id: BranchId::new("MAIN"),
        customer_id: None,
        quote_type: "INDICATIVE".to_string(),
        channel: QuoteChannel::Desk,
        status: QuoteStatus::Active,
        publication_id: Uuid::new_v4(),
        expires_at: Utc::now() + chrono::Duration::minutes(5),
        benefit_snapshot: serde_json::json!({}),
        created_by: "test_user".to_string(),
        accepted_by_transaction_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        lines,
        stale: false,
    }
}

fn test_line(quote_id: Uuid, thb_amount: Money, fee_thb: Money, settle_thb: Money) -> QuoteLine {
    QuoteLine {
        id: Uuid::new_v4(),
        organization_id: Uuid::new_v4(),
        quote_id,
        published_rate_line_id: Uuid::new_v4(),
        shop_side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        denomination_group: "ALL".to_string(),
        quantity: ForeignAmount::from_decimal(Decimal::from(100)),
        unit_size: UnitSize::from_decimal(Decimal::ONE).unwrap(),
        base_rate: Rate::parse("35.5").unwrap(),
        final_rate: Rate::parse("35.5").unwrap(),
        thb_amount,
        fee_thb,
        settle_thb,
        line_no: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn insert_quote_line_thb_amount_overflow_refused() {
    let db = TestDb::start().await;
    prepare_schema(&db).await;
    let repo = PgQuoteRepository::new(db.pool.clone());

    let quote_id = Uuid::new_v4();
    let overflowing_thb = from_hundredths(i64::MAX as i128 + 1);
    let line = test_line(quote_id, overflowing_thb, Money::ZERO, Money::ZERO);
    let quote = test_quote(quote_id, vec![line]);

    let mut tx = db.pool.begin().await.unwrap();
    let result = repo.insert_in(&mut tx, &quote).await;

    match result {
        Err(AppError::Internal(msg)) => {
            assert!(
                msg.contains("quote line thb_amount minor overflow"),
                "expected thb_amount overflow message, got: {msg}"
            );
        }
        other => panic!("expected Err(AppError::Internal), got {other:?}"),
    }
}

#[tokio::test]
async fn insert_quote_line_fee_thb_overflow_refused() {
    let db = TestDb::start().await;
    prepare_schema(&db).await;
    let repo = PgQuoteRepository::new(db.pool.clone());

    let quote_id = Uuid::new_v4();
    let overflowing_fee = from_hundredths(i64::MAX as i128 + 1);
    let line = test_line(quote_id, money("100.00"), overflowing_fee, money("100.00"));
    let quote = test_quote(quote_id, vec![line]);

    let mut tx = db.pool.begin().await.unwrap();
    let result = repo.insert_in(&mut tx, &quote).await;

    match result {
        Err(AppError::Internal(msg)) => {
            assert!(
                msg.contains("quote line fee_thb minor overflow"),
                "expected fee_thb overflow message, got: {msg}"
            );
        }
        other => panic!("expected Err(AppError::Internal), got {other:?}"),
    }
}

#[tokio::test]
async fn insert_quote_line_settle_thb_overflow_refused() {
    let db = TestDb::start().await;
    prepare_schema(&db).await;
    let repo = PgQuoteRepository::new(db.pool.clone());

    let quote_id = Uuid::new_v4();
    let overflowing_settle = from_hundredths(i64::MAX as i128 + 1);
    let line = test_line(quote_id, money("100.00"), Money::ZERO, overflowing_settle);
    let quote = test_quote(quote_id, vec![line]);

    let mut tx = db.pool.begin().await.unwrap();
    let result = repo.insert_in(&mut tx, &quote).await;

    match result {
        Err(AppError::Internal(msg)) => {
            assert!(
                msg.contains("quote line settle_thb minor overflow"),
                "expected settle_thb overflow message, got: {msg}"
            );
        }
        other => panic!("expected Err(AppError::Internal), got {other:?}"),
    }
}

#[tokio::test]
async fn insert_quote_ordinary_figures_succeeds() {
    let db = TestDb::start().await;
    prepare_schema(&db).await;
    let repo = PgQuoteRepository::new(db.pool.clone());

    let quote_id = Uuid::new_v4();
    let line = test_line(quote_id, money("3550.00"), money("50.00"), money("3500.00"));
    let quote = test_quote(quote_id, vec![line]);

    let mut tx = db.pool.begin().await.unwrap();
    let result = repo.insert_in(&mut tx, &quote).await;
    assert!(result.is_ok(), "insert_in failed: {result:?}");
    tx.commit().await.unwrap();

    let mut tx2 = db.pool.begin().await.unwrap();
    let found = repo.find_in(&mut tx2, quote_id).await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.lines.len(), 1);
    assert_eq!(found.lines[0].thb_amount, money("3550.00"));
    assert_eq!(found.lines[0].fee_thb, money("50.00"));
    assert_eq!(found.lines[0].settle_thb, money("3500.00"));
}
