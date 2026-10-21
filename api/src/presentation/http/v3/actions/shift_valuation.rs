//! Shift valuation compute action (§12.20.4).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use rust_decimal::Decimal;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, fixed2, fixed6, round_cents, typed_decimal};
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;
use xc_shift::domain::shift::value_objects::stock_delta::Direction;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::{Money as MoneyDto, Rate as RateDto};
use crate::presentation::dto::v3::shift_valuation::{
    ShiftValuation, ShiftValuationCurrency, ShiftValuationRequest, ShiftValuationResult,
    ShiftValuationRow,
};
use crate::presentation::http::v3::action::{ComputeAction, ComputeEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

pub struct ShiftValuationAction {
    shift_repo: PgShiftRepository,
}

impl ShiftValuationAction {
    pub fn new(shift_repo: PgShiftRepository) -> Self {
        Self { shift_repo }
    }
}

fn valuation_rate_error() -> V3Error {
    V3Error::new(
        "valuationRate",
        Some("rates"),
        "กรอกเรทให้ครบทุกสกุลที่มียอด",
        StatusCode::BAD_REQUEST,
    )
}

impl ComputeAction for ShiftValuationAction {
    const NAME: &'static str = "shift_valuation";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Request = ShiftValuationRequest;
    type Answer = ShiftValuation;

    fn compute(
        &self,
        branch: &BranchId,
        _who: &Actor,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Answer, V3Error>> + Send {
        let repo = self.shift_repo.clone();
        let branch = branch.clone();
        async move {
            if request.shift_id.trim().is_empty() {
                return Err(V3Error::new(
                    "invalidRequest",
                    Some("shiftId"),
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                ));
            }

            let shift = repo
                .shift(&branch, &request.shift_id)
                .await
                .map_err(|_| server_error())?
                .ok_or_else(|| {
                    V3Error::new(
                        "notFound",
                        Some("shiftId"),
                        "Not found",
                        StatusCode::NOT_FOUND,
                    )
                })?;

            let movements = xc_cash::infrastructure::transfer_moves::of_shift(
                repo.pool(),
                &branch,
                &request.shift_id,
            )
            .await
            .map_err(|_| server_error())?;

            // 1. Currencies: every register currency with o_c != 0, or e_c != 0, or at least 1 movement in c.
            let mut codes_set = HashSet::new();
            for (code, _) in shift.opening.non_zero() {
                codes_set.insert(code);
            }
            for (code, _) in shift.expected.non_zero() {
                codes_set.insert(code);
            }
            for m in &movements {
                codes_set.insert(m.currency);
            }

            // Order: THB first, then others A-Z by code.
            let mut codes: Vec<CurrencyCode> = codes_set.into_iter().collect();
            codes.sort_by(|a, b| {
                let a_is_thb = a.as_str() == "THB";
                let b_is_thb = b.as_str() == "THB";
                match (a_is_thb, b_is_thb) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    (false, false) => a.as_str().cmp(b.as_str()),
                }
            });

            let currencies: Vec<ShiftValuationCurrency> = codes
                .iter()
                .map(|&code| {
                    let o_c = shift.opening.get(code);
                    let e_c = shift.expected.get(code);
                    let needs_rate = code.as_str() != "THB";
                    ShiftValuationCurrency {
                        currency: code.as_str().to_string(),
                        opening: MoneyDto(fixed2(o_c.minor_i128())),
                        current: MoneyDto(fixed2(e_c.minor_i128())),
                        needs_rate,
                    }
                })
                .collect();

            // 2. rates absent or null -> currencies only.
            let rates = match request.rates {
                Some(r) => r,
                None => {
                    return Ok(ShiftValuation {
                        currencies,
                        result: None,
                    });
                }
            };

            // 3. Validate rates for each currency with needs_rate.
            let min_rate = Decimal::new(1, 6); // 0.000001
            let max_rate = Decimal::new(1_000_000, 0); // 1,000,000
            let mut rate_by_currency = HashMap::new();

            for c in &currencies {
                let r_c = if c.needs_rate {
                    let r_str = rates.get(&c.currency).ok_or_else(valuation_rate_error)?;
                    let r = typed_decimal(r_str).ok_or_else(valuation_rate_error)?;
                    if r < min_rate || r > max_rate {
                        return Err(valuation_rate_error());
                    }
                    r
                } else {
                    // r_THB = 1. THB rate in rates is ignored.
                    Decimal::ONE
                };
                rate_by_currency.insert(c.currency.clone(), r_c);
            }

            // 4 & 5. Compute values, rows, totals.
            let mut total_opening_cents: i128 = 0;
            let mut total_current_cents: i128 = 0;
            let mut rows = Vec::with_capacity(currencies.len());

            for c in &currencies {
                let r_c = rate_by_currency[&c.currency];
                let code = CurrencyCode::parse(&c.currency).map_err(|_| server_error())?;
                let o_c = shift.opening.get(code);
                let e_c = shift.expected.get(code);

                let o_dec = Decimal::try_from_i128_with_scale(o_c.minor_i128(), 2)
                    .map_err(|_| server_error())?;
                let e_dec = Decimal::try_from_i128_with_scale(e_c.minor_i128(), 2)
                    .map_err(|_| server_error())?;

                let val_o = round_cents(o_dec * r_c).ok_or_else(server_error)?;
                let val_e = round_cents(e_dec * r_c).ok_or_else(server_error)?;

                total_opening_cents += val_o;
                total_current_cents += val_e;

                rows.push(ShiftValuationRow {
                    currency: c.currency.clone(),
                    rate: RateDto(fixed6(r_c)),
                    value: MoneyDto(fixed2(val_e)),
                });
            }

            let mut total_movements_cents: i128 = 0;
            for m in &movements {
                let r_m = rate_by_currency
                    .get(m.currency.as_str())
                    .copied()
                    .ok_or_else(server_error)?;
                let m_dec = Decimal::try_from_i128_with_scale(m.amount.minor_i128(), 2)
                    .map_err(|_| server_error())?;
                let val_m = round_cents(m_dec * r_m).ok_or_else(server_error)?;
                match m.direction {
                    Direction::In => total_movements_cents += val_m,
                    Direction::Out => total_movements_cents -= val_m,
                }
            }

            let difference_cents = total_current_cents - total_opening_cents;
            let adjusted_cents = total_current_cents - total_opening_cents - total_movements_cents;

            let result = ShiftValuationResult {
                rows,
                opening: MoneyDto(fixed2(total_opening_cents)),
                current: MoneyDto(fixed2(total_current_cents)),
                movements: MoneyDto(fixed2(total_movements_cents)),
                difference: MoneyDto(fixed2(difference_cents)),
                adjusted: MoneyDto(fixed2(adjusted_cents)),
            };

            Ok(ShiftValuation {
                currencies,
                result: Some(result),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let repo = xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
        state.db.clone(),
    );
    let action = Arc::new(ShiftValuationAction::new(repo));
    ComputeEndpoint::register(cfg, action);
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::shift_valuation::export_types(dir)
}
