//! The `kiosk_currencies` view (§7.1).

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use actix_web::web;
use chrono::SecondsFormat;
use pos_error::AppResult;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use pos_infra::transactions::Transactor;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::entities::counter_rate::CounterRate;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk as dto_kiosk;
use crate::presentation::dto::v3::common::Rate;
use crate::presentation::dto::v3::kiosk_currencies::{
    KbankComparison, KbankReference, KioskCountry, KioskCurrencies, KioskCurrency,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

#[derive(Debug, Deserialize)]
struct KbankFile {
    source: String,
    #[serde(rename = "publishedAt")]
    published_at: String,
    round: i64,
    rates: Vec<KbankFileRate>,
}

#[derive(Debug, Deserialize)]
struct KbankFileRate {
    code: String,
    label: String,
    buy: Option<Value>,
    sell: Option<Value>,
}

static KBANK_DATA: LazyLock<Option<KbankFile>> = LazyLock::new(|| {
    let text = include_str!("kiosk_currencies.kbank.json");
    serde_json::from_str(text).ok()
});

fn parse_bank_decimal(val: &Option<Value>) -> Option<Decimal> {
    match val {
        Some(Value::Number(n)) => {
            let d = Decimal::from_str(&n.to_string()).ok()?;
            if d > Decimal::ZERO {
                Some(d)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub struct KioskCurrenciesView {
    kiosk_service: KioskService,
    rate_service: RateService,
}

impl KioskCurrenciesView {
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let audit: Arc<dyn KioskAuditLog> =
            Arc::new(PgKioskAuditLog::new(PgDeviceRepository::new(pool.clone())));
        let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
        let ports = Arc::new(PgKioskPorts::new(pool.clone(), audit, source.clone()));
        let kiosk_service = KioskService::new(branch_tx, ports);

        let branch_tx_rate = BranchTx::new(Transactor::new(pool));
        let rate_service = RateService::new(branch_tx_rate, source);

        Self {
            kiosk_service,
            rate_service,
        }
    }
}

impl BranchView for KioskCurrenciesView {
    const NAME: &'static str = "kiosk_currencies";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "manage");
    type Data = KioskCurrencies;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let kiosk_service = self.kiosk_service.clone();
        let rate_service = self.rate_service.clone();
        let branch = branch.clone();

        async move {
            let switches = kiosk_service.manage(&branch, &KioskActor::Customer).await?;
            let rates = rate_service.current(&branch).await?;

            let switch_map: HashMap<String, bool> = switches
                .into_iter()
                .map(|s| (s.code.as_str().to_string(), s.enabled))
                .collect();

            let rate_map: HashMap<String, CounterRate> = rates
                .into_iter()
                .map(|r| (r.code.as_str().to_string(), r))
                .collect();

            let currencies_table = dto_kiosk::currency_table();

            let kbank_ref = KBANK_DATA.as_ref().map(|kf| KbankReference {
                source: kf.source.clone(),
                published_at: kf.published_at.clone(),
                round: kf.round,
            });

            let mut currencies = Vec::with_capacity(currencies_table.len());

            for entry in currencies_table {
                let rate_opt = rate_map.get(&entry.code);
                let enabled = switch_map.get(&entry.code).copied().unwrap_or(false);

                let countries: Vec<KioskCountry> =
                    serde_json::from_value(entry.countries.clone()).unwrap_or_default();

                let buy = rate_opt
                    .and_then(|r| r.buy)
                    .map(|b| Rate(b.to_string()));

                let sell = rate_opt
                    .and_then(|r| r.sell)
                    .map(|s| Rate(s.to_string()));

                let updated = rate_opt
                    .and_then(|r| r.updated_at)
                    .map_or_else(String::new, |t| t.to_rfc3339_opts(SecondsFormat::Millis, true));

                let partner_name = rate_opt.map_or_else(String::new, |r| r.partner_name.clone());

                let partner_cost = rate_opt
                    .and_then(|r| r.partner_cost)
                    .map(|c| Rate(c.to_string()));

                let shop_buy_dec = rate_opt
                    .and_then(|r| r.buy)
                    .map(|b| b.value())
                    .filter(|d| *d > Decimal::ZERO);

                let shop_sell_dec = rate_opt
                    .and_then(|r| r.sell)
                    .map(|s| s.value())
                    .filter(|d| *d > Decimal::ZERO);

                let shop_buy_str = shop_buy_dec.map(|d| format!("{d:.5}"));
                let shop_sell_str = shop_sell_dec.map(|d| format!("{d:.5}"));

                let kbank_comparisons = match &*KBANK_DATA {
                    Some(kf) => {
                        let matching_rates: Vec<&KbankFileRate> =
                            kf.rates.iter().filter(|r| r.code == entry.code).collect();

                        if matching_rates.is_empty() {
                            vec![KbankComparison {
                                label: entry.code.clone(),
                                bank_buy: None,
                                shop_buy: shop_buy_str.clone(),
                                buy_difference: None,
                                bank_sell: None,
                                shop_sell: shop_sell_str.clone(),
                                sell_difference: None,
                            }]
                        } else {
                            matching_rates
                                .into_iter()
                                .map(|note| {
                                    let (buy_difference, bank_buy) =
                                        match (parse_bank_decimal(&note.buy), shop_buy_dec) {
                                            (Some(bank_d), Some(shop_d)) => {
                                                let mut diff = shop_d - bank_d;
                                                diff = diff.round_dp_with_strategy(
                                                    5,
                                                    RoundingStrategy::MidpointAwayFromZero,
                                                );
                                                (
                                                    Some(format!("{diff:.5}")),
                                                    Some(format!("{bank_d:.5}")),
                                                )
                                            }
                                            (Some(bank_d), None) => {
                                                (None, Some(format!("{bank_d:.5}")))
                                            }
                                            (None, _) => (None, None),
                                        };

                                    let (sell_difference, bank_sell) =
                                        match (parse_bank_decimal(&note.sell), shop_sell_dec) {
                                            (Some(bank_d), Some(shop_d)) => {
                                                let mut diff = shop_d - bank_d;
                                                diff = diff.round_dp_with_strategy(
                                                    5,
                                                    RoundingStrategy::MidpointAwayFromZero,
                                                );
                                                (
                                                    Some(format!("{diff:.5}")),
                                                    Some(format!("{bank_d:.5}")),
                                                )
                                            }
                                            (Some(bank_d), None) => {
                                                (None, Some(format!("{bank_d:.5}")))
                                            }
                                            (None, _) => (None, None),
                                        };

                                    KbankComparison {
                                        label: note.label.clone(),
                                        bank_buy,
                                        shop_buy: shop_buy_str.clone(),
                                        buy_difference,
                                        bank_sell,
                                        shop_sell: shop_sell_str.clone(),
                                        sell_difference,
                                    }
                                })
                                .collect()
                        }
                    }
                    None => vec![KbankComparison {
                        label: entry.code.clone(),
                        bank_buy: None,
                        shop_buy: shop_buy_str.clone(),
                        buy_difference: None,
                        bank_sell: None,
                        shop_sell: shop_sell_str.clone(),
                        sell_difference: None,
                    }],
                };

                currencies.push(KioskCurrency {
                    code: entry.code.clone(),
                    name: entry.name.clone(),
                    name_en: entry.name_en.clone(),
                    countries,
                    buy,
                    sell,
                    updated,
                    partner_name,
                    partner_cost,
                    enabled,
                    kbank: kbank_comparisons,
                });
            }

            Ok(KioskCurrencies {
                currencies,
                kbank: kbank_ref,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(KioskCurrenciesView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    KioskCountry::export_all(&cfg)?;
    KbankReference::export_all(&cfg)?;
    KbankComparison::export_all(&cfg)?;
    KioskCurrency::export_all(&cfg)?;
    KioskCurrencies::export_all(&cfg)?;
    Ok(())
}
