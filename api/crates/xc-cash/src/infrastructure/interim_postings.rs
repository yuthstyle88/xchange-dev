//! Interim implementation of CashPostings on current tables.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::money::{CurrencyCode, Money};
use xc_shift::domain::shift::value_objects::balances::Balances;

use crate::domain::posting::{CashPlace, CashPostings, PostingRequest};

#[derive(Clone, Default)]
pub struct InterimCashPostings;

impl InterimCashPostings {
    pub fn new() -> Self {
        Self
    }
}

impl CashPostings for InterimCashPostings {
    fn post_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _req: &'a [PostingRequest],
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            // Staged until transfers/movements; posting logic as needed
            Ok(())
        })
    }

    fn expected_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>> {
        Box::pin(async move {
            match place {
                CashPlace::Drawer(shift_id) => {
                    let rows: Vec<(String, String)> = sqlx::query_as(
                        "SELECT code, expected::text FROM shift_balances WHERE shift_id = $1",
                    )
                    .bind(shift_id)
                    .fetch_all(&mut **tx)
                    .await?;

                    let mut pairs = Vec::new();
                    for (code_str, exp_str) in rows {
                        if let Ok(code) = CurrencyCode::parse(code_str.trim()) {
                            if let Ok(money) = Money::parse(&exp_str) {
                                if !money.is_zero() {
                                    pairs.push((code, money));
                                }
                            }
                        }
                    }
                    Ok(Balances::of(&pairs))
                }
                CashPlace::Safe(_branch_id) => {
                    // Safe balances from ops_state if needed
                    Ok(Balances::of(&[]))
                }
                _ => Ok(Balances::of(&[])),
            }
        })
    }
}
