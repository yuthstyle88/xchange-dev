//! The `admin_bookings` and `admin_booking_settings` views (§7, §12.1, §12.14, §12.17).

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    CompatRate, CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Rate, Side, UnitSize,
};
use xc_rate::domain::publication::freshness::Freshness;

use crate::application::bookings::booking_rules_service::BookingRulesService;
use crate::bootstrap::AppState;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::v3::admin_bookings::{
    AdminBookingCounterRate, AdminBookingFilter, AdminBookingRow, AdminBookingRules,
    AdminBookingSettingsData,
};
use crate::presentation::dto::v3::common::{NoFilter, RateFreshnessDto};
use crate::presentation::http::v3::view::{BranchView, PagedView, RowId, ViewEndpoint};

struct AdminBookingsPagedView {
    db: PgPool,
}

impl AdminBookingsPagedView {
    fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl PagedView for AdminBookingsPagedView {
    const NAME: &'static str = "admin_bookings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/bookings");
    type Row = AdminBookingRow;
    type Filter = AdminBookingFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let db = self.db.clone();
        let branch = branch.clone();
        let q = filter.q.clone();

        async move {
            let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
                "SELECT id, branch_id, name, phone, member_id, type, code, amount, rate, status, \
                 created_at, expires_at, transaction_ref \
                 FROM v2_bookings \
                 WHERE branch_id = ",
            );
            qb.push_bind(branch.as_str());

            if let Some(ref before_id) = before {
                qb.push(" AND (created_at, id) < (SELECT created_at, id FROM v2_bookings WHERE id = ");
                qb.push_bind(before_id);
                qb.push(" AND branch_id = ");
                qb.push_bind(branch.as_str());
                qb.push(")");
            }

            if let Some(ref query_text) = q
                && !query_text.is_empty()
            {
                qb.push(" AND (STRPOS(LOWER(id), LOWER(");
                qb.push_bind(query_text);
                qb.push(")) > 0 OR STRPOS(LOWER(name), LOWER(");
                qb.push_bind(query_text);
                qb.push(")) > 0 OR STRPOS(LOWER(phone), LOWER(");
                qb.push_bind(query_text);
                qb.push(")) > 0)");
            }

            qb.push(" ORDER BY created_at DESC, id DESC LIMIT ");
            qb.push_bind(limit as i64);

            let query = qb.build();
            let db_rows = query.fetch_all(&db).await.map_err(AppError::Database)?;

            let line_rows: Vec<(String, Option<Decimal>, Option<Decimal>, Decimal)> = sqlx::query_as(
                "SELECT l.currency_code, l.buy_rate, l.sell_rate, l.unit_size \
                 FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 JOIN published_rate_lines l ON l.publication_id = p.id \
                 WHERE b.id = $1 AND l.denomination_group = 'ALL' AND l.tier_lower_exclusive_thb IS NULL",
            )
            .bind(branch.as_str())
            .fetch_all(&db)
            .await
            .map_err(AppError::Database)?;

            let mut lines_map: HashMap<String, (Option<Rate>, Option<Rate>, UnitSize)> =
                HashMap::new();
            for (code, buy_dec, sell_dec, unit_dec) in line_rows {
                let unit_size = UnitSize::from_decimal(unit_dec)
                    .map_err(|e| AppError::Internal(format!("invalid unit_size: {e}")))?;
                let buy_rate = buy_dec
                    .map(|d| Rate::parse(&d.normalize().to_string()))
                    .transpose()
                    .map_err(|e| AppError::Internal(format!("invalid buy_rate: {e}")))?;
                let sell_rate = sell_dec
                    .map(|d| Rate::parse(&d.normalize().to_string()))
                    .transpose()
                    .map_err(|e| AppError::Internal(format!("invalid sell_rate: {e}")))?;
                lines_map.insert(code.trim().to_string(), (buy_rate, sell_rate, unit_size));
            }

            let now = Utc::now();
            let mut out = Vec::with_capacity(db_rows.len());

            for row in db_rows {
                let id: String = row.try_get("id").map_err(AppError::Database)?;
                let branch_id: String = row.try_get("branch_id").map_err(AppError::Database)?;
                let name: String = row.try_get("name").map_err(AppError::Database)?;
                let phone: String = row.try_get("phone").map_err(AppError::Database)?;
                let member_id: Option<String> = row.try_get("member_id").map_err(AppError::Database)?;
                let member = member_id.is_some();
                let side_str: String = row.try_get("type").map_err(AppError::Database)?;
                let code_raw: String = row.try_get("code").map_err(AppError::Database)?;
                let currency = code_raw.trim().to_string();
                let amount: Decimal = row.try_get("amount").map_err(AppError::Database)?;
                let rate: Decimal = row.try_get("rate").map_err(AppError::Database)?;
                let status_raw: String = row.try_get("status").map_err(AppError::Database)?;
                let created_at: DateTime<Utc> = row.try_get("created_at").map_err(AppError::Database)?;
                let expires_at: Option<DateTime<Utc>> = row.try_get("expires_at").map_err(AppError::Database)?;
                let transaction_ref: Option<String> = row.try_get("transaction_ref").map_err(AppError::Database)?;

                let status = if let Some(st) = BookingStatus::parse(&status_raw) {
                    let booking = Booking {
                        id: String::new(),
                        branch_id: BranchId::new("MAIN"),
                        name: String::new(),
                        phone: String::new(),
                        member_id: None,
                        side: Side::Buy,
                        currency: CurrencyCode::THB,
                        amount: Money::ZERO,
                        rate: Rate::parse("1").unwrap(),
                        unit_size: UnitSize::ONE,
                        published_rate_line_id: None,
                        status: st,
                        created_at,
                        created_by: String::new(),
                        expires_at,
                        confirmed_at: None,
                        confirmed_by: None,
                        submission_id: None,
                        transaction_ref: None,
                    };
                    booking.shown_status(now).to_string()
                } else {
                    status_raw.clone()
                };

                let (counter_rate, counter_principal) = if status_raw == "PENDING" {
                    if let Some(&(buy_opt, sell_opt, unit_size)) = lines_map.get(&currency) {
                        match side_str.as_str() {
                            "BUY" => {
                                if let Some(buy_rate) = buy_opt {
                                    let per_one = CompatRate::per_one(buy_rate, unit_size)
                                        .map_err(|e| AppError::Internal(format!("invalid rate: {e}")))?;
                                    let cr = format_per_one_rate(per_one);
                                    let cp = ExchangeArithmetic::thb(
                                        ForeignAmount::from_decimal(amount),
                                        buy_rate,
                                        unit_size,
                                    )
                                    .to_string();
                                    (Some(cr), Some(cp))
                                } else {
                                    (None, None)
                                }
                            }
                            "SELL" => {
                                if let Some(sell_rate) = sell_opt {
                                    let per_one = CompatRate::per_one(sell_rate, unit_size)
                                        .map_err(|e| AppError::Internal(format!("invalid rate: {e}")))?;
                                    let cr = format_per_one_rate(per_one);
                                    let cp = ExchangeArithmetic::thb(
                                        ForeignAmount::from_decimal(amount),
                                        sell_rate,
                                        unit_size,
                                    )
                                    .to_string();
                                    (Some(cr), Some(cp))
                                } else {
                                    (None, None)
                                }
                            }
                            _ => (None, None),
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                out.push(AdminBookingRow {
                    id,
                    branch_id,
                    name,
                    phone,
                    member,
                    r#type: side_str,
                    currency,
                    amount: format!("{amount:.2}"),
                    rate: format!("{rate:.4}"),
                    status,
                    created_at: created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    expires_at: expires_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                    transaction_ref,
                    counter_rate,
                    counter_principal,
                });
            }

            Ok(out)
        }
    }
}

struct AdminBookingSettingsBranchView {
    db: PgPool,
    rules_service: BookingRulesService,
}

impl AdminBookingSettingsBranchView {
    fn new(db: PgPool, rules_service: BookingRulesService) -> Self {
        Self { db, rules_service }
    }
}

impl BranchView for AdminBookingSettingsBranchView {
    const NAME: &'static str = "admin_booking_settings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/bookings");
    const VERSIONED: bool = true;
    type Data = AdminBookingSettingsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let db = self.db.clone();
        let branch = branch.clone();
        let rules_service = self.rules_service.clone();

        async move {
            let rules_opt = rules_service.rules(&branch).await?;
            let rules = match rules_opt {
                Some(r) => AdminBookingRules {
                    enabled: r.enabled,
                    minutes: r.minutes as u32,
                },
                None => AdminBookingRules {
                    enabled: true,
                    minutes: 30,
                },
            };

            let member_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE branch_id = $1")
                    .bind(branch.as_str())
                    .fetch_one(&db)
                    .await
                    .map_err(AppError::Database)?;

            let pub_row = sqlx::query(
                "SELECT p.id, p.published_at \
                 FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 WHERE b.id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&db)
            .await
            .map_err(AppError::Database)?;

            let (freshness, counter_rates) = match pub_row {
                Some(r) => {
                    let pub_id: Uuid = r.get(0);
                    let published_at: DateTime<Utc> = r.get(1);

                    let raw_lines = sqlx::query(
                        "SELECT currency_code, denomination_group, unit_size, buy_rate, sell_rate, source_snapshot \
                         FROM published_rate_lines WHERE publication_id = $1 AND tier_lower_exclusive_thb IS NULL \
                         ORDER BY currency_code ASC",
                    )
                    .bind(pub_id)
                    .fetch_all(&db)
                    .await
                    .map_err(AppError::Database)?;

                    let now = Utc::now();
                    let mut freshnesses = Vec::with_capacity(raw_lines.len() * 2);
                    let mut counter_rates = Vec::new();

                    for row in raw_lines {
                        let code: String = row.get(0);
                        let denomination_group: String = row.get(1);
                        let unit_dec: Decimal = row.get(2);
                        let buy_dec: Option<Decimal> = row.get(3);
                        let sell_dec: Option<Decimal> = row.get(4);
                        let source_snapshot: Value = row.get(5);

                        freshnesses.push(Freshness::of_side(&source_snapshot, Side::Buy, now));
                        freshnesses.push(Freshness::of_side(&source_snapshot, Side::Sell, now));

                        if denomination_group == "ALL" {
                            let unit_size = UnitSize::from_decimal(unit_dec)
                                .map_err(|e| AppError::Internal(format!("invalid unit_size: {e}")))?;
                            let buy = buy_dec
                                .map(|d| Rate::parse(&d.normalize().to_string()))
                                .transpose()
                                .map_err(|e| AppError::Internal(format!("invalid buy_rate: {e}")))?;
                            let sell = sell_dec
                                .map(|d| Rate::parse(&d.normalize().to_string()))
                                .transpose()
                                .map_err(|e| AppError::Internal(format!("invalid sell_rate: {e}")))?;

                            let buy_str = match buy {
                                Some(b) => {
                                    let per_one = CompatRate::per_one(b, unit_size)
                                        .map_err(|e| AppError::Internal(format!("invalid rate: {e}")))?;
                                    format_per_one_rate(per_one)
                                }
                                None => String::new(),
                            };
                            let sell_str = match sell {
                                Some(s) => {
                                    let per_one = CompatRate::per_one(s, unit_size)
                                        .map_err(|e| AppError::Internal(format!("invalid rate: {e}")))?;
                                    format_per_one_rate(per_one)
                                }
                                None => String::new(),
                            };

                            counter_rates.push(AdminBookingCounterRate {
                                currency: code.trim().to_string(),
                                buy: buy_str,
                                sell: sell_str,
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

                    (freshness_dto, counter_rates)
                }
                None => (RateFreshnessDto::default(), Vec::new()),
            };

            Ok(AdminBookingSettingsData {
                rules,
                member_count: member_count as u64,
                public_url: String::new(),
                counter_rates,
                freshness,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let paged_view = AdminBookingsPagedView::new(state.db.clone());
    ViewEndpoint::register_paged(cfg, Arc::new(paged_view));

    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let store = BranchSettingStore::new(state.db.clone());
    let rules_service = BookingRulesService::new(branch_tx, store);
    let settings_view = AdminBookingSettingsBranchView::new(state.db.clone(), rules_service);
    ViewEndpoint::register(cfg, Arc::new(settings_view));
}

fn format_per_one_rate(dec: Decimal) -> String {
    let norm = dec.normalize();
    if norm.scale() <= 4 {
        format!("{norm:.4}")
    } else {
        norm.to_string()
    }
}

#[cfg(test)]
pub(crate) fn counter_principal(amount: Decimal, rate: Decimal) -> String {
    let cp = (amount * rate).round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    format!("{cp:.2}")
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::admin_bookings::export_types(dir)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rust_decimal::Decimal;

    use super::{counter_principal, format_per_one_rate};

    #[test]
    fn format_per_one_rate_formats_at_least_four_decimals() {
        assert_eq!(format_per_one_rate(Decimal::from_str("36.25").unwrap()), "36.2500");
        assert_eq!(format_per_one_rate(Decimal::from_str("0.231").unwrap()), "0.2310");
        assert_eq!(format_per_one_rate(Decimal::from_str("37").unwrap()), "37.0000");
        assert_eq!(format_per_one_rate(Decimal::from_str("0.02512345678").unwrap()), "0.02512345678");
    }

    #[test]
    fn counter_principal_rounds_half_away_from_zero() {
        // Non-exact product: truncation would produce "12085.44", while rounding gives "12085.45"
        // 333.33 * 36.2567 = 12085.445811
        let a1 = Decimal::from_str("333.33").unwrap();
        let r1 = Decimal::from_str("36.2567").unwrap();
        assert_eq!(counter_principal(a1, r1), "12085.45");

        // True midpoint where half-away-from-zero differs from half-even:
        // 2.00 * 1.0125 = 2.025
        // Half-away-from-zero rounds to 2.03; half-even (banker's rounding) rounds to 2.02 because 2 is even.
        let a2 = Decimal::from_str("2.00").unwrap();
        let r2 = Decimal::from_str("1.0125").unwrap();
        assert_eq!(counter_principal(a2, r2), "2.03");

        // Another midpoint: 1.00 * 1.0050 = 1.005 -> half-away: 1.01, half-even: 1.00
        let a3 = Decimal::from_str("1.00").unwrap();
        let r3 = Decimal::from_str("1.0050").unwrap();
        assert_eq!(counter_principal(a3, r3), "1.01");
    }
}
