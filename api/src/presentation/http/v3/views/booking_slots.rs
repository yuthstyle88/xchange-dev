//! Public `booking_slots` view (BK-6, #1388).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::booking_slots::{
    BookingSlotItemDto, BookingSlotsData, BookingSlotsFilter,
};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct BookingSlotsView {
    pool: PgPool,
}

impl BookingSlotsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn load_slots(
        pool: &PgPool,
        branch: &BranchId,
        filter: &BookingSlotsFilter,
    ) -> AppResult<BookingSlotsData> {
        let date = match &filter.date {
            Some(d) => NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .unwrap_or_else(|_| Utc::now().with_timezone(&chrono_tz::Asia::Bangkok).date_naive()),
            None => Utc::now().with_timezone(&chrono_tz::Asia::Bangkok).date_naive(),
        };

        type SlotRow = (uuid::Uuid, DateTime<Utc>, DateTime<Utc>, i32, i32);
        let rows: Vec<SlotRow> = sqlx::query_as(
            "SELECT id, start_at, end_at, capacity, reserved \
             FROM booking_slots \
             WHERE branch_id = $1 AND service_date = $2 AND status = 'OPEN' \
             ORDER BY start_at ASC",
        )
        .bind(branch.as_str())
        .bind(date)
        .fetch_all(pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        let slots = rows
            .into_iter()
            .map(|(id, start_at, end_at, capacity, reserved)| BookingSlotItemDto {
                id: id.to_string(),
                start: start_at.to_rfc3339(),
                end: end_at.to_rfc3339(),
                available: (capacity - reserved).max(0),
            })
            .collect();

        Ok(BookingSlotsData {
            as_of: Utc::now().to_rfc3339(),
            slots,
        })
    }
}

impl BranchView for BookingSlotsView {
    const NAME: &'static str = "booking_slots";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "booking_slots");
    const VERSIONED: bool = true;
    type Data = BookingSlotsData;
    type Filter = BookingSlotsFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let filter = filter.clone();
        async move { Self::load_slots(&pool, &branch, &filter).await }
    }

    fn load_as(
        &self,
        branch: &BranchId,
        _caller: &Caller,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let filter = filter.clone();
        async move { Self::load_slots(&pool, &branch, &filter).await }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(BookingSlotsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BookingSlotsData::export_all(&cfg)?;
    BookingSlotsFilter::export_all(&cfg)?;
    Ok(())
}
