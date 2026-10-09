//! The `profit_summary` compute action (V2-cash-profit, §12.20.5).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, round_cents, typed_decimal};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::Money;
use crate::presentation::dto::v3::profit_summary::{ProfitSummary, ProfitSummaryRequest};
use crate::presentation::http::v3::action::{ComputeAction, ComputeEndpoint};
use crate::presentation::http::v3::error::V3Error;

pub struct ProfitSummaryAction;

const INCOMPLETE: &str = "กรอกข้อมูลกำไรและเงินสำรองให้ครบก่อนคำนวณ";
const NEGATIVE: &str = "ต้นทุน ทุน และเงินสำรองต้องไม่ติดลบ";

const NON_NEGATIVE_KEYS: [&str; 6] = [
    "foreignValue",
    "foreignCost",
    "capital",
    "reserve",
    "cash",
    "cashMinimum",
];

impl ComputeAction for ProfitSummaryAction {
    const NAME: &'static str = "profit_summary";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Request = ProfitSummaryRequest;
    type Answer = ProfitSummary;

    async fn compute(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        request: Self::Request,
    ) -> Result<Self::Answer, V3Error> {
        let fields: [(&'static str, Option<&str>); 8] = [
                ("realized", request.realized.as_deref()),
                ("foreignValue", request.foreign_value.as_deref()),
                ("foreignCost", request.foreign_cost.as_deref()),
                ("netAssets", request.net_assets.as_deref()),
                ("capital", request.capital.as_deref()),
                ("reserve", request.reserve.as_deref()),
                ("cash", request.cash.as_deref()),
                ("cashMinimum", request.cash_minimum.as_deref()),
            ];

            // Step 1: completeness & exact typed decimal & cents fit
            let mut incomplete_keys = Vec::new();
            let mut parsed = Vec::with_capacity(8);

            for (key, opt_str) in fields {
                match opt_str {
                    Some(s) if !s.trim().is_empty() => match typed_decimal(s) {
                        Some(d) => match round_cents(d) {
                            Some(c) => parsed.push((key, d, c)),
                            None => incomplete_keys.push(key),
                        },
                        None => incomplete_keys.push(key),
                    },
                    _ => incomplete_keys.push(key),
                }
            }

            if !incomplete_keys.is_empty() {
                let field = if incomplete_keys.len() == 1 {
                    Some(incomplete_keys[0])
                } else {
                    None
                };
                return Err(V3Error::new(
                    "profitIncomplete",
                    field,
                    INCOMPLETE,
                    StatusCode::BAD_REQUEST,
                ));
            }

            // Step 2: non-negativity of the six
            let mut negative_keys = Vec::new();
            for &(key, d, _c) in &parsed {
                if NON_NEGATIVE_KEYS.contains(&key) && d < rust_decimal::Decimal::ZERO {
                    negative_keys.push(key);
                }
            }

            if !negative_keys.is_empty() {
                let field = if negative_keys.len() == 1 {
                    Some(negative_keys[0])
                } else {
                    None
                };
                return Err(V3Error::new(
                    "profitNegative",
                    field,
                    NEGATIVE,
                    StatusCode::BAD_REQUEST,
                ));
            }

            // Step 3: compute
            let mut cents_map = [0i128; 8];
            for (i, &(key, _d, c)) in parsed.iter().enumerate() {
                debug_assert_eq!(fields[i].0, key);
                cents_map[i] = c;
            }

            let c_realized = cents_map[0];
            let c_foreign_value = cents_map[1];
            let c_foreign_cost = cents_map[2];
            let c_net_assets = cents_map[3];
            let c_capital = cents_map[4];
            let c_reserve = cents_map[5];
            let c_cash = cents_map[6];
            let c_cash_minimum = cents_map[7];

            let realized = fixed2(c_realized);
            let unrealized = fixed2(c_foreign_value.saturating_sub(c_foreign_cost));

            let term2 = c_net_assets
                .saturating_sub(c_capital)
                .saturating_sub(c_reserve);
            let term3 = c_cash.saturating_sub(c_cash_minimum);
            let withdrawable_cents = 0.max(c_realized.min(term2).min(term3));
            let withdrawable = fixed2(withdrawable_cents);

        Ok(ProfitSummary {
            realized: Money(realized),
            unrealized: Money(unrealized),
            withdrawable: Money(withdrawable),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    ComputeEndpoint::register(cfg, Arc::new(ProfitSummaryAction));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ProfitSummaryRequest::export_all(&cfg)?;
    ProfitSummary::export_all(&cfg)?;
    Ok(())
}
