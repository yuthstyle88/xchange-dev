//! The `admin_rates` view (§7, §12.1).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_rates::{AdminRateRow, AdminRatesData};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

fn format_decimal_4(dec: Decimal) -> String {
    let rounded =
        dec.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    format!("{rounded:.4}")
}

pub struct AdminRatesView {
    rates: RateService,
}

impl AdminRatesView {
    pub fn new(rates: RateService) -> Self {
        Self { rates }
    }
}

impl BranchView for AdminRatesView {
    const NAME: &'static str = "admin_rates";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = true;
    type Data = AdminRatesData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let rates_svc = self.rates.clone();
        let branch = branch.clone();
        async move {
            let rates = rates_svc.current(&branch).await?;
            let history = rates_svc.history(&branch).await?;
            let updated = history
                .first()
                .map(|c| c.updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .unwrap_or_default();

            let rows = rates
                .into_iter()
                .filter(|r| !r.code.is_thb())
                .map(|r| {
                    let buy_dec = r.buy.as_ref().map(|b| b.value());
                    let sell_dec = r.sell.as_ref().map(|s| s.value());
                    let partner_cost_dec = r.partner_cost.as_ref().map(|c| c.value());

                    let buy = buy_dec.map(format_decimal_4);
                    let sell = sell_dec.map(format_decimal_4);
                    let spread = match (sell_dec, buy_dec) {
                        (Some(s), Some(b)) => Some(format_decimal_4(s - b)),
                        _ => None,
                    };
                    let partner_cost = partner_cost_dec.map(format_decimal_4);
                    let partner_spread = match (sell_dec, partner_cost_dec) {
                        (Some(s), Some(c)) => Some(format_decimal_4(s - c)),
                        _ => None,
                    };

                    AdminRateRow {
                        code: r.code.as_str().to_string(),
                        name: r.name,
                        symbol: r.symbol.unwrap_or_default(),
                        buy,
                        sell,
                        spread,
                        updated: r
                            .updated_at
                            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                            .unwrap_or_default(),
                        partner_name: r.partner_name,
                        partner_cost,
                        partner_spread,
                    }
                })
                .collect();

            Ok(AdminRatesData {
                updated,
                rates: rows,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
    let rates = RateService::new(branch_tx, source);
    ViewEndpoint::register(cfg, Arc::new(AdminRatesView::new(rates)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminRateRow::export_all(&cfg)?;
    AdminRatesData::export_all(&cfg)?;
    Ok(())
}
