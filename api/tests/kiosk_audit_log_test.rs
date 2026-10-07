//! Tests for `PgKioskAuditLog`: writes to `device_events` on commit, leaves none on rollback.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use chrono::{TimeZone, Utc};
use compat_common::Person;
use rpc_common::Desk;
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side};
use xc_kiosk::domain::kiosk::value_objects::audit_line::{AuditLine, CUSTOMER_KIOSK, KioskAudit};
use xc_kiosk::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use xc_kiosk::domain::kiosk::value_objects::payment_method::PaymentMethod;
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use xchange::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog};

fn test_quote() -> KioskQuote {
    KioskQuote {
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        rate: Rate::parse("32.1").unwrap(),
        foreign: Money::from_minor(25000).unwrap(),
        thb: Money::from_minor(802500).unwrap(),
        input_amount: Money::from_minor(25000).unwrap(),
        payment_method: PaymentMethod::Cash,
    }
}

#[actix_web::test]
async fn kiosk_audit_log_writes_device_events_on_commit_and_none_on_rollback() {
    let desk = Desk::open().await;
    let employee = Person {
        key: "FR1".to_string(),
        kind: "staff".to_string(),
        id: "00000000-0000-4000-8000-000000000011".to_string(),
        username: "fr1".to_string(),
        name: "FR1 of the test".to_string(),
        role: "FRONT".to_string(),
        branch_id: Some("MAIN".to_string()),
        must_change: false,
    };
    desk.employ(std::slice::from_ref(&employee)).await;

    let audit = PgKioskAuditLog::new(PgDeviceRepository::new(desk.db.pool.clone()));
    let branch = BranchId::new("MAIN");
    let at = Utc.with_ymd_and_hms(2026, 10, 6, 5, 0, 0).unwrap();

    // 1. Rollback test: nothing is left in device_events
    {
        let mut tx = desk.db.pool.begin().await.expect("begin tx");
        let line = AuditLine {
            actor: CUSTOMER_KIOSK.to_string(),
            subject: "req-rollback".to_string(),
            what: KioskAudit::Request {
                quote: test_quote(),
            },
        };
        audit
            .record_in(&mut tx, &branch, &line, at)
            .await
            .expect("record line");
        tx.rollback().await.expect("rollback");
    }

    let rolled_back_count: (i64,) = sqlx::query_as("SELECT count(*) FROM device_events")
        .fetch_one(&desk.db.pool)
        .await
        .expect("read count");
    assert_eq!(
        rolled_back_count.0, 0,
        "rollback left nothing in device_events"
    );

    // 2. Commit test: customer request line
    {
        let mut tx = desk.db.pool.begin().await.expect("begin tx");
        let line = AuditLine {
            actor: CUSTOMER_KIOSK.to_string(),
            subject: "req-commit-1".to_string(),
            what: KioskAudit::Request {
                quote: test_quote(),
            },
        };
        audit
            .record_in(&mut tx, &branch, &line, at)
            .await
            .expect("record line");
        tx.commit().await.expect("commit");
    }

    // 3. Commit test: employee accepted line with actor id resolved to username
    {
        let mut tx = desk.db.pool.begin().await.expect("begin tx");
        let line = AuditLine {
            actor: employee.id.clone(),
            subject: "req-commit-1".to_string(),
            what: KioskAudit::Accepted,
        };
        audit
            .record_in(&mut tx, &branch, &line, at)
            .await
            .expect("record line");
        tx.commit().await.expect("commit");
    }

    let rows: Vec<(i64, String, String, String, String, Value)> = sqlx::query_as(
        "SELECT seq, branch_id, actor, kind, ref, detail FROM device_events ORDER BY seq",
    )
    .fetch_all(&desk.db.pool)
    .await
    .expect("read device_events");

    assert_eq!(rows.len(), 2, "two committed audit events");

    // Check customer request event
    let req_event = &rows[0];
    assert_eq!(req_event.1, "MAIN");
    assert_eq!(req_event.2, "CUSTOMER-KIOSK");
    assert_eq!(req_event.3, "KIOSK_REQUEST");
    assert_eq!(req_event.4, "req-commit-1");
    assert_eq!(
        req_event.5,
        json!({
            "type": "BUY",
            "currency": "USD",
            "rate": "32.1000",
            "foreign": "250.00",
            "thb": "8025.00",
            "inputAmount": "250.00",
            "paymentMethod": "Cash",
        })
    );

    // Check employee accepted event: actor was employee.id, stored as employee.username "fr1"
    let accepted_event = &rows[1];
    assert_eq!(accepted_event.1, "MAIN");
    assert_eq!(accepted_event.2, "fr1", "actor id resolved to username");
    assert_eq!(accepted_event.3, "KIOSK_ACCEPTED");
    assert_eq!(accepted_event.4, "req-commit-1");
    assert_eq!(accepted_event.5, json!({}));
}
