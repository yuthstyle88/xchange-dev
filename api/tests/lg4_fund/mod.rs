//! LG-4 (#929): the one helper every route harness uses to put cash in a branch's ledger SAFE. Included by
//! `snapshot_common`, `quote_common` and `auth_http_common` with `#[path]`, so a binary may hold more than one copy:
//! the source id carries the time as well as a counter, so two copies never post under the same id (which the
//! ledger would replay instead of posting).
#![allow(dead_code)]

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// LG-4 (#929, lane controller's 1:1 ruling): 10,000,000 of every currency in the register into `branch`'s SAFE
/// through the ledger (`OPENING` from BANK), the store a shift's opening now takes its cash from (design §4.4).
/// Before the cutover nothing had to be there: an opening wrote only `shift_balances`.
pub async fn lg4_fund_safe(pool: &sqlx::PgPool, branch: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let codes: Vec<String> = sqlx::query_scalar("SELECT trim(code) FROM currencies ORDER BY 1")
        .fetch_all(pool)
        .await
        .expect("the currency register");
    let n = LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let mut tx = pool.begin().await.expect("begin");
    // One posting per currency: a posting with no stored line carries at most one foreign leg.
    for code in &codes {
        let request = PostingRequest {
            from: CashPlace::Bank,
            to: CashPlace::Safe(xc_kernel::ids::BranchId::new(branch.to_owned())),
            currency: xc_kernel::money::CurrencyCode::parse(code).expect("a register currency"),
            quantity: xc_kernel::money::ForeignAmount::from_decimal(rust_decimal::Decimal::from(
                10_000_000,
            )),
            source: SourceRef {
                source_type: SourceType::Opening,
                source_id: format!("lg4-fund:{branch}:{code}:{nanos}:{n}"),
                source_key: "fund".to_string(),
            },
            line: None,
            reverses: None,
        };
        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .post_in(&mut tx, &[request], None)
            .await
            .expect("fund the safe through the ledger");
    }
    tx.commit().await.expect("commit");
}

/// R1 (TX-6 #674, lane controller's ruling: compat saves post through the ledger): funds the ledger drawer of
/// every OPEN v2 shift with that shift's opening (`shift_balances.opening`), from the BANK as `OPENING`. A shift
/// custody opens writes only `shift_balances`, so its drawer would otherwise be empty in the ledger. The source
/// id is the shift's and the currency's, so a second call replays the posting instead of funding twice.
pub async fn r1_fund_open_drawers(pool: &sqlx::PgPool) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let openings: Vec<(String, String, rust_decimal::Decimal)> = sqlx::query_as(
        "SELECT b.shift_id, trim(b.code), b.opening FROM shift_balances b \
         JOIN shifts s ON s.id = b.shift_id WHERE s.status = 'OPEN' AND b.opening > 0 \
         ORDER BY b.shift_id, b.code",
    )
    .fetch_all(pool)
    .await
    .expect("the open shifts' openings");
    if openings.is_empty() {
        return;
    }
    let mut tx = pool.begin().await.expect("begin");
    for (shift, code, opening) in openings {
        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Drawer(shift.clone()),
                    currency: xc_kernel::money::CurrencyCode::parse(&code)
                        .expect("a register currency"),
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(opening),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: format!("r1-fund:{shift}:{code}"),
                        source_key: "fund".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund the open shift's drawer through the ledger");
    }
    tx.commit().await.expect("commit");
}
