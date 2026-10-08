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
use rust_decimal::RoundingStrategy::MidpointAwayFromZero;
use sqlx::{PgPool, Row};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side};

use crate::application::bookings::booking_rules_service::BookingRulesService;
use crate::bootstrap::AppState;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::v3::admin_bookings::{
    AdminBookingCounterRate, AdminBookingFilter, AdminBookingRow, AdminBookingRules,
    AdminBookingSettingsData,
};
use crate::presentation::dto::v3::common::NoFilter;
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
                 FROM bookings \
                 WHERE branch_id = ",
            );
            qb.push_bind(branch.as_str());

            if let Some(ref before_id) = before {
                qb.push(" AND (created_at, id) < (SELECT created_at, id FROM bookings WHERE id = ");
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

            let rate_rows: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
                "SELECT DISTINCT ON (code) code, buy, sell FROM rates WHERE branch_id = $1 ORDER BY code ASC, id DESC",
            )
            .bind(branch.as_str())
            .fetch_all(&db)
            .await
            .map_err(AppError::Database)?;

            let rates_map: HashMap<String, (Decimal, Decimal)> = rate_rows
                .into_iter()
                .map(|(code, buy, sell)| (code.trim().to_string(), (buy, sell)))
                .collect();

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
                    if let Some(&(buy, sell)) = rates_map.get(&currency) {
                        let cr = match side_str.as_str() {
                            "BUY" => buy,
                            "SELL" => sell,
                            _ => buy,
                        };
                        let cp = counter_principal(amount, cr);
                        (Some(format!("{cr:.4}")), Some(cp))
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

            let rate_rows: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
                "SELECT DISTINCT ON (code) code, buy, sell FROM rates WHERE branch_id = $1 ORDER BY code ASC, id DESC",
            )
            .bind(branch.as_str())
            .fetch_all(&db)
            .await
            .map_err(AppError::Database)?;

            let counter_rates = rate_rows
                .into_iter()
                .map(|(code, buy, sell)| AdminBookingCounterRate {
                    currency: code.trim().to_string(),
                    buy: format!("{buy:.4}"),
                    sell: format!("{sell:.4}"),
                })
                .collect();

            Ok(AdminBookingSettingsData {
                rules,
                member_count: member_count as u64,
                public_url: String::new(),
                counter_rates,
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

pub(crate) fn counter_principal(amount: Decimal, rate: Decimal) -> String {
    let cp = (amount * rate).round_dp_with_strategy(2, MidpointAwayFromZero);
    format!("{cp:.2}")
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::admin_bookings::export_types(dir)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rust_decimal::Decimal;

    use super::counter_principal;

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
