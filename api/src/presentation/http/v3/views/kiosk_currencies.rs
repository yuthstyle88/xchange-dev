//! The `kiosk_currencies` view (§7.1).

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_rate::domain::publication::freshness::Freshness;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk as dto_kiosk;
use crate::presentation::dto::v3::common::{NoFilter, Rate, RateFreshnessDto};
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
        Some(Value::String(s)) => {
            let d = Decimal::from_str(s).ok()?;
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
    pool: PgPool,
    kiosk_service: KioskService,
}

impl KioskCurrenciesView {
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let audit: Arc<dyn KioskAuditLog> =
            Arc::new(PgKioskAuditLog::new(PgDeviceRepository::new(pool.clone())));
        let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
        let ports = Arc::new(PgKioskPorts::new(pool.clone(), audit, source.clone()));
        let kiosk_service = KioskService::new(branch_tx, ports);

        Self {
            pool,
            kiosk_service,
        }
    }
}

#[derive(sqlx::FromRow)]
struct PublishedLineRow {
    currency_code: String,
    unit_size: Decimal,
    buy_rate: Option<Decimal>,
    sell_rate: Option<Decimal>,
    published_at: DateTime<Utc>,
    source_snapshot: Value,
}

impl BranchView for KioskCurrenciesView {
    const NAME: &'static str = "kiosk_currencies";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "manage");
    type Filter = NoFilter;
    type Data = KioskCurrencies;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let kiosk_service = self.kiosk_service.clone();
        let pool = self.pool.clone();
        let branch = branch.clone();

        async move {
            let switches = kiosk_service.manage(&branch, &KioskActor::Customer).await?;

            let switch_map: HashMap<String, bool> = switches
                .into_iter()
                .map(|s| (s.code.as_str().to_string(), s.enabled))
                .collect();

            let register_units: HashMap<String, Decimal> = sqlx::query_as::<_, (String, Decimal)>(
                "SELECT trim(code) AS code, unit_size FROM currencies",
            )
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?
            .into_iter()
            .collect();

            let pub_row = sqlx::query(
                "SELECT rp.id, rp.published_at \
                 FROM branches b \
                 JOIN rate_publications rp ON rp.id = b.current_publication_id \
                 WHERE b.id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let (freshness, lines) = match pub_row {
                Some(r) => {
                    let pub_id: Uuid = r.get(0);
                    let published_at: DateTime<Utc> = r.get(1);

                    let raw_lines = sqlx::query(
                        "SELECT trim(currency_code), denomination_group, unit_size, buy_rate, sell_rate, source_snapshot \
                         FROM published_rate_lines WHERE publication_id = $1 AND tier_lower_exclusive_thb IS NULL",
                    )
                    .bind(pub_id)
                    .fetch_all(&pool)
                    .await
                    .map_err(pos_error::AppError::Database)?;

                    let now = Utc::now();
                    let mut freshnesses = Vec::with_capacity(raw_lines.len() * 2);
                    let mut lines = Vec::new();

                    for row in raw_lines {
                        let currency_code: String = row.get(0);
                        let denomination_group: String = row.get(1);
                        let unit_size: Decimal = row.get(2);
                        let buy_rate: Option<Decimal> = row.get(3);
                        let sell_rate: Option<Decimal> = row.get(4);
                        let source_snapshot: Value = row.get(5);

                        freshnesses.push(Freshness::of_side(&source_snapshot, Side::Buy, now));
                        freshnesses.push(Freshness::of_side(&source_snapshot, Side::Sell, now));

                        if denomination_group == "ALL" {
                            lines.push(PublishedLineRow {
                                currency_code,
                                unit_size,
                                buy_rate,
                                sell_rate,
                                published_at,
                                source_snapshot,
                            });
                        }
                    }

                    let pub_freshness = Freshness::of_publication(freshnesses);
                    let freshness_dto = RateFreshnessDto {
                        stale: pub_freshness.stale,
                        source_date: pub_freshness
                            .source_date
                            .map(|d| d.format("%Y-%m-%d").to_string()),
                        published_at: Some(published_at.to_rfc3339()),
                    };

                    (freshness_dto, lines)
                }
                None => (
                    RateFreshnessDto {
                        stale: false,
                        source_date: None,
                        published_at: None,
                    },
                    Vec::new(),
                ),
            };

            let line_map: HashMap<String, PublishedLineRow> = lines
                .into_iter()
                .map(|l| (l.currency_code.clone(), l))
                .collect();

            let currencies_table = dto_kiosk::currency_table();

            let kbank_ref = KBANK_DATA.as_ref().map(|kf| KbankReference {
                source: kf.source.clone(),
                published_at: kf.published_at.clone(),
                round: kf.round,
            });

            let mut currencies = Vec::with_capacity(currencies_table.len());

            for entry in currencies_table {
                let line = line_map.get(&entry.code);
                let enabled = switch_map.get(&entry.code).copied().unwrap_or(false);

                let countries: Vec<KioskCountry> =
                    serde_json::from_value(entry.countries.clone()).unwrap_or_default();

                let (
                    buy,
                    sell,
                    unit_size,
                    updated,
                    partner_name,
                    partner_cost,
                    shop_buy_dec,
                    shop_sell_dec,
                ) = if let Some(l) = line {
                    let buy = l.buy_rate.map(|b| Rate(format!("{:.8}", b)));
                    let sell = l.sell_rate.map(|s| Rate(format!("{:.8}", s)));
                    let unit_size = l.unit_size.normalize().to_string();
                    let updated = l.published_at.to_rfc3339_opts(SecondsFormat::Millis, true);

                    let partner = l.source_snapshot.get("partner");
                    let (partner_name, partner_cost) = match partner {
                        Some(Value::Object(p)) => {
                            let name =
                                p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let cost =
                                p.get("cost").and_then(|v| v.as_str()).map(|c| Rate(c.to_string()));
                            (name, cost)
                        }
                        _ => (String::new(), None),
                    };

                    let shop_buy_dec =
                        l.buy_rate.filter(|b| *b > Decimal::ZERO).map(|b| b / l.unit_size);
                    let shop_sell_dec =
                        l.sell_rate.filter(|s| *s > Decimal::ZERO).map(|s| s / l.unit_size);

                    (
                        buy,
                        sell,
                        unit_size,
                        updated,
                        partner_name,
                        partner_cost,
                        shop_buy_dec,
                        shop_sell_dec,
                    )
                } else {
                    let reg_unit = register_units
                        .get(&entry.code)
                        .copied()
                        .ok_or_else(|| {
                            pos_error::AppError::Internal(format!(
                                "missing currency unit_size for {}",
                                entry.code
                            ))
                        })?;
                    let unit_size = reg_unit.normalize().to_string();
                    (
                        None,
                        None,
                        unit_size,
                        String::new(),
                        String::new(),
                        None,
                        None,
                        None,
                    )
                };

                let shop_buy_str = shop_buy_dec.map(|d| {
                    let rounded =
                        d.round_dp_with_strategy(5, RoundingStrategy::MidpointAwayFromZero);
                    format!("{rounded:.5}")
                });
                let shop_sell_str = shop_sell_dec.map(|d| {
                    let rounded =
                        d.round_dp_with_strategy(5, RoundingStrategy::MidpointAwayFromZero);
                    format!("{rounded:.5}")
                });

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
                                                let diff = (shop_d - bank_d).round_dp_with_strategy(
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
                                                let diff = (shop_d - bank_d).round_dp_with_strategy(
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
                    unit_size,
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
                freshness,
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
